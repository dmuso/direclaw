use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use direclaw::setup::navigation::{
    setup_action_from_key, setup_screen_item_count, setup_transition, NavState, SetupAction,
    SetupNavEffect, SetupScreen,
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
