use direclaw::provider::output_parse::parse_openai_jsonl;

#[test]
fn output_parse_module_reads_last_openai_agent_message() {
    let data = r#"
{"type":"item.completed","item":{"type":"agent_message","text":"first"}}
{"type":"item.completed","item":{"type":"agent_message","content":[{"text":"second"}]}}
"#;

    let parsed = parse_openai_jsonl(data).expect("parsed jsonl");
    assert_eq!(parsed, "second");
}
