//! Which nodes were active in recent epochs, for the activity strips Uplink shows.

use serde_json::{Value, json};

use super::Service;
use crate::domain::{Result, codec};

impl Service {
    /// For each of `nodes` (hex), whether it was active in each of the last `count` published
    /// epochs, oldest first: `1` active, `0` not, `-1` unknown (an epoch this module didn't see
    /// live and the indexer couldn't supply). Epochs never published are left out.
    pub fn activity(&mut self, nodes: &[String], count: u32) -> Result<Value> {
        let nodes = nodes.iter().map(|node| codec::node(node)).collect::<Result<Vec<_>>>()?;
        let registry = self.read_registry()?;
        let first = registry.epoch.saturating_sub(count.saturating_sub(1)).max(1);

        let mut epochs = Vec::new();
        for epoch in first..=registry.epoch {
            if let Some(digest) = self.archived_digest(epoch)? {
                epochs.push((epoch, digest));
            }
        }
        // One trip to the indexer at most, for every epoch not known locally.
        if epochs.iter().any(|&(epoch, digest)| self.known_active(&registry, epoch, digest).is_none()) {
            self.fill_publications()?;
        }

        let sets: Vec<_> = epochs.iter().map(|&(epoch, digest)| self.known_active(&registry, epoch, digest)).collect();
        let per_node = nodes
            .iter()
            .map(|node| {
                let marks: Vec<i8> = sets
                    .iter()
                    .map(|set| match set {
                        Some(active) => i8::from(active.contains(node)),
                        None => -1,
                    })
                    .collect();
                (codec::node_hex(node), marks)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        Ok(json!({ "epochs": epochs.iter().map(|(epoch, _)| epoch).collect::<Vec<_>>(), "nodes": per_node }))
    }
}
