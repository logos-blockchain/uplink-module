//! What a participant can claim. Ported from logos-blockchain/uplink's wallet (`batches`,
//! `batch_claims`, `first_unsettled`, `claim_request`).
//!
//! Every published epoch since the participant's earliest unsettled one is claimable, oldest
//! first, and only those its own node was active in. Each child is settled once per epoch,
//! so epochs may be claimed late. Announced children are adopted in the first epoch claimed.
//! Claims are capped at [`MAX_CLAIM_EPOCHS`] epochs and [`MAX_CLAIM_ACTIVE_ENTRIES`]
//! active-set entries; later batches are built after the earlier ones settle.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

use lee_core::account::AccountId;
use referral_core::{Claim, Epoch, EpochClaim, NodeId, Participant, Points, State, active_digest};

use super::{DomainError, Result};

pub const MAX_CLAIM_EPOCHS: usize = 16;
pub const MAX_CLAIM_ACTIVE_ENTRIES: usize = 2_048;

/// One epoch of a claim: the notes it consumes and what it pays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochPlan {
    pub epoch: Epoch,
    pub active: BTreeSet<NodeId>,
    pub notes: Vec<AccountId>,
    pub total: Points,
}

pub type Batch = Vec<EpochPlan>;

/// Notes addressed to `me`: children announced to it and credits for it.
pub fn addressed_to(me: NodeId, notes: Vec<(AccountId, State)>) -> Vec<(AccountId, State)> {
    notes
        .into_iter()
        .filter(|(_, state)| match *state {
            State::Child { referrer, .. } => referrer == me,
            State::Credit { recipient_node, .. } => recipient_node == me,
            _ => false,
        })
        .collect()
}

/// The first epoch not yet settled for a child, given its settled ranges. Epoch 0 is never
/// published, so it counts as settled.
pub fn first_unsettled(settled: &[(Epoch, Epoch)]) -> Epoch {
    let mut next: Epoch = 1;
    for &(first, last) in settled {
        if first > next {
            break;
        }
        next = next.max(last.saturating_add(1));
    }
    next
}

fn settle(participant: &mut Participant, claim: &EpochClaim) -> Result<Points> {
    catch_unwind(AssertUnwindSafe(|| participant.claim(claim)))
        .map_err(|_| DomainError::new("rejected", format!("the program's rules reject a claim for epoch {}", claim.epoch)))
}

/// Plans every claimable epoch, batched. `published(epoch)` is the epoch's active set, or
/// `None` if it was never published.
pub fn plan(
    claimant: &Participant,
    current_epoch: Epoch,
    notes: Vec<(AccountId, State)>,
    mut published: impl FnMut(Epoch) -> Result<Option<BTreeSet<NodeId>>>,
) -> Result<Vec<Batch>> {
    let notes = addressed_to(claimant.node, notes);
    let earliest = claimant
        .children
        .values()
        .map(|settled| first_unsettled(settled))
        .chain(notes.iter().map(|(_, state)| match *state {
            State::Child { since, .. } => since,
            State::Credit { epoch, .. } => epoch,
            _ => Epoch::MAX,
        }))
        .min();
    let Some(earliest) = earliest else { return Ok(Vec::new()) };

    let (children, credits): (Vec<_>, Vec<_>) =
        notes.into_iter().partition(|(_, state)| matches!(state, State::Child { .. }));
    let mut simulated = claimant.clone();
    for (_, child) in &children {
        if let State::Child { node, since, .. } = *child {
            simulated.adopt(node, since);
        }
    }

    let mut planned: Vec<EpochPlan> = Vec::new();
    for epoch in earliest.max(1)..=current_epoch {
        let Some(active) = published(epoch)? else { continue };
        if !active.contains(&claimant.node) {
            continue;
        }
        let digest = active_digest(&active);
        let (ids, states): (Vec<AccountId>, Vec<State>) = credits
            .iter()
            .filter(|(_, state)| {
                matches!(*state, State::Credit { epoch: earned, active_digest: under, .. } if (earned, under) == (epoch, digest))
            })
            .cloned()
            .unzip();
        let total = settle(&mut simulated, &EpochClaim { epoch, active: active.clone(), notes: states, total: 0 })?;
        if total > 0 {
            planned.push(EpochPlan { epoch, active, notes: ids, total });
        }
    }
    if let Some(first) = planned.first_mut() {
        first.notes.splice(0..0, children.iter().map(|(id, _)| *id));
    }
    Ok(batch(planned))
}

/// Splits planned epochs, in order, into claims of at most [`MAX_CLAIM_EPOCHS`] epochs and
/// [`MAX_CLAIM_ACTIVE_ENTRIES`] entries. An epoch over the entry cap alone still gets one.
pub fn batch(planned: Vec<EpochPlan>) -> Vec<Batch> {
    let mut batches = Vec::new();
    let mut current: Batch = Vec::new();
    let mut entries = 0usize;
    for epoch in planned {
        let full = current.len() >= MAX_CLAIM_EPOCHS || entries.saturating_add(epoch.active.len()) > MAX_CLAIM_ACTIVE_ENTRIES;
        if full && !current.is_empty() {
            batches.push(std::mem::take(&mut current));
            entries = 0;
        }
        entries = entries.saturating_add(epoch.active.len());
        current.push(epoch);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// The `Claim` for a batch against the participant's current state, and for each epoch
/// whether it forwards a credit to the referrer. `note(id)` is a note addressed to it.
pub fn build(
    claimant: &Participant,
    batch: &[EpochPlan],
    note: impl Fn(&AccountId) -> Option<State>,
) -> Result<(Claim, Vec<bool>)> {
    let mut simulated = claimant.clone();
    let mut epochs = Vec::with_capacity(batch.len());
    let mut forwards = Vec::with_capacity(batch.len());
    for planned in batch {
        if !planned.active.contains(&claimant.node) {
            return Err(DomainError::new("not_active", "the participant cannot claim that epoch"));
        }
        let notes = planned
            .notes
            .iter()
            .map(|id| note(id).ok_or_else(|| DomainError::new("unknown_note", "a note is not addressed to the participant")))
            .collect::<Result<Vec<_>>>()?;
        let mut epoch_claim = EpochClaim { epoch: planned.epoch, active: planned.active.clone(), notes, total: 0 };
        epoch_claim.total = settle(&mut simulated, &epoch_claim)?;
        if epoch_claim.total == 0 {
            return Err(DomainError::new("nothing_to_claim", format!("nothing to claim in epoch {}", planned.epoch)));
        }
        forwards.push(epoch_claim.forward_to(claimant.referrer_node()).is_some());
        epochs.push(epoch_claim);
    }
    Ok((Claim { epochs, referrer: claimant.referrer_node() }, forwards))
}

pub fn total(batches: &[Batch]) -> Points {
    batches.iter().flatten().map(|epoch| epoch.total).sum()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    const ME: NodeId = NodeId::new([7; 32]);
    const A: NodeId = NodeId::new([1; 32]);
    const B: NodeId = NodeId::new([2; 32]);

    fn active(nodes: &[NodeId]) -> BTreeSet<NodeId> {
        nodes.iter().copied().collect()
    }

    #[test]
    fn late_claims_pay_every_epoch_the_child_and_i_were_active_in() {
        let me = Participant::new(ME, None);
        let notes = vec![(AccountId::new([11; 32]), State::Child { node: A, referrer: ME, since: 2 })];
        let history = BTreeMap::from([(1, active(&[ME, A])), (2, active(&[ME, A])), (3, active(&[A])), (4, active(&[ME, A]))]);
        let batches = plan(&me, 4, notes.clone(), |epoch| Ok(history.get(&epoch).cloned())).unwrap();

        // Epoch 1 is before A registered; epoch 3 I wasn't active.
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].iter().map(|e| e.epoch).collect::<Vec<_>>(), vec![2, 4]);
        assert_eq!(total(&batches), 2);
        assert_eq!(batches[0][0].notes, vec![AccountId::new([11; 32])], "children are adopted in the first epoch");

        let lookup = |id: &AccountId| notes.iter().find(|(n, _)| n == id).map(|(_, s)| s.clone());
        let (claim, forwards) = build(&me, &batches[0], lookup).unwrap();
        assert_eq!(claim.epochs.iter().map(|e| e.total).collect::<Vec<_>>(), vec![1, 1]);
        assert_eq!(forwards, vec![false, false]);
    }

    #[test]
    fn already_settled_epochs_pay_nothing() {
        let mut me = Participant::new(ME, None);
        me.children.insert(A, vec![(0, 3)]);
        let batches = plan(&me, 3, vec![], |_| Ok(Some(active(&[ME, A])))).unwrap();
        assert!(batches.is_empty());
        assert_eq!(first_unsettled(&[(0, 3)]), 4);
        assert_eq!(first_unsettled(&[]), 1);
    }

    #[test]
    fn credits_are_collected_in_the_epoch_they_were_earned() {
        let me = Participant::new(ME, None);
        let set = active(&[ME, B]);
        let credit = State::Credit { recipient_node: ME, amount: 4, epoch: 2, active_digest: active_digest(&set) };
        let batches = plan(&me, 5, vec![(AccountId::new([12; 32]), credit)], |epoch| Ok((epoch == 2).then(|| set.clone()))).unwrap();
        assert_eq!(total(&batches), 4);
        assert_eq!(batches[0][0].epoch, 2);
    }

    #[test]
    fn batches_respect_the_epoch_cap() {
        let planned: Vec<EpochPlan> = (1..=20)
            .map(|epoch| EpochPlan { epoch, active: active(&[ME]), notes: vec![], total: 1 })
            .collect();
        let batches = batch(planned);
        assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), vec![16, 4]);
    }

    #[test]
    fn notes_for_someone_else_are_ignored() {
        let others = vec![(AccountId::new([14; 32]), State::Child { node: A, referrer: B, since: 1 })];
        assert!(addressed_to(ME, others).is_empty());
    }
}
