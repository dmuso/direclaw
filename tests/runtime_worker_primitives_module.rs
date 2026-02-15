use direclaw::runtime::worker_primitives::{queue_polling_defaults, QueuePollingDefaults};

#[test]
fn runtime_worker_primitives_module_exposes_queue_polling_defaults() {
    assert_eq!(
        queue_polling_defaults(),
        QueuePollingDefaults {
            max_concurrency: 4,
            min_poll_ms: 100,
            max_poll_ms: 1000,
        }
    );
}
