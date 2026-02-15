use direclaw::provider::prompt_files::{
    consume_reset_flag, read_to_string, write_file_backed_prompt,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn prompt_files_module_supports_prompt_io_and_reset_consumption() {
    let dir = tempdir().expect("tempdir");

    let artifacts =
        write_file_backed_prompt(dir.path(), "req-module", "prompt body", "context body")
            .expect("write prompt files");
    let prompt = read_to_string(&artifacts.prompt_file).expect("read prompt file");

    assert_eq!(prompt, "prompt body");
    assert_eq!(artifacts.context_files.len(), 1);

    let reset_flag = dir.path().join("agents/a1/reset.flag");
    fs::create_dir_all(reset_flag.parent().expect("parent")).expect("create parent");
    fs::write(&reset_flag, "1").expect("write flag");

    let first = consume_reset_flag(&reset_flag).expect("consume reset");
    assert!(first.reset_requested);
    assert!(first.consumed_agent);

    let second = consume_reset_flag(&reset_flag).expect("consume reset second");
    assert!(!second.reset_requested);
    assert!(!second.consumed_agent);
}
