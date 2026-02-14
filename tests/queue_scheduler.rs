use direclaw::queue::{OrderingKey, PerKeyScheduler};

#[test]
fn mixed_keys_preserve_sequence_and_allow_concurrency() {
    let key_a = OrderingKey::WorkflowRun("run-a".to_string());
    let key_b = OrderingKey::WorkflowRun("run-b".to_string());
    let key_c = OrderingKey::Conversation {
        channel: "slack".to_string(),
        channel_profile_id: "eng".to_string(),
        conversation_id: "thread-9".to_string(),
    };

    let mut scheduler = PerKeyScheduler::default();
    scheduler.enqueue(key_a.clone(), "a1");
    scheduler.enqueue(key_a.clone(), "a2");
    scheduler.enqueue(key_b.clone(), "b1");
    scheduler.enqueue(key_c.clone(), "c1");

    let first_batch = scheduler.dequeue_runnable(2);
    assert_eq!(first_batch.len(), 2);
    assert_eq!(first_batch[0].value, "a1");
    assert_eq!(first_batch[1].value, "b1");

    let second_batch = scheduler.dequeue_runnable(2);
    assert_eq!(second_batch.len(), 1);
    assert_eq!(second_batch[0].value, "c1");

    scheduler.complete(&key_a);
    let third_batch = scheduler.dequeue_runnable(2);
    assert_eq!(third_batch.len(), 1);
    assert_eq!(third_batch[0].value, "a2");

    scheduler.complete(&key_b);
    scheduler.complete(&key_c);
    scheduler.complete(&key_a);

    assert!(scheduler.dequeue_runnable(2).is_empty());
}

#[test]
fn bounded_scheduler_avoids_starvation_under_heavy_mixed_load() {
    let mut scheduler = PerKeyScheduler::default();
    let hot_key = OrderingKey::WorkflowRun("hot".to_string());

    for idx in 0..50 {
        scheduler.enqueue(hot_key.clone(), format!("hot-{idx}"));
    }
    for idx in 0..25 {
        scheduler.enqueue(
            OrderingKey::WorkflowRun(format!("cold-{idx}")),
            format!("cold-{idx}"),
        );
    }

    let mut completed = Vec::new();
    while scheduler.pending_len() > 0 || scheduler.active_len() > 0 {
        let batch = scheduler.dequeue_runnable(4);
        if batch.is_empty() {
            break;
        }
        for item in batch {
            completed.push(item.value.clone());
            scheduler.complete(&item.key);
        }
    }

    assert_eq!(completed.len(), 75);
    for idx in 0..25 {
        assert!(
            completed.iter().any(|v| v == &format!("cold-{idx}")),
            "missing cold key cold-{idx}"
        );
    }
}
