//! Which nodes were active in recent epochs, for the activity strips Uplink shows.

use serde_json::{Value, json};

use super::Service;
use referral_core::NodeId;

use crate::domain::claims::Known;
use crate::domain::{Result, codec};

impl Service {
    /// For each of `nodes` (hex), whether it was active in each of the last `count` published
    /// epochs, oldest first: `1` active, `0` not, `-1` unknown (an epoch this module didn't see
    /// live and the indexer couldn't supply). Epochs never published are left out.
    pub fn activity(&mut self, nodes: &[String], count: u32) -> Result<Value> {
        let nodes = nodes.iter().map(|node| codec::node(node)).collect::<Result<Vec<_>>>()?;
        let registry = self.read_registry()?;
        // Followed from now on, so their activity outlives the epochs' whole sets.
        if self.watch(&nodes)? {
            self.refresh_marks()?;
            self.persist_lists()?;
        }
        let first = registry.epoch.saturating_sub(count.saturating_sub(1)).max(1);
        let mut epochs = Vec::new();
        for epoch in first..=registry.epoch {
            if let Some(digest) = self.archived_digest(epoch)? {
                epochs.push((epoch, digest));
            }
        }
        let answers = |service: &Self| -> Vec<Vec<i8>> {
            epochs
                .iter()
                .map(|&(epoch, digest)| {
                    let known = service.known(&registry, epoch, digest);
                    nodes.iter().map(|node| mark(known.as_ref(), node)).collect()
                })
                .collect()
        };
        let mut by_epoch = answers(self);
        // One trip to the indexer at most, for the epochs nothing here can answer.
        let unknown: Vec<u32> = epochs.iter().zip(&by_epoch).filter(|(_, m)| m.contains(&-1)).map(|((e, _), _)| *e).collect();
        if !unknown.is_empty() && self.fill_publications(&unknown)? {
            by_epoch = answers(self);
            // Their marks are noted; the sets themselves needn't stay.
            if self.prune(registry.epoch)? {
                self.persist_lists()?;
            }
        }

        let per_node = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (codec::node_hex(node), by_epoch.iter().map(|marks| marks[i]).collect::<Vec<_>>()))
            .collect::<std::collections::BTreeMap<_, _>>();
        Ok(json!({ "epochs": epochs.iter().map(|(epoch, _)| epoch).collect::<Vec<_>>(), "nodes": per_node }))
    }
}

/// `1` active, `0` not, `-1` unknown.
fn mark(known: Option<&Known>, node: &NodeId) -> i8 {
    match known {
        Some(Known::Full(active)) => i8::from(active.contains(node)),
        Some(Known::Some { checked, active }) if checked.contains(node) => i8::from(active.contains(node)),
        _ => -1,
    }
}
