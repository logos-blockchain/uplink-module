//! The participant account (the points account in the wallet) and invitations.

use referral_core::Participant;
use serde_json::{Value, json};

use super::{Service, port};
use crate::domain::invitation::Invitation;
use crate::domain::{DomainError, Result, codec, state};

impl Service {
    /// A new private LEZ account for the programme. With a label, returns the account
    /// already carrying it instead of creating another.
    pub fn create_participant(&mut self, label: &str) -> Result<Value> {
        if !label.is_empty() {
            let existing = self.lez.resolve_label(label).map_err(port)?;
            let id = existing.split('/').next_back().unwrap_or_default();
            if !id.is_empty() {
                self.deployment_mut()?.participants.entry(id.to_lowercase()).or_default();
                self.persist()?;
                return Ok(json!({ "participant": id.to_lowercase(), "created": false }));
            }
        }
        let id = self.lez.create_account_private().map_err(port)?.to_lowercase();
        codec::account(&id)?;
        self.deployment_mut()?.participants.entry(id.clone()).or_default();
        self.persist()?;
        if !label.is_empty() {
            // lez_core reports success even when the wallet refuses a label, so read it back.
            self.lez.add_label(label, &id, true).map_err(port)?;
            let kept = self.lez.resolve_label(label).map_err(port)?;
            if !kept.split('/').next_back().unwrap_or_default().eq_ignore_ascii_case(&id) {
                return Err(DomainError::new(
                    "label_not_kept",
                    format!("created {id}, but the wallet did not keep the label \"{label}\""),
                ));
            }
        }
        Ok(json!({ "participant": id, "created": true }))
    }

    pub fn participant(&self, participant: &str) -> Result<Value> {
        match self.read_participant(participant) {
            Ok(me) => Ok(json!({
                "registered": true,
                "node": codec::node_hex(&me.node),
                "referrer": me.referrer_node().as_ref().map(codec::node_hex),
                // Per child, the epochs already settled, as inclusive ranges.
                "children": me.children.iter().map(|(node, settled)| (codec::node_hex(node), settled.clone())).collect::<std::collections::BTreeMap<_, _>>(),
                "rewardBalance": me.reward_balance.to_string(),
            })),
            Err(e) if e.code == "not_registered" => Ok(json!({ "registered": false })),
            Err(e) => Err(e),
        }
    }

    pub(super) fn read_participant(&self, participant: &str) -> Result<Participant> {
        codec::account(participant)?;
        let account = self.lez.get_account_private(participant).map_err(port)?;
        if account.is_empty() {
            return Err(DomainError::new("unknown_account", "this wallet has no such private account"));
        }
        state::participant(&codec::shard(&account, &self.program()?)?)
    }

    /// The blob to hand to a node you invite.
    pub fn invitation(&self, participant: &str, node: &str) -> Result<Value> {
        codec::account(participant)?;
        let keys = self.lez.get_private_account_keys(participant).map_err(port)?;
        let invitation = Invitation::from_keys(codec::node(node)?, &keys)?;
        Ok(json!({ "invitation": invitation.encode()? }))
    }

    pub fn import_invitation(&mut self, participant: &str, blob: &str) -> Result<Value> {
        codec::account(participant)?;
        let invitation = Invitation::decode(blob)?;
        let referrer = invitation.referrer()?;
        if let Ok(me) = self.read_participant(participant) {
            if me.referrer_node() != Some(referrer) {
                return Err(DomainError::new("conflict", "this participant already joined under another referrer"));
            }
        }
        let record = self.deployment_mut()?.participants.entry(participant.to_lowercase()).or_default();
        record.invitation = Some(invitation);
        self.persist()?;
        Ok(json!({ "referrer": codec::node_hex(&referrer) }))
    }

    /// Reads a pasted invitation without saving it: who invited, and whether that node is
    /// registered (only a registered inviter can be joined under).
    pub fn check_invitation(&mut self, blob: &str) -> Result<Value> {
        let referrer = Invitation::decode(blob)?.referrer()?;
        let registry = self.read_registry()?;
        Ok(json!({ "referrer": codec::node_hex(&referrer), "registered": registry.registered_nodes.contains(&referrer) }))
    }

    /// Whether a participant label already names an enrolled participant.
    pub fn find_participant(&self, label: &str) -> Result<Value> {
        let resolved = self.lez.resolve_label(label).map_err(port)?;
        let id = resolved.split('/').next_back().unwrap_or_default().to_lowercase();
        if id.is_empty() {
            return Ok(json!({ "participant": null }));
        }
        let mut found = self.participant(&id)?;
        found["participant"] = json!(id);
        Ok(found)
    }
}
