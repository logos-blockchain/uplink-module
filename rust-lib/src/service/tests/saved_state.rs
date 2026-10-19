use super::*;

#[test]
fn saved_state_survives_a_restart() {
    let world = World::new();
    accept_terms(&mut world.service());
    let restarted = world.service();
    assert_eq!(restarted.terms_status()["accepted"], true);
    assert_eq!(restarted.info()["stateProblem"], Value::Null);
}

#[test]
fn a_state_file_that_does_not_parse_is_set_aside_not_overwritten() {
    let world = World::new();
    world.store.0.borrow_mut().json = Some("{ half written".into());

    let mut service = world.started(None);
    assert_eq!(service.info()["stateProblem"], json!({ "code": "state_set_aside", "file": "aside-1" }));
    service.configure(&hex::encode(PROGRAM)).unwrap();

    let disk = world.store.0.borrow();
    assert_eq!(disk.aside, ["{ half written"], "the broken file is kept");
    assert!(disk.json.as_deref().unwrap().contains(&hex::encode(PROGRAM)), "and saving starts fresh");
}

#[test]
fn a_state_file_that_cannot_be_read_is_never_saved_over() {
    let world = World::new();
    world.store.0.borrow_mut().json = Some("the user's real state".into());
    world.store.0.borrow_mut().unreadable = Some("permission denied".into());

    let mut service = world.started(None);
    assert_eq!(service.info()["stateProblem"]["code"], "state_unreadable");
    assert_eq!(service.configure(&hex::encode(PROGRAM)).unwrap_err().code, "state_unreadable");

    let disk = world.store.0.borrow();
    assert_eq!(disk.json.as_deref(), Some("the user's real state"));
    assert!(disk.aside.is_empty());
}
