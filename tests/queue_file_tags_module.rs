use direclaw::queue::file_tags::{
    append_inbound_file_tags, extract_inbound_file_tags, prepare_outbound_content,
};

#[test]
fn queue_file_tags_module_exposes_existing_file_tag_helpers() {
    let text = "hello [file: /tmp/a.txt] [file: relative.txt]";
    assert_eq!(extract_inbound_file_tags(text), vec!["/tmp/a.txt"]);

    let rendered = append_inbound_file_tags("base", &["/tmp/one.txt".to_string()]);
    assert_eq!(rendered, "base\n[file: /tmp/one.txt]");

    let prepared = prepare_outbound_content("done");
    assert_eq!(prepared.message, "done");
    assert!(prepared.files.is_empty());
    assert!(prepared.omitted_files.is_empty());
}
