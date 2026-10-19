//! Claiming points: every published epoch the participant was active in, oldest first,
//! one batch (≤16 epochs) per transaction. A later batch is built once the earlier one
//! settles, so `claim` submits one batch and `auto_claim` keeps going on later polls.

use std::collections::BTreeSet;

use referral_core::{NodeId, Participant, Registry};
use serde_json::{Value, json};

use super::saved::{OperationKind, OperationStatus};
use super::{Service, participant_id, derived_reference, port};
use crate::domain::claims::{self, Batch};
use crate::domain::{DomainError, Result, codec, instructions};
use crate::ports::AccountEntry;

/// The participant's notes, as `discover_notes` finds them.
type Notes = Vec<(lee_core::account::AccountId, referral_core::State)>;

/// Blocks `auto_claim` waits before sending a batch again after its `attempts`-th failure:
/// 10, 20, 40, … up to 640 (about 1-3 hours at LEZ's 5-15 s blocks).
fn retry_wait(attempts: u32) -> i64 {
    10 << attempts.saturating_sub(1).min(6)
}

impl Service {
    fn claim_plan(&mut self, participant: &str) -> Result<(Participant, Vec<Batch>, Notes)> {
        let me = self.read_participant(participant)?;
        let registry = self.read_registry()?;
        let notes = self.discover_notes(participant, me.node)?;
        // A claim consumes the join notes, so their epochs are kept before it's built.
        self.remember_joins(participant, &notes)?;
        let mut lookup_error = None;
        let batches = claims::plan(&me, registry.epoch, notes.clone(), |epoch| {
            self.published_active(&registry, epoch).map_err(|e| {
                lookup_error = Some(e.clone());
                e
            })
        });
        let batches = batches.map_err(|e| lookup_error.unwrap_or(e))?;
        Ok((me, batches, notes))
    }

    pub fn claimable(&mut self, participant: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        let (_, batches, _) = self.claim_plan(participant)?;
        Ok(json!({
            "points": claims::total(&batches).to_string(),
            "batches": batches.iter().map(|batch| json!({
                "epochs": batch.iter().map(|e| e.epoch).collect::<Vec<_>>(),
                "points": batch.iter().map(|e| e.total).sum::<u128>().to_string(),
            })).collect::<Vec<_>>(),
        }))
    }

    /// Submits the oldest batch. Call again (or use `auto_claim`) once it settles for the next.
    pub fn claim(&mut self, reference: &str, participant: &str) -> Result<Value> {
        let participant = &participant_id(participant)?;
        if let Some(existing) = self.replay(reference, OperationKind::Claim, participant)? {
            return Ok(existing);
        }
        self.require_no_other_open(participant, reference)?;
        let (me, batches, notes) = self.claim_plan(participant)?;
        let Some(batch) = batches.into_iter().next() else {
            return Err(DomainError::new("nothing_to_claim", "nothing to claim"));
        };
        self.submit_batch(reference, participant, &me, &batch, &notes)
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
        let (me, batches, notes) = self.claim_plan(participant)?;
        let Some(batch) = batches.into_iter().next() else {
            return Ok(json!({ "skipped": "nothing_to_claim" }));
        };
        let epochs: Vec<u8> = batch.iter().flat_map(|e| e.epoch.to_le_bytes()).collect();
        let reference = derived_reference(&[b"auto-claim", participant.as_bytes(), &epochs]);
        if let Some(failed) = self.deployment().operations.get(&reference).filter(|op| op.status == OperationStatus::Failed) {
            let retry_at = failed.failed_at_block.unwrap_or(0) + retry_wait(failed.attempts);
            if self.lez.get_current_block_height().map_err(port)? < retry_at {
                return Ok(json!({ "skipped": "backoff", "reference": reference, "error": failed.error, "retryAtBlock": retry_at }));
            }
        }
        self.submit_batch(&reference, participant, &me, &batch, &notes).map(|operation| json!({ "claimed": operation }))
    }

    /// The active set of published `epoch`, checked against its archived publication account,
    /// or `None` if the epoch was never published.
    fn published_active(&mut self, registry: &Registry, epoch: u32) -> Result<Option<BTreeSet<NodeId>>> {
        let Some(digest) = self.archived_digest(epoch)? else { return Ok(None) };
        if let Some(active) = self.known_active(registry, epoch, digest) {
            return Ok(Some(active));
        }
        self.fill_publications()?;
        self.publication((epoch, digest)).map(Some).ok_or_else(|| {
            DomainError::new("history_needed", format!("epoch {epoch}'s active set needs lez_indexer_module"))
        })
    }
}
