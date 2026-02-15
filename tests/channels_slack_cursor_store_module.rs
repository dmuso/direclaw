use direclaw::channels::slack::cursor_store::{
    load_cursor_state, save_cursor_state, SlackCursorState,
};
use tempfile::tempdir;

#[test]
fn channels_slack_cursor_store_module_round_trips_cursor_state() {
    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");

    let mut state = SlackCursorState::default();
    state
        .conversations
        .insert("C123".to_string(), "1700000000.1".to_string());

    save_cursor_state(&state_root, "profile.main", &state).expect("save");
    let loaded = load_cursor_state(&state_root, "profile.main").expect("load");
    assert_eq!(loaded, state);
}
