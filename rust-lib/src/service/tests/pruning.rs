//! Keeping only the epoch sets a claim still needs (`service::lists`).

use super::*;
use crate::service::lists::Lists;

fn kept(world: &World) -> Vec<u32> {
    let lists: Lists = serde_json::from_str(&world.lists.load().unwrap().unwrap()).unwrap();
    lists.deployments[&hex::encode(PROGRAM)].sets.keys().copied().collect()
}

/// An indexer that answers from a fixed history and records each call (offset, limit).
struct Recording(Vec<String>, Rc<RefCell<Vec<(u64, u64)>>>);
impl History for Recording {
    fn transactions_by_account(&self, _: &str, offset: u64, limit: u64) -> PortResult<String> {
        self.1.borrow_mut().push((offset, limit));
        let page: Vec<Value> = self.0.iter().skip(offset as usize).take(limit as usize).map(|tx| serde_json::from_str(tx).unwrap()).collect();
        Ok(serde_json::to_string(&page).unwrap())
    }
    fn status(&self) -> PortResult<String> {
        Ok(json!({ "state": "CaughtUp" }).to_string())
    }
}

/// One transaction of `publish_json`'s page.
fn publish_tx(epoch: u32, active: BTreeSet<NodeId>) -> String {
    let page: Vec<Value> = serde_json::from_str(&publish_json(epoch, active)).unwrap();
    page[0].to_string()
}

#[test]
fn a_set_is_dropped_once_its_epoch_is_claimed_and_its_activity_stays() {
    let world = World::new();
    let me = enrolled(&world);
    let child = NodeId::new([3; 32]);
    world.add_note(0x31, State::Child { node: child, referrer: me, since: 2 });
    let mut service = world.service();
    for epoch in 2..=5 {
        world.set_registry(epoch, &[REFERRER, me, child], &[me, child]);
        service.registry().unwrap();
    }
    assert_eq!(service.claimable(PARTICIPANT).unwrap()["points"], "4");
    assert_eq!(kept(&world), vec![2, 3, 4, 5], "the claimable ones; 1 pays nothing (the child joined in 2)");

    // The claim lands: the child is settled through 5 and its note consumed.
    world.lez.0.borrow_mut().private.remove(&hex::encode([0x31; 32]));
    let mut settled = Participant::new(me, Some(referrer()));
    settled.children.insert(child, vec![(0, 5)]);
    world.set_participant(settled);
    assert_eq!(service.claimable(PARTICIPANT).unwrap()["points"], "0");
    assert_eq!(kept(&world), vec![4, 5], "only the most recent stay whole");

    let activity = service.activity(&[codec::node_hex(&me), codec::node_hex(&child)], 5).unwrap();
    assert_eq!(activity["epochs"], json!([1, 2, 3, 4, 5]));
    // Epoch 1 was never seen (the first look was at 2); 2 and 3 come from their marks.
    assert_eq!(activity["nodes"][codec::node_hex(&child)], json!([-1, 1, 1, 1, 1]));
}

#[test]
fn an_epoch_i_was_inactive_in_is_settled_from_its_marks_without_the_indexer() {
    let world = World::new();
    let me = enrolled(&world);
    let child = NodeId::new([3; 32]);
    world.add_note(0x31, State::Child { node: child, referrer: me, since: 2 });
    let mut service = world.service();
    world.set_registry(2, &[REFERRER, me, child], &[child]); // I wasn't active
    service.registry().unwrap();
    for epoch in 3..=6 {
        world.set_registry(epoch, &[REFERRER, me, child], &[REFERRER]);
        service.registry().unwrap();
    }
    service.claimable(PARTICIPANT).unwrap();
    assert!(!kept(&world).contains(&2), "nothing to claim in 2, so not kept");

    // The child stays unsettled for epoch 2 forever, so every plan looks at it again.
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut service = world.service_with(Some(Box::new(Recording(vec![], calls.clone()))));
    for _ in 0..3 {
        assert_eq!(service.claimable(PARTICIPANT).unwrap()["history"], Value::Null);
    }
    assert!(calls.borrow().is_empty(), "decided from the marks: {:?}", calls.borrow());
}

#[test]
fn a_dropped_set_comes_back_from_the_indexer_in_one_call() {
    let world = World::new();
    let me = enrolled(&world);
    let old = BTreeSet::from([me, REFERRER]);
    world.archive(2, &old);
    world.set_registry(5, &[REFERRER, me], &[REFERRER]);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let history = vec![publish_tx(9, BTreeSet::new()), publish_tx(2, old.clone())];
    let mut service = world.service_with(Some(Box::new(Recording(history, calls.clone()))));

    // Epoch 2 is read from the indexer for the activity strip, noted, and dropped.
    let activity = service.activity(&[codec::node_hex(&me)], 4).unwrap();
    assert_eq!((activity["epochs"].clone(), activity["nodes"][codec::node_hex(&me)].clone()), (json!([2, 5]), json!([1, 0])));
    assert!(!kept(&world).contains(&2));

    // Then a credit for epoch 2 arrives: claiming it needs the whole set again.
    world.add_note(0x41, State::Credit { recipient_node: me, amount: 4, epoch: 2, active_digest: active_digest(&old) });
    calls.borrow_mut().clear();
    let claimable = service.claimable(PARTICIPANT).unwrap();
    assert_eq!((claimable["points"].clone(), claimable["history"].clone()), (json!("4"), Value::Null));
    assert_eq!(*calls.borrow(), vec![(1, 1)], "straight to its place in the history");
}

#[test]
fn a_participant_that_never_registered_does_not_hold_sets() {
    let world = World::new();
    let mut service = world.service();
    accept_terms(&mut service);
    // Enrolled with a node, but the Register never landed.
    world.lez.0.borrow_mut().reply = Some(json!({ "success": false, "error": "rejected" }).to_string());
    service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap();
    for epoch in 2..=5 {
        world.set_registry(epoch, &[REFERRER], &[REFERRER]);
        service.registry().unwrap();
    }
    assert_eq!(service.claimable(PARTICIPANT).unwrap_err().code, "not_registered");
    assert_eq!(kept(&world), vec![4, 5]);
}

#[test]
fn points_known_but_not_yet_claimable_are_reported_as_waiting() {
    let world = World::new();
    let me = enrolled(&world);
    let old = BTreeSet::from([me, REFERRER]);
    world.archive(2, &old);
    world.set_registry(5, &[REFERRER, me], &[REFERRER]);
    let mut service = world.service_with(Some(Box::new(Recording(vec![publish_tx(2, old.clone())], Rc::default()))));
    service.activity(&[codec::node_hex(&me)], 4).unwrap(); // epoch 2 noted, its set dropped

    // A late credit for epoch 2, and now no indexer: the set can't come back yet.
    world.add_note(0x41, State::Credit { recipient_node: me, amount: 4, epoch: 2, active_digest: active_digest(&old) });
    let history = world.service().claimable(PARTICIPANT).unwrap()["history"].clone();
    assert_eq!((history["code"].clone(), history["missingEpochs"].clone()), (json!("history_needed"), json!([2])));
    assert_eq!(history["pointsWaiting"], "4");
}
