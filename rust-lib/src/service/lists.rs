//! Epoch active sets, kept in their own file (`uplink_lists.json`) rather than in the saved
//! state: a set is one entry per active node, so at tens of thousands of nodes each is a few
//! MB, and the saved state is rewritten on every operation. This file changes only when a set
//! is learned or dropped.
//!
//! It is a cache: everything in it can be learned again from the indexer, so a file that
//! doesn't parse just starts empty.

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use referral_core::{NodeId, active_digest};
use serde::{Deserialize, Serialize};

use super::Service;
use crate::domain::{DomainError, Result};
use crate::ports::Store;

/// One epoch's active set: its digest (hex) and its nodes, 32 bytes each, in order, base64.
/// About half the size of a JSON list of hex ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSet {
    pub digest: String,
    pub nodes: String,
}

impl StoredSet {
    pub fn new(active: &BTreeSet<NodeId>) -> Self {
        let raw: Vec<u8> = active.iter().flat_map(|node| node.to_bytes()).collect();
        Self { digest: hex::encode(active_digest(active)), nodes: STANDARD.encode(raw) }
    }

    pub fn digest(&self) -> Option<[u8; 32]> {
        hex::decode(&self.digest).ok()?.try_into().ok()
    }

    pub fn nodes(&self) -> Option<BTreeSet<NodeId>> {
        let raw = STANDARD.decode(&self.nodes).ok()?;
        if raw.len() % 32 != 0 {
            return None;
        }
        Some(raw.chunks_exact(32).map(|id| NodeId::new(id.try_into().expect("32 bytes"))).collect())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentLists {
    /// By epoch.
    #[serde(default)]
    pub sets: BTreeMap<u32, StoredSet>,
    /// How far the indexer's history of the registry has been read.
    #[serde(default)]
    pub history_offset: u64,
}

/// Per program account (hex).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lists {
    pub deployments: BTreeMap<String, DeploymentLists>,
}

impl Lists {
    pub fn load(store: &dyn Store) -> Self {
        store.load().ok().flatten().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
    }
}

/// Moves active sets saved by earlier versions into `lists`. `true` when there were any.
/// The indexer is read again from the start, so the offsets it finds can be remembered.
pub fn migrate(saved: &mut super::saved::Saved, lists: &mut Lists) -> bool {
    let mut moved = false;
    for (program, deployment) in &mut saved.deployments {
        let old = std::mem::take(&mut deployment.publications);
        deployment.history_offset = 0;
        if old.is_empty() {
            continue;
        }
        let target = lists.deployments.entry(program.clone()).or_default();
        for (key, nodes) in old {
            let Some(epoch) = key.split(':').next().and_then(|e| e.parse().ok()) else { continue };
            let Ok(active) = nodes.iter().map(|n| crate::domain::codec::node(n)).collect::<Result<BTreeSet<_>>>() else { continue };
            target.sets.entry(epoch).or_insert_with(|| StoredSet::new(&active));
        }
        moved = true;
    }
    moved
}

impl Service {
    pub(super) fn lists(&self) -> Option<&DeploymentLists> {
        self.lists.deployments.get(&self.program_hex()?)
    }

    pub(super) fn lists_mut(&mut self) -> Result<&mut DeploymentLists> {
        let program = crate::domain::codec::account_hex(&self.program()?);
        Ok(self.lists.deployments.entry(program).or_default())
    }

    pub(super) fn persist_lists(&self) -> Result<()> {
        let json = serde_json::to_string(&self.lists).expect("lists serialize");
        self.lists_store.save(&json).map_err(|e| DomainError::new("store", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_round_trips_at_half_the_size_of_hex() {
        let active: BTreeSet<NodeId> = (0..100u8).map(|i| NodeId::new([i; 32])).collect();
        let stored = StoredSet::new(&active);
        assert_eq!(stored.nodes().unwrap(), active);
        assert_eq!(stored.digest().unwrap(), active_digest(&active));
        let hex_json = serde_json::to_string(&active.iter().map(|n| hex::encode(n.to_bytes())).collect::<Vec<_>>()).unwrap();
        assert!(stored.nodes.len() * 10 < hex_json.len() * 7, "{} vs {}", stored.nodes.len(), hex_json.len());
    }
}
