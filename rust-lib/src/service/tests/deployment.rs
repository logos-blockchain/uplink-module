use super::*;

#[test]
fn configuring_nothing_clears_the_choice() {
    let world = World::new();
    let mut service = world.service();
    let cleared = service.configure("").unwrap();
    assert_eq!(cleared["programAccount"], std::env::var(crate::service::PROGRAM_ACCOUNT_ENV).map_or(Value::Null, |v| json!(v.to_lowercase())));
    assert_eq!(service.configure("not an account").unwrap_err().code, "invalid_account");

    // Base58, as the uplink devnet prints the program account.
    let base58 = program().to_string();
    assert_eq!(service.configure(&base58).unwrap()["programAccount"], hex::encode(PROGRAM));
}

#[test]
fn the_program_is_checked_against_the_deployment() {
    let world = World::new();
    let mut service = world.service();
    assert_eq!(service.check_program().unwrap()["programAccount"], hex::encode(PROGRAM));

    // Nothing deployed at the account: likely the wallet is on another network.
    let missing = hex::encode([0x61; 32]);
    service.configure(&missing).unwrap();
    assert_eq!(service.check_program().unwrap_err().code, "program_not_deployed");

    // Another program, or another build of this one.
    let other = hex::encode([0x62; 32]);
    world.deploy(&other, [9; 8]);
    service.configure(&other).unwrap();
    assert_eq!(service.check_program().unwrap_err().code, "program_mismatch");
}

#[test]
fn a_program_that_changes_after_a_send_is_caught_before_the_next() {
    let world = World::new();
    let mut service = world.service();
    accept_terms(&mut service);
    service.import_invitation(PARTICIPANT, &invitation_blob()).unwrap();
    assert_eq!(service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", "").unwrap()["status"], "pending");
    world.lez.0.borrow_mut().settled = true;
    service.status(&reference(1)).unwrap();

    // Same account, another program (a redeploy, or the wallet on another network).
    world.deploy(&hex::encode(PROGRAM), [9; 8]);
    let mut participant = Participant::new(node_key().node(), Some(referrer()));
    participant.reward_balance = 3;
    world.set_participant(participant);
    assert_eq!(service.cash_out(&reference(2), PARTICIPANT, "/node/keystore.yaml").unwrap_err().code, "program_mismatch");
    assert_eq!(world.sent().len(), 1);
}
