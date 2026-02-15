#[test]
fn channels_slack_api_module_path_exports_client_type() {
    let _ = std::mem::size_of::<direclaw::channels::slack::api::SlackApiClient>();
}
