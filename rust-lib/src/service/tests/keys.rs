//! Keystore paths off (no `UPLINK_KEYSTORE_PATHS=1`): the key stays with its holder.

use super::*;

#[test]
fn with_keystore_paths_off_nothing_reads_a_key_and_the_handed_in_calls_still_work() {
    let mut world = World::new();
    world.keystore_paths = false;
    let me = node_key().node();
    let mut service = world.service();
    accept_terms(&mut service);
    service.import_invitation(PARTICIPANT, &invitation_blob()).unwrap();

    let path = "/node/keystore.yaml";
    let off = |r: crate::domain::Result<Value>| r.unwrap_err().code;
    assert_eq!(off(service.enrol(&reference(1), PARTICIPANT, path, "")), "keystore_paths_off");
    assert_eq!(off(service.keystore_node(path)), "keystore_paths_off");
    assert!(world.sent().is_empty());

    // Signed by the node's holder instead.
    let message = service.enrolment_message(PARTICIPANT, &codec::node_hex(&me)).unwrap()["message"].as_str().unwrap().to_owned();
    let signature = hex::encode(node_key().sign(&hex::decode(message).unwrap()));
    assert_eq!(service.enrol_with_signature(&reference(1), PARTICIPANT, &codec::node_hex(&me), &signature).unwrap()["status"], "pending");

    let mut participant = Participant::new(me, Some(referrer()));
    participant.reward_balance = 3;
    world.set_participant(participant);
    world.lez.0.borrow_mut().settled = true;
    service.status(&reference(1)).unwrap();
    assert_eq!(off(service.cash_out(&reference(2), PARTICIPANT, path)), "keystore_paths_off");
    assert_eq!(off(service.receipts(PARTICIPANT, path)), "keystore_paths_off");
    assert_eq!(off(service.payout_code(PARTICIPANT, 0, path)), "keystore_paths_off");
    assert!(service.receipts(PARTICIPANT, "").is_ok(), "the remembered list needs no key");
    let secret = hex::encode(*crate::domain::cashout::receipt_secret(&[1; 32]));
    assert_eq!(service.cash_out_with_secret(&reference(2), PARTICIPANT, &secret).unwrap()["status"], "pending");
}
