//! The oracle's registry and each epoch's publication: live, saved, or from the indexer.

use std::collections::BTreeSet;

use referral_core::{NodeId, ORACLE_ACCOUNT_ID, Registry, active_digest};
use serde_json::{Value, json};

use super::{Service, port};
use crate::domain::{DomainError, Result, codec, state};

impl Service {
    pub fn registry(&mut self) -> Result<Value> {
        let registry = self.read_registry()?;
        Ok(json!({
            "epoch": registry.epoch,
            "nodes": registry.registered_nodes.iter().map(codec::node_hex).collect::<Vec<_>>(),
            "active": registry.active_nodes.iter().map(codec::node_hex).collect::<Vec<_>>(),
        }))
    }

    pub(super) fn read_registry(&mut self) -> Result<Registry> {
        let oracle = codec::account_hex(&ORACLE_ACCOUNT_ID);
        let account = self.lez.get_account_public(&oracle).map_err(port)?;
        let registry =
            if account.is_empty() { Registry::default() } else { state::registry(&codec::shard(&account, &self.program()?)?)? };
        if registry.epoch > 0 && self.remember_publication(registry.epoch, &registry.active_nodes)? {
            self.persist()?;
        }
        Ok(registry)
    }

    pub(super) fn publication_key(epoch: u32, digest: &[u8; 32]) -> String {
        format!("{epoch}:{}", hex::encode(digest))
    }

    /// `true` when it's new.
    pub(super) fn remember_publication(&mut self, epoch: u32, active: &BTreeSet<NodeId>) -> Result<bool> {
        let key = Self::publication_key(epoch, &active_digest(active));
        Ok(self.deployment_mut()?.publications.insert(key, active.iter().map(codec::node_hex).collect()).is_none())
    }

    pub(super) fn publication(&self, (epoch, digest): (u32, [u8; 32])) -> Option<BTreeSet<NodeId>> {
        self.deployment()
            .publications
            .get(&Self::publication_key(epoch, &digest))?
            .iter()
            .map(|node| codec::node(node).ok())
            .collect()
    }

    /// The digest the program archived for `epoch`, or `None` if it was never published.
    pub(super) fn archived_digest(&self, epoch: u32) -> Result<Option<[u8; 32]>> {
        let program = self.program()?;
        let archive = codec::account_hex(&referral_core::epoch_account(program, epoch));
        let account = self.lez.get_account_public(&archive).map_err(port)?;
        let data = if account.is_empty() { Vec::new() } else { codec::shard(&account, &program)? };
        match state::publication(&data) {
            None => Ok(None),
            Some((published, _)) if published != epoch => {
                Err(DomainError::new("bad_publication", "an epoch account holds another epoch's publication"))
            }
            Some((_, digest)) => Ok(Some(digest)),
        }
    }

    /// The active set matching an archived digest, from the registry or what this module saved.
    pub(super) fn known_active(&self, registry: &Registry, epoch: u32, digest: [u8; 32]) -> Option<BTreeSet<NodeId>> {
        if (registry.epoch, active_digest(&registry.active_nodes)) == (epoch, digest) {
            return Some(registry.active_nodes.clone());
        }
        self.publication((epoch, digest))
    }

    /// Reads publications this module didn't see live from the indexer, if one is set up.
    pub(super) fn fill_publications(&mut self) -> Result<bool> {
        let Some(history) = &self.history else { return Ok(false) };
        const PAGE: u64 = 100;
        let program = self.program()?;
        let oracle = codec::account_hex(&ORACLE_ACCOUNT_ID);
        let mut offset = self.deployment().history_offset;
        let mut found = Vec::new();
        loop {
            // An indexer that isn't installed or running just means no history.
            let Ok(page) = history.transactions_by_account(&oracle, offset, PAGE) else { break };
            let (count, publications) = crate::domain::publication::page(&page, program);
            offset += count as u64;
            found.extend(publications);
            if (count as u64) < PAGE {
                break;
            }
        }
        if offset == self.deployment().history_offset {
            return Ok(false);
        }
        self.deployment_mut()?.history_offset = offset;
        for (epoch, active) in found {
            self.remember_publication(epoch, &active)?;
        }
        self.persist()?;
        Ok(true)
    }
}
