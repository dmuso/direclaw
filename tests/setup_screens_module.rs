use direclaw::config::WorkflowStepConfig;
use direclaw::setup::navigation::NavState;
use direclaw::setup::screens::{
    field_row, project_setup_menu_view_model, tail_for_display, workflow_step_menu_rows,
};

#[test]
fn setup_screens_module_projects_menu_and_clamps_selection() {
    let mut nav = NavState::root();
    nav.selected = 99;
    nav.status_text = "status".to_string();
    nav.hint_text = "hint".to_string();

    let model = project_setup_menu_view_model(true, &nav);
    assert_eq!(model.mode_line, "Mode: existing setup (edit + apply)");
    assert_eq!(model.items.len(), 5);
    assert_eq!(model.selected, 4);
    assert_eq!(model.status_text, "status");
    assert_eq!(model.hint_text, "hint");
}

#[test]
fn setup_screens_module_field_row_and_tail_helpers_are_stable() {
    let row = field_row("Provider", Some("anthropic".to_string()));
    assert_eq!(row.field, "Provider");
    assert_eq!(row.value.as_deref(), Some("anthropic"));

    assert_eq!(tail_for_display("abcdef", 4), "cdef");
    assert_eq!(tail_for_display("abc", 8), "abc");
    assert_eq!(tail_for_display("abc", 0), "");
}

#[test]
fn setup_screens_module_projects_workflow_step_rows() {
    let step: WorkflowStepConfig = serde_yaml::from_str(
        r#"
id: summarize
type: agent_task
agent: worker
prompt: Do work
workspace_mode: run_workspace
outputs:
  - summary
output_files:
  summary: outputs/summary.json
limits:
  max_retries: 3
"#,
    )
    .expect("parse step");

    let rows = workflow_step_menu_rows(&step);
    assert_eq!(rows.len(), 11);
    assert_eq!(rows[0].field, "Step ID");
    assert_eq!(rows[0].value.as_deref(), Some("summarize"));
    assert_eq!(rows[4].field, "Workspace Mode");
    assert_eq!(rows[4].value.as_deref(), Some("run_workspace"));
    assert_eq!(rows[10].field, "Max Retries");
    assert_eq!(rows[10].value.as_deref(), Some("3"));
}
