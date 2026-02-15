use direclaw::shared::time::now_secs;

#[test]
fn shared_time_module_returns_non_negative_unix_seconds() {
    assert!(now_secs() >= 0);
}
