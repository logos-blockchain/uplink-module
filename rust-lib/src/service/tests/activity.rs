use super::*;

#[test]
fn activity_marks_each_node_per_recent_epoch_and_unknown_when_unseen() {
    let world = World::new();
    let (a, b) = (NodeId::new([3; 32]), NodeId::new([4; 32]));
    let mut service = world.service();
    world.archive(2, &BTreeSet::from([a, b])); // published, but never seen live
    world.set_registry(3, &[a, b], &[a]);
    service.registry().unwrap();
    world.set_registry(4, &[a, b], &[b]);

    let nodes = [codec::node_hex(&a), codec::node_hex(&b)];
    let activity = service.activity(&nodes, 3).unwrap();
    assert_eq!(activity["epochs"], json!([2, 3, 4]));
    assert_eq!(activity["nodes"][codec::node_hex(&a)], json!([-1, 1, 0]));
    assert_eq!(activity["nodes"][codec::node_hex(&b)], json!([-1, 0, 1]));
}
