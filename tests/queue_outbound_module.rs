use direclaw::queue::outbound::{
    prepare_outbound_content, OutboundContent, OUTBOUND_MAX_CHARS, OUTBOUND_TRUNCATE_KEEP_CHARS,
    OUTBOUND_TRUNCATION_SUFFIX,
};

#[test]
fn queue_outbound_module_exposes_existing_outbound_apis() {
    let prepared = prepare_outbound_content("done");
    assert_eq!(
        prepared,
        OutboundContent {
            message: "done".to_string(),
            files: Vec::new(),
            omitted_files: Vec::new(),
        }
    );

    assert_eq!(OUTBOUND_MAX_CHARS, 4000);
    assert_eq!(OUTBOUND_TRUNCATE_KEEP_CHARS, 3900);
    assert_eq!(OUTBOUND_TRUNCATION_SUFFIX, "\n\n[Response truncated...]");
}
