//! Sends whose outcome is uncertain are settled from what the chain shows, never failed or
//! sent again when they landed.

use super::*;
use crate::service::operations::INCLUSION_BLOCKS;

/// A participant with one claimable epoch (a child active in epoch 2) and `balance` points.
fn claimable(world: &World, balance: u128) -> NodeId {
    let me = node_key().node();
    let child = NodeId::new([3; 32]);
    let mut participant = Participant::new(me, Some(referrer()));
    participant.reward_balance = balance;
    world.set_participant(participant);
    world.set_registry(2, &[REFERRER, me, child], &[me, child]);
    world.add_note(0x31, State::Child { node: child, referrer: me, since: 2 });
    me
}

fn with_balance(world: &World, me: NodeId, balance: u128) {
    let mut participant = Participant::new(me, Some(referrer()));
    participant.reward_balance = balance;
    world.set_participant(participant);
}

#[test]
fn a_claim_cut_off_mid_send_that_landed_settles_instead_of_failing() {
    let world = World::new();
    let me = claimable(&world, 5);
    world.service().claim(&reference(1), PARTICIPANT).unwrap();
    // As if the module had stopped before lez_core answered, and the claim went through.
    let saved = world.store.load().unwrap().unwrap().replace("\"pending\"", "\"submitting\"");
    world.store.save(&saved).unwrap();
    with_balance(&world, me, 6);

    let mut restarted = world.service();
    restarted.refresh_open(PARTICIPANT).unwrap();
    assert_eq!(restarted.status(&reference(1)).unwrap()["status"], "settled");
}

#[test]
fn a_cash_out_reported_failed_whose_receipt_exists_is_not_sent_again() {
    let world = World::new();
    let me = node_key().node();
    with_balance(&world, me, 9);
    let mut service = world.service();
    // lez_core gave up (a timeout), but the transaction went on and landed.
    world.lez.0.borrow_mut().reply = Some(json!({ "success": false, "error": "timed out" }).to_string());
    let failed = service.cash_out(&reference(4), PARTICIPANT, "/node/keystore.yaml").unwrap();
    assert_eq!(failed["status"], "failed");
    let receipt = failed["detail"]["receipt"].as_str().unwrap().to_owned();
    world.lez.0.borrow_mut().public.insert(receipt, account_json(&State::CashOut { points: 9 }.to_data().into_inner()));
    with_balance(&world, me, 0);

    world.lez.0.borrow_mut().reply = None;
    assert_eq!(service.cash_out(&reference(4), PARTICIPANT, "/node/keystore.yaml").unwrap()["status"], "settled");
    assert_eq!(world.sent().len(), 1, "not sent again");
    assert_eq!(service.receipts(PARTICIPANT, "").unwrap()["receipts"][0]["points"], "9");
}

#[test]
fn a_claim_past_its_deadline_that_landed_settles() {
    let world = World::new();
    let me = claimable(&world, 5);
    let mut service = world.service();
    service.claim(&reference(1), PARTICIPANT).unwrap();
    with_balance(&world, me, 6);
    world.lez.0.borrow_mut().height = INCLUSION_BLOCKS + 1;
    assert_eq!(service.status(&reference(1)).unwrap()["status"], "settled");
}

#[test]
fn a_pending_claim_settles_from_its_points_when_its_transaction_cant_be_looked_up() {
    // No indexer has the transaction, but the balance shows the claim's points.
    let world = World::new();
    let me = claimable(&world, 5);
    let mut service = world.service();
    service.claim(&reference(1), PARTICIPANT).unwrap();
    assert_eq!(service.status(&reference(1)).unwrap()["status"], "pending");
    with_balance(&world, me, 6);
    assert_eq!(service.status(&reference(1)).unwrap()["status"], "settled");
}

#[test]
fn a_pending_claim_stays_pending_while_lez_core_is_too_busy_to_answer() {
    // It used to be an error, so every auto-claim failed while lez_core was busy.
    let world = World::new();
    claimable(&world, 5);
    let mut service = world.service();
    service.claim(&reference(1), PARTICIPANT).unwrap();
    world.lez.0.borrow_mut().height_unanswered = true;
    assert_eq!(service.status(&reference(1)).unwrap()["status"], "pending");
    assert_eq!(service.auto_claim(PARTICIPANT).unwrap()["skipped"], "in_flight");
}

#[test]
fn a_claim_that_did_not_land_still_fails_and_is_retried() {
    let world = World::new();
    claimable(&world, 5);
    let mut service = world.service();
    service.claim(&reference(1), PARTICIPANT).unwrap();
    world.lez.0.borrow_mut().height = INCLUSION_BLOCKS + 1;
    assert_eq!(service.status(&reference(1)).unwrap()["status"], "failed");
    assert_eq!(service.claim(&reference(1), PARTICIPANT).unwrap()["status"], "pending");
    assert_eq!(world.sent().len(), 2);
}
