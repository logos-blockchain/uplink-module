//! Submitting transactions and following them until they settle.

use serde_json::{Value, json};

use super::saved::{Operation, OperationKind, OperationStatus};
use super::{Service, participant_id, reference_id};
use crate::domain::{DomainError, Result, codec};
use crate::ports::AccountEntry;

/// How many blocks a pending transaction may stay out of the chain before it counts as
/// dropped. LEZ makes a block every 5-15 s, so this is about 8-25 minutes; including one
/// takes a block or two.
pub(super) const INCLUSION_BLOCKS: i64 = 100;

impl Service {
    /// Records the operation, hands the transaction to `lez_core`, and records the result.
    /// A reference that is already open or settled returns its record instead of resending.
    pub(super) fn submit(
        &mut self,
        reference: &str,
        kind: OperationKind,
        participant: &str,
        accounts: Vec<AccountEntry>,
        instruction: Vec<u8>,
        detail: Value,
    ) -> Result<Value> {
        if let Some(existing) = self.replay(reference, kind, participant)? {
            return Ok(existing);
        }
        let reference = reference_id(reference)?;
        let program_kind = json!({ "kind": "disclosed", "account_id": codec::account_hex(&self.verified_program()?) }).to_string();

        let created_at_ms = self.clock.now_ms();
        let attempts = self.deployment().operations.get(&reference).map_or(0, |failed| failed.attempts) + 1;
        self.deployment_mut()?.operations.insert(
            reference.clone(),
            Operation {
                reference: reference.clone(),
                kind,
                participant: participant.to_owned(),
                status: OperationStatus::Submitting,
                tx_hash: None,
                error: None,
                created_at_ms,
                pending_since_block: None,
                attempts,
                failed_at_block: None,
                detail,
            },
        );
        self.persist()?;

        let reply = self.lez.send_private_transaction(&accounts, &instruction, self.binary.elf, &program_kind);
        // Unknown if the read fails; the first status check then starts the count.
        let height = self.lez.get_current_block_height().ok();
        let operation = self.deployment_mut()?.operations.get_mut(&reference).expect("just recorded");
        match reply.map(|reply| serde_json::from_str::<Value>(&reply).unwrap_or(Value::Null)) {
            Ok(reply) if reply.get("success").and_then(Value::as_bool) == Some(true) => {
                operation.status = OperationStatus::Pending;
                operation.tx_hash = reply.get("tx_hash").and_then(Value::as_str).map(str::to_owned);
                operation.pending_since_block = height;
            }
            Ok(reply) => operation.fail(reply.get("error").and_then(Value::as_str).unwrap_or("rejected").to_owned(), height),
            Err(e) => operation.fail(e, height),
        }
        let result = operation_json(operation);
        self.persist()?;
        Ok(result)
    }

    /// The record of an operation already made under `reference` (brought up to date), unless
    /// it failed and the chain doesn't show it landed. Each operation checks this first: once
    /// one settles, the checks before sending ("already registered", "nothing to claim") would
    /// refuse the same request.
    pub(super) fn replay(&mut self, reference: &str, kind: OperationKind, participant: &str) -> Result<Option<Value>> {
        let reference = reference_id(reference)?;
        let Some(existing) = self.deployment().operations.get(&reference).cloned() else { return Ok(None) };
        if existing.status == OperationStatus::Failed {
            // A failed send may have landed after all (lez_core timed out but went on): look
            // before it's sent again.
            if existing.kind != kind || existing.participant != participant || !self.landed(&existing) {
                return Ok(None);
            }
            self.settle(&reference, &existing)?;
            return Ok(Some(operation_json(&self.deployment().operations[&reference])));
        }
        if existing.kind != kind || existing.participant != participant {
            return Err(DomainError::new("conflict", "this reference was used for another operation"));
        }
        self.status(&reference).map(Some)
    }

    /// Brings the participant's open operations up to date: pending ones settle once their
    /// transaction is in, and one still "submitting" was cut off before `lez_core` answered
    /// (submitting finishes within one call): it settles if the chain shows it landed, else
    /// fails and may be retried. The program also refuses a second Register, an epoch already
    /// paid, or a second receipt, so even a blind retry does no harm.
    pub(super) fn refresh_open(&mut self, participant: &str) -> Result<()> {
        let open: Vec<(String, OperationStatus)> = self
            .deployment()
            .operations
            .values()
            .filter(|op| op.is_open() && op.participant == participant)
            .map(|op| (op.reference.clone(), op.status))
            .collect();
        for (reference, status) in open {
            if status == OperationStatus::Submitting {
                // Cut off before lez_core answered: it may still have gone through.
                let operation = self.deployment().operations[&reference].clone();
                if self.landed(&operation) {
                    self.settle(&reference, &operation)?;
                    continue;
                }
                let height = self.lez.get_current_block_height().ok();
                let operation = self.deployment_mut()?.operations.get_mut(&reference).expect("exists");
                operation.fail("interrupted before lez_core answered; retry".into(), height);
                self.persist()?;
            } else {
                self.status(&reference)?;
            }
        }
        Ok(())
    }

    /// Only one operation per participant may be open; resubmitting the same reference is fine.
    pub(super) fn require_no_other_open(&mut self, participant: &str, reference: &str) -> Result<()> {
        self.refresh_open(participant)?;
        match self.open_operation(participant) {
            Some(open) if !open.reference.eq_ignore_ascii_case(reference.trim().trim_start_matches("0x")) => {
                Err(DomainError::new("in_flight", "another operation for this participant is still pending"))
            }
            _ => Ok(()),
        }
    }

    pub(super) fn open_operation(&self, participant: &str) -> Option<&Operation> {
        self.deployment()
            .operations
            .values()
            .find(|op| op.is_open() && op.participant == participant)
    }

    /// Settles a pending operation once its transaction is in or the chain shows its outcome,
    /// or fails it once it has stayed out for `INCLUSION_BLOCKS` (the sequencer dropped it), so
    /// the participant isn't blocked. Never waits for the transaction.
    pub fn status(&mut self, reference: &str) -> Result<Value> {
        let reference = reference_id(reference)?;
        let Some(operation) = self.deployment().operations.get(&reference).cloned() else {
            return Err(DomainError::new("unknown_reference", "no operation with this reference"));
        };
        let settled = operation.status == OperationStatus::Pending
            && (operation.tx_hash.as_deref().is_some_and(|tx_hash| self.lez.transaction_included(tx_hash))
                || self.landed(&operation));
        if !settled && operation.status == OperationStatus::Pending {
            return self.check_dropped(&reference, &operation);
        }
        if settled {
            self.settle(&reference, &operation)?;
        }
        Ok(operation_json(&self.deployment().operations[&reference]))
    }

    fn settle(&mut self, reference: &str, operation: &Operation) -> Result<()> {
        let op = self.deployment_mut()?.operations.get_mut(reference).expect("exists");
        op.status = OperationStatus::Settled;
        op.error = None;
        if operation.kind == OperationKind::CashOut {
            self.remember_receipt(operation)?;
        }
        self.persist()
    }

    /// Whether the chain shows the operation's effect, whatever became of its transaction:
    /// the participant registered, its balance up by the claim's points (one operation per
    /// participant is open at a time, so nothing else moves it meanwhile), or the receipt
    /// created. Used wherever the outcome is uncertain, so a send that landed is never
    /// reported failed or sent again.
    fn landed(&self, operation: &Operation) -> bool {
        let detail = &operation.detail;
        match operation.kind {
            OperationKind::Register => self.read_participant(&operation.participant).is_ok(),
            OperationKind::Claim => {
                let (Some(before), Some(points)) = (
                    detail["balanceBefore"].as_str().and_then(|b| b.parse::<u128>().ok()),
                    detail["points"].as_str().and_then(|p| p.parse::<u128>().ok()),
                ) else {
                    return false;
                };
                self.read_participant(&operation.participant).is_ok_and(|me| me.reward_balance >= before + points)
            }
            OperationKind::CashOut => detail["receipt"].as_str().is_some_and(|receipt| {
                let Ok(program) = self.program() else { return false };
                self.lez
                    .get_account_public(receipt)
                    .ok()
                    .filter(|json| !json.is_empty())
                    .and_then(|json| codec::shard(&json, &program).ok())
                    .is_some_and(|data| crate::domain::state::receipt_points(&data).is_some())
            }),
        }
    }

    /// A pending transaction no block has included yet: starts or checks its deadline.
    fn check_dropped(&mut self, reference: &str, operation: &Operation) -> Result<Value> {
        // lez_core busy (it answers one call at a time): still pending, checked again next time.
        let Ok(height) = self.lez.get_current_block_height() else {
            return Ok(operation_json(operation));
        };
        let Some(since) = operation.pending_since_block else {
            // Recorded before the height was known (or by an older version): count from now.
            self.deployment_mut()?.operations.get_mut(reference).expect("exists").pending_since_block = Some(height);
            self.persist()?;
            return Ok(operation_json(operation));
        };
        // A lower height means the chain was reset under the transaction.
        if height >= since && height - since <= INCLUSION_BLOCKS {
            return Ok(operation_json(operation));
        }
        if self.landed(operation) {
            self.settle(reference, operation)?;
        } else {
            let op = self.deployment_mut()?.operations.get_mut(reference).expect("exists");
            op.fail(format!("no block included it in {INCLUSION_BLOCKS} blocks; retry"), Some(height));
            self.persist()?;
        }
        Ok(operation_json(&self.deployment().operations[reference]))
    }

    /// The participant's operations; `""` for every participant's.
    pub fn operations(&self, participant: &str) -> Result<Value> {
        let participant = if participant.trim().is_empty() { None } else { Some(participant_id(participant)?) };
        Ok(json!({
            "operations": self
                .deployment()
                .operations
                .values()
                .filter(|op| participant.as_ref().is_none_or(|p| &op.participant == p))
                .map(operation_json)
                .collect::<Vec<_>>()
        }))
    }
}

pub(super) fn operation_json(op: &Operation) -> Value {
    json!({
        "reference": op.reference,
        "kind": op.kind,
        "participant": op.participant,
        "status": op.status,
        "txHash": op.tx_hash,
        "error": op.error,
        "attempts": op.attempts,
        "detail": op.detail,
    })
}
