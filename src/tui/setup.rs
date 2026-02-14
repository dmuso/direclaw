use crate::cli::{
    ensure_runtime_root, load_settings, map_config_err, save_orchestrator_registry,
    save_preferences, save_settings, RuntimePreferences,
};
use crate::config::{
    agent_editable_fields, default_global_config_path, AgentEditableField, ConfigProviderKind,
    OrchestrationLimitField, OrchestratorConfig, OutputKey, PathTemplate, SettingsOrchestrator,
    SetupDraft, WorkflowInputs, WorkflowStepConfig, WorkflowStepWorkspaceMode,
};
use crate::runtime::StatePaths;
use crate::workflow::{initial_orchestrator_config, WorkflowTemplate as SetupWorkflowTemplate};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Padding, Paragraph, Row, Table};
use ratatui::{Frame, Terminal};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::time::Duration;

type SetupState = SetupDraft;

pub(crate) fn cmd_setup() -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let config_exists = default_global_config_path()
        .map_err(map_config_err)?
        .exists();
    let mut state = load_setup_bootstrap(&paths)?;
    if let Some(scripted_keys) = load_scripted_setup_keys()? {
        match run_setup_scripted(&mut state, scripted_keys)? {
            SetupExit::Save => {}
            SetupExit::Cancel => return Ok("setup canceled".to_string()),
        }
    } else if is_interactive_setup() {
        match run_setup_tui(&mut state, config_exists)? {
            SetupExit::Save => {}
            SetupExit::Cancel => return Ok("setup canceled".to_string()),
        }
    }
    fs::create_dir_all(&state.workspaces_path).map_err(|e| {
        format!(
            "failed to create workspace {}: {e}",
            state.workspaces_path.display()
        )
    })?;
    let existing_settings = if config_exists {
        Some(load_settings()?)
    } else {
        None
    };
    let settings = state.normalize_for_save(existing_settings)?;

    let path = save_settings(&settings)?;
    save_orchestrator_registry(&settings, &state.orchestrator_configs)?;
    let orchestrator_path = settings
        .resolve_private_workspace(&state.orchestrator_id)
        .map_err(map_config_err)?
        .join("orchestrator.yaml");
    let prefs = RuntimePreferences {
        provider: Some(state.provider.clone()),
        model: Some(state.model.clone()),
    };
    save_preferences(&paths, &prefs)?;
    Ok(format!(
        "setup complete\nconfig={}\nstate_root={}\nworkspace={}\norchestrator={}\nnew_workflow_template={}\nprovider={}\nmodel={}\norchestrator_config={}",
        path.display(),
        paths.root.display(),
        state.workspaces_path.display(),
        state.orchestrator_id,
        state.workflow_template.as_str(),
        state.provider,
        state.model,
        orchestrator_path.display()
    ))
}

enum SetupExit {
    Save,
    Cancel,
}

fn is_interactive_setup() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn load_scripted_setup_keys() -> Result<Option<Vec<crossterm::event::KeyEvent>>, String> {
    let Ok(raw) = std::env::var("DIRECLAW_SETUP_SCRIPT_KEYS") else {
        return Ok(None);
    };
    let mut keys = Vec::new();
    for token in raw.split(',') {
        let normalized = token.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        let key = match normalized.as_str() {
            "up" => crossterm::event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            "down" => crossterm::event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            "enter" => crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            "esc" => crossterm::event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            "ctrl-c" => crossterm::event::KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            "a" => crossterm::event::KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            "d" => crossterm::event::KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
            "e" => crossterm::event::KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            "t" => crossterm::event::KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
            "s" => crossterm::event::KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
            other => {
                return Err(format!(
                    "invalid DIRECLAW_SETUP_SCRIPT_KEYS token `{other}`; valid tokens: up,down,enter,esc,ctrl-c,a,d,e,t,s"
                ));
            }
        };
        keys.push(key);
    }
    Ok(Some(keys))
}

fn default_model_for_provider(provider: &str) -> &'static str {
    if provider == "openai" {
        "gpt-5.3-codex"
    } else {
        "sonnet"
    }
}

const PROVIDER_OPTIONS: [ConfigProviderKind; 2] =
    [ConfigProviderKind::Anthropic, ConfigProviderKind::OpenAi];

fn provider_options() -> &'static [ConfigProviderKind] {
    &PROVIDER_OPTIONS
}

fn model_options_for_provider(provider: ConfigProviderKind) -> &'static [&'static str] {
    if provider == ConfigProviderKind::OpenAi {
        &["gpt-5.2", "gpt-5.3-codex"]
    } else {
        &["sonnet", "opus", "claude-sonnet-4-5", "claude-opus-4-6"]
    }
}

#[cfg(test)]
fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
    match kind {
        "orchestrator id" => crate::config::OrchestratorId::parse(value).map(|_| ()),
        "workflow id" => crate::config::WorkflowId::parse(value).map(|_| ()),
        "step id" => crate::config::StepId::parse(value).map(|_| ()),
        "agent id" => crate::config::AgentId::parse(value).map(|_| ()),
        _ => Err(format!("unsupported identifier kind `{kind}`")),
    }
}

fn infer_workflow_template(orchestrator: &OrchestratorConfig) -> SetupWorkflowTemplate {
    if orchestrator.agents.contains_key("planner")
        && orchestrator.agents.contains_key("builder")
        && orchestrator.agents.contains_key("reviewer")
    {
        return SetupWorkflowTemplate::Engineering;
    }
    if orchestrator.agents.contains_key("researcher") && orchestrator.agents.contains_key("writer")
    {
        return SetupWorkflowTemplate::Product;
    }
    SetupWorkflowTemplate::Minimal
}

fn load_setup_bootstrap(paths: &StatePaths) -> Result<SetupState, String> {
    let default_workspace = paths.root.join("workspaces");
    let mut bootstrap = SetupState {
        workspaces_path: default_workspace,
        orchestrator_id: "main".to_string(),
        provider: "anthropic".to_string(),
        model: "sonnet".to_string(),
        workflow_template: SetupWorkflowTemplate::Minimal,
        orchestrators: BTreeMap::from_iter([(
            "main".to_string(),
            SettingsOrchestrator {
                private_workspace: None,
                shared_access: Vec::new(),
            },
        )]),
        orchestrator_configs: BTreeMap::from_iter([(
            "main".to_string(),
            initial_orchestrator_config(
                "main",
                "anthropic",
                "sonnet",
                SetupWorkflowTemplate::Minimal,
            ),
        )]),
    };

    let config_path = default_global_config_path().map_err(map_config_err)?;
    if !config_path.exists() {
        return Ok(bootstrap);
    }

    let settings = load_settings()?;
    bootstrap.workspaces_path = settings.workspaces_path.clone();
    bootstrap.orchestrators = settings.orchestrators.clone();
    let mut configs = BTreeMap::new();
    for orchestrator_id in bootstrap.orchestrators.keys() {
        let private_workspace = settings
            .resolve_private_workspace(orchestrator_id)
            .map_err(map_config_err)?;
        let orchestrator_path = private_workspace.join("orchestrator.yaml");
        if orchestrator_path.exists() {
            let raw = fs::read_to_string(&orchestrator_path)
                .map_err(|e| format!("failed to read {}: {e}", orchestrator_path.display()))?;
            let config = serde_yaml::from_str::<OrchestratorConfig>(&raw)
                .map_err(|e| format!("failed to parse {}: {e}", orchestrator_path.display()))?;
            configs.insert(orchestrator_id.clone(), config);
        } else {
            configs.insert(
                orchestrator_id.clone(),
                initial_orchestrator_config(
                    orchestrator_id,
                    &bootstrap.provider,
                    &bootstrap.model,
                    SetupWorkflowTemplate::Minimal,
                ),
            );
        }
    }
    bootstrap.orchestrator_configs = configs;
    if let Some(first_orchestrator) = settings.orchestrators.keys().next() {
        bootstrap.orchestrator_id = first_orchestrator.clone();
        if let Some(orchestrator) = bootstrap.orchestrator_configs.get(first_orchestrator) {
            if let Some(selector) = orchestrator.agents.get(&orchestrator.selector_agent) {
                bootstrap.provider = selector.provider.to_string();
                bootstrap.model = selector.model.clone();
            } else if let Some((_, agent)) = orchestrator.agents.iter().next() {
                bootstrap.provider = agent.provider.to_string();
                bootstrap.model = agent.model.clone();
            }
            bootstrap.workflow_template = infer_workflow_template(orchestrator);
        }
    }

    Ok(bootstrap)
}

const SETUP_MENU_ITEMS: [&str; 5] = [
    "Workspaces",
    "Orchestrators",
    "Initial Agent Defaults",
    "Save Setup",
    "Cancel",
];

const ROOT_STATUS_TEXT: &str = "Enter opens a section. Esc cancels setup.";
const ROOT_HINT_TEXT: &str = "Up/Down move | Enter open | Esc cancel";
const WORKSPACES_STATUS_TEXT: &str = "Enter to edit workspace path. Esc back.";
const WORKSPACES_HINT_TEXT: &str = "Enter edit | Esc back";
const INITIAL_DEFAULTS_STATUS_TEXT: &str = "Enter to edit/toggle. Esc back.";
const INITIAL_DEFAULTS_HINT_TEXT: &str = "Up/Down move | Enter edit/toggle | Esc back";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupScreen {
    Root,
    Workspaces,
    Orchestrators,
    InitialAgentDefaults,
    NewWorkflowTemplate,
    OrchestratorDetail,
    OrchestrationLimits,
    Workflows,
    WorkflowDetail,
    WorkflowSteps,
    WorkflowStepDetail,
    AddStarterWorkflows,
    Agents,
    AgentDetail,
}

const ALL_SETUP_SCREENS: [SetupScreen; 14] = [
    SetupScreen::Root,
    SetupScreen::Workspaces,
    SetupScreen::Orchestrators,
    SetupScreen::InitialAgentDefaults,
    SetupScreen::NewWorkflowTemplate,
    SetupScreen::OrchestratorDetail,
    SetupScreen::OrchestrationLimits,
    SetupScreen::Workflows,
    SetupScreen::WorkflowDetail,
    SetupScreen::WorkflowSteps,
    SetupScreen::WorkflowStepDetail,
    SetupScreen::AddStarterWorkflows,
    SetupScreen::Agents,
    SetupScreen::AgentDetail,
];

impl SetupScreen {
    fn as_str(self) -> &'static str {
        match self {
            SetupScreen::Root => "root",
            SetupScreen::Workspaces => "workspaces",
            SetupScreen::Orchestrators => "orchestrators",
            SetupScreen::InitialAgentDefaults => "initial_agent_defaults",
            SetupScreen::NewWorkflowTemplate => "new_workflow_template",
            SetupScreen::OrchestratorDetail => "orchestrator_detail",
            SetupScreen::OrchestrationLimits => "orchestration_limits",
            SetupScreen::Workflows => "workflows",
            SetupScreen::WorkflowDetail => "workflow_detail",
            SetupScreen::WorkflowSteps => "workflow_steps",
            SetupScreen::WorkflowStepDetail => "workflow_step_detail",
            SetupScreen::AddStarterWorkflows => "add_starter_workflows",
            SetupScreen::Agents => "agents",
            SetupScreen::AgentDetail => "agent_detail",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupAction {
    MovePrev,
    MoveNext,
    Enter,
    Back,
    Save,
    Cancel,
    Edit,
    Add,
    Delete,
    Toggle,
    ReconcileSelection(usize),
}

impl SetupAction {
    fn as_str(self) -> &'static str {
        match self {
            SetupAction::MovePrev => "move_prev",
            SetupAction::MoveNext => "move_next",
            SetupAction::Enter => "enter",
            SetupAction::Back => "back",
            SetupAction::Save => "save",
            SetupAction::Cancel => "cancel",
            SetupAction::Edit => "edit",
            SetupAction::Add => "add",
            SetupAction::Delete => "delete",
            SetupAction::Toggle => "toggle",
            SetupAction::ReconcileSelection(_) => "reconcile_selection",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavState {
    screen: SetupScreen,
    selected: usize,
    status_text: String,
    hint_text: String,
}

impl NavState {
    fn root() -> Self {
        Self {
            screen: SetupScreen::Root,
            selected: 0,
            status_text: ROOT_STATUS_TEXT.to_string(),
            hint_text: ROOT_HINT_TEXT.to_string(),
        }
    }

    fn clamp_selection(&mut self, len: usize) {
        self.selected = clamp_selection(self.selected, len);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupNavEffect {
    None,
    OpenScreen(SetupScreen),
    OpenOrchestratorManager,
    EditWorkspacePath,
    ToggleDefaultProvider,
    EditDefaultModel,
    SaveSetup,
    CancelSetup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetupTransition {
    effect: SetupNavEffect,
    feedback: Option<String>,
}

impl SetupTransition {
    fn no_op(feedback: Option<String>) -> Self {
        Self {
            effect: SetupNavEffect::None,
            feedback,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupNavError {
    InvalidTransition {
        screen: SetupScreen,
        action: SetupAction,
    },
}

impl std::fmt::Display for SetupNavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupNavError::InvalidTransition { screen, action } => {
                write!(
                    f,
                    "invalid setup transition: screen={} action={}",
                    screen.as_str(),
                    action.as_str()
                )
            }
        }
    }
}

fn clamp_selection(selected: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    selected.min(len - 1)
}

fn setup_action_from_key(
    screen: SetupScreen,
    key: crossterm::event::KeyEvent,
) -> Option<SetupAction> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(SetupAction::Cancel);
    }
    match key.code {
        KeyCode::Up => Some(SetupAction::MovePrev),
        KeyCode::Down => Some(SetupAction::MoveNext),
        KeyCode::Esc => Some(if screen == SetupScreen::Root {
            SetupAction::Cancel
        } else {
            SetupAction::Back
        }),
        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => Some(SetupAction::Enter),
        KeyCode::Char('a') => Some(SetupAction::Add),
        KeyCode::Char('d') => Some(SetupAction::Delete),
        KeyCode::Char('e') => Some(SetupAction::Edit),
        KeyCode::Char('t') => Some(SetupAction::Toggle),
        KeyCode::Char('s') => Some(SetupAction::Save),
        _ => None,
    }
}

fn setup_transition(
    state: &mut NavState,
    action: SetupAction,
    root_item_count: usize,
) -> Result<SetupTransition, SetupNavError> {
    if let SetupAction::ReconcileSelection(len) = action {
        let previous = state.selected;
        state.clamp_selection(len);
        if previous != state.selected {
            return Ok(SetupTransition::no_op(Some(
                "selection adjusted".to_string(),
            )));
        }
        return Ok(SetupTransition::no_op(None));
    }

    match state.screen {
        SetupScreen::Root => match action {
            SetupAction::MovePrev => {
                state.selected = state.selected.saturating_sub(1);
                Ok(SetupTransition::no_op(None))
            }
            SetupAction::MoveNext => {
                let max_index = root_item_count.saturating_sub(1);
                state.selected = std::cmp::min(state.selected + 1, max_index);
                Ok(SetupTransition::no_op(None))
            }
            SetupAction::Enter => {
                let effect = match state.selected {
                    0 => {
                        state.screen = SetupScreen::Workspaces;
                        state.selected = 0;
                        state.status_text = WORKSPACES_STATUS_TEXT.to_string();
                        state.hint_text = WORKSPACES_HINT_TEXT.to_string();
                        SetupNavEffect::OpenScreen(SetupScreen::Workspaces)
                    }
                    1 => SetupNavEffect::OpenOrchestratorManager,
                    2 => {
                        state.screen = SetupScreen::InitialAgentDefaults;
                        state.selected = 0;
                        state.status_text = INITIAL_DEFAULTS_STATUS_TEXT.to_string();
                        state.hint_text = INITIAL_DEFAULTS_HINT_TEXT.to_string();
                        SetupNavEffect::OpenScreen(SetupScreen::InitialAgentDefaults)
                    }
                    3 => SetupNavEffect::SaveSetup,
                    _ => SetupNavEffect::CancelSetup,
                };
                Ok(SetupTransition {
                    effect,
                    feedback: None,
                })
            }
            SetupAction::Back => Ok(SetupTransition {
                effect: SetupNavEffect::CancelSetup,
                feedback: None,
            }),
            SetupAction::Save => Ok(SetupTransition {
                effect: SetupNavEffect::SaveSetup,
                feedback: None,
            }),
            SetupAction::Cancel => Ok(SetupTransition {
                effect: SetupNavEffect::CancelSetup,
                feedback: None,
            }),
            SetupAction::Edit
            | SetupAction::Add
            | SetupAction::Delete
            | SetupAction::Toggle
            | SetupAction::ReconcileSelection(_) => Err(SetupNavError::InvalidTransition {
                screen: state.screen,
                action,
            }),
        },
        SetupScreen::Workspaces => match action {
            SetupAction::MovePrev | SetupAction::MoveNext => Ok(SetupTransition::no_op(None)),
            SetupAction::Enter | SetupAction::Edit => Ok(SetupTransition {
                effect: SetupNavEffect::EditWorkspacePath,
                feedback: None,
            }),
            SetupAction::Back => {
                state.screen = SetupScreen::Root;
                state.selected = 0;
                state.hint_text = ROOT_HINT_TEXT.to_string();
                Ok(SetupTransition::no_op(Some(
                    "Closed Workspaces.".to_string(),
                )))
            }
            SetupAction::Cancel => Ok(SetupTransition {
                effect: SetupNavEffect::CancelSetup,
                feedback: None,
            }),
            SetupAction::Add | SetupAction::Delete | SetupAction::Toggle | SetupAction::Save => {
                Err(SetupNavError::InvalidTransition {
                    screen: state.screen,
                    action,
                })
            }
            SetupAction::ReconcileSelection(_) => unreachable!(),
        },
        SetupScreen::InitialAgentDefaults => match action {
            SetupAction::MovePrev => {
                state.selected = state.selected.saturating_sub(1);
                Ok(SetupTransition::no_op(None))
            }
            SetupAction::MoveNext => {
                state.selected = std::cmp::min(state.selected + 1, 1);
                Ok(SetupTransition::no_op(None))
            }
            SetupAction::Enter => {
                let effect = if state.selected == 0 {
                    SetupNavEffect::ToggleDefaultProvider
                } else {
                    SetupNavEffect::EditDefaultModel
                };
                Ok(SetupTransition {
                    effect,
                    feedback: None,
                })
            }
            SetupAction::Toggle if state.selected == 0 => Ok(SetupTransition {
                effect: SetupNavEffect::ToggleDefaultProvider,
                feedback: None,
            }),
            SetupAction::Edit if state.selected == 1 => Ok(SetupTransition {
                effect: SetupNavEffect::EditDefaultModel,
                feedback: None,
            }),
            SetupAction::Back => {
                state.screen = SetupScreen::Root;
                state.selected = 0;
                state.hint_text = ROOT_HINT_TEXT.to_string();
                Ok(SetupTransition::no_op(Some(
                    "Closed Initial Agent Defaults.".to_string(),
                )))
            }
            SetupAction::Cancel => Ok(SetupTransition {
                effect: SetupNavEffect::CancelSetup,
                feedback: None,
            }),
            SetupAction::Add | SetupAction::Delete | SetupAction::Save => {
                Err(SetupNavError::InvalidTransition {
                    screen: state.screen,
                    action,
                })
            }
            SetupAction::Edit | SetupAction::Toggle => Ok(SetupTransition::no_op(Some(
                "Choose a compatible field before editing.".to_string(),
            ))),
            SetupAction::ReconcileSelection(_) => unreachable!(),
        },
        _ => match action {
            SetupAction::Back => {
                state.screen = SetupScreen::Root;
                state.selected = 0;
                state.hint_text = ROOT_HINT_TEXT.to_string();
                Ok(SetupTransition::no_op(Some(
                    "Returned to setup menu.".to_string(),
                )))
            }
            SetupAction::Cancel => Ok(SetupTransition {
                effect: SetupNavEffect::CancelSetup,
                feedback: None,
            }),
            SetupAction::Save | SetupAction::ReconcileSelection(_) => {
                Err(SetupNavError::InvalidTransition {
                    screen: state.screen,
                    action,
                })
            }
            SetupAction::MovePrev
            | SetupAction::MoveNext
            | SetupAction::Enter
            | SetupAction::Edit
            | SetupAction::Add
            | SetupAction::Delete
            | SetupAction::Toggle => Ok(SetupTransition::no_op(Some(
                "Action is not mapped for this setup screen.".to_string(),
            ))),
        },
    }
}

struct SetupMenuViewModel {
    mode_line: String,
    items: Vec<String>,
    selected: usize,
    status_text: String,
    hint_text: String,
}

fn project_setup_menu_view_model(config_exists: bool, state: &NavState) -> SetupMenuViewModel {
    debug_assert!(ALL_SETUP_SCREENS.contains(&state.screen));
    SetupMenuViewModel {
        mode_line: if config_exists {
            "Mode: existing setup (edit + apply)".to_string()
        } else {
            "Mode: first-time setup".to_string()
        },
        items: SETUP_MENU_ITEMS
            .iter()
            .map(|item| (*item).to_string())
            .collect(),
        selected: clamp_selection(state.selected, SETUP_MENU_ITEMS.len()),
        status_text: state.status_text.clone(),
        hint_text: state.hint_text.clone(),
    }
}

fn run_setup_tui(bootstrap: &mut SetupState, config_exists: bool) -> Result<SetupExit, String> {
    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|e| format!("failed to enable raw mode: {e}"))?;
    execute!(stdout, EnterAlternateScreen, Hide)
        .map_err(|e| format!("failed to enter setup screen: {e}"))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        Terminal::new(backend).map_err(|e| format!("failed to create setup terminal: {e}"))?;
    let result = run_setup_tui_loop(bootstrap, config_exists, &mut terminal);
    disable_raw_mode().map_err(|e| format!("failed to disable raw mode: {e}"))?;
    execute!(terminal.backend_mut(), Show, LeaveAlternateScreen)
        .map_err(|e| format!("failed to leave setup screen: {e}"))?;
    result
}

fn run_setup_tui_loop(
    bootstrap: &mut SetupState,
    config_exists: bool,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<SetupExit, String> {
    let mut nav = NavState::root();
    loop {
        let item_count = setup_screen_item_count(nav.screen);
        let transition = setup_transition(
            &mut nav,
            SetupAction::ReconcileSelection(item_count),
            SETUP_MENU_ITEMS.len(),
        )
        .map_err(|err| err.to_string())?;
        if let Some(feedback) = transition.feedback {
            nav.status_text = feedback;
        }
        draw_active_setup_screen(terminal, config_exists, &nav, bootstrap)?;
        if !event::poll(Duration::from_millis(250))
            .map_err(|e| format!("failed to poll setup input: {e}"))?
        {
            continue;
        }
        let ev = event::read().map_err(|e| format!("failed to read setup input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        let Some(action) = setup_action_from_key(nav.screen, key) else {
            continue;
        };
        let transition = match setup_transition(&mut nav, action, SETUP_MENU_ITEMS.len()) {
            Ok(transition) => transition,
            Err(err) => {
                nav.status_text = err.to_string();
                continue;
            }
        };
        if let Some(feedback) = transition.feedback {
            nav.status_text = feedback;
        }
        if let Some(exit) = apply_setup_effect_tui(
            terminal,
            bootstrap,
            config_exists,
            &mut nav,
            transition.effect,
        )? {
            return Ok(exit);
        }
    }
}

fn setup_screen_item_count(screen: SetupScreen) -> usize {
    match screen {
        SetupScreen::Root => SETUP_MENU_ITEMS.len(),
        SetupScreen::Workspaces => 1,
        SetupScreen::InitialAgentDefaults => 2,
        _ => 0,
    }
}

fn draw_active_setup_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config_exists: bool,
    nav: &NavState,
    bootstrap: &SetupState,
) -> Result<(), String> {
    match nav.screen {
        SetupScreen::Root => {
            let view_model = project_setup_menu_view_model(config_exists, nav);
            terminal
                .draw(|frame| draw_setup_ui(frame, &view_model))
                .map_err(|e| format!("failed to render setup ui: {e}"))?;
        }
        SetupScreen::Workspaces => {
            let items = vec![format!(
                "Workspace Path: {}",
                bootstrap.workspaces_path.display()
            )];
            draw_list_screen(
                terminal,
                "Setup > Workspaces",
                config_exists,
                &items,
                0,
                &nav.status_text,
                &nav.hint_text,
            )?;
        }
        SetupScreen::InitialAgentDefaults => {
            let items = vec![
                format!("Provider: {}", bootstrap.provider),
                format!("Model: {}", bootstrap.model),
            ];
            draw_list_screen(
                terminal,
                "Setup > Initial Agent Defaults",
                config_exists,
                &items,
                nav.selected,
                &nav.status_text,
                &nav.hint_text,
            )?;
        }
        _ => {
            let view_model = project_setup_menu_view_model(config_exists, nav);
            terminal
                .draw(|frame| draw_setup_ui(frame, &view_model))
                .map_err(|e| format!("failed to render setup ui: {e}"))?;
        }
    }
    Ok(())
}

fn toggle_default_provider(bootstrap: &mut SetupState) -> String {
    let next_provider = if bootstrap.provider == "anthropic" {
        "openai".to_string()
    } else {
        "anthropic".to_string()
    };
    bootstrap.set_default_provider(next_provider);
    if bootstrap.model == "sonnet" || bootstrap.model == "gpt-5.3-codex" {
        bootstrap.set_default_model(default_model_for_provider(&bootstrap.provider).to_string());
    }
    format!("provider set to {}", bootstrap.provider)
}

fn apply_setup_effect_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    nav: &mut NavState,
    effect: SetupNavEffect,
) -> Result<Option<SetupExit>, String> {
    match effect {
        SetupNavEffect::None | SetupNavEffect::OpenScreen(_) => Ok(None),
        SetupNavEffect::OpenOrchestratorManager => {
            if let Some(message) = run_orchestrator_manager_tui(terminal, bootstrap, config_exists)?
            {
                nav.status_text = message;
            }
            Ok(None)
        }
        SetupNavEffect::EditWorkspacePath => {
            if let Some(value) = prompt_line_tui(
                terminal,
                "Workspace Path",
                "Enter workspace path:",
                &bootstrap.workspaces_path.display().to_string(),
            )? {
                if value.trim().is_empty() {
                    nav.status_text = "workspace path must be non-empty".to_string();
                } else {
                    bootstrap.set_workspaces_path(PathBuf::from(value.trim()));
                    nav.status_text = "workspace path updated".to_string();
                }
            }
            Ok(None)
        }
        SetupNavEffect::ToggleDefaultProvider => {
            nav.status_text = toggle_default_provider(bootstrap);
            Ok(None)
        }
        SetupNavEffect::EditDefaultModel => {
            if let Some(value) =
                prompt_line_tui(terminal, "Default Model", "Enter model:", &bootstrap.model)?
            {
                if value.trim().is_empty() {
                    nav.status_text = "model must be non-empty".to_string();
                } else {
                    bootstrap.set_default_model(value.trim().to_string());
                    nav.status_text = "model updated".to_string();
                }
            }
            Ok(None)
        }
        SetupNavEffect::SaveSetup => Ok(Some(SetupExit::Save)),
        SetupNavEffect::CancelSetup => Ok(Some(SetupExit::Cancel)),
    }
}

fn run_setup_scripted(
    bootstrap: &mut SetupState,
    scripted_keys: Vec<crossterm::event::KeyEvent>,
) -> Result<SetupExit, String> {
    let mut nav = NavState::root();
    for key in scripted_keys {
        let item_count = setup_screen_item_count(nav.screen);
        let reconcile = setup_transition(
            &mut nav,
            SetupAction::ReconcileSelection(item_count),
            SETUP_MENU_ITEMS.len(),
        )
        .map_err(|err| err.to_string())?;
        if let Some(feedback) = reconcile.feedback {
            nav.status_text = feedback;
        }
        let Some(action) = setup_action_from_key(nav.screen, key) else {
            continue;
        };
        let transition = setup_transition(&mut nav, action, SETUP_MENU_ITEMS.len())
            .map_err(|e| e.to_string())?;
        if let Some(feedback) = transition.feedback {
            nav.status_text = feedback;
        }
        if let Some(exit) = apply_setup_effect_scripted(bootstrap, &mut nav, transition.effect)? {
            return Ok(exit);
        }
    }
    Err("scripted setup did not terminate; include save/cancel key".to_string())
}

fn apply_setup_effect_scripted(
    bootstrap: &mut SetupState,
    nav: &mut NavState,
    effect: SetupNavEffect,
) -> Result<Option<SetupExit>, String> {
    match effect {
        SetupNavEffect::None | SetupNavEffect::OpenScreen(_) => Ok(None),
        SetupNavEffect::OpenOrchestratorManager => {
            Err("scripted setup does not support orchestrator manager actions".to_string())
        }
        SetupNavEffect::EditWorkspacePath => {
            Err("scripted setup does not support workspace path prompt actions".to_string())
        }
        SetupNavEffect::ToggleDefaultProvider => {
            nav.status_text = toggle_default_provider(bootstrap);
            Ok(None)
        }
        SetupNavEffect::EditDefaultModel => {
            Err("scripted setup does not support default model prompt actions".to_string())
        }
        SetupNavEffect::SaveSetup => Ok(Some(SetupExit::Save)),
        SetupNavEffect::CancelSetup => Ok(Some(SetupExit::Cancel)),
    }
}

fn setup_workflow_template_index(template: SetupWorkflowTemplate) -> usize {
    match template {
        SetupWorkflowTemplate::Minimal => 0,
        SetupWorkflowTemplate::Engineering => 1,
        SetupWorkflowTemplate::Product => 2,
    }
}

fn workflow_template_from_index(index: usize) -> SetupWorkflowTemplate {
    match index {
        0 => SetupWorkflowTemplate::Minimal,
        1 => SetupWorkflowTemplate::Engineering,
        _ => SetupWorkflowTemplate::Product,
    }
}

fn workflow_template_options() -> Vec<String> {
    vec![
        "minimal: default agent + default workflow (single-step baseline)".to_string(),
        "engineering: planner/builder/reviewer + feature_delivery, quick_answer".to_string(),
        "product: researcher/writer + prd_draft, release_notes".to_string(),
    ]
}

struct TemplatePickerUi<'a> {
    title: &'a str,
    closed_message: &'a str,
    apply_message_prefix: &'a str,
    status_text: &'a str,
    hint_text: &'a str,
}

fn run_template_picker_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    initial: SetupWorkflowTemplate,
    ui: TemplatePickerUi<'_>,
    config_exists: bool,
) -> Result<(Option<SetupWorkflowTemplate>, String), String> {
    let mut selected = setup_workflow_template_index(initial);
    let status = ui.status_text.to_string();
    loop {
        let items = workflow_template_options();
        draw_list_screen(
            terminal,
            ui.title,
            config_exists,
            &items,
            selected,
            &status,
            ui.hint_text,
        )?;
        let ev =
            event::read().map_err(|e| format!("failed to read workflow template input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok((None, ui.closed_message.to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = std::cmp::min(selected + 1, 2),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                let template = workflow_template_from_index(selected);
                let message = format!("{} {}", ui.apply_message_prefix, template.as_str());
                return Ok((Some(template), message));
            }
            _ => {}
        }
    }
}

fn run_new_workflow_template_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
) -> Result<Option<String>, String> {
    let (selection, message) = run_template_picker_tui(
        terminal,
        bootstrap.workflow_template,
        TemplatePickerUi {
            title: "Setup > Orchestrators > Default New-Orchestrator Workflow Template",
            closed_message: "Closed workflow template selector.",
            apply_message_prefix: "new workflow template set to",
            status_text:
                "Workflow template used when creating orchestrators. Enter sets default template. Esc back.",
            hint_text: "Up/Down move | Enter set default template | Esc back",
        },
        config_exists,
    )?;
    if let Some(template) = selection {
        bootstrap.set_default_workflow_template(template);
    }
    Ok(Some(message))
}

fn run_orchestrator_manager_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status = "Enter open orchestrator. a add, d delete, e set primary, t set default workflow template. Esc back.".to_string();
    loop {
        bootstrap.ensure_minimum_orchestrator();
        let ids: Vec<String> = bootstrap.orchestrators.keys().cloned().collect();
        selected = selected.min(ids.len().saturating_sub(1));
        let selected_id = ids[selected].clone();

        let items = ids
            .iter()
            .map(|id| {
                if *id == bootstrap.orchestrator_id {
                    format!("{id} (primary)")
                } else {
                    id.clone()
                }
            })
            .collect::<Vec<_>>();
        draw_list_screen(
            terminal,
            "Setup > Orchestrators",
            config_exists,
            &items,
            selected,
            &status,
            "Up/Down move | Enter open | a add | d delete | e set primary | t set default workflow template | Esc back",
        )?;

        if !event::poll(Duration::from_millis(250))
            .map_err(|e| format!("failed to poll orchestrator manager input: {e}"))?
        {
            continue;
        }
        let ev =
            event::read().map_err(|e| format!("failed to read orchestrator manager input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(None);
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed Orchestrators.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = std::cmp::min(selected + 1, ids.len().saturating_sub(1)),
            KeyCode::Enter => {
                if let Some(message) =
                    run_orchestrator_detail_tui(terminal, bootstrap, config_exists, &selected_id)?
                {
                    status = message;
                }
            }
            KeyCode::Char('e') => match bootstrap.set_primary_orchestrator(&selected_id) {
                Ok(_) => status = "Primary orchestrator updated.".to_string(),
                Err(err) => status = err,
            },
            KeyCode::Char('t') => {
                if let Some(message) =
                    run_new_workflow_template_tui(terminal, bootstrap, config_exists)?
                {
                    status = message;
                }
            }
            KeyCode::Char('a') => {
                if let Some(id) = prompt_line_tui(
                    terminal,
                    "Add Orchestrator",
                    "New orchestrator id (slug, non-empty):",
                    "",
                )? {
                    let id = id.trim().to_string();
                    if id.is_empty() {
                        status = "orchestrator id must be non-empty".to_string();
                    } else {
                        match bootstrap.add_orchestrator(&id) {
                            Ok(_) => {
                                if let Some(pos) =
                                    bootstrap.orchestrators.keys().position(|v| v == &id)
                                {
                                    selected = pos;
                                }
                                status = "orchestrator created".to_string();
                            }
                            Err(err) => status = err,
                        }
                    }
                }
            }
            KeyCode::Char('d') => match bootstrap.remove_orchestrator(&selected_id) {
                Ok(_) => {
                    selected = selected.saturating_sub(1);
                    status = "orchestrator removed".to_string();
                }
                Err(err) => status = err,
            },
            _ => {}
        }
    }
}

fn run_orchestrator_detail_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status = "Enter to edit selected option. Esc back.".to_string();
    loop {
        let rows = orchestrator_detail_menu_rows(bootstrap, orchestrator_id);
        draw_field_screen(
            terminal,
            &format!("Setup > Orchestrators > {orchestrator_id}"),
            config_exists,
            &rows,
            selected,
            &status,
            "Up/Down move | Enter select | Esc back",
        )?;
        let ev =
            event::read().map_err(|e| format!("failed to read orchestrator detail input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed orchestrator view.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = std::cmp::min(selected + 1, rows.len().saturating_sub(1)),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => match selected {
                0 => match bootstrap.set_primary_orchestrator(orchestrator_id) {
                    Ok(_) => status = "primary orchestrator updated".to_string(),
                    Err(err) => status = err,
                },
                1 => {
                    let current = bootstrap
                        .orchestrators
                        .get(orchestrator_id)
                        .and_then(|o| o.private_workspace.as_ref())
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Private Workspace",
                        "Set private workspace (empty clears):",
                        &current,
                    )? {
                        let next = if value.trim().is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(value.trim()))
                        };
                        match bootstrap.set_orchestrator_private_workspace(orchestrator_id, next) {
                            Ok(_) => status = "private workspace updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                2 => {
                    let current = bootstrap
                        .orchestrators
                        .get(orchestrator_id)
                        .map(|o| o.shared_access.join(","))
                        .unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Shared Access",
                        "Comma-separated shared workspace keys:",
                        &current,
                    )? {
                        let shared_access = value
                            .split(',')
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty())
                            .collect();
                        match bootstrap
                            .set_orchestrator_shared_access(orchestrator_id, shared_access)
                        {
                            Ok(_) => status = "shared_access updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                3 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .map(|o| o.default_workflow.clone())
                        .unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Default Workflow",
                        "Set default_workflow:",
                        &current,
                    )? {
                        if value.trim().is_empty() {
                            status = "default_workflow must be non-empty".to_string();
                        } else {
                            match bootstrap
                                .set_orchestrator_default_workflow(orchestrator_id, value.trim())
                            {
                                Ok(_) => status = "default_workflow updated".to_string(),
                                Err(err) => status = err,
                            }
                        }
                    }
                }
                4 => {
                    let current_template = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .map(infer_workflow_template)
                        .unwrap_or(SetupWorkflowTemplate::Minimal);
                    let (selection, message) = run_template_picker_tui(
                        terminal,
                        current_template,
                        TemplatePickerUi {
                            title: &format!(
                                "Setup > Orchestrators > {orchestrator_id} > Add Starter Workflows"
                            ),
                            closed_message: "Closed template picker (no changes).",
                            apply_message_prefix: &format!(
                                "applied starter workflow template to orchestrator {orchestrator_id}:"
                            ),
                            status_text:
                                "Non-destructive: adds template workflows and missing agents; does not remove existing config.",
                            hint_text: "Up/Down move | Enter add starter workflows | Esc back",
                        },
                        config_exists,
                    )?;
                    if let Some(template) = selection {
                        bootstrap
                            .apply_workflow_template_to_orchestrator(orchestrator_id, template)?;
                    }
                    status = message;
                }
                5 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .map(|o| o.selection_max_retries.to_string())
                        .unwrap_or_else(|| "1".to_string());
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Selection Max Retries",
                        "Set selection_max_retries (>=1):",
                        &current,
                    )? {
                        match value.trim().parse::<u32>() {
                            Ok(v) => match bootstrap
                                .set_orchestrator_selection_max_retries(orchestrator_id, v)
                            {
                                Ok(_) => status = "selection_max_retries updated".to_string(),
                                Err(err) => status = err,
                            },
                            _ => status = "selection_max_retries must be >= 1".to_string(),
                        }
                    }
                }
                6 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .map(|o| o.selector_timeout_seconds.to_string())
                        .unwrap_or_else(|| "30".to_string());
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Selector Timeout Seconds",
                        "Set selector_timeout_seconds (>=1):",
                        &current,
                    )? {
                        match value.trim().parse::<u64>() {
                            Ok(v) => match bootstrap
                                .set_orchestrator_selector_timeout_seconds(orchestrator_id, v)
                            {
                                Ok(_) => status = "selector_timeout_seconds updated".to_string(),
                                Err(err) => status = err,
                            },
                            _ => status = "selector_timeout_seconds must be >= 1".to_string(),
                        }
                    }
                }
                7 => {
                    if let Some(message) = run_workflow_orchestration_tui(
                        terminal,
                        bootstrap,
                        config_exists,
                        orchestrator_id,
                    )? {
                        status = message;
                    }
                }
                8 => {
                    if let Some(message) = run_workflow_manager_tui(
                        terminal,
                        bootstrap,
                        config_exists,
                        orchestrator_id,
                    )? {
                        status = message;
                    }
                }
                _ => {
                    if let Some(message) =
                        run_agent_manager_tui(terminal, bootstrap, config_exists, orchestrator_id)?
                    {
                        status = message;
                    }
                }
            },
            _ => {}
        }
    }
}

struct SetupFieldRow {
    field: String,
    value: Option<String>,
}

fn field_row(field: &str, value: Option<String>) -> SetupFieldRow {
    SetupFieldRow {
        field: field.to_string(),
        value,
    }
}

fn orchestrator_detail_menu_rows(
    bootstrap: &SetupState,
    orchestrator_id: &str,
) -> Vec<SetupFieldRow> {
    let private_workspace = bootstrap
        .orchestrators
        .get(orchestrator_id)
        .and_then(|o| o.private_workspace.as_ref())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<default>".to_string());
    let shared_access = bootstrap
        .orchestrators
        .get(orchestrator_id)
        .map(|o| {
            if o.shared_access.is_empty() {
                "<none>".to_string()
            } else {
                o.shared_access.join(",")
            }
        })
        .unwrap_or_else(|| "<none>".to_string());
    let (default_workflow, selection_max_retries, selector_timeout_seconds) = bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .map(|cfg| {
            (
                cfg.default_workflow.clone(),
                cfg.selection_max_retries.to_string(),
                cfg.selector_timeout_seconds.to_string(),
            )
        })
        .unwrap_or_else(|| {
            (
                "<missing>".to_string(),
                "<missing>".to_string(),
                "<missing>".to_string(),
            )
        });

    vec![
        field_row(
            "Set As Primary",
            Some(if bootstrap.orchestrator_id == orchestrator_id {
                "yes".to_string()
            } else {
                "no".to_string()
            }),
        ),
        field_row("Private Workspace", Some(private_workspace)),
        field_row("Shared Access", Some(shared_access)),
        field_row("Default Workflow", Some(default_workflow)),
        field_row(
            "Add Starter Workflows",
            bootstrap
                .orchestrator_configs
                .get(orchestrator_id)
                .map(infer_workflow_template)
                .map(|template| format!("suggested workflow template: {}", template.as_str())),
        ),
        field_row("Selection Max Retries", Some(selection_max_retries)),
        field_row("Selector Timeout Seconds", Some(selector_timeout_seconds)),
        field_row("Workflow Orchestration Limits", None),
        field_row("Workflows", None),
        field_row("Agents", None),
    ]
}

fn workflow_orchestration_menu_rows(
    bootstrap: &SetupState,
    orchestrator_id: &str,
) -> Vec<SetupFieldRow> {
    let orchestration = bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .and_then(|cfg| cfg.workflow_orchestration.as_ref());
    vec![
        field_row(
            "Max Total Iterations",
            Some(
                orchestration
                    .and_then(|o| o.max_total_iterations)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<none>".to_string()),
            ),
        ),
        field_row(
            "Default Run Timeout Seconds",
            Some(
                orchestration
                    .and_then(|o| o.default_run_timeout_seconds)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<none>".to_string()),
            ),
        ),
        field_row(
            "Default Step Timeout Seconds",
            Some(
                orchestration
                    .and_then(|o| o.default_step_timeout_seconds)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<none>".to_string()),
            ),
        ),
        field_row(
            "Max Step Timeout Seconds",
            Some(
                orchestration
                    .and_then(|o| o.max_step_timeout_seconds)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<none>".to_string()),
            ),
        ),
    ]
}

fn run_workflow_orchestration_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status = "Enter to edit selected limit. Esc back.".to_string();
    loop {
        let rows = workflow_orchestration_menu_rows(bootstrap, orchestrator_id);
        draw_field_screen(
            terminal,
            &format!("Setup > Orchestrators > {orchestrator_id} > Workflow Orchestration Limits"),
            config_exists,
            &rows,
            selected,
            &status,
            "Up/Down move | Enter edit | Esc back",
        )?;
        let ev = event::read()
            .map_err(|e| format!("failed to read workflow orchestration input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed workflow orchestration limits.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = std::cmp::min(selected + 1, rows.len().saturating_sub(1)),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                let (label, prompt) = match selected {
                    0 => (
                        "Max Total Iterations",
                        "Set max_total_iterations (empty clears, >=1):",
                    ),
                    1 => (
                        "Default Run Timeout Seconds",
                        "Set default_run_timeout_seconds (empty clears, >=1):",
                    ),
                    2 => (
                        "Default Step Timeout Seconds",
                        "Set default_step_timeout_seconds (empty clears, >=1):",
                    ),
                    _ => (
                        "Max Step Timeout Seconds",
                        "Set max_step_timeout_seconds (empty clears, >=1):",
                    ),
                };
                let current = rows
                    .get(selected)
                    .and_then(|row| row.value.clone())
                    .unwrap_or_default();
                let initial = if current == "<none>" { "" } else { &current };
                if let Some(value) = prompt_line_tui(terminal, label, prompt, initial)? {
                    let field = match selected {
                        0 => OrchestrationLimitField::MaxTotalIterations,
                        1 => OrchestrationLimitField::DefaultRunTimeoutSeconds,
                        2 => OrchestrationLimitField::DefaultStepTimeoutSeconds,
                        _ => OrchestrationLimitField::MaxStepTimeoutSeconds,
                    };
                    if value.trim().is_empty() {
                        match bootstrap.set_orchestrator_workflow_orchestration_limit(
                            orchestrator_id,
                            field,
                            None,
                        ) {
                            Ok(_) => status = "workflow orchestration limit cleared".to_string(),
                            Err(err) => status = err,
                        }
                        continue;
                    }
                    let parsed = match value.trim().parse::<u64>() {
                        Ok(v) => v,
                        _ => {
                            status = "value must be >= 1".to_string();
                            continue;
                        }
                    };
                    match bootstrap.set_orchestrator_workflow_orchestration_limit(
                        orchestrator_id,
                        field,
                        Some(parsed),
                    ) {
                        Ok(_) => status = "workflow orchestration limit updated".to_string(),
                        Err(err) => status = err,
                    }
                }
            }
            _ => {}
        }
    }
}

fn run_workflow_manager_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status =
        "Enter opens workflow settings. f set default, a add, d delete. Esc back.".to_string();
    loop {
        let cfg = bootstrap
            .orchestrator_configs
            .get(orchestrator_id)
            .ok_or_else(|| format!("orchestrator `{orchestrator_id}` missing in setup state"))?;
        let workflow_ids: Vec<String> = cfg.workflows.iter().map(|w| w.id.clone()).collect();
        if !workflow_ids.is_empty() {
            selected = selected.min(workflow_ids.len().saturating_sub(1));
        } else {
            selected = 0;
        }
        let selected_workflow = workflow_ids.get(selected).cloned().unwrap_or_default();
        let items = workflow_ids
            .iter()
            .map(|id| {
                if *id == cfg.default_workflow {
                    format!("{id} (default)")
                } else {
                    id.clone()
                }
            })
            .collect::<Vec<_>>();
        draw_list_screen(
            terminal,
            &format!("Setup > Orchestrators > {orchestrator_id} > Workflows"),
            config_exists,
            &items,
            selected,
            &status,
            "Up/Down move | Enter open | f set default | a add | d delete | Esc back",
        )?;
        if !event::poll(Duration::from_millis(250))
            .map_err(|e| format!("failed to poll workflow manager input: {e}"))?
        {
            continue;
        }
        let ev =
            event::read().map_err(|e| format!("failed to read workflow manager input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(None);
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed workflows.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => {
                selected = std::cmp::min(selected + 1, workflow_ids.len().saturating_sub(1))
            }
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                if workflow_ids.is_empty() {
                    status = "no workflows configured".to_string();
                } else if let Some(message) = run_workflow_detail_tui(
                    terminal,
                    bootstrap,
                    config_exists,
                    orchestrator_id,
                    &selected_workflow,
                )? {
                    status = message;
                }
            }
            KeyCode::Char('f') => {
                if workflow_ids.is_empty() {
                    status = "no workflows configured".to_string();
                } else {
                    match bootstrap
                        .set_orchestrator_default_workflow(orchestrator_id, &selected_workflow)
                    {
                        Ok(_) => status = "default workflow updated".to_string(),
                        Err(err) => status = err,
                    }
                }
            }
            KeyCode::Char('a') => {
                if let Some(workflow_id) = prompt_line_tui(
                    terminal,
                    "Add Workflow",
                    "New workflow id (slug, non-empty):",
                    "",
                )? {
                    let workflow_id = workflow_id.trim().to_string();
                    if workflow_id.is_empty() {
                        status = "workflow id must be non-empty".to_string();
                        continue;
                    }
                    match bootstrap.add_workflow(orchestrator_id, &workflow_id) {
                        Ok(_) => status = "workflow added".to_string(),
                        Err(err) => status = err,
                    }
                }
            }
            KeyCode::Char('d') => {
                if workflow_ids.is_empty() {
                    status = "no workflows to delete".to_string();
                    continue;
                }
                match bootstrap.remove_workflow(orchestrator_id, &selected_workflow) {
                    Ok(_) => {
                        selected = selected.saturating_sub(1);
                        status = "workflow removed".to_string();
                    }
                    Err(err) => status = err,
                }
            }
            _ => {}
        }
    }
}

fn workflow_detail_menu_rows(
    bootstrap: &SetupState,
    orchestrator_id: &str,
    workflow_id: &str,
) -> Vec<SetupFieldRow> {
    let Some(cfg) = bootstrap.orchestrator_configs.get(orchestrator_id) else {
        return vec![
            field_row("Set As Default", Some("no".to_string())),
            field_row("Workflow ID", Some("<missing>".to_string())),
            field_row("Version", Some("<missing>".to_string())),
            field_row("Max Total Iterations", Some("<none>".to_string())),
            field_row("Run Timeout Seconds", Some("<none>".to_string())),
        ];
    };
    let Some(workflow) = cfg.workflows.iter().find(|w| w.id == workflow_id) else {
        return vec![
            field_row("Set As Default", Some("no".to_string())),
            field_row("Workflow ID", Some("<missing>".to_string())),
            field_row("Version", Some("<missing>".to_string())),
            field_row("Max Total Iterations", Some("<none>".to_string())),
            field_row("Run Timeout Seconds", Some("<none>".to_string())),
        ];
    };

    let max_total_iterations = workflow
        .limits
        .as_ref()
        .and_then(|l| l.max_total_iterations)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    let run_timeout_seconds = workflow
        .limits
        .as_ref()
        .and_then(|l| l.run_timeout_seconds)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    let inputs = workflow_inputs_as_csv(&workflow.inputs);
    let step_count = workflow.steps.len().to_string();

    vec![
        field_row(
            "Set As Default",
            Some(if cfg.default_workflow == workflow_id {
                "yes".to_string()
            } else {
                "no".to_string()
            }),
        ),
        field_row("Workflow ID", Some(workflow.id.clone())),
        field_row("Version", Some(workflow.version.to_string())),
        field_row("Inputs", Some(inputs)),
        field_row("Max Total Iterations", Some(max_total_iterations)),
        field_row("Run Timeout Seconds", Some(run_timeout_seconds)),
        field_row("Steps", Some(step_count)),
    ]
}

fn workflow_inputs_as_csv(inputs: &WorkflowInputs) -> String {
    let parts: Vec<String> = inputs
        .as_slice()
        .iter()
        .map(|key| key.as_str().to_string())
        .collect();
    if parts.is_empty() {
        "<none>".to_string()
    } else {
        parts.join(",")
    }
}

fn parse_csv_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn output_files_as_csv(output_files: &BTreeMap<OutputKey, PathTemplate>) -> String {
    if output_files.is_empty() {
        return "<none>".to_string();
    }
    output_files
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_output_files(raw: &str) -> Result<BTreeMap<OutputKey, PathTemplate>, String> {
    let mut output_files = BTreeMap::new();
    for entry in parse_csv_values(raw) {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| "output_files must use key=path entries".to_string())?;
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            return Err("output_files entries require non-empty key and path".to_string());
        }
        let key = OutputKey::parse_output_file_key(key)?;
        let value = PathTemplate::parse(value)?;
        output_files.insert(key, value);
    }
    Ok(output_files)
}

fn unique_step_id(existing: &[WorkflowStepConfig], base: &str) -> String {
    if !existing.iter().any(|step| step.id == base) {
        return base.to_string();
    }
    let mut idx = 2usize;
    loop {
        let candidate = format!("{base}_{idx}");
        if !existing.iter().any(|step| step.id == candidate) {
            return candidate;
        }
        idx += 1;
    }
}

fn run_workflow_detail_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
    workflow_id: &str,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut current_workflow_id = workflow_id.to_string();
    let mut status = "Enter to edit selected option. Esc back.".to_string();
    loop {
        let rows = workflow_detail_menu_rows(bootstrap, orchestrator_id, &current_workflow_id);
        draw_field_screen(
            terminal,
            &format!(
                "Setup > Orchestrators > {orchestrator_id} > Workflows > {current_workflow_id}"
            ),
            config_exists,
            &rows,
            selected,
            &status,
            "Up/Down move | Enter select | Esc back",
        )?;
        let ev = event::read().map_err(|e| format!("failed to read workflow detail input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed workflow view.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = std::cmp::min(selected + 1, rows.len().saturating_sub(1)),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => match selected {
                0 => {
                    match bootstrap
                        .set_orchestrator_default_workflow(orchestrator_id, &current_workflow_id)
                    {
                        Ok(_) => status = "default workflow updated".to_string(),
                        Err(err) => status = err,
                    }
                }
                1 => {
                    let current = current_workflow_id.clone();
                    if let Some(value) =
                        prompt_line_tui(terminal, "Workflow ID", "Set workflow id:", &current)?
                    {
                        let next_id = value.trim().to_string();
                        if next_id.is_empty() {
                            status = "workflow id must be non-empty".to_string();
                            continue;
                        }
                        match bootstrap.rename_workflow(
                            orchestrator_id,
                            &current_workflow_id,
                            &next_id,
                        ) {
                            Ok(_) => {
                                current_workflow_id = next_id;
                                status = "workflow id updated".to_string();
                            }
                            Err(err) => {
                                status = err;
                            }
                        }
                    }
                }
                2 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .and_then(|cfg| cfg.workflows.iter().find(|w| w.id == current_workflow_id))
                        .map(|w| w.version.to_string())
                        .unwrap_or_else(|| "1".to_string());
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Workflow Version",
                        "Set version (>=1):",
                        &current,
                    )? {
                        match value.trim().parse::<u32>() {
                            Ok(v) => match bootstrap.set_workflow_version(
                                orchestrator_id,
                                &current_workflow_id,
                                v,
                            ) {
                                Ok(_) => status = "workflow version updated".to_string(),
                                Err(err) => status = err,
                            },
                            _ => status = "version must be >= 1".to_string(),
                        }
                    }
                }
                3 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .and_then(|cfg| cfg.workflows.iter().find(|w| w.id == current_workflow_id))
                        .map(|w| workflow_inputs_as_csv(&w.inputs))
                        .unwrap_or_else(|| "<none>".to_string());
                    let initial = if current == "<none>" { "" } else { &current };
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Workflow Inputs",
                        "Comma-separated input keys (empty clears):",
                        initial,
                    )? {
                        let parsed = parse_csv_values(&value);
                        match bootstrap.set_workflow_inputs(
                            orchestrator_id,
                            &current_workflow_id,
                            parsed,
                        ) {
                            Ok(_) => status = "workflow inputs updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                4 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .and_then(|cfg| cfg.workflows.iter().find(|w| w.id == current_workflow_id))
                        .and_then(|w| w.limits.as_ref())
                        .and_then(|l| l.max_total_iterations)
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Max Total Iterations",
                        "Set max_total_iterations (empty clears, >=1):",
                        &current,
                    )? {
                        if value.trim().is_empty() {
                            match bootstrap.set_workflow_max_total_iterations(
                                orchestrator_id,
                                &current_workflow_id,
                                None,
                            ) {
                                Ok(_) => status = "max_total_iterations cleared".to_string(),
                                Err(err) => status = err,
                            }
                        } else {
                            match value.trim().parse::<u32>() {
                                Ok(v) => match bootstrap.set_workflow_max_total_iterations(
                                    orchestrator_id,
                                    &current_workflow_id,
                                    Some(v),
                                ) {
                                    Ok(_) => status = "max_total_iterations updated".to_string(),
                                    Err(err) => status = err,
                                },
                                _ => status = "max_total_iterations must be >= 1".to_string(),
                            }
                        }
                    }
                }
                5 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .and_then(|cfg| cfg.workflows.iter().find(|w| w.id == current_workflow_id))
                        .and_then(|w| w.limits.as_ref())
                        .and_then(|l| l.run_timeout_seconds)
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Run Timeout Seconds",
                        "Set run_timeout_seconds (empty clears, >=1):",
                        &current,
                    )? {
                        if value.trim().is_empty() {
                            match bootstrap.set_workflow_run_timeout_seconds(
                                orchestrator_id,
                                &current_workflow_id,
                                None,
                            ) {
                                Ok(_) => status = "run_timeout_seconds cleared".to_string(),
                                Err(err) => status = err,
                            }
                        } else {
                            match value.trim().parse::<u64>() {
                                Ok(v) => match bootstrap.set_workflow_run_timeout_seconds(
                                    orchestrator_id,
                                    &current_workflow_id,
                                    Some(v),
                                ) {
                                    Ok(_) => status = "run_timeout_seconds updated".to_string(),
                                    Err(err) => status = err,
                                },
                                _ => status = "run_timeout_seconds must be >= 1".to_string(),
                            }
                        }
                    }
                }
                _ => {
                    if let Some(message) = run_workflow_steps_tui(
                        terminal,
                        bootstrap,
                        config_exists,
                        orchestrator_id,
                        &current_workflow_id,
                    )? {
                        status = message;
                    }
                }
            },
            _ => {}
        }
    }
}

fn workflow_step_menu_rows(step: &WorkflowStepConfig) -> Vec<SetupFieldRow> {
    let workspace_mode = match step.workspace_mode {
        WorkflowStepWorkspaceMode::OrchestratorWorkspace => "orchestrator_workspace",
        WorkflowStepWorkspaceMode::RunWorkspace => "run_workspace",
        WorkflowStepWorkspaceMode::AgentWorkspace => "agent_workspace",
    };
    let outputs = if step.outputs.is_empty() {
        "<none>".to_string()
    } else {
        step.outputs
            .iter()
            .map(|key| key.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    let max_retries = step
        .limits
        .as_ref()
        .and_then(|limits| limits.max_retries)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    vec![
        field_row("Step ID", Some(step.id.clone())),
        field_row("Step Type", Some(step.step_type.to_string())),
        field_row("Agent", Some(step.agent.clone())),
        field_row("Prompt", Some(step.prompt.clone())),
        field_row("Workspace Mode", Some(workspace_mode.to_string())),
        field_row(
            "Next",
            Some(step.next.clone().unwrap_or_else(|| "<none>".to_string())),
        ),
        field_row(
            "On Approve",
            Some(
                step.on_approve
                    .clone()
                    .unwrap_or_else(|| "<none>".to_string()),
            ),
        ),
        field_row(
            "On Reject",
            Some(
                step.on_reject
                    .clone()
                    .unwrap_or_else(|| "<none>".to_string()),
            ),
        ),
        field_row("Outputs", Some(outputs)),
        field_row(
            "Output Files",
            Some(output_files_as_csv(&step.output_files)),
        ),
        field_row("Max Retries", Some(max_retries)),
    ]
}

fn run_workflow_steps_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
    workflow_id: &str,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status = "Enter opens step settings. a add, d delete. Esc back.".to_string();
    loop {
        let cfg = bootstrap
            .orchestrator_configs
            .get(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        let workflow = cfg
            .workflows
            .iter()
            .find(|workflow| workflow.id == workflow_id)
            .ok_or_else(|| "workflow missing".to_string())?;
        let step_ids: Vec<String> = workflow.steps.iter().map(|step| step.id.clone()).collect();
        if !step_ids.is_empty() {
            selected = selected.min(step_ids.len().saturating_sub(1));
        } else {
            selected = 0;
        }
        let selected_step = step_ids.get(selected).cloned().unwrap_or_default();
        let items = workflow
            .steps
            .iter()
            .map(|step| format!("{} [{}] {}", step.id, step.step_type, step.agent))
            .collect::<Vec<_>>();
        draw_list_screen(
            terminal,
            &format!(
                "Setup > Orchestrators > {orchestrator_id} > Workflows > {workflow_id} > Steps"
            ),
            config_exists,
            &items,
            selected,
            &status,
            "Up/Down move | Enter open | a add | d delete | Esc back",
        )?;
        if !event::poll(Duration::from_millis(250))
            .map_err(|e| format!("failed to poll workflow steps input: {e}"))?
        {
            continue;
        }
        let ev = event::read().map_err(|e| format!("failed to read workflow steps input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed workflow steps.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => {
                selected = std::cmp::min(selected + 1, step_ids.len().saturating_sub(1))
            }
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                if step_ids.is_empty() {
                    status = "no steps configured".to_string();
                } else if let Some(message) = run_workflow_step_detail_tui(
                    terminal,
                    bootstrap,
                    config_exists,
                    orchestrator_id,
                    workflow_id,
                    &selected_step,
                )? {
                    status = message;
                }
            }
            KeyCode::Char('a') => {
                let suggested = unique_step_id(&workflow.steps, "step");
                if let Some(step_id) =
                    prompt_line_tui(terminal, "Add Step", "New step id (non-empty):", &suggested)?
                {
                    let step_id = step_id.trim().to_string();
                    if step_id.is_empty() {
                        status = "step id must be non-empty".to_string();
                        continue;
                    }
                    match bootstrap.add_step(orchestrator_id, workflow_id, &step_id) {
                        Ok(_) => status = "step added".to_string(),
                        Err(err) => status = err,
                    }
                }
            }
            KeyCode::Char('d') => {
                if step_ids.is_empty() {
                    status = "no steps to delete".to_string();
                    continue;
                }
                match bootstrap.remove_step(orchestrator_id, workflow_id, &selected_step) {
                    Ok(_) => {
                        selected = selected.saturating_sub(1);
                        status = "step removed".to_string();
                    }
                    Err(err) => status = err,
                }
            }
            _ => {}
        }
    }
}

fn run_workflow_step_detail_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
    workflow_id: &str,
    step_id: &str,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut current_step_id = step_id.to_string();
    let mut status = "Enter to edit selected option. Esc back.".to_string();
    loop {
        let step = bootstrap
            .orchestrator_configs
            .get(orchestrator_id)
            .and_then(|cfg| cfg.workflows.iter().find(|w| w.id == workflow_id))
            .and_then(|workflow| {
                workflow
                    .steps
                    .iter()
                    .find(|step| step.id == current_step_id)
            })
            .cloned();
        let Some(step) = step else {
            return Ok(Some("Step no longer exists.".to_string()));
        };
        let rows = workflow_step_menu_rows(&step);
        draw_field_screen(
            terminal,
            &format!(
                "Setup > Orchestrators > {orchestrator_id} > Workflows > {workflow_id} > Steps > {current_step_id}"
            ),
            config_exists,
            &rows,
            selected,
            &status,
            "Up/Down move | Enter select | Esc back",
        )?;
        let ev =
            event::read().map_err(|e| format!("failed to read workflow step detail input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed workflow step view.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = std::cmp::min(selected + 1, rows.len().saturating_sub(1)),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => match selected {
                0 => {
                    if let Some(value) =
                        prompt_line_tui(terminal, "Step ID", "Set step id:", &current_step_id)?
                    {
                        let next_id = value.trim().to_string();
                        if next_id.is_empty() {
                            status = "step id must be non-empty".to_string();
                            continue;
                        }
                        match bootstrap.rename_step(
                            orchestrator_id,
                            workflow_id,
                            &current_step_id,
                            &next_id,
                        ) {
                            Ok(_) => {
                                current_step_id = next_id;
                                status = "step id updated".to_string();
                            }
                            Err(err) => status = err,
                        }
                    }
                }
                1 => {
                    match bootstrap.toggle_step_type(orchestrator_id, workflow_id, &current_step_id)
                    {
                        Ok(_) => status = "step type toggled".to_string(),
                        Err(err) => status = err,
                    }
                }
                2 => {
                    if let Some(value) =
                        prompt_line_tui(terminal, "Step Agent", "Set agent id:", &step.agent)?
                    {
                        if value.trim().is_empty() {
                            status = "agent must be non-empty".to_string();
                            continue;
                        }
                        match bootstrap.set_step_agent(
                            orchestrator_id,
                            workflow_id,
                            &current_step_id,
                            value.trim(),
                        ) {
                            Ok(_) => status = "step agent updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                3 => {
                    if let Some(value) =
                        prompt_line_tui(terminal, "Step Prompt", "Set step prompt:", &step.prompt)?
                    {
                        if value.trim().is_empty() {
                            status = "prompt must be non-empty".to_string();
                            continue;
                        }
                        match bootstrap.set_step_prompt(
                            orchestrator_id,
                            workflow_id,
                            &current_step_id,
                            &value,
                        ) {
                            Ok(_) => status = "step prompt updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                4 => {
                    match bootstrap.toggle_step_workspace_mode(
                        orchestrator_id,
                        workflow_id,
                        &current_step_id,
                    ) {
                        Ok(_) => status = "step workspace_mode toggled".to_string(),
                        Err(err) => status = err,
                    }
                }
                5 => {
                    let current = step.next.clone().unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Next Step",
                        "Set next step id (empty clears):",
                        &current,
                    )? {
                        let next = if value.trim().is_empty() {
                            None
                        } else {
                            Some(value.trim().to_string())
                        };
                        match bootstrap.set_step_next(
                            orchestrator_id,
                            workflow_id,
                            &current_step_id,
                            next,
                        ) {
                            Ok(_) => status = "step next updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                6 => {
                    let current = step.on_approve.clone().unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "On Approve",
                        "Set on_approve step id (empty clears):",
                        &current,
                    )? {
                        let on_approve = if value.trim().is_empty() {
                            None
                        } else {
                            Some(value.trim().to_string())
                        };
                        match bootstrap.set_step_on_approve(
                            orchestrator_id,
                            workflow_id,
                            &current_step_id,
                            on_approve,
                        ) {
                            Ok(_) => status = "step on_approve updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                7 => {
                    let current = step.on_reject.clone().unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "On Reject",
                        "Set on_reject step id (empty clears):",
                        &current,
                    )? {
                        let on_reject = if value.trim().is_empty() {
                            None
                        } else {
                            Some(value.trim().to_string())
                        };
                        match bootstrap.set_step_on_reject(
                            orchestrator_id,
                            workflow_id,
                            &current_step_id,
                            on_reject,
                        ) {
                            Ok(_) => status = "step on_reject updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                8 => {
                    let current = step
                        .outputs
                        .iter()
                        .map(|key| key.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Outputs",
                        "Comma-separated output keys (empty clears):",
                        &current,
                    )? {
                        let parsed = match parse_csv_values(&value)
                            .into_iter()
                            .map(|key| OutputKey::parse(&key))
                            .collect::<Result<Vec<_>, _>>()
                        {
                            Ok(parsed) => parsed,
                            Err(err) => {
                                status = err;
                                continue;
                            }
                        };
                        match bootstrap.set_step_outputs(
                            orchestrator_id,
                            workflow_id,
                            &current_step_id,
                            parsed,
                        ) {
                            Ok(_) => status = "step outputs updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                9 => {
                    let current = output_files_as_csv(&step.output_files);
                    let initial = if current == "<none>" { "" } else { &current };
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Output Files",
                        "Comma-separated key=path mappings (empty clears):",
                        initial,
                    )? {
                        if value.trim().is_empty() {
                            match bootstrap.set_step_output_files(
                                orchestrator_id,
                                workflow_id,
                                &current_step_id,
                                BTreeMap::new(),
                            ) {
                                Ok(_) => status = "step output_files cleared".to_string(),
                                Err(err) => status = err,
                            }
                            continue;
                        }
                        let parsed = match parse_output_files(&value) {
                            Ok(parsed) => parsed,
                            Err(err) => {
                                status = err;
                                continue;
                            }
                        };
                        match bootstrap.set_step_output_files(
                            orchestrator_id,
                            workflow_id,
                            &current_step_id,
                            parsed,
                        ) {
                            Ok(_) => status = "step output_files updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                _ => {
                    let current = step
                        .limits
                        .as_ref()
                        .and_then(|limits| limits.max_retries)
                        .map(|value| value.to_string())
                        .unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Step Max Retries",
                        "Set max_retries (empty clears, >=1):",
                        &current,
                    )? {
                        if value.trim().is_empty() {
                            match bootstrap.set_step_max_retries(
                                orchestrator_id,
                                workflow_id,
                                &current_step_id,
                                None,
                            ) {
                                Ok(_) => status = "step max_retries cleared".to_string(),
                                Err(err) => status = err,
                            }
                            continue;
                        }
                        match value.trim().parse::<u32>() {
                            Ok(parsed) => match bootstrap.set_step_max_retries(
                                orchestrator_id,
                                workflow_id,
                                &current_step_id,
                                Some(parsed),
                            ) {
                                Ok(_) => status = "step max_retries updated".to_string(),
                                Err(err) => status = err,
                            },
                            _ => status = "max_retries must be >= 1".to_string(),
                        }
                    }
                }
            },
            _ => {}
        }
    }
}

fn run_agent_manager_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status = "Enter opens agent. a add, d delete. Esc back.".to_string();
    loop {
        let config = bootstrap
            .orchestrator_configs
            .get(orchestrator_id)
            .ok_or_else(|| format!("orchestrator `{orchestrator_id}` missing in setup state"))?;
        let agent_ids: Vec<String> = config.agents.keys().cloned().collect();
        if !agent_ids.is_empty() {
            selected = selected.min(agent_ids.len().saturating_sub(1));
        } else {
            selected = 0;
        }
        let selected_agent = agent_ids.get(selected).cloned().unwrap_or_default();
        draw_list_screen(
            terminal,
            &format!("Setup > Orchestrators > {orchestrator_id} > Agents"),
            config_exists,
            &agent_ids,
            selected,
            &status,
            "Up/Down move | Enter open | a add | d delete | Esc back",
        )?;
        if !event::poll(Duration::from_millis(250))
            .map_err(|e| format!("failed to poll agent manager input: {e}"))?
        {
            continue;
        }
        let ev = event::read().map_err(|e| format!("failed to read agent manager input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(None);
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed agents.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => {
                selected = std::cmp::min(selected + 1, agent_ids.len().saturating_sub(1))
            }
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                if agent_ids.is_empty() {
                    status = "no agents configured".to_string();
                } else if let Some(message) = run_agent_detail_tui(
                    terminal,
                    bootstrap,
                    config_exists,
                    orchestrator_id,
                    &selected_agent,
                )? {
                    status = message;
                }
            }
            KeyCode::Char('a') => {
                if let Some(agent_id) =
                    prompt_line_tui(terminal, "Add Agent", "New agent id (slug, non-empty):", "")?
                {
                    let agent_id = agent_id.trim().to_string();
                    if agent_id.is_empty() {
                        status = "agent id must be non-empty".to_string();
                        continue;
                    }
                    match bootstrap.add_agent(orchestrator_id, &agent_id) {
                        Ok(_) => status = "agent added".to_string(),
                        Err(err) => status = err,
                    }
                }
            }
            KeyCode::Char('d') => {
                if agent_ids.is_empty() {
                    status = "no agents to delete".to_string();
                    continue;
                }
                match bootstrap.remove_agent(orchestrator_id, &selected_agent) {
                    Ok(_) => {
                        selected = selected.saturating_sub(1);
                        status = "agent removed".to_string();
                    }
                    Err(err) => status = err,
                }
            }
            _ => {}
        }
    }
}

fn run_agent_detail_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
    agent_id: &str,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status = "Enter to edit selected option. Esc back.".to_string();
    loop {
        let rows = project_agent_detail_rows(bootstrap, orchestrator_id, agent_id);
        draw_field_screen(
            terminal,
            &format!("Setup > Orchestrators > {orchestrator_id} > Agents > {agent_id}"),
            config_exists,
            &rows,
            selected,
            &status,
            "Up/Down move | Enter select | Esc back",
        )?;
        let ev = event::read().map_err(|e| format!("failed to read agent detail input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed agent view.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = std::cmp::min(selected + 1, rows.len().saturating_sub(1)),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                let descriptors = agent_detail_descriptors();
                let action = descriptors
                    .get(selected)
                    .map(|descriptor| descriptor.action)
                    .unwrap_or(AgentDetailAction::SetAsSelectorAgent);
                if let Some(message) = apply_agent_detail_field_edit(
                    terminal,
                    bootstrap,
                    config_exists,
                    orchestrator_id,
                    agent_id,
                    action,
                )? {
                    status = message;
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum AgentDetailAction {
    ConfigField(AgentEditableField),
    SetAsSelectorAgent,
}

struct AgentDetailFieldDescriptor {
    action: AgentDetailAction,
    label: String,
}

fn agent_detail_descriptors() -> Vec<AgentDetailFieldDescriptor> {
    let mut descriptors: Vec<AgentDetailFieldDescriptor> = agent_editable_fields()
        .iter()
        .map(|field| AgentDetailFieldDescriptor {
            action: AgentDetailAction::ConfigField(*field),
            label: field.label().to_string(),
        })
        .collect();
    descriptors.push(AgentDetailFieldDescriptor {
        action: AgentDetailAction::SetAsSelectorAgent,
        label: "Set As Selector Agent".to_string(),
    });
    descriptors
}

fn project_agent_detail_rows(
    bootstrap: &SetupState,
    orchestrator_id: &str,
    agent_id: &str,
) -> Vec<SetupFieldRow> {
    let descriptors = agent_detail_descriptors();
    descriptors
        .iter()
        .map(|descriptor| {
            field_row(
                &descriptor.label,
                Some(agent_detail_field_value(
                    bootstrap,
                    orchestrator_id,
                    agent_id,
                    descriptor.action,
                )),
            )
        })
        .collect()
}

fn agent_detail_field_value(
    bootstrap: &SetupState,
    orchestrator_id: &str,
    agent_id: &str,
    action: AgentDetailAction,
) -> String {
    let resolved = bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .and_then(|cfg| cfg.agents.get(agent_id).map(|agent| (cfg, agent)));
    match action {
        AgentDetailAction::ConfigField(field) => resolved
            .map(|(_, agent)| agent.display_value_for_field(field))
            .unwrap_or_else(|| "<missing>".to_string()),
        AgentDetailAction::SetAsSelectorAgent => resolved
            .map(|(cfg, _)| {
                if cfg.selector_agent == agent_id {
                    "yes".to_string()
                } else {
                    "no".to_string()
                }
            })
            .unwrap_or_else(|| "no".to_string()),
    }
}

fn apply_agent_detail_field_edit(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
    agent_id: &str,
    action: AgentDetailAction,
) -> Result<Option<String>, String> {
    match action {
        AgentDetailAction::ConfigField(AgentEditableField::Provider) => edit_agent_provider_field(
            terminal,
            bootstrap,
            config_exists,
            orchestrator_id,
            agent_id,
        ),
        AgentDetailAction::ConfigField(AgentEditableField::Model) => edit_agent_model_field(
            terminal,
            bootstrap,
            config_exists,
            orchestrator_id,
            agent_id,
        ),
        AgentDetailAction::ConfigField(AgentEditableField::PrivateWorkspace) => {
            edit_agent_private_workspace_field(terminal, bootstrap, orchestrator_id, agent_id)
        }
        AgentDetailAction::ConfigField(AgentEditableField::SharedAccess) => {
            edit_agent_shared_access_field(terminal, bootstrap, orchestrator_id, agent_id)
        }
        AgentDetailAction::ConfigField(AgentEditableField::CanOrchestrateWorkflows) => {
            match bootstrap.toggle_agent_orchestration_capability(orchestrator_id, agent_id) {
                Ok(_) => Ok(Some("agent orchestration capability toggled".to_string())),
                Err(err) => Ok(Some(err)),
            }
        }
        AgentDetailAction::SetAsSelectorAgent => {
            match bootstrap.set_selector_agent(orchestrator_id, agent_id) {
                Ok(_) => Ok(Some("selector agent updated".to_string())),
                Err(err) => Ok(Some(err)),
            }
        }
    }
}

fn edit_agent_provider_field(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
    agent_id: &str,
) -> Result<Option<String>, String> {
    let current_provider = bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .and_then(|cfg| cfg.agents.get(agent_id))
        .map(|a| a.provider)
        .unwrap_or(ConfigProviderKind::Anthropic);
    let current_model = bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .and_then(|cfg| cfg.agents.get(agent_id))
        .map(|a| a.model.clone())
        .unwrap_or_else(|| default_model_for_provider(current_provider.as_str()).to_string());
    let provider_labels: Vec<String> = provider_options()
        .iter()
        .map(|provider| provider.to_string())
        .collect();
    let selected_provider_index = provider_options()
        .iter()
        .position(|provider| *provider == current_provider)
        .unwrap_or(0);
    let Some(selected_index) = prompt_select_index_tui(
        terminal,
        config_exists,
        "Agent Provider",
        "Select provider. Enter applies selection. Esc cancels.",
        "Up/Down move | Enter apply | Esc cancel",
        &provider_labels,
        selected_provider_index,
    )?
    else {
        return Ok(None);
    };
    let provider = provider_options()[selected_index];
    match bootstrap.set_agent_provider(orchestrator_id, agent_id, provider.as_str()) {
        Ok(_) => {
            if !model_options_for_provider(provider).contains(&current_model.as_str()) {
                let fallback_model = default_model_for_provider(provider.as_str());
                match bootstrap.set_agent_model(orchestrator_id, agent_id, fallback_model) {
                    Ok(_) => Ok(Some(format!(
                        "agent provider updated; model set to {fallback_model}"
                    ))),
                    Err(err) => Ok(Some(err)),
                }
            } else {
                Ok(Some("agent provider updated".to_string()))
            }
        }
        Err(err) => Ok(Some(err)),
    }
}

fn edit_agent_model_field(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    config_exists: bool,
    orchestrator_id: &str,
    agent_id: &str,
) -> Result<Option<String>, String> {
    let provider = bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .and_then(|cfg| cfg.agents.get(agent_id))
        .map(|a| a.provider)
        .unwrap_or(ConfigProviderKind::Anthropic);
    let current = bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .and_then(|cfg| cfg.agents.get(agent_id))
        .map(|a| a.model.clone())
        .unwrap_or_else(|| default_model_for_provider(provider.as_str()).to_string());
    let model_options = model_options_for_provider(provider);
    let model_labels: Vec<String> = model_options
        .iter()
        .map(|model| (*model).to_string())
        .collect();
    let selected_model_index = model_options
        .iter()
        .position(|model| *model == current)
        .unwrap_or(0);
    let Some(selected_index) = prompt_select_index_tui(
        terminal,
        config_exists,
        "Agent Model",
        "Select model valid for current provider. Enter applies selection. Esc cancels.",
        "Up/Down move | Enter apply | Esc cancel",
        &model_labels,
        selected_model_index,
    )?
    else {
        return Ok(None);
    };
    let model = model_options[selected_index];
    match bootstrap.set_agent_model(orchestrator_id, agent_id, model) {
        Ok(_) => Ok(Some("agent model updated".to_string())),
        Err(err) => Ok(Some(err)),
    }
}

fn edit_agent_private_workspace_field(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    orchestrator_id: &str,
    agent_id: &str,
) -> Result<Option<String>, String> {
    let current = bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .and_then(|cfg| cfg.agents.get(agent_id))
        .and_then(|a| a.private_workspace.as_ref())
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let Some(value) = prompt_line_tui(
        terminal,
        "Agent Private Workspace",
        "private workspace (empty clears):",
        &current,
    )?
    else {
        return Ok(None);
    };
    let workspace = if value.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(value.trim()))
    };
    match bootstrap.set_agent_private_workspace(orchestrator_id, agent_id, workspace) {
        Ok(_) => Ok(Some("agent private workspace updated".to_string())),
        Err(err) => Ok(Some(err)),
    }
}

fn edit_agent_shared_access_field(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupState,
    orchestrator_id: &str,
    agent_id: &str,
) -> Result<Option<String>, String> {
    let current = bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .and_then(|cfg| cfg.agents.get(agent_id))
        .map(|a| a.shared_access.join(","))
        .unwrap_or_default();
    let Some(value) = prompt_line_tui(
        terminal,
        "Agent Shared Access",
        "Comma-separated shared workspace keys:",
        &current,
    )?
    else {
        return Ok(None);
    };
    let shared_access = value
        .split(',')
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();
    match bootstrap.set_agent_shared_access(orchestrator_id, agent_id, shared_access) {
        Ok(_) => Ok(Some("agent shared_access updated".to_string())),
        Err(err) => Ok(Some(err)),
    }
}

fn draw_field_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    title: &str,
    config_exists: bool,
    rows: &[SetupFieldRow],
    selected: usize,
    status: &str,
    hint: &str,
) -> Result<(), String> {
    terminal
        .draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(8),
                    Constraint::Length(4),
                ])
                .split(frame.area());
            let header = Paragraph::new(vec![
                Line::from(Span::styled(
                    title.to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(if config_exists {
                    "Mode: existing setup"
                } else {
                    "Mode: first-time setup"
                }),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, chunks[0]);

            let table_rows = rows.iter().enumerate().map(|(idx, row)| {
                let style = if idx == selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(row.field.clone()),
                    Cell::from(row.value.clone().unwrap_or_default()),
                ])
                .style(style)
            });
            let table = Table::new(
                table_rows,
                [Constraint::Percentage(45), Constraint::Percentage(55)],
            )
            .column_spacing(2)
            .block(main_panel_block());
            frame.render_widget(table, chunks[1]);

            let footer = Paragraph::new(vec![
                Line::from(hint.to_string()),
                Line::from(format!("Status: {status}")),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(footer, chunks[2]);
        })
        .map_err(|e| format!("failed to render field screen: {e}"))?;
    Ok(())
}

fn prompt_line_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    title: &str,
    prompt: &str,
    initial: &str,
) -> Result<Option<String>, String> {
    let mut value = initial.to_string();
    loop {
        terminal
            .draw(|frame| {
                let area = centered_rect(70, 30, frame.area());
                let block = Block::default()
                    .borders(Borders::ALL)
                    .padding(Padding::new(2, 2, 1, 1));
                frame.render_widget(block.clone(), area);
                let inner = block.inner(area);
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                    ])
                    .split(inner);
                let max_input_width = rows[3].width.saturating_sub(2) as usize;
                let display_value = tail_for_display(&value, max_input_width);

                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        title,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ))),
                    rows[0],
                );
                frame.render_widget(Paragraph::new(prompt), rows[2]);
                frame.render_widget(
                    Paragraph::new(Line::from(format!("> {display_value}"))),
                    rows[3],
                );
                frame.render_widget(Paragraph::new("Enter apply, Esc cancel"), rows[4]);
                frame.set_cursor_position((
                    rows[3].x + 2 + display_value.chars().count() as u16,
                    rows[3].y,
                ));
            })
            .map_err(|e| format!("failed to render prompt: {e}"))?;
        let ev = event::read().map_err(|e| format!("failed to read prompt input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => return Ok(Some(value)),
            KeyCode::Backspace => {
                value.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => value.push(ch),
            _ => {}
        }
    }
}

fn prompt_select_index_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config_exists: bool,
    title: &str,
    status: &str,
    hint: &str,
    options: &[String],
    initial_selected: usize,
) -> Result<Option<usize>, String> {
    let mut selected = initial_selected.min(options.len().saturating_sub(1));
    loop {
        draw_list_screen(
            terminal,
            title,
            config_exists,
            options,
            selected,
            status,
            hint,
        )?;
        let ev = event::read().map_err(|e| format!("failed to read selection input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => {
                selected = std::cmp::min(selected + 1, options.len().saturating_sub(1))
            }
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                return Ok(Some(selected))
            }
            _ => {}
        }
    }
}

fn tail_for_display(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn draw_list_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    title: &str,
    config_exists: bool,
    items: &[String],
    selected: usize,
    status: &str,
    hint: &str,
) -> Result<(), String> {
    terminal
        .draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(8),
                    Constraint::Length(4),
                ])
                .split(frame.area());
            let header = Paragraph::new(vec![
                Line::from(Span::styled(
                    title.to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(if config_exists {
                    "Mode: existing setup"
                } else {
                    "Mode: first-time setup"
                }),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, chunks[0]);

            let mut list_items = Vec::with_capacity(items.len());
            for (idx, line) in items.iter().enumerate() {
                let mut item = ListItem::new(Line::from(Span::raw(line.clone())));
                if idx == selected {
                    item = item.style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    );
                }
                list_items.push(item);
            }
            frame.render_widget(List::new(list_items).block(main_panel_block()), chunks[1]);

            let footer = Paragraph::new(vec![
                Line::from(hint.to_string()),
                Line::from(format!("Status: {status}")),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(footer, chunks[2]);
        })
        .map_err(|e| format!("failed to render list screen: {e}"))?;
    Ok(())
}

fn draw_setup_ui(frame: &mut Frame<'_>, view_model: &SetupMenuViewModel) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(frame.area());

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            "DireClaw Setup",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(view_model.mode_line.clone()),
    ])
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, chunks[0]);

    let mut items = Vec::with_capacity(view_model.items.len());
    for (idx, label) in view_model.items.iter().enumerate() {
        let text = label.clone();
        let mut item = ListItem::new(Line::from(Span::raw(text)));
        if idx == view_model.selected {
            item = item.style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        }
        items.push(item);
    }
    let menu = List::new(items).block(main_panel_block());
    frame.render_widget(menu, chunks[1]);

    let footer = Paragraph::new(vec![
        Line::from(view_model.hint_text.clone()),
        Line::from(format!("Status: {}", view_model.status_text)),
    ])
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
}

fn main_panel_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(3, 3, 2, 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_event_with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn setup_key_mapping_maps_root_and_subscreen_escape_differently() {
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
    fn setup_key_mapping_maps_movement_enter_and_hotkeys() {
        assert_eq!(
            setup_action_from_key(SetupScreen::Root, key_event(KeyCode::Up)),
            Some(SetupAction::MovePrev)
        );
        assert_eq!(
            setup_action_from_key(SetupScreen::Root, key_event(KeyCode::Down)),
            Some(SetupAction::MoveNext)
        );
        assert_eq!(
            setup_action_from_key(SetupScreen::Root, key_event(KeyCode::Enter)),
            Some(SetupAction::Enter)
        );
        assert_eq!(
            setup_action_from_key(SetupScreen::Orchestrators, key_event(KeyCode::Char('a'))),
            Some(SetupAction::Add)
        );
        assert_eq!(
            setup_action_from_key(SetupScreen::Orchestrators, key_event(KeyCode::Char('d'))),
            Some(SetupAction::Delete)
        );
        assert_eq!(
            setup_action_from_key(SetupScreen::Orchestrators, key_event(KeyCode::Char('e'))),
            Some(SetupAction::Edit)
        );
    }

    #[test]
    fn setup_key_mapping_maps_ctrl_c_to_cancel() {
        assert_eq!(
            setup_action_from_key(
                SetupScreen::Root,
                key_event_with_modifiers(KeyCode::Char('c'), KeyModifiers::CONTROL)
            ),
            Some(SetupAction::Cancel)
        );
    }

    #[test]
    fn nav_state_initialization_and_selection_boundaries() {
        let mut nav = NavState::root();
        assert_eq!(nav.screen, SetupScreen::Root);
        assert_eq!(nav.selected, 0);
        assert_eq!(nav.status_text, ROOT_STATUS_TEXT);
        assert_eq!(nav.hint_text, ROOT_HINT_TEXT);

        nav.selected = 12;
        setup_transition(&mut nav, SetupAction::ReconcileSelection(0), 5).expect("reconcile empty");
        assert_eq!(nav.selected, 0);

        nav.selected = 12;
        setup_transition(&mut nav, SetupAction::ReconcileSelection(2), 5)
            .expect("reconcile non-empty");
        assert_eq!(nav.selected, 1);
    }

    #[test]
    fn setup_transition_covers_root_enter_paths() {
        let mut nav = NavState::root();
        nav.selected = 0;
        let transition =
            setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len()).expect("enter");
        assert_eq!(
            transition.effect,
            SetupNavEffect::OpenScreen(SetupScreen::Workspaces)
        );

        nav.screen = SetupScreen::Root;
        nav.selected = 1;
        let transition =
            setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len()).expect("enter");
        assert_eq!(transition.effect, SetupNavEffect::OpenOrchestratorManager);

        nav.screen = SetupScreen::Root;
        nav.selected = 2;
        let transition =
            setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len()).expect("enter");
        assert_eq!(
            transition.effect,
            SetupNavEffect::OpenScreen(SetupScreen::InitialAgentDefaults)
        );

        nav.screen = SetupScreen::Root;
        nav.selected = 3;
        let transition =
            setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len()).expect("save");
        assert_eq!(transition.effect, SetupNavEffect::SaveSetup);

        nav.screen = SetupScreen::Root;
        nav.selected = 4;
        let transition =
            setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len()).expect("cancel");
        assert_eq!(transition.effect, SetupNavEffect::CancelSetup);
    }

    #[test]
    fn setup_transition_rejects_invalid_actions_without_panicking() {
        let mut nav = NavState::root();
        let err =
            setup_transition(&mut nav, SetupAction::Add, SETUP_MENU_ITEMS.len()).expect_err("err");
        assert!(matches!(
            err,
            SetupNavError::InvalidTransition {
                screen: SetupScreen::Root,
                action: SetupAction::Add
            }
        ));
    }

    #[test]
    fn setup_transition_supports_navigation_and_save_cancel_sequences() {
        let mut nav = NavState::root();
        nav.selected = 0;
        let open = setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len())
            .expect("open workspaces");
        assert_eq!(
            open.effect,
            SetupNavEffect::OpenScreen(SetupScreen::Workspaces)
        );
        assert_eq!(nav.screen, SetupScreen::Workspaces);

        let back =
            setup_transition(&mut nav, SetupAction::Back, SETUP_MENU_ITEMS.len()).expect("back");
        assert_eq!(back.effect, SetupNavEffect::None);
        assert_eq!(nav.screen, SetupScreen::Root);

        nav.selected = 2;
        let defaults = setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len())
            .expect("open initial defaults");
        assert_eq!(
            defaults.effect,
            SetupNavEffect::OpenScreen(SetupScreen::InitialAgentDefaults)
        );
        assert_eq!(nav.screen, SetupScreen::InitialAgentDefaults);
        let toggle = setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len())
            .expect("toggle provider");
        assert_eq!(toggle.effect, SetupNavEffect::ToggleDefaultProvider);

        nav.screen = SetupScreen::Root;
        nav.selected = 3;
        let save =
            setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len()).expect("save");
        assert_eq!(save.effect, SetupNavEffect::SaveSetup);

        nav.screen = SetupScreen::Root;
        nav.selected = 4;
        let cancel =
            setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len()).expect("cancel");
        assert_eq!(cancel.effect, SetupNavEffect::CancelSetup);
    }

    #[test]
    fn setup_transition_handles_workspace_and_initial_defaults_screen_actions() {
        let mut nav = NavState::root();
        nav.screen = SetupScreen::Workspaces;
        let workspace_edit = setup_transition(&mut nav, SetupAction::Enter, SETUP_MENU_ITEMS.len())
            .expect("workspace edit");
        assert_eq!(workspace_edit.effect, SetupNavEffect::EditWorkspacePath);

        nav.screen = SetupScreen::InitialAgentDefaults;
        nav.selected = 0;
        let toggle = setup_transition(&mut nav, SetupAction::Toggle, SETUP_MENU_ITEMS.len())
            .expect("toggle provider");
        assert_eq!(toggle.effect, SetupNavEffect::ToggleDefaultProvider);

        nav.selected = 1;
        let edit = setup_transition(&mut nav, SetupAction::Edit, SETUP_MENU_ITEMS.len())
            .expect("edit model");
        assert_eq!(edit.effect, SetupNavEffect::EditDefaultModel);
    }

    #[test]
    fn parse_csv_values_trims_and_filters_empty() {
        assert_eq!(
            parse_csv_values(" alpha, ,beta,gamma  "),
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
    }

    #[test]
    fn parse_output_files_requires_key_value_pairs() {
        let parsed = parse_output_files("result=output/result.md,summary=out/summary.md")
            .expect("valid output files");
        assert_eq!(
            parsed.get("result").map(|template| template.as_str()),
            Some("output/result.md")
        );
        assert_eq!(
            parsed.get("summary").map(|template| template.as_str()),
            Some("out/summary.md")
        );
        assert!(parse_output_files("missing_equals").is_err());
    }

    #[test]
    fn workflow_inputs_as_csv_handles_empty_sequence() {
        assert_eq!(workflow_inputs_as_csv(&WorkflowInputs::default()), "<none>");
    }

    #[test]
    fn validate_identifier_accepts_slug_and_rejects_spaces() {
        assert!(validate_identifier("workflow id", "feature_delivery").is_ok());
        assert!(validate_identifier("workflow id", "feature delivery").is_err());
    }

    #[test]
    fn validate_identifier_rejects_unknown_kind() {
        let err = validate_identifier("profile id", "profile_1").expect_err("unsupported kind");
        assert!(err.contains("unsupported identifier kind"));
    }

    #[test]
    fn provider_and_model_options_match_supported_variants() {
        assert_eq!(
            provider_options(),
            &[ConfigProviderKind::Anthropic, ConfigProviderKind::OpenAi]
        );
        assert_eq!(
            model_options_for_provider(ConfigProviderKind::Anthropic),
            &["sonnet", "opus", "claude-sonnet-4-5", "claude-opus-4-6"]
        );
        assert_eq!(
            model_options_for_provider(ConfigProviderKind::OpenAi),
            &["gpt-5.2", "gpt-5.3-codex"]
        );
    }

    #[test]
    fn agent_detail_projection_uses_typed_descriptors() {
        let state = test_setup_state();
        let descriptors = agent_detail_descriptors();
        assert_eq!(descriptors.len(), 6);
        let rows = project_agent_detail_rows(&state, "main", "default");
        assert_eq!(rows.len(), descriptors.len());
        assert_eq!(rows[0].field, "Provider");
        assert_eq!(rows[1].field, "Model");
        assert_eq!(rows[0].value.as_deref(), Some("anthropic"));
        assert_eq!(rows[1].value.as_deref(), Some("sonnet"));
    }

    fn test_setup_state() -> SetupState {
        SetupState {
            workspaces_path: PathBuf::from("/tmp/workspaces"),
            orchestrator_id: "main".to_string(),
            provider: "anthropic".to_string(),
            model: "sonnet".to_string(),
            workflow_template: SetupWorkflowTemplate::Minimal,
            orchestrators: BTreeMap::from_iter([(
                "main".to_string(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            )]),
            orchestrator_configs: BTreeMap::from_iter([(
                "main".to_string(),
                initial_orchestrator_config(
                    "main",
                    "anthropic",
                    "sonnet",
                    SetupWorkflowTemplate::Minimal,
                ),
            )]),
        }
    }

    #[test]
    fn setup_state_enforces_orchestrator_and_workflow_invariants() {
        let mut state = test_setup_state();
        state.add_orchestrator("alpha").expect("add orchestrator");
        assert_eq!(state.orchestrator_id, "alpha");
        assert!(state.add_orchestrator("alpha").is_err());
        state
            .remove_orchestrator("alpha")
            .expect("remove orchestrator");
        assert_eq!(state.orchestrators.len(), 1);
        assert!(state.remove_orchestrator("main").is_err());
        assert!(state.remove_orchestrator("missing").is_err());

        assert!(state
            .set_orchestrator_default_workflow("main", "missing")
            .is_err());
        state.add_workflow("main", "triage").expect("add workflow");
        assert!(state.add_workflow("main", "triage").is_err());
        state
            .set_orchestrator_default_workflow("main", "triage")
            .expect("set default");
        state
            .remove_workflow("main", "default")
            .expect("remove default");
        assert!(state.remove_workflow("main", "triage").is_err());
        assert!(state.remove_workflow("main", "missing").is_err());
    }

    #[test]
    fn setup_state_enforces_step_and_agent_invariants() {
        let mut state = test_setup_state();
        state
            .add_step("main", "default", "step_2")
            .expect("add step");
        assert!(state.add_step("main", "default", "step_2").is_err());
        state
            .remove_step("main", "default", "step_2")
            .expect("remove step");
        assert!(state.remove_step("main", "default", "step_1").is_err());
        assert!(state.remove_step("main", "default", "missing").is_err());

        assert!(state
            .set_step_agent("main", "default", "step_1", "missing")
            .is_err());

        state.add_agent("main", "helper").expect("add agent");
        assert!(state.add_agent("main", "helper").is_err());
        state
            .set_selector_agent("main", "helper")
            .expect("set selector");
        state
            .set_step_agent("main", "default", "step_1", "helper")
            .expect("retarget step agent");
        state
            .remove_agent("main", "default")
            .expect("remove default agent");
        assert!(state.remove_agent("main", "helper").is_err());
        assert!(state.remove_agent("main", "missing").is_err());

        assert!(state
            .toggle_agent_orchestration_capability("main", "helper")
            .is_err());
    }

    #[test]
    fn setup_state_rejects_empty_or_inconsistent_step_output_contracts() {
        let mut state = test_setup_state();
        let key = OutputKey::parse("summary").expect("output key");
        let missing_key = OutputKey::parse("missing").expect("output key");
        let template = PathTemplate::parse("outputs/summary.md").expect("path template");

        assert!(state
            .set_step_outputs("main", "default", "step_1", Vec::new())
            .is_err());
        assert!(state
            .set_step_output_files("main", "default", "step_1", BTreeMap::new())
            .is_err());

        assert!(state
            .set_step_outputs("main", "default", "step_1", vec![missing_key.clone()])
            .is_err());

        let valid_files = BTreeMap::from_iter([(key.clone(), template.clone())]);
        state
            .set_step_outputs("main", "default", "step_1", vec![key.clone()])
            .expect("set outputs");
        state
            .set_step_output_files("main", "default", "step_1", valid_files)
            .expect("set output files");

        let invalid_files = BTreeMap::from_iter([(missing_key, template)]);
        assert!(state
            .set_step_output_files("main", "default", "step_1", invalid_files)
            .is_err());
    }

    #[test]
    fn setup_state_normalize_for_save_rejects_invalid_domain_state() {
        let mut state = test_setup_state();
        let cfg = state
            .orchestrator_configs
            .get_mut("main")
            .expect("main orchestrator config");
        cfg.default_workflow = "missing".to_string();
        let err = state
            .normalize_for_save(None)
            .expect_err("invalid default workflow");
        assert!(err.contains("default workflow") || err.contains("missing"));
    }
}
