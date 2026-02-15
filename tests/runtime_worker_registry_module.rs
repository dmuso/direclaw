use direclaw::runtime::worker_registry::{WorkerEvent, WorkerKind, WorkerRegistry, WorkerState};

#[test]
fn worker_registry_module_tracks_lifecycle() {
    let mut registry = WorkerRegistry::default();
    let queue = WorkerKind::QueueProcessor;
    let orchestrator = WorkerKind::Orchestrator;
    let slack = WorkerKind::ChannelAdapter("slack".to_string());
    let heartbeat = WorkerKind::Heartbeat;

    registry.register(queue.clone());
    registry.register(orchestrator.clone());
    registry.register(slack.clone());
    registry.register(heartbeat.clone());

    registry.start(&queue);
    registry.start(&slack);

    assert_eq!(registry.state(&queue), Some(WorkerState::Running));
    assert_eq!(registry.state(&orchestrator), Some(WorkerState::Stopped));
    assert_eq!(registry.state(&slack), Some(WorkerState::Running));
    assert_eq!(registry.state(&heartbeat), Some(WorkerState::Stopped));

    registry.fail(&slack);
    assert_eq!(registry.state(&slack), Some(WorkerState::Error));

    registry.stop(&slack);
    assert_eq!(registry.state(&slack), Some(WorkerState::Stopped));
}

#[test]
fn worker_registry_module_exposes_worker_event_type() {
    let event = WorkerEvent::Started {
        worker_id: "queue_processor".to_string(),
        at: 42,
    };
    match event {
        WorkerEvent::Started { worker_id, at } => {
            assert_eq!(worker_id, "queue_processor");
            assert_eq!(at, 42);
        }
        _ => panic!("unexpected event variant"),
    }
}
