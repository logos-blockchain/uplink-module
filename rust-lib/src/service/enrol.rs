//! Joining: Register, signed by the node's BlendSigning key.

use referral_core::{NodeId, ORACLE_ACCOUNT_ID};
use serde_json::{Value, json};

use super::saved::OperationKind;
use super::Service;
use crate::domain::keystore::{self, NodeKey};
use crate::domain::{DomainError, Result, codec, enrolment, instructions};
use crate::ports::AccountEntry;

impl Service {
    fn referrer_of(&self, participant: &str) -> Result<Option<NodeId>> {
        self.invitation_for(participant).map(|invitation| invitation.referrer()).transpose()
    }

    pub(crate) fn load_node_key(&self, keystore_path: &str, node: Option<NodeId>) -> Result<NodeKey> {
        let yaml = self.files.read_secret(keystore_path).map_err(|e| DomainError::new("keystore", e))?;
        keystore::blend_signing_key(&yaml, node)
    }

    /// The bytes the node signs to join, for callers that sign elsewhere.
    pub fn enrolment_message(&self, participant: &str, node: &str) -> Result<Value> {
        let referrer = self.referrer_of(participant)?;
        let message = enrolment::message(self.program()?, codec::node(node)?, codec::account(participant)?, referrer);
        Ok(json!({ "message": hex::encode(message), "referrer": referrer.as_ref().map(codec::node_hex) }))
    }

    /// Reads the node's key from the keystore the user pointed at, signs, and joins. `node`
    /// is the node being enrolled (the running node's provider id): a keystore without its
    /// key is refused, so a wrong file can't enrol another node. `""` takes the keystore's
    /// BlendSigning key. The key is wiped as soon as the signature is made.
    pub fn enrol(&mut self, reference: &str, participant: &str, keystore_path: &str, node: &str) -> Result<Value> {
        // Before the keystore: a repeat needs no key.
        if let Some(existing) = self.replay(reference, OperationKind::Register, participant)? {
            return Ok(existing);
        }
        let referrer = self.referrer_of(participant)?;
        let expected = codec::optional_node(node)?;
        let (node, signature) = {
            let key = self.load_node_key(keystore_path, expected)?;
            let node = key.node();
            let message = enrolment::message(self.program()?, node, codec::account(participant)?, referrer);
            (node, key.sign(&message))
        };
        self.enrol_with_signature(reference, participant, &codec::node_hex(&node), &hex::encode(signature))
    }

    pub fn enrol_with_signature(&mut self, reference: &str, participant: &str, node: &str, signature: &str) -> Result<Value> {
        if let Some(existing) = self.replay(reference, OperationKind::Register, participant)? {
            return Ok(existing);
        }
        let terms = self.require_terms()?;
        let program = self.program()?;
        let participant_id = codec::account(participant)?;
        let node = codec::node(node)?;
        let signature: [u8; 64] = codec::bytes(signature)?
            .try_into()
            .map_err(|_| DomainError::new("bad_signature", "a signature is 64 bytes"))?;
        let referrer = self.referrer_of(participant)?;
        if !enrolment::verify(program, node, participant_id, referrer, &signature) {
            return Err(DomainError::new("bad_signature", "the signature is not the node's over this enrolment"));
        }

        self.require_no_other_open(participant, reference)?;
        match self.read_participant(participant) {
            Ok(_) => return Err(DomainError::new("already_registered", "this participant is already registered")),
            Err(e) if e.code == "not_registered" => {}
            Err(e) => return Err(e),
        }
        let registry = self.read_registry()?;
        if registry.registered_nodes.contains(&node) {
            return Err(DomainError::new("already_registered", "this node is already registered"));
        }
        let mut accounts = vec![
            AccountEntry::Own(codec::account_hex(&participant_id)),
            AccountEntry::PublicNoSign(codec::account_hex(&ORACLE_ACCOUNT_ID)),
        ];
        let mut referrer_keys = None;
        if let Some(parent) = referrer {
            if !registry.registered_nodes.contains(&parent) {
                return Err(DomainError::new("referrer_not_registered", "the referrer is not registered"));
            }
            let invitation = self.invitation_for(participant).expect("referrer comes from the invitation");
            accounts.push(AccountEntry::PrivateForeign { npk: invitation.npk.clone(), vpk: invitation.vpk.clone() });
            // Recorded in the participant's state, so claims address the referrer without the invitation.
            referrer_keys = Some(invitation.referrer_keys()?);
        }

        let epoch = registry.epoch;
        let instruction = instructions::register(node, referrer_keys, signature, registry.registered_nodes, epoch);
        let record = self.deployment_mut()?.participants.entry(participant.to_lowercase()).or_default();
        record.node = Some(codec::node_hex(&node));
        record.terms = Some(super::saved::TermsAcceptance { node: Some(codec::node_hex(&node)), ..terms });
        self.submit(
            reference,
            OperationKind::Register,
            participant,
            accounts,
            instruction,
            json!({ "node": codec::node_hex(&node), "referrer": referrer.as_ref().map(codec::node_hex), "epoch": epoch }),
        )
    }

    /// The node a keystore is for, from its public keys only.
    pub fn keystore_node(&self, keystore_path: &str) -> Result<Value> {
        let yaml = self.files.read_secret(keystore_path).map_err(|e| DomainError::new("keystore", e))?;
        Ok(json!({ "node": codec::node_hex(&crate::domain::keystore::blend_signing_public(&yaml)?) }))
    }
}
