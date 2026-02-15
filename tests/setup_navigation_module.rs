use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use direclaw::setup::navigation::{
    parse_scripted_setup_keys, setup_action_from_key, setup_screen_item_count, setup_transition,
    NavState, SetupAction, SetupNavEffect, SetupScreen,
};

fn key_event(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn setup_navigation_module_maps_escape_by_screen() {
    assert_eq!(
        setup_action_from_key(SetupScreen::Root, key_event(KeyCode::Esc)),
        Some(SetupAction::Cancel)
    );
    assert_eq!(
        setup_action_from_key(SetupScreen::Workspaces, key_event(KeyCode::Esc)),
        Some(SetupAction::Back)
    );
}

#[test]
fn setup_navigation_module_routes_root_enter_to_initial_defaults() {
    let mut nav = NavState::root();
    nav.selected = 2;

    let transition =
        setup_transition(&mut nav, SetupAction::Enter, 5).expect("root enter transition");

    assert_eq!(
        transition.effect,
        SetupNavEffect::OpenScreen(SetupScreen::InitialAgentDefaults)
    );
    assert_eq!(setup_screen_item_count(nav.screen, 5), 2);
}

#[test]
fn setup_navigation_module_parses_scripted_keys() {
    let keys = parse_scripted_setup_keys("down,down,enter,t,esc,s").expect("parse scripted keys");
    let mapped = keys
        .iter()
        .map(|key| setup_action_from_key(SetupScreen::Root, *key))
        .collect::<Vec<_>>();
    assert_eq!(
        mapped,
        vec![
            Some(SetupAction::MoveNext),
            Some(SetupAction::MoveNext),
            Some(SetupAction::Enter),
            Some(SetupAction::Toggle),
            Some(SetupAction::Cancel),
            Some(SetupAction::Save),
        ]
    );
}
