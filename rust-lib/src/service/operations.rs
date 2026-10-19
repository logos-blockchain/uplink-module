//! Submitting transactions and following them until they settle.

use serde_json::{Value, json};

use super::saved::{Operation, OperationKind, OperationStatus};
use super::{Service, port, reference_id};
use crate::domain::{DomainError, Result, codec};
use crate::ports::AccountEntry;

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
        let reference = reference_id(reference)?;
        if let Some(existing) = self.deployment().operations.get(&reference).filter(|op| op.status != OperationStatus::Failed) {
            if existing.kind != kind || !existing.participant.eq_ignore_ascii_case(participant) {
                return Err(DomainError::new("conflict", "this reference was used for another operation"));
            }
            return Ok(operation_json(existing));
        }
        let program_kind = json!({ "kind": "disclosed", "account_id": codec::account_hex(&self.verified_program()?) }).to_string();

        let created_at_ms = self.clock.now_ms();
        self.deployment_mut()?.operations.insert(
            reference.clone(),
            Operation {
                reference: reference.clone(),
                kind,
                participant: participant.to_lowercase(),
                status: OperationStatus::Submitting,
                tx_hash: None,
                error: None,
                created_at_ms,
                detail,
            },
        );
        self.persist()?;

        let reply = self.lez.send_private_transaction(&accounts, &instruction, self.binary.elf, &program_kind);
        let operation = self.deployment_mut()?.operations.get_mut(&reference).expect("just recorded");
        match reply.map(|reply| serde_json::from_str::<Value>(&reply).unwrap_or(Value::Null)) {
            Ok(reply) if reply.get("success").and_then(Value::as_bool) == Some(true) => {
                operation.status = OperationStatus::Pending;
                operation.tx_hash = reply.get("tx_hash").and_then(Value::as_str).map(str::to_owned);
            }
            Ok(reply) => {
                operation.status = OperationStatus::Failed;
                operation.error = Some(reply.get("error").and_then(Value::as_str).unwrap_or("rejected").to_owned());
            }
            Err(e) => {
                operation.status = OperationStatus::Failed;
                operation.error = Some(e);
            }
        }
        let result = operation_json(operation);
        self.persist()?;
        Ok(result)
    }

    /// Brings the participant's open operations up to date: pending ones settle once their
    /// transaction is in, and one still "submitting" was cut off before `lez_core` answered
    /// (submitting finishes within one call), so it fails and may be retried. Retrying is safe:
    /// the program refuses a second Register, an epoch already paid, or a zero cash-out.
    pub(super) fn refresh_open(&mut self, participant: &str) -> Result<()> {
        let open: Vec<(String, OperationStatus)> = self
            .deployment()
            .operations
            .values()
            .filter(|op| op.is_open() && op.participant.eq_ignore_ascii_case(participant))
            .map(|op| (op.reference.clone(), op.status))
            .collect();
        for (reference, status) in open {
            if status == OperationStatus::Submitting {
                let operation = self.deployment_mut()?.operations.get_mut(&reference).expect("exists");
                operation.status = OperationStatus::Failed;
                operation.error = Some("interrupted before lez_core answered; retry".into());
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
            .find(|op| op.is_open() && op.participant.eq_ignore_ascii_case(participant))
    }

    /// Settles a pending operation once its transaction is in.
    pub fn status(&mut self, reference: &str) -> Result<Value> {
        let reference = reference_id(reference)?;
        let Some(operation) = self.deployment().operations.get(&reference).cloned() else {
            return Err(DomainError::new("unknown_reference", "no operation with this reference"));
        };
        let settled = match (&operation.status, &operation.tx_hash) {
            (OperationStatus::Pending, Some(tx_hash)) => self.lez.poll_transaction_status(tx_hash).map_err(port)?,
            _ => false,
        };
        if settled {
            self.deployment_mut()?.operations.get_mut(&reference).expect("exists").status = OperationStatus::Settled;
            if operation.kind == OperationKind::CashOut {
                self.remember_receipt(&operation)?;
            }
            self.persist()?;
        }
        Ok(operation_json(&self.deployment().operations[&reference]))
    }

    pub fn operations(&self, participant: &str) -> Value {
        json!({
            "operations": self
                .deployment()
                .operations
                .values()
                .filter(|op| participant.is_empty() || op.participant.eq_ignore_ascii_case(participant))
                .map(operation_json)
                .collect::<Vec<_>>()
        })
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
        "detail": op.detail,
    })
}
