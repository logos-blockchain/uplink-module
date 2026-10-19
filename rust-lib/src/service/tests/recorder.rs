use super::*;
use crate::service::recorder::{Recorded, record_once};

fn recorded(world: &World) -> Recorded {
    serde_json::from_str(&world.log.load().unwrap().unwrap()).unwrap()
}

#[test]
fn the_recorder_waits_for_a_deployment_then_saves_each_epoch_once() {
    let world = World::empty();
    world.set_registry(3, &[REFERRER], &[REFERRER]);
    assert_eq!(record_once(&world.lez, &world.store, &world.log).unwrap(), None, "nothing configured yet");

    world.service(); // configures PROGRAM
    assert_eq!(record_once(&world.lez, &world.store, &world.log).unwrap(), Some(3));
    assert_eq!(record_once(&world.lez, &world.store, &world.log).unwrap(), None, "already saved");
    world.set_registry(4, &[REFERRER], &[]);
    assert_eq!(record_once(&world.lez, &world.store, &world.log).unwrap(), Some(4));

    let seen = &recorded(&world).deployments[&hex::encode(PROGRAM)];
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[&format!("3:{}", hex::encode(active_digest(&BTreeSet::from([REFERRER]))))], vec![codec::node_hex(&REFERRER)]);
}

#[test]
fn an_epoch_only_the_recorder_saw_is_claimable_without_an_indexer() {
    let world = World::new();
    let me = enrolled(&world);
    let mut service = world.service();

    // Epoch 2 is current while nobody asks the service anything; only the recorder looks.
    let active = BTreeSet::from([me, REFERRER]);
    world.set_registry(2, &[REFERRER, me], &[me, REFERRER]);
    world.add_note(0x71, State::Credit { recipient_node: me, amount: 6, epoch: 2, active_digest: active_digest(&active) });
    assert_eq!(record_once(&world.lez, &world.store, &world.log).unwrap(), Some(2));
    world.set_registry(3, &[REFERRER, me], &[REFERRER]);

    let claimable = service.claimable(PARTICIPANT).unwrap();
    assert_eq!((claimable["points"].clone(), claimable["history"].clone()), (json!("6"), Value::Null));
    // Taken into the service's own state, so it survives the log going away.
    world.log.0.borrow_mut().json = None;
    assert_eq!(world.service().claimable(PARTICIPANT).unwrap()["points"], "6");
}

#[test]
fn the_recorder_keeps_each_deployment_apart() {
    let world = World::new();
    world.service();
    assert_eq!(record_once(&world.lez, &world.store, &world.log).unwrap(), Some(1));
    let other = hex::encode([0x51; 32]);
    world.deploy(&other, IMAGE_ID);
    world.service().configure(&other).unwrap();
    assert_eq!(record_once(&world.lez, &world.store, &world.log).unwrap(), None);

    let deployments = recorded(&world).deployments;
    assert_eq!(deployments.keys().cloned().collect::<Vec<_>>(), vec![hex::encode(PROGRAM)], "the other has no registry shard yet");
}

#[test]
fn the_recorder_forgets_what_the_service_took_in() {
    let world = World::new();
    let mut service = world.service();
    for epoch in 2..=3 {
        world.set_registry(epoch, &[REFERRER], &[REFERRER]);
        record_once(&world.lez, &world.store, &world.log).unwrap();
    }
    service.registry().unwrap(); // takes in 2 and 3

    world.set_registry(4, &[REFERRER], &[]);
    assert_eq!(record_once(&world.lez, &world.store, &world.log).unwrap(), Some(4));
    let seen = &recorded(&world).deployments[&hex::encode(PROGRAM)];
    assert_eq!(seen.keys().map(|k| k.split(':').next().unwrap().to_owned()).collect::<Vec<_>>(), ["4"]);
}

#[test]
fn the_recorder_looks_often_only_around_an_epoch_change() {
    use std::time::Duration;
    use crate::service::recorder::next_look;
    assert_eq!(next_look(Duration::ZERO), Duration::from_secs(15));
    assert_eq!(next_look(Duration::from_secs(40)), Duration::from_secs(15), "devnet epochs");
    assert_eq!(next_look(Duration::from_secs(600)), Duration::from_secs(60));
    assert_eq!(next_look(Duration::from_secs(5 * 3600)), Duration::from_secs(600), "at most 10 minutes");
}
