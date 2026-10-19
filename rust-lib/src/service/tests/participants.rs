use super::*;

#[test]
fn creating_a_participant_with_a_label_is_idempotent() {
    let world = World::new();
    let mut service = world.service();
    let first = service.create_participant("uplink").unwrap();
    let second = service.create_participant("uplink").unwrap();
    assert_eq!(first["participant"], second["participant"]);
    assert_eq!((first["created"].clone(), second["created"].clone()), (json!(true), json!(false)));
}

#[test]
fn checking_an_invitation_reports_the_inviter_without_saving_it() {
    let world = World::new();
    let mut service = world.service();
    let checked = service.check_invitation(&invitation_blob()).unwrap();
    assert_eq!((checked["referrer"].clone(), checked["registered"].clone()), (json!(codec::node_hex(&REFERRER)), json!(true)));
    assert!(!world.store.load().unwrap().unwrap_or_default().contains("invitation"), "nothing was saved");
    assert_eq!(service.check_invitation("not an invitation").unwrap_err().code, "bad_invitation");
}

#[test]
fn a_participant_is_the_same_in_any_case_with_0x_or_in_base58() {
    let world = World::new();
    let mut service = world.service();
    accept_terms(&mut service);
    let shouted = format!(" 0x{} ", PARTICIPANT.to_uppercase());
    let base58 = codec::account(PARTICIPANT).unwrap().to_string();
    assert_ne!(base58.len(), 64, "base58, not hex");

    service.import_invitation(&shouted, &invitation_blob()).unwrap();
    let op = service.enrol(&reference(1), &base58, "/node/keystore.yaml", "").unwrap();
    assert_eq!(op["participant"], PARTICIPANT);
    // The invitation imported under one spelling was found under the other.
    assert_eq!(world.sent()[0].0[0], AccountEntry::Own(PARTICIPANT.into()));
    assert_eq!(world.sent()[0].0.len(), 3, "the referrer's note account is there");
    assert_eq!(service.operations(&shouted).unwrap()["operations"].as_array().unwrap().len(), 1);
    assert_eq!(service.enrol(&reference(2), PARTICIPANT, "/node/keystore.yaml", "").unwrap_err().code, "in_flight");
    assert_eq!(service.participant("not an account").unwrap_err().code, "invalid_account");
}
