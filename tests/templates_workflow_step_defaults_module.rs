use direclaw::templates::workflow_step_defaults::{
    default_step_output_contract, default_step_output_files, default_step_scaffold,
};

#[test]
fn workflow_step_defaults_module_exposes_default_scaffold_and_output_contract() {
    let scaffold = default_step_scaffold("agent_task");
    assert!(scaffold.contains("workflow.output_paths.summary"));

    let outputs = default_step_output_contract("agent_task");
    assert_eq!(outputs.len(), 2);

    let output_files = default_step_output_files("agent_task");
    assert_eq!(output_files.len(), 2);
}
