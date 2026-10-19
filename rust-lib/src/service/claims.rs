//! Claiming points: every published epoch the participant was active in, oldest first,
//! one batch (≤16 epochs) per transaction. A later batch is built once the earlier one
//! settles, so `claim` submits one batch and `auto_claim` keeps going on later polls.

use referral_core::{Participant, Registry};
use serde_json::{Value, json};

use super::saved::{OperationKind, OperationStatus};
use super::{Service, participant_id, derived_reference, port};
use crate::domain::claims::{self, Batch};
use crate::domain::{DomainError, Result, codec, instructions};
use crate::ports::AccountEntry;

/// The participant's notes, as `discover_notes` finds them.
type Notes = Vec<(lee_core::account::AccountId, referral_core::State)>;

struct ClaimPlan {
    me: Participant,
    batches: Vec<Batch>,
    notes: Notes,
    /// Published epochs whose active set isn't known yet, oldest first.
    missing: Vec<u32>,
}

/// Blocks `auto_claim` waits before sending a batch again after its `attempts`-th failure:
/// 10, 20, 40, … up to 640 (about 1-3 hours at LEZ's 5-15 s blocks).
fn retry_wait(attempts: u32) -> i64 {
    10 << attempts.saturating_sub(1).min(6)
}

impl Service {
    /// Everything claimable now, batched, and the published epochs left out because their
    /// active set isn't known well enough (see `claims::Plan`). Those can be paid later: the
    /// program settles epochs in any order, each once. Missing sets are asked of the indexer
    /// once. Afterwards, sets no participant's plan needs any more are dropped (`lists`).
    fn claim_plan(&mut self, participant: &str) -> Result<ClaimPlan> {
        let registry = self.read_registry()?;
        let me = match self.read_participant(participant) {
            Err(e) if e.code == "not_registered" => {
                // Nothing to claim, so nothing to keep a set for.
                self.planned(participant, registry.epoch, Vec::new())?;
                if self.prune(registry.epoch)? {
                    self.persist_lists()?;
                }
                return Err(e);
            }
            result => result?,
        };
        self.follow(participant, &me)?;
        let notes = self.discover_notes(participant, me.node)?;
        // A claim consumes the join notes, so their epochs are kept before it's built.
        self.remember_joins(participant, &notes)?;

        let mut plan = self.plan_once(&me, &registry, &notes)?;
        if !plan.missing.is_empty() && self.fill_publications(&plan.missing)? {
            plan = self.plan_once(&me, &registry, &notes)?;
        }
        let needs = plan.batches.iter().flatten().map(|e| e.epoch).collect();
        self.planned(participant, registry.epoch, needs)?;
        if self.keep_digests()? | self.prune(registry.epoch)? {
            self.persist_lists()?;
        }
        Ok(ClaimPlan { me, batches: plan.batches, notes, missing: plan.missing })
    }

    fn plan_once(&self, me: &Participant, registry: &Registry, notes: &Notes) -> Result<claims::Plan> {
        claims::plan(me, registry.epoch, notes.clone(), |epoch| {
            let Some(digest) = self.archived_digest(epoch)? else { return Ok(None) };
            Ok(Some(claims::Published { digest, known: self.known(registry, epoch, digest) }))
        })
    }

    /// Records what the participant's plan needs, for `prune`.
    fn planned(&mut self, participant: &str, through: u32, needs: Vec<u32>) -> Result<()> {
        let Some(record) = self.deployment_mut()?.participants.get_mut(participant) else { return Ok(()) };
        if (record.planned_through, &record.needs) == (Some(through), &needs) {
            return Ok(());
        }
        (record.planned_through, record.needs) = (Some(through), needs);
        self.persist()
    }

    /// The points claimable now, by batch, and `history`: `null`, or which published epochs
    /// couldn't be counted and why (see `history_gap`).
    pub fn claimable(&mut self, participant: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        let plan = self.claim_plan(participant)?;
        Ok(json!({
            "points": claims::total(&plan.batches).to_string(),
            "batches": plan.batches.iter().map(|batch| json!({
                "epochs": batch.iter().map(|e| e.epoch).collect::<Vec<_>>(),
                "points": batch.iter().map(|e| e.total).sum::<u128>().to_string(),
            })).collect::<Vec<_>>(),
            "history": self.history_gap(&plan.missing),
        }))
    }

    /// Submits the oldest batch. Call again (or use `auto_claim`) once it settles for the next.
    pub fn claim(&mut self, reference: &str, participant: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        if let Some(existing) = self.replay(reference, OperationKind::Claim, participant)? {
            return Ok(existing);
        }
        self.require_no_other_open(participant, reference)?;
        let plan = self.claim_plan(participant)?;
        let Some(batch) = plan.batches.first() else {
            return Err(self.nothing_claimable(&plan.missing));
        };
        self.submit_batch(reference, participant, &plan.me, batch, &plan.notes)
    }

    fn submit_batch(&mut self, reference: &str, participant: &str, me: &Participant, batch: &Batch, notes: &Notes) -> Result<Value> {
        let (claim, forwards) =
            claims::build(me, batch, |id| notes.iter().find(|(note, _)| note == id).map(|(_, state)| state.clone()))?;

        let program = self.program()?;
        let referrer_entry = me.referrer.as_ref().map(|referrer| AccountEntry::PrivateForeign {
            npk: hex::encode(referrer.npk.0),
            vpk: hex::encode(referrer.vpk.to_bytes()),
        });
        let mut accounts = vec![AccountEntry::Own(participant.to_owned())];
        for (planned, forwards) in batch.iter().zip(&forwards) {
            accounts.push(AccountEntry::PublicNoSign(codec::account_hex(&referral_core::epoch_account(program, planned.epoch))));
            accounts.extend(planned.notes.iter().map(|id| AccountEntry::Own(codec::account_hex(id))));
            if *forwards {
                accounts.push(referrer_entry.clone().ok_or_else(|| DomainError::new("bad_participant", "a credit to forward but no referrer"))?);
            }
        }
        let detail = json!({
            "epochs": batch.iter().map(|e| e.epoch).collect::<Vec<_>>(),
            "points": claim.epochs.iter().map(|e| e.total).sum::<u128>().to_string(),
            // To tell later, whatever became of the transaction, whether it landed.
            "balanceBefore": me.reward_balance.to_string(),
        });
        self.submit(reference, OperationKind::Claim, participant, accounts, instructions::claim(claim), detail)
    }

    /// Claims whatever is claimable, one batch at a time. Safe to call on every poll: it does
    /// nothing while an operation is pending or when there is nothing to claim. Points are
    /// never lost by waiting; each epoch is paid once, whenever it's claimed.
    ///
    /// Each batch has one reference (from its epochs), so a failed batch is retried under it,
    /// waiting longer after each failure (`retry_wait`): proving takes minutes, and a claim
    /// that keeps failing shouldn't be proved again on every poll.
    pub fn auto_claim(&mut self, participant: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        self.refresh_open(participant)?;
        if self.open_operation(participant).is_some() {
            return Ok(json!({ "skipped": "in_flight" }));
        }
        let plan = self.claim_plan(participant)?;
        let Some(batch) = plan.batches.first() else {
            // Not an error on a poll: the gap closes when the indexer has the epochs.
            return Ok(match self.history_gap(&plan.missing) {
                Value::Null => json!({ "skipped": "nothing_to_claim" }),
                gap => json!({ "skipped": gap["code"], "history": gap }),
            });
        };
        let epochs: Vec<u8> = batch.iter().flat_map(|e| e.epoch.to_le_bytes()).collect();
        let reference = derived_reference(&[b"auto-claim", participant.as_bytes(), &epochs]);
        if let Some(failed) = self.deployment().operations.get(&reference).filter(|op| op.status == OperationStatus::Failed) {
            let retry_at = failed.failed_at_block.unwrap_or(0) + retry_wait(failed.attempts);
            if self.lez.get_current_block_height().map_err(port)? < retry_at {
                return Ok(json!({ "skipped": "backoff", "reference": reference, "error": failed.error, "retryAtBlock": retry_at }));
            }
        }
        self.submit_batch(&reference, participant, &plan.me, batch, &plan.notes).map(|operation| json!({ "claimed": operation }))
    }

    /// Why `missing` epochs couldn't be counted, or `null` when none are missing:
    /// - `history_syncing`: the indexer is still catching up; they'll come.
    /// - `history_needed`: no indexer is running (or it failed, or it has caught up without
    ///   them); start `lez_indexer_module`, e.g. from the LEZ Explorer app.
    pub(super) fn history_gap(&self, missing: &[u32]) -> Value {
        if missing.is_empty() {
            return Value::Null;
        }
        let status = self.history.as_ref().and_then(|h| h.status().ok()).and_then(|s| serde_json::from_str::<Value>(&s).ok());
        let state = status.as_ref().and_then(|s| s["state"].as_str()).unwrap_or("NotRunning");
        let code = if matches!(state, "Starting" | "Syncing") { "history_syncing" } else { "history_needed" };
        json!({
            "code": code,
            "missingEpochs": missing,
            "indexer": {
                "state": state,
                "indexedBlock": status.as_ref().map_or(Value::Null, |s| s["indexed_block_id"].clone()),
                "error": status.as_ref().map_or(Value::Null, |s| s["last_error"].clone()),
            },
        })
    }

    /// The error for a claim with no batch: nothing to claim, or only epochs still unknown.
    fn nothing_claimable(&self, missing: &[u32]) -> DomainError {
        match self.history_gap(missing) {
            Value::Null => DomainError::new("nothing_to_claim", "nothing to claim"),
            gap if gap["code"] == "history_syncing" => {
                DomainError::new("history_syncing", format!("the indexer is still catching up to epochs {missing:?}"))
            }
            _ => DomainError::new(
                "history_needed",
                format!("epochs {missing:?} need their active sets: start lez_indexer_module (the LEZ Explorer app)"),
            ),
        }
    }
}
