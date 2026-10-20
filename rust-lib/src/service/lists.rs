//! Epoch active sets, kept in their own file (`uplink_lists.json`) rather than in the saved
//! state: a set is one entry per active node, so at tens of thousands of nodes each is a few
//! MB, and the saved state is rewritten on every operation. This file changes only when a set
//! is learned or dropped.
//!
//! It is a cache: everything in it can be learned again from the indexer, so a file that
//! doesn't parse just starts empty.
//!
//! Only what claims still need is kept whole (`prune`): a claim must name its epoch's whole
//! set, but what an epoch pays depends only on whether the claimant and its children were
//! active. So for every epoch it learns, the module also notes which of the nodes it follows
//! (each participant's node, referrer and children, and nodes `activity` was asked about)
//! were active, a few entries per epoch, and drops a whole set once no plan claims it. The
//! indexer's position of each epoch's publication is kept too, so a dropped set comes back
//! with one call.

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use referral_core::{NodeId, Participant, Registry, active_digest};
use serde::{Deserialize, Serialize};

use super::Service;
use crate::domain::claims::Known;
use crate::domain::{DomainError, Result, codec};
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
    /// Where each epoch's publication is in that history.
    #[serde(default)]
    pub indexed_at: BTreeMap<u32, u64>,
    /// How `history_offset` was reached (`HISTORY_FORMAT`). Earlier versions skipped every
    /// publication lez_indexer_module returns, so an offset they saved is read again.
    #[serde(default)]
    pub history_format: u32,
    /// By epoch: which followed nodes were active.
    #[serde(default)]
    pub marks: BTreeMap<u32, Marks>,
    /// Nodes `activity` was asked about, followed like the participants' own.
    #[serde(default)]
    pub watched: BTreeSet<String>,
    /// Each published epoch's archived digest (hex), read once: it never changes.
    #[serde(default)]
    pub archived: BTreeMap<u32, String>,
}

/// Which followed nodes were active in an epoch (hex), and which were checked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marks {
    pub digest: String,
    pub checked: BTreeSet<String>,
    pub active: BTreeSet<String>,
}

/// The current way of reading the indexer's pages; see `DeploymentLists::history_format`.
pub const HISTORY_FORMAT: u32 = 1;

/// Epochs within this many of the registry's are kept whole whatever the plans say: a
/// referral's join note or credit for them may still be on its way.
pub const KEEP_RECENT: u32 = 2;

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

    /// What's known of `epoch`'s set, if it matches the archived `digest`: the whole set (live
    /// or kept), or the marks for followed nodes.
    pub(super) fn known(&self, registry: &Registry, epoch: u32, digest: [u8; 32]) -> Option<Known> {
        if let Some(active) = self.known_active(registry, epoch, digest) {
            return Some(Known::Full(active));
        }
        let marks = self.lists()?.marks.get(&epoch).filter(|m| m.digest == hex::encode(digest))?;
        let nodes = |hex: &BTreeSet<String>| hex.iter().filter_map(|n| codec::node(n).ok()).collect();
        Some(Known::Some { checked: nodes(&marks.checked), active: nodes(&marks.active) })
    }

    /// The nodes whose activity is noted for every epoch (see the module docs).
    fn followed(&self) -> BTreeSet<String> {
        let mut nodes: BTreeSet<String> = self.lists().map(|l| l.watched.clone()).unwrap_or_default();
        for record in self.deployment().participants.values() {
            nodes.extend(record.node.clone());
            nodes.extend(record.invitation.as_ref().map(|i| i.node.clone()));
            nodes.extend(record.children_since.keys().cloned());
            nodes.extend(record.children.iter().cloned());
        }
        nodes
    }

    /// Records the participant's node and children from its state, so they're followed.
    pub(super) fn follow(&mut self, participant: &str, me: &Participant) -> Result<()> {
        let node = codec::node_hex(&me.node);
        let children: BTreeSet<String> = me.children.keys().map(codec::node_hex).collect();
        let referrer = me.referrer_node().map(|n| codec::node_hex(&n));
        let record = self.deployment_mut()?.participants.entry(participant.to_owned()).or_default();
        let mut changed = record.node.as_ref() != Some(&node) || !children.is_subset(&record.children);
        record.node = Some(node);
        record.children.extend(children);
        if let Some(referrer) = referrer {
            // A restored wallet has no invitation; the state names the referrer.
            let watched = &mut self.lists_mut()?.watched;
            changed |= watched.insert(referrer);
        }
        if changed {
            self.persist()?;
            self.persist_lists()?;
        }
        Ok(())
    }

    /// Adds `nodes` to the followed ones. `true` when any is new.
    pub(super) fn watch(&mut self, nodes: &[NodeId]) -> Result<bool> {
        let watched = &mut self.lists_mut()?.watched;
        let before = watched.len();
        watched.extend(nodes.iter().map(codec::node_hex));
        Ok(watched.len() > before)
    }

    /// Notes the followed nodes' activity in every whole set kept. `true` when anything changed.
    pub(super) fn refresh_marks(&mut self) -> Result<bool> {
        let followed = self.followed();
        let Some(lists) = self.lists() else { return Ok(false) };
        let mut updates = Vec::new();
        for (&epoch, set) in &lists.sets {
            if lists.marks.get(&epoch).is_some_and(|m| m.digest == set.digest && m.checked == followed) {
                continue;
            }
            let Some(nodes) = set.nodes() else { continue };
            let active = followed.iter().filter(|n| codec::node(n).is_ok_and(|n| nodes.contains(&n))).cloned().collect();
            updates.push((epoch, Marks { digest: set.digest.clone(), checked: followed.clone(), active }));
        }
        if updates.is_empty() {
            return Ok(false);
        }
        self.lists_mut()?.marks.extend(updates);
        Ok(true)
    }

    /// Drops whole sets no claim needs: older than `KEEP_RECENT` epochs before `current`, and
    /// neither claimed by a participant's last plan nor beyond where it looked. Marks are
    /// noted first, so activity and plans still know what those epochs pay. Participants
    /// without a node (not enrolling) hold nothing. `true` when anything changed.
    pub(super) fn prune(&mut self, current: u32) -> Result<bool> {
        let mut changed = self.refresh_marks()?;
        let held: Vec<(Option<u32>, Vec<u32>)> = self
            .deployment()
            .participants
            .values()
            .filter(|r| r.node.is_some())
            .map(|r| (r.planned_through, r.needs.clone()))
            .collect();
        let needed = |epoch: u32| {
            epoch.saturating_add(KEEP_RECENT) > current
                || held.iter().any(|(through, needs)| through.is_none_or(|t| t < epoch) || needs.contains(&epoch))
        };
        let Some(lists) = self.lists() else { return Ok(changed) };
        let drop: Vec<u32> = lists.sets.keys().copied().filter(|&e| !needed(e)).collect();
        if !drop.is_empty() {
            let sets = &mut self.lists_mut()?.sets;
            for epoch in drop {
                sets.remove(&epoch);
            }
            changed = true;
        }
        Ok(changed)
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
