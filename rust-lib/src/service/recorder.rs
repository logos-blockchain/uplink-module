//! The recorder: saves each published epoch's active set while the module is loaded, so
//! epochs the user doesn't claim while they're current can still be claimed later without
//! an indexer. The archive on chain keeps only each set's digest, and the registry only the
//! latest set, so a set nobody saved while it was current needs the indexer.
//!
//! It runs on its own thread (see `glue`), next to the service but never touching it: it
//! reads the deployment from the service's saved state, and writes only its own file
//! (`uplink_epochs.json`), which the service takes in when it next reads the registry.
//! One writer per file, so neither can lose the other's writes.

use std::collections::BTreeMap;

use referral_core::active_digest;
use serde::{Deserialize, Serialize};

use super::deployment::configured_program;
use super::registry::registry_at;
use super::saved::Saved;
use super::Service;
use crate::domain::{DomainError, Result, codec};
use crate::ports::{Lez, Store};

/// How often the recorder looks. An epoch is current for far longer, and each look is one
/// public account read.
pub const INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// What the recorder saw: per program account (hex), `"<epoch>:<digest hex>"` → active nodes,
/// hex, the same keys as `Deployment::publications`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    pub deployments: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

fn load(log: &dyn Store) -> Result<Recorded> {
    match log.load().map_err(|e| DomainError::new("store", e))? {
        None => Ok(Recorded::default()),
        // Only the recorder writes it, so one that doesn't parse is ours to start over.
        Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
    }
}

/// One look: saves the current epoch's active set for the deployment in use, if it's new.
/// Returns the epoch saved. Does nothing until a deployment is configured.
pub fn record_once(lez: &dyn Lez, state: &dyn Store, log: &dyn Store) -> Result<Option<u32>> {
    let saved: Saved = match state.load().map_err(|e| DomainError::new("store", e))? {
        Some(json) => serde_json::from_str(&json).map_err(|_| DomainError::new("store", "the saved state doesn't parse"))?,
        None => Saved::default(),
    };
    let Some(program_hex) = configured_program(&saved) else { return Ok(None) };
    let program = codec::account(&program_hex)?;
    let registry = registry_at(lez, &program)?;
    if registry.epoch == 0 {
        return Ok(None);
    }
    let key = Service::publication_key(registry.epoch, &active_digest(&registry.active_nodes));
    let mut recorded = load(log)?;
    let seen = recorded.deployments.entry(program_hex).or_default();
    if seen.contains_key(&key) {
        return Ok(None);
    }
    seen.insert(key, registry.active_nodes.iter().map(codec::node_hex).collect());
    let json = serde_json::to_string(&recorded).expect("recorded serializes");
    log.save(&json).map_err(|e| DomainError::new("store", e))?;
    Ok(Some(registry.epoch))
}

impl Service {
    /// Takes in what the recorder saved for this deployment. `true` when anything was new.
    pub(super) fn take_recorded(&mut self) -> Result<bool> {
        let Some(log) = &self.recorded else { return Ok(false) };
        let Some(program) = self.program_hex() else { return Ok(false) };
        let Some(seen) = load(log.as_ref())?.deployments.remove(&program) else { return Ok(false) };
        let publications = &mut self.deployment_mut()?.publications;
        let before = publications.len();
        for (key, nodes) in seen {
            publications.entry(key).or_insert(nodes);
        }
        Ok(publications.len() > before)
    }
}
