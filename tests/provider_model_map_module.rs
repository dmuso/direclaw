use direclaw::provider::model_map::resolve_anthropic_model;

#[test]
fn model_map_module_resolves_aliases() {
    assert_eq!(
        resolve_anthropic_model("sonnet").expect("map sonnet"),
        "claude-sonnet-4-5"
    );
    assert!(resolve_anthropic_model("unknown-model").is_err());
}
