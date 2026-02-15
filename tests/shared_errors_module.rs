use direclaw::runtime;
use direclaw::shared;

#[test]
fn runtime_error_is_exposed_via_shared_and_runtime_paths() {
    let shared_error = shared::errors::RuntimeError::NotRunning;
    let runtime_error = runtime::RuntimeError::NotRunning;

    assert_eq!(shared_error.to_string(), "no running supervisor instance");
    assert_eq!(runtime_error.to_string(), "no running supervisor instance");
}
