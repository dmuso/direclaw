use direclaw::queue::paths::{is_valid_queue_json_filename, outgoing_filename, QueuePaths};

#[test]
fn queue_paths_module_exposes_existing_path_apis() {
    let queue = QueuePaths::from_state_root(std::path::Path::new("/tmp/state"));
    assert_eq!(
        queue.incoming,
        std::path::Path::new("/tmp/state/queue/incoming")
    );
    assert_eq!(
        queue.processing,
        std::path::Path::new("/tmp/state/queue/processing")
    );
    assert_eq!(
        queue.outgoing,
        std::path::Path::new("/tmp/state/queue/outgoing")
    );

    assert_eq!(outgoing_filename("heartbeat", "hb-1", 100), "hb-1.json");
    assert_eq!(outgoing_filename("slack", "m-1", 100), "slack_m-1_100.json");
    assert!(is_valid_queue_json_filename("message.json"));
    assert!(!is_valid_queue_json_filename("message.txt"));
}
