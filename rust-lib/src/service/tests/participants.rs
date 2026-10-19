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
