//! Wallet sync and the notes addressed to the participant: new referrals and credits.

use lee_core::account::AccountId;
use referral_core::{NodeId, State};
use serde_json::{Value, json};

use super::{Service, participant_id, port};
use crate::domain::{DomainError, Result, codec, state};

impl Service {
    /// Brings the wallet up to the chain tip, so notes addressed to it are found.
    pub fn sync(&self) -> Result<Value> {
        let height = self.lez.get_current_block_height().map_err(port)?;
        self.lez.sync_to_block(height).map_err(port)?;
        Ok(json!({ "block": height }))
    }

    /// Notes (new referrals, credits) in the wallet addressed to the participant.
    pub(super) fn discover_notes(&self, participant: &str, me: NodeId) -> Result<Vec<(AccountId, State)>> {
        let program = self.program()?;
        let npk_of = |id: &str| -> Option<String> {
            let keys: Value = serde_json::from_str(&self.lez.get_private_account_keys(id).ok()?).ok()?;
            keys.get("nullifier_public_key")?.as_str().map(str::to_lowercase)
        };
        let mine = npk_of(participant).ok_or_else(|| DomainError::new("unknown_account", "no keys for this participant"))?;
        let mut notes = Vec::new();
        for (id, is_public) in self.lez.list_accounts().map_err(port)? {
            if is_public || id.eq_ignore_ascii_case(participant) || npk_of(&id).as_deref() != Some(mine.as_str()) {
                continue;
            }
            let account = self.lez.get_account_private(&id).map_err(port)?;
            if account.is_empty() {
                continue;
            }
            if let Some(note) = state::note(&codec::shard(&account, &program)?) {
                notes.push((codec::account(&id)?, note));
            }
        }
        Ok(crate::domain::claims::addressed_to(me, notes))
    }

    pub fn notes(&mut self, participant: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        let me = self.read_participant(participant)?;
        let notes = self.discover_notes(participant, me.node)?;
        self.remember_joins(participant, &notes)?;
        Ok(json!({ "notes": notes.iter().map(|(id, note)| note_json(id, note)).collect::<Vec<_>>() }))
    }

    /// Records the join epoch of every child announced in `notes`, the first time it's seen.
    pub(crate) fn remember_joins(&mut self, participant: &str, notes: &[(AccountId, State)]) -> Result<()> {
        let joins: Vec<(String, u32)> = notes
            .iter()
            .filter_map(|(_, note)| match note {
                State::Child { node, since, .. } => Some((codec::node_hex(node), *since)),
                _ => None,
            })
            .collect();
        if joins.is_empty() {
            return Ok(());
        }
        let record = self.deployment_mut()?.participants.entry(participant.to_owned()).or_default();
        let mut changed = false;
        for (node, since) in joins {
            if !record.children_since.contains_key(&node) {
                record.children_since.insert(node, since);
                changed = true;
            }
        }
        if changed {
            self.persist()?;
        }
        Ok(())
    }

    pub(super) fn children_since(&self, participant: &str) -> std::collections::BTreeMap<String, u32> {
        self.deployment()
            .participants
            .get(participant)
            .map(|record| record.children_since.clone())
            .unwrap_or_default()
    }
}

fn note_json(id: &AccountId, note: &State) -> Value {
    match note {
        State::Child { node, since, .. } => {
            json!({ "account": codec::account_hex(id), "kind": "child", "node": codec::node_hex(node), "since": since })
        }
        State::Credit { amount, epoch, .. } => {
            json!({ "account": codec::account_hex(id), "kind": "credit", "amount": amount.to_string(), "epoch": epoch })
        }
        _ => json!({ "account": codec::account_hex(id), "kind": "other" }),
    }
}
