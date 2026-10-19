use super::*;

#[test]
fn only_the_current_terms_can_be_accepted() {
    let world = World::new();
    let mut service = world.service();
    assert_eq!(service.accept_terms(crate::domain::terms::version(), "00").unwrap_err().code, "terms_changed");
    assert_eq!(service.terms_status()["accepted"], false);
    accept_terms(&mut service);
    assert_eq!(service.terms_status()["accepted"], true);
    assert_eq!(service.terms()["sha256"], crate::domain::terms::sha256());
}
