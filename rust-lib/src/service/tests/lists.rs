use super::*;

#[test]
fn active_sets_are_kept_apart_from_the_saved_state() {
    let world = World::new();
    let me = enrolled(&world);
    world.set_registry(2, &[REFERRER, me], &[REFERRER, me]);
    let mut service = world.service();
    service.registry().unwrap();
    accept_terms(&mut service);

    let state = world.store.load().unwrap().unwrap();
    assert!(!state.contains(&codec::node_hex(&me)), "no node ids in the saved state");
    let lists = world.lists.load().unwrap().unwrap();
    assert!(lists.contains("\"2\":{\"digest\""), "{lists}");
}

#[test]
fn sets_saved_by_an_earlier_version_move_to_the_lists() {
    let world = World::new();
    let me = enrolled(&world);
    let old = BTreeSet::from([me, REFERRER]);
    world.archive(1, &old);
    world.add_note(0x41, State::Credit { recipient_node: me, amount: 4, epoch: 1, active_digest: active_digest(&old) });
    world.set_registry(2, &[REFERRER, me], &[REFERRER]);
    accept_terms(&mut world.service()); // so the state has this deployment's section
    // As an earlier version saved epoch 1's set: in the state, keyed "<epoch>:<digest>".
    let mut state: Value = serde_json::from_str(&world.store.load().unwrap().unwrap()).unwrap();
    let key = format!("1:{}", hex::encode(active_digest(&old)));
    state["deployments"][hex::encode(PROGRAM)]["publications"] = json!({ (key): old.iter().map(codec::node_hex).collect::<Vec<_>>() });
    state["deployments"][hex::encode(PROGRAM)]["historyOffset"] = json!(7);
    world.store.save(&state.to_string()).unwrap();

    let mut service = world.service();
    assert_eq!(service.claimable(PARTICIPANT).unwrap()["points"], "4");
    assert!(!world.store.load().unwrap().unwrap().contains("publications"), "not written back");
}

#[test]
fn a_lists_file_that_does_not_parse_is_an_empty_cache() {
    let world = World::new();
    world.lists.save("{ not json").unwrap();
    let mut service = world.service();
    assert_eq!(service.registry().unwrap()["epoch"], 1);
    assert!(world.lists.load().unwrap().unwrap().starts_with('{'));
}
