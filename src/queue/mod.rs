pub mod file_tags;
pub mod lifecycle;
pub mod logging;
pub mod message;
pub mod outbound;
pub mod paths;
pub mod scheduler;
pub use file_tags::{
    append_inbound_file_tags, extract_inbound_file_tags, prepare_outbound_content,
};
pub use lifecycle::{claim_oldest, complete_success, requeue_failure, ClaimedMessage};
pub use message::{IncomingMessage, OutgoingMessage};
pub use outbound::{
    sorted_outgoing_paths, OutboundContent, OUTBOUND_MAX_CHARS, OUTBOUND_TRUNCATE_KEEP_CHARS,
    OUTBOUND_TRUNCATION_SUFFIX,
};
pub use paths::{is_valid_queue_json_filename, outgoing_filename, QueuePaths};
pub use scheduler::{derive_ordering_key, OrderingKey, PerKeyScheduler, Scheduled};

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("queue io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid queue payload in {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_incoming_file(dir: &std::path::Path, name: &str, payload: &IncomingMessage) {
        let path = dir.join(name);
        fs::write(
            path,
            serde_json::to_string(payload).expect("serialize payload"),
        )
        .expect("write incoming");
    }

    fn sample_incoming(message_id: &str) -> IncomingMessage {
        IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("profile-1".to_string()),
            sender: "Alice".to_string(),
            sender_id: "U123".to_string(),
            message: "hello".to_string(),
            timestamp: 1,
            message_id: message_id.to_string(),
            conversation_id: Some("thread-1".to_string()),
            files: vec![],
            workflow_run_id: None,
            workflow_step_id: None,
        }
    }

    #[test]
    fn outgoing_filename_rules_match_spec() {
        assert_eq!(outgoing_filename("heartbeat", "hb-1", 100), "hb-1.json");
        assert_eq!(outgoing_filename("slack", "m1", 100), "slack_m1_100.json");
    }

    #[test]
    fn queue_claims_oldest_file_first() {
        let tmp = tempdir().expect("tempdir");
        let queue = QueuePaths::from_state_root(tmp.path());
        fs::create_dir_all(&queue.incoming).expect("incoming dir");
        fs::create_dir_all(&queue.processing).expect("processing dir");
        fs::create_dir_all(&queue.outgoing).expect("outgoing dir");

        write_incoming_file(&queue.incoming, "a.json", &sample_incoming("a"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        write_incoming_file(&queue.incoming, "b.json", &sample_incoming("b"));

        let claim = claim_oldest(&queue).expect("claim").expect("a claim");
        assert_eq!(claim.payload.message_id, "a");
        assert!(claim.processing_path.exists());
        assert!(!claim.incoming_path.exists());
    }

    #[test]
    fn requeue_moves_processing_back_to_incoming() {
        let tmp = tempdir().expect("tempdir");
        let queue = QueuePaths::from_state_root(tmp.path());
        fs::create_dir_all(&queue.incoming).expect("incoming dir");
        fs::create_dir_all(&queue.processing).expect("processing dir");
        fs::create_dir_all(&queue.outgoing).expect("outgoing dir");
        write_incoming_file(&queue.incoming, "a.json", &sample_incoming("a"));

        let claim = claim_oldest(&queue).expect("claim").expect("item");
        let requeued = requeue_failure(&queue, &claim).expect("requeue");
        assert!(requeued.exists());
        assert!(!claim.processing_path.exists());
    }

    #[test]
    fn derive_ordering_key_prefers_workflow_then_conversation() {
        let mut payload = sample_incoming("m1");
        payload.workflow_run_id = Some("run-1".to_string());
        assert_eq!(
            derive_ordering_key(&payload),
            OrderingKey::WorkflowRun("run-1".to_string())
        );

        payload.workflow_run_id = None;
        assert_eq!(
            derive_ordering_key(&payload),
            OrderingKey::Conversation {
                channel: "slack".to_string(),
                channel_profile_id: "profile-1".to_string(),
                conversation_id: "thread-1".to_string(),
            }
        );

        payload.channel_profile_id = None;
        assert_eq!(
            derive_ordering_key(&payload),
            OrderingKey::Message("m1".to_string())
        );
    }

    #[test]
    fn scheduler_allows_independent_keys_without_reordering_same_key() {
        let key_a = OrderingKey::WorkflowRun("run-a".to_string());
        let key_b = OrderingKey::WorkflowRun("run-b".to_string());

        let mut scheduler = PerKeyScheduler::default();
        scheduler.enqueue(key_a.clone(), "a1");
        scheduler.enqueue(key_a.clone(), "a2");
        scheduler.enqueue(key_b.clone(), "b1");

        let batch = scheduler.dequeue_runnable(2);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].value, "a1");
        assert_eq!(batch[1].value, "b1");

        scheduler.complete(&key_b);
        let blocked = scheduler.dequeue_runnable(2);
        assert!(blocked.is_empty());

        scheduler.complete(&key_a);
        let next = scheduler.dequeue_runnable(1);
        assert_eq!(next[0].value, "a2");
    }

    #[test]
    fn inbound_file_tag_extraction_and_append_are_deterministic() {
        let text = "hello [file: /tmp/a.txt] and [file: relative.txt] [file: /tmp/b.txt]";
        let tags = extract_inbound_file_tags(text);
        assert_eq!(tags, vec!["/tmp/a.txt", "/tmp/b.txt"]);

        let rendered = append_inbound_file_tags(
            "base",
            &[
                "/tmp/one.png".to_string(),
                "relative.png".to_string(),
                "/tmp/two.png".to_string(),
            ],
        );
        assert_eq!(rendered, "base\n[file: /tmp/one.png]\n[file: /tmp/two.png]");
    }

    #[test]
    fn outbound_send_file_tags_are_stripped_and_truncated_after_strip() {
        let tmp = tempdir().expect("tempdir");
        let sendable = tmp.path().join("artifact.txt");
        fs::write(&sendable, "x").expect("write file");

        let raw = format!(
            "preface [send_file: {}] tail [send_file: relative.txt]",
            sendable.display()
        );
        let prepared = prepare_outbound_content(&raw);
        assert_eq!(prepared.files, vec![sendable.display().to_string()]);
        assert_eq!(prepared.omitted_files, vec!["relative.txt".to_string()]);
        assert!(!prepared.message.contains("[send_file:"));

        let long = "a".repeat(4100);
        let prepared_long = prepare_outbound_content(&long);
        assert_eq!(prepared_long.message.chars().count(), 3925);
        assert!(prepared_long
            .message
            .ends_with("\n\n[Response truncated...]"));
    }
}
