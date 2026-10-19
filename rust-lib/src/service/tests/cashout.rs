use super::*;

#[test]
fn cash_out_burns_the_balance_into_a_receipt_only_the_node_can_find() {
    let world = World::new();
    let me = node_key().node();
    let mut participant = Participant::new(me, None);
    participant.reward_balance = 9;
    world.set_participant(participant);
    let mut service = world.service();

    let op = service.cash_out(&reference(9), PARTICIPANT, "/node/keystore.yaml").unwrap();
    assert_eq!(op["detail"]["index"], 0);
    let receipt = op["detail"]["receipt"].as_str().unwrap().to_owned();
    let (accounts, instruction) = world.sent().pop().unwrap();
    assert_eq!(accounts, vec![AccountEntry::Own(PARTICIPANT.into()), AccountEntry::PublicNoSign(receipt.clone())]);
    let Instruction::CashOut { node, points, blinding_factor } = borsh::from_slice(&instruction).unwrap() else {
        panic!("not a CashOut");
    };
    assert_eq!((node, points), (me, 9));
    assert_eq!(codec::account_hex(&referral_core::cash_out_receipt(program(), me, blinding_factor)), receipt);
    let secret = crate::domain::cashout::receipt_secret(&[1; 32]);
    assert_eq!(blinding_factor, referral_core::blinding_factor(&secret, program(), codec::account(PARTICIPANT).unwrap(), 0));

    // Once the receipt exists, it's listed and the next cash-out takes index 1.
    world.lez.0.borrow_mut().public.insert(receipt.clone(), account_json(&State::CashOut { points: 9 }.to_data().into_inner()));
    let receipts = service.receipts(PARTICIPANT, "/node/keystore.yaml").unwrap();
    assert_eq!(receipts["receipts"][0]["points"], "9");

    // Once settled and the balance is spent, repeating the reference returns the record.
    world.lez.0.borrow_mut().settled = true;
    world.set_participant(Participant::new(me, None));
    let again = service.cash_out(&reference(9), PARTICIPANT, "/node/keystore.yaml").unwrap();
    assert_eq!((again["status"].clone(), again["detail"]["receipt"].clone()), (json!("settled"), json!(receipt)));
    assert_eq!(world.sent().len(), 1, "nothing sent again");

    let code = service.payout_code(PARTICIPANT, 0, "/node/keystore.yaml").unwrap();
    let body = code["code"].as_str().unwrap().strip_prefix(crate::service::cashout::PAYOUT_PREFIX).unwrap();
    use base64::Engine as _;
    let opening: BTreeMap<String, String> =
        serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body).unwrap()).unwrap();
    assert_eq!(opening["blinding_factor"], hex::encode(blinding_factor));
    let mut signed = opening.clone();
    let signature: [u8; 64] = hex::decode(signed.remove("signature").unwrap()).unwrap().try_into().unwrap();
    let key = ed25519_dalek::VerifyingKey::from_bytes(&me.to_bytes()).unwrap();
    assert!(key.verify_strict(serde_json::to_string(&signed).unwrap().as_bytes(), &ed25519_dalek::Signature::from_bytes(&signature)).is_ok());
}

#[test]
fn cash_out_refuses_a_keystore_for_another_node() {
    let world = World::new();
    let mut participant = Participant::new(NodeId::new([6; 32]), None);
    participant.reward_balance = 9;
    world.set_participant(participant);
    let err = world.service().cash_out(&reference(9), PARTICIPANT, "/node/keystore.yaml").unwrap_err();
    assert_eq!(err.code, "keystore_mismatch");
}

#[test]
fn receipts_are_listed_without_the_key_once_known() {
    let world = World::new();
    let mut participant = Participant::new(node_key().node(), None);
    participant.reward_balance = 5;
    world.set_participant(participant);
    let mut service = world.service();

    assert_eq!(service.receipts(PARTICIPANT, "").unwrap()["receipts"], json!([]));
    let op = service.cash_out(&reference(4), PARTICIPANT, "/node/keystore.yaml").unwrap();
    world.lez.0.borrow_mut().settled = true;
    service.status(op["reference"].as_str().unwrap()).unwrap();

    let known = service.receipts(PARTICIPANT, "").unwrap();
    assert_eq!(known["receipts"][0]["points"], "5");
    assert_eq!(known["receipts"][0]["account"], op["detail"]["receipt"]);
    assert_eq!(service.keystore_node("/node/keystore.yaml").unwrap()["node"], codec::node_hex(&node_key().node()));
}
