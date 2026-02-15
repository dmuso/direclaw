use direclaw::runtime::worker_registry::{WorkerKind, WorkerRegistry, WorkerState};

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
