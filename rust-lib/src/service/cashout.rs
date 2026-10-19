//! Cash-out: turn the whole reward balance into a public receipt, and the payout code
//! that proves it. What the node's key provides (its receipt secret, its signature on a
//! payout) comes either from a keystore, on the command line, or from whoever holds the
//! key: in Basecamp, the Blockchain app, which keeps it (see the README).

use lee_core::account::AccountId;
use referral_core::Participant;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::saved::{KnownReceipt, OperationKind};
use super::{Service, participant_id, port};
use crate::domain::cashout::{self, ReceiptKeys};
use crate::domain::keystore::NodeKey;
use crate::domain::{DomainError, Result, codec, instructions, state};
use crate::ports::AccountEntry;

struct Receipt {
    index: u64,
    account: AccountId,
    points: u128,
}

impl Service {
    /// The participant and its node key, from a keystore (see `Files::keystore_paths`).
    fn participant_key(&self, participant: &str, keystore_path: &str) -> Result<(Participant, NodeKey)> {
        let me = self.read_participant(participant)?;
        let key = self.load_node_key(keystore_path, Some(me.node))?;
        Ok((me, key))
    }

    /// The participant and its receipt keys from a handed-in receipt secret (hex), checked
    /// before use: a wrong secret would put a cash-out where no one can find it again.
    fn keys_from_secret(&mut self, participant: &str, receipt_secret: &str) -> Result<(Participant, ReceiptKeys)> {
        let me = self.read_participant(participant)?;
        let secret = Zeroizing::new(
            codec::bytes32(receipt_secret).map_err(|_| DomainError::new("bad_receipt_secret", "a receipt secret is 32 bytes of hex"))?,
        );
        let keys = ReceiptKeys::from_secret(me.node, secret);
        self.accept_keys(participant, &keys)?;
        Ok((me, keys))
    }

    /// Refuses receipt keys other than the ones this participant used before: the fingerprint
    /// remembered at first use, or, for receipts found before fingerprints were kept, the
    /// first known receipt's account.
    fn accept_keys(&mut self, participant: &str, keys: &ReceiptKeys) -> Result<()> {
        let fingerprint = keys.fingerprint();
        let record = self.deployment().participants.get(participant);
        let mismatch = || DomainError::new("receipt_secret_mismatch", "this receipt secret is not the node's");
        match record.and_then(|r| r.receipt_fingerprint.as_ref()) {
            Some(known) if *known == fingerprint => return Ok(()),
            Some(_) => return Err(mismatch()),
            None => {}
        }
        if let Some(first) = record.and_then(|r| r.receipts.first()).cloned() {
            let blinding = keys.blinding_factor(self.program()?, codec::account(participant)?, first.index);
            if codec::account_hex(&cashout::receipt(self.program()?, keys.node, blinding)) != first.account {
                return Err(mismatch());
            }
        }
        self.deployment_mut()?.participants.entry(participant.to_owned()).or_default().receipt_fingerprint = Some(fingerprint);
        self.persist()
    }

    /// The participant's receipts on chain, and the next free index.
    fn scan_receipts(&self, keys: &ReceiptKeys, participant: &str) -> Result<(Vec<Receipt>, u64)> {
        let program = self.program()?;
        let participant_id = codec::account(participant)?;
        let mut receipts = Vec::new();
        for index in 0.. {
            let blinding = keys.blinding_factor(program, participant_id, index);
            let account = cashout::receipt(program, keys.node, blinding);
            let json = self.lez.get_account_public(&codec::account_hex(&account)).map_err(port)?;
            let data = if json.is_empty() { Vec::new() } else { codec::shard(&json, &program)? };
            match state::receipt_points(&data) {
                Some(points) => receipts.push(Receipt { index, account, points }),
                None => return Ok((receipts, index)),
            }
        }
        unreachable!("the index range is unbounded")
    }

    /// Finds the participant's receipts on chain and remembers them, with their blinding
    /// factors, so they can be listed and paid out without the secret again.
    fn find_receipts(&mut self, participant: &str, keys: &ReceiptKeys) -> Result<()> {
        let (found, _) = self.scan_receipts(keys, participant)?;
        let (program, participant_id) = (self.program()?, codec::account(participant)?);
        let known = found
            .iter()
            .map(|r| KnownReceipt {
                index: r.index,
                account: codec::account_hex(&r.account),
                points: r.points.to_string(),
                blinding_factor: Some(hex::encode(keys.blinding_factor(program, participant_id, r.index))),
            })
            .collect();
        self.deployment_mut()?.participants.entry(participant.to_owned()).or_default().receipts = known;
        self.persist()
    }

    fn known_receipts(&self, participant: &str) -> Value {
        let known = self.deployment().participants.get(participant).map(|r| r.receipts.clone()).unwrap_or_default();
        json!({ "receipts": known.iter().map(|r| json!({ "index": r.index, "account": r.account, "points": r.points })).collect::<Vec<_>>() })
    }

    /// The participant's receipts. With a keystore path they're found on chain and remembered;
    /// with `""` the remembered list comes back without touching any key.
    pub fn receipts(&mut self, participant: &str, keystore_path: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        if !keystore_path.trim().is_empty() {
            let (_, key) = self.participant_key(participant, keystore_path)?;
            let keys = ReceiptKeys::from_key(&key);
            self.accept_keys(participant, &keys)?;
            self.find_receipts(participant, &keys)?;
        }
        Ok(self.known_receipts(participant))
    }

    /// `receipts` with the node's receipt secret instead of its keystore.
    pub fn receipts_with_secret(&mut self, participant: &str, receipt_secret: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        let (_, keys) = self.keys_from_secret(participant, receipt_secret)?;
        self.find_receipts(participant, &keys)?;
        Ok(self.known_receipts(participant))
    }

    pub fn cash_out(&mut self, reference: &str, participant: &str, keystore_path: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        if let Some(existing) = self.replay(reference, OperationKind::CashOut, participant)? {
            return Ok(existing);
        }
        let (me, key) = self.participant_key(participant, keystore_path)?;
        let keys = ReceiptKeys::from_key(&key);
        drop(key);
        self.accept_keys(participant, &keys)?;
        self.cash_out_with(reference, participant, &me, &keys)
    }

    /// `cash_out` with the node's receipt secret instead of its keystore.
    pub fn cash_out_with_secret(&mut self, reference: &str, participant: &str, receipt_secret: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        if let Some(existing) = self.replay(reference, OperationKind::CashOut, participant)? {
            return Ok(existing);
        }
        let (me, keys) = self.keys_from_secret(participant, receipt_secret)?;
        self.cash_out_with(reference, participant, &me, &keys)
    }

    fn cash_out_with(&mut self, reference: &str, participant: &str, me: &Participant, keys: &ReceiptKeys) -> Result<Value> {
        self.require_no_other_open(participant, reference)?;
        if me.reward_balance == 0 {
            return Err(DomainError::new("nothing_to_cash_out", "the reward balance is zero"));
        }
        let (_, index) = self.scan_receipts(keys, participant)?;
        let blinding = keys.blinding_factor(self.program()?, codec::account(participant)?, index);
        let receipt = cashout::receipt(self.program()?, me.node, blinding);
        let accounts = vec![
            AccountEntry::Own(participant.to_owned()),
            AccountEntry::PublicNoSign(codec::account_hex(&receipt)),
        ];
        let instruction = instructions::cash_out(me.node, me.reward_balance, blinding);
        self.submit(
            reference,
            OperationKind::CashOut,
            participant,
            accounts,
            instruction,
            json!({
                "index": index,
                "receipt": codec::account_hex(&receipt),
                "points": me.reward_balance.to_string(),
                "blindingFactor": hex::encode(blinding),
            }),
        )
    }

    /// The opening of receipt `index` and its blinding factor: from what's remembered, or found
    /// on chain with `keys`.
    fn receipt_opening(&mut self, participant: &str, index: u64, keys: Option<&ReceiptKeys>) -> Result<(KnownReceipt, String)> {
        let remembered = |service: &Self| {
            service.deployment().participants.get(participant).and_then(|r| r.receipts.iter().find(|r| r.index == index).cloned())
        };
        let receipt = match (remembered(self), keys) {
            (Some(known), _) if known.blinding_factor.is_some() => known,
            (_, Some(keys)) => {
                self.find_receipts(participant, keys)?;
                remembered(self).ok_or_else(|| DomainError::new("unknown_receipt", "no receipt with this index"))?
            }
            _ => return Err(DomainError::new("receipt_secret_needed", "this receipt isn't known here; pass the receipt secret")),
        };
        let me = self.read_participant(participant)?;
        let blinding = codec::bytes32(receipt.blinding_factor.as_deref().expect("found with its blinding factor"))?;
        Ok((receipt, cashout::payout_message(blinding, me.node, self.program()?)))
    }

    /// The code the user pastes into the payout form: the receipt's opening (program,
    /// node, blinding factor), signed by the node so a copied code is useless to others.
    pub fn payout_code(&mut self, participant: &str, index: u64, keystore_path: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        let (_, key) = self.participant_key(participant, keystore_path)?;
        let keys = ReceiptKeys::from_key(&key);
        self.accept_keys(participant, &keys)?;
        let (receipt, message) = self.receipt_opening(participant, index, Some(&keys))?;
        let code = cashout::payout_code(&message, key.node(), &key.sign(message.as_bytes())).expect("our own signature");
        Ok(json!({ "code": code, "receipt": receipt.account, "points": receipt.points }))
    }

    /// What the node signs for receipt `index`'s payout code (see `payout_code_with_signature`).
    /// `receipt_secret` may be `""` for a receipt this module cashed out or found before.
    pub fn payout_message(&mut self, participant: &str, index: u64, receipt_secret: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        let keys = if receipt_secret.trim().is_empty() { None } else { Some(self.keys_from_secret(participant, receipt_secret)?.1) };
        let (receipt, message) = self.receipt_opening(participant, index, keys.as_ref())?;
        Ok(json!({ "message": message, "receipt": receipt.account, "points": receipt.points }))
    }

    /// The payout code from the node's signature over `payout_message`'s message (hex).
    pub fn payout_code_with_signature(&mut self, participant: &str, index: u64, signature: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        let (receipt, message) = self.receipt_opening(participant, index, None)?;
        let me = self.read_participant(participant)?;
        let signature: [u8; 64] = codec::bytes(signature)?
            .try_into()
            .map_err(|_| DomainError::new("bad_signature", "a signature is 64 bytes"))?;
        let code = cashout::payout_code(&message, me.node, &signature)
            .ok_or_else(|| DomainError::new("bad_signature", "the signature is not the node's over this payout"))?;
        Ok(json!({ "code": code, "receipt": receipt.account, "points": receipt.points }))
    }

    /// A settled cash-out's receipt joins the participant's known receipts.
    pub(super) fn remember_receipt(&mut self, operation: &super::saved::Operation) -> Result<()> {
        let detail = &operation.detail;
        let (Some(index), Some(account), Some(points)) =
            (detail["index"].as_u64(), detail["receipt"].as_str(), detail["points"].as_str())
        else {
            return Ok(());
        };
        let record = self.deployment_mut()?.participants.entry(operation.participant.to_owned()).or_default();
        if !record.receipts.iter().any(|r| r.index == index) {
            let blinding_factor = detail["blindingFactor"].as_str().map(str::to_owned);
            record.receipts.push(KnownReceipt { index, account: account.to_owned(), points: points.to_owned(), blinding_factor });
            record.receipts.sort_by_key(|r| r.index);
        }
        Ok(())
    }
}
