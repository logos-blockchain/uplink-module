use super::*;

#[test]
fn joining_needs_the_terms_accepted() {
    let world = World::new();
    let mut service = world.service();
    assert_eq!(service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap_err().code, "terms_not_accepted");
    accept_terms(&mut service);
    service.import_invitation(PARTICIPANT, &invitation_blob()).unwrap();
    assert!(service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").is_ok());
}

#[test]
fn enrolment_signs_with_the_blend_key_and_sends_the_three_accounts() {
    let world = World::new();
    let mut service = world.service();
    accept_terms(&mut service);
    service.import_invitation(PARTICIPANT, &invitation_blob()).unwrap();

    let op = service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap();
    assert_eq!(op["status"], "pending");

    let sent = world.sent();
    assert_eq!(sent.len(), 1);
    let (accounts, instruction) = &sent[0];
    assert_eq!(
        accounts,
        &vec![
            AccountEntry::Own(PARTICIPANT.into()),
            AccountEntry::PublicNoSign(codec::account_hex(&ORACLE_ACCOUNT_ID)),
            AccountEntry::PrivateForeign { npk: hex::encode([0x22; 32]), vpk: hex::encode([0x22; VPK_LEN]) },
        ]
    );
    let Instruction::Register { node, referrer, node_signature, pool, epoch } = borsh::from_slice(instruction).unwrap() else {
        panic!("not a Register");
    };
    let referrer = referrer.expect("the invitation's referrer");
    assert_eq!((node, referrer.node, referrer.npk.0, epoch), (node_key().node(), REFERRER, [0x22; 32], 1));
    assert_eq!(pool, BTreeSet::from([REFERRER]));
    assert!(enrolment::verify(program(), node, codec::account(PARTICIPANT).unwrap(), Some(REFERRER), &node_signature));

    // Same reference: no second transaction.
    service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap();
    assert_eq!(world.sent().len(), 1);
    // Another one while it's pending is refused.
    assert_eq!(service.enrol(&reference(2), PARTICIPANT, "/node/keystore.yaml", "").unwrap_err().code, "in_flight");

    world.lez.0.borrow_mut().settled = true;
    assert_eq!(service.status(&reference(1)).unwrap()["status"], "settled");
}

#[test]
fn enrolment_refuses_a_registered_node_or_an_unregistered_referrer() {
    let world = World::new();
    let mut service = world.service();
    accept_terms(&mut service);

    world.set_registry(1, &[node_key().node()], &[]);
    assert_eq!(service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap_err().code, "already_registered");

    world.set_registry(1, &[], &[]);
    service.import_invitation(PARTICIPANT, &invitation_blob()).unwrap();
    assert_eq!(service.enrol(&reference(2), PARTICIPANT, "/node/keystore.yaml", "").unwrap_err().code, "referrer_not_registered");
    assert!(world.sent().is_empty());
}

#[test]
fn enrolment_refuses_a_keystore_that_is_not_the_running_nodes() {
    let world = World::new();
    let mut service = world.service();
    accept_terms(&mut service);

    let running = codec::node_hex(&NodeId::new([6; 32]));
    let err = service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", &running).unwrap_err();
    assert_eq!(err.code, "keystore_mismatch");
    assert!(err.message.contains(&running));
    assert!(world.sent().is_empty());

    // The keystore that does hold the running node's key enrols that node.
    let ours = codec::node_hex(&node_key().node());
    assert_eq!(service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", &ours).unwrap()["detail"]["node"], ours);
}

#[test]
fn a_failed_send_is_recorded_and_can_be_retried_with_the_same_reference() {
    let world = World::new();
    let mut service = world.service();
    accept_terms(&mut service);
    world.lez.0.borrow_mut().reply = Some(json!({ "success": false, "error": "AccountNotFound" }).to_string());
    let failed = service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap();
    assert_eq!((failed["status"].clone(), failed["error"].clone()), (json!("failed"), json!("AccountNotFound")));

    world.lez.0.borrow_mut().reply = None;
    assert_eq!(service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap()["status"], "pending");
    assert_eq!(world.sent().len(), 2);
}

#[test]
fn an_operation_cut_off_mid_send_no_longer_blocks_the_participant() {
    let world = World::new();
    let mut service = world.service();
    accept_terms(&mut service);
    service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap();

    // As if the module had stopped between recording the operation and lez_core answering.
    let saved = world.store.load().unwrap().unwrap().replace("\"pending\"", "\"submitting\"");
    world.store.save(&saved).unwrap();

    let mut restarted = world.service();
    let retried = restarted.enrol(&reference(2), PARTICIPANT, "/node/keystore.yaml", "").unwrap();
    assert_eq!(retried["status"], "pending");
    assert_eq!(restarted.status(&reference(1)).unwrap()["status"], "failed");
}

#[test]
fn nothing_is_sent_to_a_program_that_is_not_ours() {
    let world = World::new();
    world.deploy(&hex::encode(PROGRAM), [9; 8]);
    let mut service = world.service();
    accept_terms(&mut service);
    let err = service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap_err();
    assert_eq!(err.code, "program_mismatch");
    assert!(world.lez.0.borrow().sent.is_empty());
}

#[test]
fn saved_state_survives_a_restart() {
    let world = World::new();
    world.service().import_invitation(PARTICIPANT, &invitation_blob()).unwrap();
    let restarted = world.service();
    let message = restarted.enrolment_message(PARTICIPANT, &codec::node_hex(&node_key().node())).unwrap();
    assert_eq!(message["referrer"], codec::node_hex(&REFERRER));
    assert_eq!(world.store.load().unwrap().map(|json| serde_json::from_str::<Value>(&json).is_ok()), Some(true));
}

#[test]
fn each_deployment_keeps_its_own_state() {
    let world = World::new();
    let mut service = world.service();
    let node = codec::node_hex(&node_key().node());
    service.import_invitation(PARTICIPANT, &invitation_blob()).unwrap();
    accept_terms(&mut service);

    // Another network: nothing from the first one carries over.
    let other = hex::encode([0x60; 32]);
    service.configure(&other).unwrap();
    assert_eq!(service.enrolment_message(PARTICIPANT, &node).unwrap()["referrer"], Value::Null);
    assert_eq!(service.terms_status()["accepted"], false);
    accept_terms(&mut service);

    // Back to the first: its invitation and terms are as they were.
    service.configure(&hex::encode(PROGRAM)).unwrap();
    assert_eq!(service.enrolment_message(PARTICIPANT, &node).unwrap()["referrer"], codec::node_hex(&REFERRER));
    assert_eq!(service.terms_status()["accepted"], true);
    assert_eq!(service.info()["deployments"].as_array().unwrap().len(), 2);
}
