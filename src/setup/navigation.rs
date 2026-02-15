use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};

const ROOT_STATUS_TEXT: &str = "Enter opens a section. Esc cancels setup.";
const ROOT_HINT_TEXT: &str = "Up/Down move | Enter open | Esc cancel";
const WORKSPACES_STATUS_TEXT: &str = "Enter to edit workspace path. Esc back.";
const WORKSPACES_HINT_TEXT: &str = "Enter edit | Esc back";
const INITIAL_DEFAULTS_STATUS_TEXT: &str = "Enter to edit/toggle. Esc back.";
const INITIAL_DEFAULTS_HINT_TEXT: &str = "Up/Down move | Enter edit/toggle | Esc back";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupScreen {
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

pub const ALL_SETUP_SCREENS: [SetupScreen; 14] = [
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
pub enum SetupAction {
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
pub struct NavState {
    pub screen: SetupScreen,
    pub selected: usize,
    pub status_text: String,
    pub hint_text: String,
}

impl NavState {
    pub fn root() -> Self {
        Self {
            screen: SetupScreen::Root,
            selected: 0,
            status_text: ROOT_STATUS_TEXT.to_string(),
            hint_text: ROOT_HINT_TEXT.to_string(),
        }
    }

    pub fn clamp_selection(&mut self, len: usize) {
        self.selected = clamp_selection(self.selected, len);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupNavEffect {
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
pub struct SetupTransition {
    pub effect: SetupNavEffect,
    pub feedback: Option<String>,
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
pub enum SetupNavError {
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

pub fn clamp_selection(selected: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    selected.min(len - 1)
}

pub fn setup_action_from_key(
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

pub fn parse_scripted_setup_keys(raw: &str) -> Result<Vec<crossterm::event::KeyEvent>, String> {
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
    Ok(keys)
}

pub fn setup_transition(
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

pub fn setup_screen_item_count(screen: SetupScreen, root_item_count: usize) -> usize {
    match screen {
        SetupScreen::Root => root_item_count,
        SetupScreen::Workspaces => 1,
        SetupScreen::InitialAgentDefaults => 2,
        _ => 0,
    }
}
