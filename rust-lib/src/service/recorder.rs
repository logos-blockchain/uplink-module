//! The recorder: saves each published epoch's active set while the module is loaded, so
//! epochs the user doesn't claim while they're current can still be claimed later without
//! an indexer. The archive on chain keeps only each set's digest, and the registry only the
//! latest set, so a set nobody saved while it was current needs the indexer.
//!
//! It runs on its own thread (see `glue`), next to the service but never touching it: it
//! reads the deployment from the service's saved state, and writes only its own file
//! (`uplink_epochs.json`), which the service takes in when it next reads the registry.
//! One writer per file, so neither can lose the other's writes.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use referral_core::active_digest;
use serde::{Deserialize, Serialize};

use super::deployment::configured_program;
use super::registry::registry_at;
use super::saved::Saved;
use super::Service;
use crate::domain::{DomainError, Result, codec};
use crate::ports::{Lez, Store};

/// When to look next, `since_new` after the last new epoch was seen. Each look reads the
/// registry account, a few MB on a large network, so the recorder looks often only around
/// an epoch change: every 15 s at first, slowing to a tenth of the time since, at most 10
/// minutes. Short epochs (the devnet's 40 s) are seen every 15 s; testnet's 10-hour ones
/// every 10 minutes once an hour and a half has passed.
pub fn next_look(since_new: Duration) -> Duration {
    (since_new / 10).clamp(Duration::from_secs(15), Duration::from_secs(600))
}

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
    let taken = saved.deployments.get(&program_hex).and_then(|d| d.recorded_through);
    let seen = recorded.deployments.entry(program_hex).or_default();
    // What the service has taken in is its now.
    let before = seen.len();
    seen.retain(|key, _| epoch_of(key).is_some_and(|epoch| taken.is_none_or(|t| epoch > t)));
    let forgot = seen.len() < before;
    let new = registry.epoch > taken.unwrap_or(0) && !seen.contains_key(&key);
    if new {
        seen.insert(key, registry.active_nodes.iter().map(codec::node_hex).collect());
    }
    if !new && !forgot {
        return Ok(None);
    }
    let json = serde_json::to_string(&recorded).expect("recorded serializes");
    log.save(&json).map_err(|e| DomainError::new("store", e))?;
    Ok(new.then_some(registry.epoch))
}

fn epoch_of(key: &str) -> Option<u32> {
    key.split(':').next()?.parse().ok()
}

impl Service {
    /// Takes in what the recorder saved for this deployment. `true` when anything was new.
    pub(super) fn take_recorded(&mut self) -> Result<bool> {
        let Some(log) = &self.recorded else { return Ok(false) };
        let Some(program) = self.program_hex() else { return Ok(false) };
        let Some(seen) = load(log.as_ref())?.deployments.remove(&program) else { return Ok(false) };
        let taken = self.deployment().recorded_through;
        let mut new = false;
        let mut through = taken;
        for (key, nodes) in seen {
            let Some(epoch) = epoch_of(&key) else { continue };
            let Ok(active) = nodes.iter().map(|n| codec::node(n)).collect::<Result<BTreeSet<_>>>() else { continue };
            new |= self.remember_publication(epoch, &active)?;
            through = through.max(Some(epoch));
        }
        if through != taken {
            // Tells the recorder it may forget them.
            self.deployment_mut()?.recorded_through = through;
            self.persist()?;
        }
        Ok(new)
    }
}
