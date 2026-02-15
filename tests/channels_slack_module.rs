#[test]
fn channels_slack_module_path_exports_slack_sync_types() {
    let _ = std::mem::size_of::<direclaw::channels::slack::SlackSyncReport>();
    let _ = std::mem::size_of::<direclaw::channels::slack::SlackError>();
}
