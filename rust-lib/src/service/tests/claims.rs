use super::*;

#[test]
fn a_late_claim_pays_every_epoch_and_forwards_credit_when_the_referrer_was_active() {
    let world = World::new();
    let me = enrolled(&world);
    let child = NodeId::new([3; 32]);
    let mut service = world.service();
    // Epoch 2 is seen live, then epoch 3 is published; the referrer missed epoch 3.
    world.set_registry(2, &[REFERRER, me, child], &[REFERRER, me, child]);
    service.registry().unwrap();
    world.set_registry(3, &[REFERRER, me, child], &[me, child]);
    world.add_note(0x31, State::Child { node: child, referrer: me, since: 2 });
    world.add_note(0x32, State::Child { node: NodeId::new([4; 32]), referrer: REFERRER, since: 2 }); // not ours

    assert_eq!(service.claimable(PARTICIPANT).unwrap()["points"], "2");

    service.claim(&reference(5), PARTICIPANT).unwrap();
    let (accounts, instruction) = world.sent().pop().unwrap();
    assert_eq!(
        accounts,
        vec![
            AccountEntry::Own(PARTICIPANT.into()),
            AccountEntry::PublicNoSign(codec::account_hex(&epoch_account(program(), 2))),
            AccountEntry::Own(hex::encode([0x31; 32])),
            referrer_entry(),
            AccountEntry::PublicNoSign(codec::account_hex(&epoch_account(program(), 3))),
        ]
    );
    let Instruction::Claim(claim) = borsh::from_slice(&instruction).unwrap() else { panic!("not a Claim") };
    assert_eq!(claim.referrer, Some(REFERRER));
    assert_eq!(claim.epochs.iter().map(|e| (e.epoch, e.total)).collect::<Vec<_>>(), vec![(2, 1), (3, 1)]);
}

#[test]
fn auto_claim_waits_for_the_pending_claim_and_stops_when_settled() {
    let world = World::new();
    let me = enrolled(&world);
    let child = NodeId::new([3; 32]);
    world.set_registry(2, &[REFERRER, me, child], &[me, child]);
    world.add_note(0x31, State::Child { node: child, referrer: me, since: 2 });

    let mut service = world.service();
    let first = service.auto_claim(PARTICIPANT).unwrap();
    assert!(first.get("claimed").is_some(), "{first}");
    assert_eq!(service.auto_claim(PARTICIPANT).unwrap()["skipped"], "in_flight");

    // The claim lands: the note is consumed and the child's epoch 2 is settled.
    world.lez.0.borrow_mut().settled = true;
    service.status(first["claimed"]["reference"].as_str().unwrap()).unwrap();
    world.lez.0.borrow_mut().private.remove(&hex::encode([0x31; 32]));
    let mut settled = Participant::new(me, Some(referrer()));
    settled.children.insert(child, vec![(0, 2)]);
    settled.reward_balance = 1;
    world.set_participant(settled);

    assert_eq!(service.auto_claim(PARTICIPANT).unwrap()["skipped"], "nothing_to_claim");
    assert_eq!(world.sent().len(), 1);

    // Repeating the settled claim's reference returns its record; nothing is left to claim.
    let reference = first["claimed"]["reference"].as_str().unwrap();
    assert_eq!(service.claim(reference, PARTICIPANT).unwrap()["status"], "settled");
    assert_eq!(service.claim(reference, "1111111111111111111111111111111111111111111111111111111111111112").unwrap_err().code, "conflict");
}

#[test]
fn a_childs_join_epoch_outlives_the_claim_that_consumes_its_note() {
    let world = World::new();
    let me = enrolled(&world);
    let child = NodeId::new([3; 32]);
    world.set_registry(5, &[REFERRER, me, child], &[me, child]);
    world.add_note(0x31, State::Child { node: child, referrer: me, since: 4 });
    let mut service = world.service();
    service.auto_claim(PARTICIPANT).unwrap();   // reads the note before claiming it

    // Settled: the note is gone and epochs 0..=5 merge into one range, the join with them.
    world.lez.0.borrow_mut().private.remove(&hex::encode([0x31; 32]));
    let mut settled = Participant::new(me, Some(referrer()));
    settled.children.insert(child, vec![(0, 5)]);
    world.set_participant(settled);

    assert_eq!(service.participant(PARTICIPANT).unwrap()["childrenSince"][codec::node_hex(&child)], 4);
    // And across a restart, from the saved state.
    assert_eq!(world.service().participant(PARTICIPANT).unwrap()["childrenSince"][codec::node_hex(&child)], 4);
}

#[test]
fn past_active_sets_come_from_the_indexer_and_are_checked_against_the_archive() {
    let world = World::new();
    let me = enrolled(&world);
    let old_active = BTreeSet::from([me, REFERRER]);
    world.archive(1, &old_active);
    world.add_note(
        0x41,
        State::Credit { recipient_node: me, amount: 4, epoch: 1, active_digest: active_digest(&old_active) },
    );
    world.set_registry(7, &[REFERRER, me], &[REFERRER, me]);

    let publish = indexer_service_protocol::Transaction::Public(indexer_service_protocol::PublicTransaction {
        hash: indexer_service_protocol::HashType([0; 32]),
        message: indexer_service_protocol::PublicMessage {
            program_account_id: indexer_service_protocol::AccountId { value: PROGRAM },
            shard_selectors: vec![],
            nonces: vec![],
            instruction_data: borsh::to_vec(&Instruction::Publish { epoch: 1, active: old_active }).unwrap(),
            fee: None,
        },
        witness_set: indexer_service_protocol::WitnessSet { signatures_and_public_keys: vec![], proof: None },
    });
    let history = FakeHistory(serde_json::to_string(&vec![publish]).unwrap());

    assert_eq!(world.service().claimable(PARTICIPANT).unwrap_err().code, "history_needed");
    let mut service = world.service_with(Some(Box::new(history)));
    let claimable = service.claimable(PARTICIPANT).unwrap();
    assert_eq!(claimable["points"], "4", "{claimable}");
    assert_eq!(claimable["batches"][0]["epochs"], json!([1]));
}

#[test]
fn a_missing_indexer_is_asked_once_per_claim_and_fails_fast() {
    let world = World::new();
    let me = enrolled(&world);
    // Credits in three epochs the module never saw live.
    for epoch in 1..=3 {
        let active = BTreeSet::from([me, REFERRER]);
        world.archive(epoch, &active);
        world.add_note(
            0x50 + epoch as u8,
            State::Credit { recipient_node: me, amount: 1, epoch, active_digest: active_digest(&active) },
        );
    }
    world.set_registry(9, &[REFERRER, me], &[REFERRER, me]);

    let calls = Rc::new(RefCell::new(0));
    let mut service = world.service_with(Some(Box::new(DeadIndexer(calls.clone()))));
    assert_eq!(service.claimable(PARTICIPANT).unwrap_err().code, "history_needed");
    assert_eq!(*calls.borrow(), 1, "one indexer call per claim, however many epochs are missing");
    assert_eq!(service.claim(&reference(7), PARTICIPANT).unwrap_err().code, "history_needed");
    assert!(world.sent().is_empty());
}

#[test]
fn auto_claim_retries_a_failed_batch_under_its_reference_waiting_longer_each_time() {
    let world = World::new();
    let me = enrolled(&world);
    let child = NodeId::new([3; 32]);
    world.set_registry(2, &[REFERRER, me, child], &[me, child]);
    world.add_note(0x31, State::Child { node: child, referrer: me, since: 2 });
    world.lez.0.borrow_mut().reply = Some(json!({ "success": false, "error": "InvalidProof" }).to_string());
    world.lez.0.borrow_mut().height = 100;
    let mut service = world.service();

    let first = service.auto_claim(PARTICIPANT).unwrap();
    let reference = first["claimed"]["reference"].as_str().unwrap().to_owned();
    assert_eq!((first["claimed"]["status"].clone(), first["claimed"]["attempts"].clone()), (json!("failed"), json!(1)));

    // Polls right after the failure send nothing.
    let waiting = service.auto_claim(PARTICIPANT).unwrap();
    assert_eq!((waiting["skipped"].clone(), waiting["retryAtBlock"].clone()), (json!("backoff"), json!(110)));
    assert_eq!(world.sent().len(), 1);

    // Then the same batch goes again under the same reference, and the next wait doubles.
    world.lez.0.borrow_mut().height = 110;
    let second = service.auto_claim(PARTICIPANT).unwrap();
    assert_eq!((second["claimed"]["reference"].as_str().unwrap(), second["claimed"]["attempts"].clone()), (reference.as_str(), json!(2)));
    assert_eq!(service.auto_claim(PARTICIPANT).unwrap()["retryAtBlock"], 130);

    // Once it gets through, it's pending under that reference like any claim.
    world.lez.0.borrow_mut().reply = None;
    world.lez.0.borrow_mut().height = 130;
    assert_eq!(service.auto_claim(PARTICIPANT).unwrap()["claimed"]["status"], "pending");
    assert_eq!(service.operations(PARTICIPANT)["operations"].as_array().unwrap().len(), 1, "one record, not one per attempt");
}
