use direclaw::setup::navigation::NavState;
use direclaw::setup::screens::{field_row, project_setup_menu_view_model, tail_for_display};

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
