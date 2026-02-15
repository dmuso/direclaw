use direclaw::shared::fs_atomic::{atomic_write_file, canonicalize_existing};
use std::fs;

#[test]
fn shared_fs_atomic_writes_and_canonicalizes_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("nested/output.txt");

    fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
    atomic_write_file(&target, b"first").expect("write first");
    assert_eq!(fs::read_to_string(&target).expect("read first"), "first");

    atomic_write_file(&target, b"second").expect("write second");
    assert_eq!(fs::read_to_string(&target).expect("read second"), "second");

    let canonical = canonicalize_existing(&target).expect("canonicalize");
    assert!(canonical.is_absolute());
    assert!(canonical.ends_with("nested/output.txt"));
}
