use direclaw::runtime::worker_registry::{
    apply_worker_event, WorkerEvent, WorkerKind, WorkerRegistry, WorkerState,
};
use std::collections::{BTreeMap, BTreeSet};

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

#[test]
fn worker_registry_module_applies_worker_events_to_health_state() {
    let mut workers = BTreeMap::new();
    let mut active = BTreeSet::from(["queue_processor".to_string()]);

    let started = apply_worker_event(
        &mut workers,
        &mut active,
        WorkerEvent::Started {
            worker_id: "queue_processor".to_string(),
            at: 42,
        },
    );
    assert_eq!(started.expect("log").event, "worker.started");

    let worker = workers.get("queue_processor").expect("worker health");
    assert_eq!(worker.state, WorkerState::Running);
    assert_eq!(worker.last_heartbeat, Some(42));

    let stopped = apply_worker_event(
        &mut workers,
        &mut active,
        WorkerEvent::Stopped {
            worker_id: "queue_processor".to_string(),
            at: 43,
        },
    );
    assert_eq!(stopped.expect("log").event, "worker.stopped");
    assert!(active.is_empty());
}
