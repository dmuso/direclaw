use direclaw::orchestration::conversation_context::{
    append_inbound_turn, append_outbound_turn, render_recent_thread_context, ThreadContextLimits,
};
use tempfile::tempdir;

#[test]
fn conversation_context_module_persists_and_renders_recent_thread_turns() {
    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");

    append_inbound_turn(
        &state_root,
        "engineering",
        "C1:100.1",
        "msg-1",
        100,
        "U1",
        "first inbound",
    )
    .expect("append inbound 1");
    append_outbound_turn(
        &state_root,
        "engineering",
        "C1:100.1",
        "msg-1-out",
        101,
        "orchestrator",
        "workflow started\nrun_id=run-1",
        Some("run-1"),
        None,
    )
    .expect("append outbound");
    append_inbound_turn(
        &state_root,
        "engineering",
        "C1:100.1",
        "msg-2",
        102,
        "U1",
        "can you investigate why this failed?",
    )
    .expect("append inbound 2");

    let rendered = render_recent_thread_context(
        &state_root,
        "engineering",
        "C1:100.1",
        ThreadContextLimits {
            max_turns: 8,
            max_chars: 4000,
        },
    )
    .expect("render thread context")
    .expect("thread context should exist");

    assert!(rendered.contains("first inbound"));
    assert!(rendered.contains("run_id=run-1"));
    assert!(rendered.contains("can you investigate why this failed?"));
}

#[test]
fn conversation_context_module_applies_turn_and_char_limits() {
    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");

    for idx in 0..12 {
        append_inbound_turn(
            &state_root,
            "engineering",
            "C1:100.2",
            &format!("msg-{idx}"),
            100 + idx as i64,
            "U1",
            &format!("inbound turn {idx}"),
        )
        .expect("append inbound");
    }

    let rendered = render_recent_thread_context(
        &state_root,
        "engineering",
        "C1:100.2",
        ThreadContextLimits {
            max_turns: 3,
            max_chars: 80,
        },
    )
    .expect("render thread context")
    .expect("thread context should exist");

    assert!(!rendered.contains("inbound turn 0"));
    assert!(rendered.contains("inbound turn 11"));
    assert!(rendered.chars().count() <= 80);
}
