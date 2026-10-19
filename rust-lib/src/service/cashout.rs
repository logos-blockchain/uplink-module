//! Cash-out: turn the whole reward balance into a public receipt, and the payout code
//! that proves it. Every step re-derives receipt secrets from the node key.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use lee_core::account::AccountId;
use serde_json::{Value, json};

use super::saved::{KnownReceipt, Operation, OperationKind};
use super::{Service, port};
use crate::domain::keystore::NodeKey;
use crate::domain::{DomainError, Result, cashout, codec, instructions, state};
use crate::ports::AccountEntry;

/// The prefix of payout codes Uplink already understands (`PayoutCode.h`).
pub const PAYOUT_PREFIX: &str = "uplink-payout-v2:";

struct Receipt {
    index: u64,
    account: AccountId,
    points: u128,
}

impl Service {
    fn participant_key(&self, participant: &str, keystore_path: &str) -> Result<NodeKey> {
        let me = self.read_participant(participant)?;
        self.load_node_key(keystore_path, Some(me.node))
    }

    /// The participant's receipts, and the next free index.
    fn scan_receipts(&self, key: &NodeKey, participant: &str) -> Result<(Vec<Receipt>, u64)> {
        let program = self.program()?;
        let participant_id = codec::account(participant)?;
        let mut receipts = Vec::new();
        for index in 0.. {
            let blinding = cashout::blinding_factor(key, program, participant_id, index);
            let account = cashout::receipt(program, key.node(), blinding);
            let json = self.lez.get_account_public(&codec::account_hex(&account)).map_err(port)?;
            let data = if json.is_empty() { Vec::new() } else { codec::shard(&json, &program)? };
            match state::receipt_points(&data) {
                Some(points) => receipts.push(Receipt { index, account, points }),
                None => return Ok((receipts, index)),
            }
        }
        unreachable!("the index range is unbounded")
    }

    /// The participant's receipts. With a keystore path they're found on-chain with the node
    /// key and remembered; with `""` the remembered list comes back without touching the key.
    pub fn receipts(&mut self, participant: &str, keystore_path: &str) -> Result<Value> {
        if !keystore_path.trim().is_empty() {
            let key = self.participant_key(participant, keystore_path)?;
            let (found, _) = self.scan_receipts(&key, participant)?;
            let known = found
                .iter()
                .map(|r| KnownReceipt { index: r.index, account: codec::account_hex(&r.account), points: r.points.to_string() })
                .collect();
            self.deployment_mut()?.participants.entry(participant.to_lowercase()).or_default().receipts = known;
            self.persist()?;
        }
        let known = self.deployment().participants.get(&participant.to_lowercase()).map(|r| r.receipts.clone()).unwrap_or_default();
        Ok(json!({
            "receipts": known.iter().map(|r| json!({ "index": r.index, "account": r.account, "points": r.points })).collect::<Vec<_>>()
        }))
    }

    pub fn cash_out(&mut self, reference: &str, participant: &str, keystore_path: &str) -> Result<Value> {
        if let Some(existing) = self.replay(reference, OperationKind::CashOut, participant)? {
            return Ok(existing);
        }
        self.require_no_other_open(participant, reference)?;
        let me = self.read_participant(participant)?;
        if me.reward_balance == 0 {
            return Err(DomainError::new("nothing_to_cash_out", "the reward balance is zero"));
        }
        let (index, receipt, blinding) = {
            let key = self.load_node_key(keystore_path, Some(me.node))?;
            let (_, index) = self.scan_receipts(&key, participant)?;
            let blinding = cashout::blinding_factor(&key, self.program()?, codec::account(participant)?, index);
            (index, cashout::receipt(self.program()?, me.node, blinding), blinding)
        };
        let accounts = vec![
            AccountEntry::Own(participant.to_lowercase()),
            AccountEntry::PublicNoSign(codec::account_hex(&receipt)),
        ];
        let instruction = instructions::cash_out(me.node, me.reward_balance, blinding);
        self.submit(
            reference,
            OperationKind::CashOut,
            participant,
            accounts,
            instruction,
            json!({ "index": index, "receipt": codec::account_hex(&receipt), "points": me.reward_balance.to_string() }),
        )
    }

    /// The code the user pastes into the payout form: the receipt's opening (program,
    /// node, blinding factor), signed by the node so a copied code is useless to others.
    pub fn payout_code(&self, participant: &str, index: u64, keystore_path: &str) -> Result<Value> {
        let key = self.participant_key(participant, keystore_path)?;
        let (receipts, _) = self.scan_receipts(&key, participant)?;
        let receipt = receipts
            .iter()
            .find(|r| r.index == index)
            .ok_or_else(|| DomainError::new("unknown_receipt", "no receipt with this index"))?;
        let program = self.program()?;
        let blinding = cashout::blinding_factor(&key, program, codec::account(participant)?, index);

        let mut opening = BTreeMap::from([
            ("blinding_factor", hex::encode(blinding)),
            ("node", codec::node_hex(&key.node())),
            ("program_account", codec::account_hex(&program)),
        ]);
        let signed = serde_json::to_string(&opening).expect("strings serialize");
        opening.insert("signature", hex::encode(key.sign(signed.as_bytes())));
        let code = format!("{PAYOUT_PREFIX}{}", URL_SAFE_NO_PAD.encode(serde_json::to_string(&opening).expect("strings serialize")));
        Ok(json!({
            "code": code,
            "receipt": codec::account_hex(&receipt.account),
            "points": receipt.points.to_string(),
        }))
    }

    /// A settled cash-out's receipt joins the participant's known receipts.
    pub(super) fn remember_receipt(&mut self, operation: &Operation) -> Result<()> {
        let detail = &operation.detail;
        let (Some(index), Some(account), Some(points)) =
            (detail["index"].as_u64(), detail["receipt"].as_str(), detail["points"].as_str())
        else {
            return Ok(());
        };
        let record = self.deployment_mut()?.participants.entry(operation.participant.clone()).or_default();
        if !record.receipts.iter().any(|r| r.index == index) {
            record.receipts.push(KnownReceipt { index, account: account.to_owned(), points: points.to_owned() });
            record.receipts.sort_by_key(|r| r.index);
        }
        Ok(())
    }
}
