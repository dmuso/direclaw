use super::*;
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

pub(super) fn cmd_setup() -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let config_exists = default_global_config_path()
        .map_err(map_config_err)?
        .exists();
    let mut bootstrap = load_setup_bootstrap(&paths)?;
    if is_interactive_setup() {
        match run_setup_tui(&mut bootstrap, config_exists)? {
            SetupExit::Save => {}
            SetupExit::Cancel => return Ok("setup canceled".to_string()),
        }
    }
    fs::create_dir_all(&bootstrap.workspaces_path).map_err(|e| {
        format!(
            "failed to create workspace {}: {e}",
            bootstrap.workspaces_path.display()
        )
    })?;

    let mut settings = if config_exists {
        load_settings()?
    } else {
        Settings {
            workspaces_path: bootstrap.workspaces_path.clone(),
            shared_workspaces: BTreeMap::new(),
            orchestrators: bootstrap.orchestrators.clone(),
            channel_profiles: BTreeMap::new(),
            monitoring: Default::default(),
            channels: BTreeMap::new(),
            auth_sync: AuthSyncConfig::default(),
        }
    };
    settings.workspaces_path = bootstrap.workspaces_path.clone();
    settings.orchestrators = bootstrap.orchestrators.clone();
    settings
        .orchestrators
        .entry(bootstrap.orchestrator_id.clone())
        .or_insert(SettingsOrchestrator {
            private_workspace: None,
            shared_access: Vec::new(),
        });
    if settings.channel_profiles.is_empty() {
        settings.channel_profiles.insert(
            "local-default".to_string(),
            ChannelProfile {
                channel: "local".to_string(),
                orchestrator_id: bootstrap.orchestrator_id.clone(),
                slack_app_user_id: None,
                require_mention_in_channels: None,
            },
        );
    }
    let has_primary_profile = settings
        .channel_profiles
        .values()
        .any(|profile| profile.orchestrator_id == bootstrap.orchestrator_id);
    if !has_primary_profile {
        settings.channel_profiles.insert(
            format!("{}-local", bootstrap.orchestrator_id),
            ChannelProfile {
                channel: "local".to_string(),
                orchestrator_id: bootstrap.orchestrator_id.clone(),
                slack_app_user_id: None,
                require_mention_in_channels: None,
            },
        );
    }
    let path = save_settings(&settings)?;
    bootstrap
        .orchestrator_configs
        .entry(bootstrap.orchestrator_id.clone())
        .or_insert_with(|| {
            initial_orchestrator_config(
                &bootstrap.orchestrator_id,
                &bootstrap.provider,
                &bootstrap.model,
                bootstrap.workflow_template,
            )
        });
    save_orchestrator_registry(&settings, &bootstrap.orchestrator_configs)?;
    let orchestrator_path = settings
        .resolve_private_workspace(&bootstrap.orchestrator_id)
        .map_err(map_config_err)?
        .join("orchestrator.yaml");
    let prefs = RuntimePreferences {
        provider: Some(bootstrap.provider.clone()),
        model: Some(bootstrap.model.clone()),
    };
    save_preferences(&paths, &prefs)?;
    Ok(format!(
        "setup complete\nconfig={}\nstate_root={}\nworkspace={}\norchestrator={}\nnew_workflow_template={}\nprovider={}\nmodel={}\norchestrator_config={}",
        path.display(),
        paths.root.display(),
        bootstrap.workspaces_path.display(),
        bootstrap.orchestrator_id,
        bootstrap.workflow_template.as_str(),
        bootstrap.provider,
        bootstrap.model,
        orchestrator_path.display()
    ))
}

#[derive(Debug, Clone, Copy)]
pub(super) enum SetupWorkflowTemplate {
    Minimal,
    Engineering,
    Product,
}

impl SetupWorkflowTemplate {
    fn as_str(self) -> &'static str {
        match self {
            SetupWorkflowTemplate::Minimal => "minimal",
            SetupWorkflowTemplate::Engineering => "engineering",
            SetupWorkflowTemplate::Product => "product",
        }
    }
}

#[derive(Debug, Clone)]
struct SetupBootstrap {
    workspaces_path: PathBuf,
    orchestrator_id: String,
    provider: String,
    model: String,
    workflow_template: SetupWorkflowTemplate,
    orchestrators: BTreeMap<String, SettingsOrchestrator>,
    orchestrator_configs: BTreeMap<String, OrchestratorConfig>,
}

enum SetupExit {
    Save,
    Cancel,
}

fn is_interactive_setup() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn default_model_for_provider(provider: &str) -> &'static str {
    if provider == "openai" {
        "gpt-5.3-codex"
    } else {
        "sonnet"
    }
}

fn parse_provider(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized == "anthropic" || normalized == "openai" {
        return Ok(normalized);
    }
    Err("provider must be one of: anthropic, openai".to_string())
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

fn load_setup_bootstrap(paths: &StatePaths) -> Result<SetupBootstrap, String> {
    let default_workspace = paths.root.join("workspaces");
    let mut bootstrap = SetupBootstrap {
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
                bootstrap.provider = parse_provider(&selector.provider)?;
                bootstrap.model = selector.model.clone();
            } else if let Some((_, agent)) = orchestrator.agents.iter().next() {
                if let Ok(provider) = parse_provider(&agent.provider) {
                    bootstrap.provider = provider;
                    bootstrap.model = agent.model.clone();
                }
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

fn run_setup_tui(bootstrap: &mut SetupBootstrap, config_exists: bool) -> Result<SetupExit, String> {
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
    bootstrap: &mut SetupBootstrap,
    config_exists: bool,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<SetupExit, String> {
    let mut selected = 0usize;
    let mut status = "Enter opens a section. Esc cancels setup.".to_string();
    loop {
        terminal
            .draw(|frame| draw_setup_ui(frame, config_exists, selected, &status))
            .map_err(|e| format!("failed to render setup ui: {e}"))?;
        if !event::poll(Duration::from_millis(250))
            .map_err(|e| format!("failed to poll setup input: {e}"))?
        {
            continue;
        }
        let ev = event::read().map_err(|e| format!("failed to read setup input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(SetupExit::Cancel);
        }
        match key.code {
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => {
                selected = std::cmp::min(selected + 1, SETUP_MENU_ITEMS.len().saturating_sub(1))
            }
            KeyCode::Esc => return Ok(SetupExit::Cancel),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => match selected {
                0 => {
                    if let Some(message) = run_workspaces_tui(terminal, bootstrap, config_exists)? {
                        status = message;
                    }
                }
                1 => {
                    if let Some(message) =
                        run_orchestrator_manager_tui(terminal, bootstrap, config_exists)?
                    {
                        status = message;
                    }
                }
                2 => {
                    if let Some(message) =
                        run_initial_defaults_tui(terminal, bootstrap, config_exists)?
                    {
                        status = message;
                    }
                }
                3 => return Ok(SetupExit::Save),
                _ => return Ok(SetupExit::Cancel),
            },
            _ => {}
        }
    }
}

fn run_workspaces_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
    config_exists: bool,
) -> Result<Option<String>, String> {
    let mut status = "Enter to edit workspace path. Esc back.".to_string();
    loop {
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
            &status,
            "Enter edit | Esc back",
        )?;
        let ev = event::read().map_err(|e| format!("failed to read workspaces input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed Workspaces.".to_string())),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                if let Some(value) = prompt_line_tui(
                    terminal,
                    "Workspace Path",
                    "Enter workspace path:",
                    &bootstrap.workspaces_path.display().to_string(),
                )? {
                    if value.trim().is_empty() {
                        status = "workspace path must be non-empty".to_string();
                    } else {
                        bootstrap.workspaces_path = PathBuf::from(value.trim());
                        status = "workspace path updated".to_string();
                    }
                }
            }
            _ => {}
        }
    }
}

fn run_initial_defaults_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
    config_exists: bool,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status = "Enter to edit/toggle. Esc back.".to_string();
    loop {
        let items = vec![
            format!("Provider: {}", bootstrap.provider),
            format!("Model: {}", bootstrap.model),
        ];
        draw_list_screen(
            terminal,
            "Setup > Initial Agent Defaults",
            config_exists,
            &items,
            selected,
            &status,
            "Up/Down move | Enter edit/toggle | Esc back",
        )?;
        let ev = event::read().map_err(|e| format!("failed to read defaults input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed Initial Agent Defaults.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = std::cmp::min(selected + 1, 1),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                if selected == 0 {
                    bootstrap.provider = if bootstrap.provider == "anthropic" {
                        "openai".to_string()
                    } else {
                        "anthropic".to_string()
                    };
                    if bootstrap.model == "sonnet" || bootstrap.model == "gpt-5.3-codex" {
                        bootstrap.model =
                            default_model_for_provider(&bootstrap.provider).to_string();
                    }
                    status = format!("provider set to {}", bootstrap.provider);
                } else if let Some(value) =
                    prompt_line_tui(terminal, "Default Model", "Enter model:", &bootstrap.model)?
                {
                    if value.trim().is_empty() {
                        status = "model must be non-empty".to_string();
                    } else {
                        bootstrap.model = value.trim().to_string();
                        status = "model updated".to_string();
                    }
                }
            }
            _ => {}
        }
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
    bootstrap: &mut SetupBootstrap,
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
        bootstrap.workflow_template = template;
    }
    Ok(Some(message))
}

fn run_orchestrator_manager_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
    config_exists: bool,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status = "Enter open orchestrator. a add, d delete, e set primary, t set default workflow template. Esc back.".to_string();
    loop {
        let mut ids: Vec<String> = bootstrap.orchestrators.keys().cloned().collect();
        if ids.is_empty() {
            let id = "main".to_string();
            bootstrap.orchestrators.insert(
                id.clone(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            );
            bootstrap.orchestrator_configs.insert(
                id.clone(),
                initial_orchestrator_config(
                    &id,
                    &bootstrap.provider,
                    &bootstrap.model,
                    bootstrap.workflow_template,
                ),
            );
            bootstrap.orchestrator_id = id.clone();
            ids.push(id);
        }
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
            KeyCode::Char('e') => {
                bootstrap.orchestrator_id = selected_id.clone();
                status = "Primary orchestrator updated.".to_string();
            }
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
                    } else if bootstrap.orchestrators.contains_key(&id) {
                        status = "orchestrator id already exists".to_string();
                    } else {
                        bootstrap.orchestrators.insert(
                            id.clone(),
                            SettingsOrchestrator {
                                private_workspace: None,
                                shared_access: Vec::new(),
                            },
                        );
                        bootstrap.orchestrator_configs.insert(
                            id.clone(),
                            initial_orchestrator_config(
                                &id,
                                &bootstrap.provider,
                                &bootstrap.model,
                                bootstrap.workflow_template,
                            ),
                        );
                        bootstrap.orchestrator_id = id.clone();
                        if let Some(pos) = bootstrap.orchestrators.keys().position(|v| v == &id) {
                            selected = pos;
                        }
                        status = "orchestrator created".to_string();
                    }
                }
            }
            KeyCode::Char('d') => {
                if bootstrap.orchestrators.len() <= 1 {
                    status = "at least one orchestrator must remain".to_string();
                } else {
                    bootstrap.orchestrators.remove(&selected_id);
                    bootstrap.orchestrator_configs.remove(&selected_id);
                    if bootstrap.orchestrator_id == selected_id {
                        if let Some(next_id) = bootstrap.orchestrators.keys().next() {
                            bootstrap.orchestrator_id = next_id.clone();
                        }
                    }
                    selected = selected.saturating_sub(1);
                    status = "orchestrator removed".to_string();
                }
            }
            _ => {}
        }
    }
}

fn run_orchestrator_detail_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
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
                0 => {
                    bootstrap.orchestrator_id = orchestrator_id.to_string();
                    status = "primary orchestrator updated".to_string();
                }
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
                        if let Some(entry) = bootstrap.orchestrators.get_mut(orchestrator_id) {
                            if value.trim().is_empty() {
                                entry.private_workspace = None;
                            } else {
                                entry.private_workspace = Some(PathBuf::from(value.trim()));
                            }
                            status = "private workspace updated".to_string();
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
                        if let Some(entry) = bootstrap.orchestrators.get_mut(orchestrator_id) {
                            entry.shared_access = value
                                .split(',')
                                .map(|v| v.trim().to_string())
                                .filter(|v| !v.is_empty())
                                .collect();
                            status = "shared_access updated".to_string();
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
                        } else if let Some(cfg) =
                            bootstrap.orchestrator_configs.get_mut(orchestrator_id)
                        {
                            cfg.default_workflow = value.trim().to_string();
                            status = "default_workflow updated".to_string();
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
                        apply_workflow_template_to_orchestrator(
                            bootstrap,
                            orchestrator_id,
                            template,
                        )?;
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
                            Ok(v) if v >= 1 => {
                                if let Some(cfg) =
                                    bootstrap.orchestrator_configs.get_mut(orchestrator_id)
                                {
                                    cfg.selection_max_retries = v;
                                    status = "selection_max_retries updated".to_string();
                                }
                            }
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
                            Ok(v) if v >= 1 => {
                                if let Some(cfg) =
                                    bootstrap.orchestrator_configs.get_mut(orchestrator_id)
                                {
                                    cfg.selector_timeout_seconds = v;
                                    status = "selector_timeout_seconds updated".to_string();
                                }
                            }
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

fn provider_model_for_orchestrator(
    bootstrap: &SetupBootstrap,
    orchestrator_id: &str,
) -> (String, String) {
    if let Some(cfg) = bootstrap.orchestrator_configs.get(orchestrator_id) {
        if let Some(selector) = cfg.agents.get(&cfg.selector_agent) {
            if let Ok(provider) = parse_provider(&selector.provider) {
                return (provider, selector.model.clone());
            }
        }
        if let Some((_, agent)) = cfg.agents.iter().next() {
            if let Ok(provider) = parse_provider(&agent.provider) {
                return (provider, agent.model.clone());
            }
        }
    }
    (bootstrap.provider.clone(), bootstrap.model.clone())
}

fn unique_workflow_id(existing: &BTreeMap<String, WorkflowConfig>, base: &str) -> String {
    if !existing.contains_key(base) {
        return base.to_string();
    }
    let mut idx = 2usize;
    loop {
        let candidate = format!("{base}_{idx}");
        if !existing.contains_key(&candidate) {
            return candidate;
        }
        idx += 1;
    }
}

fn apply_workflow_template_to_orchestrator(
    bootstrap: &mut SetupBootstrap,
    orchestrator_id: &str,
    workflow_template: SetupWorkflowTemplate,
) -> Result<(), String> {
    let (provider, model) = provider_model_for_orchestrator(bootstrap, orchestrator_id);
    let template =
        initial_orchestrator_config(orchestrator_id, &provider, &model, workflow_template);
    let target = bootstrap
        .orchestrator_configs
        .get_mut(orchestrator_id)
        .ok_or_else(|| format!("orchestrator `{orchestrator_id}` missing in setup state"))?;

    for (agent_id, agent_cfg) in template.agents {
        target.agents.entry(agent_id).or_insert(agent_cfg);
    }

    let mut existing_workflows = BTreeMap::from_iter(
        target
            .workflows
            .iter()
            .map(|wf| (wf.id.clone(), wf.clone())),
    );
    let mut new_default_workflow = None::<String>;
    for mut workflow in template.workflows {
        let original_id = workflow.id.clone();
        let mapped_id = unique_workflow_id(&existing_workflows, &workflow.id);
        workflow.id = mapped_id.clone();
        if original_id == template.default_workflow {
            new_default_workflow = Some(mapped_id.clone());
        }
        existing_workflows.insert(mapped_id, workflow);
    }
    target.workflows = existing_workflows.into_values().collect();
    if let Some(workflow_id) = new_default_workflow {
        target.default_workflow = workflow_id;
    }
    Ok(())
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
    bootstrap: &SetupBootstrap,
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
    bootstrap: &SetupBootstrap,
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
    bootstrap: &mut SetupBootstrap,
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
                    let cfg = bootstrap
                        .orchestrator_configs
                        .get_mut(orchestrator_id)
                        .ok_or_else(|| "orchestrator missing".to_string())?;
                    if value.trim().is_empty() {
                        if let Some(orchestration) = cfg.workflow_orchestration.as_mut() {
                            match selected {
                                0 => orchestration.max_total_iterations = None,
                                1 => orchestration.default_run_timeout_seconds = None,
                                2 => orchestration.default_step_timeout_seconds = None,
                                _ => orchestration.max_step_timeout_seconds = None,
                            }
                            if orchestration.max_total_iterations.is_none()
                                && orchestration.default_run_timeout_seconds.is_none()
                                && orchestration.default_step_timeout_seconds.is_none()
                                && orchestration.max_step_timeout_seconds.is_none()
                            {
                                cfg.workflow_orchestration = None;
                            }
                        }
                        status = "workflow orchestration limit cleared".to_string();
                        continue;
                    }
                    let parsed = match value.trim().parse::<u64>() {
                        Ok(v) if v >= 1 => v,
                        _ => {
                            status = "value must be >= 1".to_string();
                            continue;
                        }
                    };
                    let orchestration =
                        cfg.workflow_orchestration
                            .get_or_insert(WorkflowOrchestrationConfig {
                                max_total_iterations: None,
                                default_run_timeout_seconds: None,
                                default_step_timeout_seconds: None,
                                max_step_timeout_seconds: None,
                            });
                    match selected {
                        0 => {
                            if parsed > u32::MAX as u64 {
                                status = "max_total_iterations exceeds u32 range".to_string();
                                continue;
                            }
                            orchestration.max_total_iterations = Some(parsed as u32);
                        }
                        1 => orchestration.default_run_timeout_seconds = Some(parsed),
                        2 => orchestration.default_step_timeout_seconds = Some(parsed),
                        _ => orchestration.max_step_timeout_seconds = Some(parsed),
                    }
                    status = "workflow orchestration limit updated".to_string();
                }
            }
            _ => {}
        }
    }
}

fn run_workflow_manager_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
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
                } else if let Some(orchestrator) =
                    bootstrap.orchestrator_configs.get_mut(orchestrator_id)
                {
                    orchestrator.default_workflow = selected_workflow;
                    status = "default workflow updated".to_string();
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
                    let Some(orchestrator) =
                        bootstrap.orchestrator_configs.get_mut(orchestrator_id)
                    else {
                        return Err("orchestrator missing".to_string());
                    };
                    if orchestrator.workflows.iter().any(|w| w.id == workflow_id) {
                        status = "workflow id already exists".to_string();
                        continue;
                    }
                    let selector_agent = orchestrator.selector_agent.clone();
                    orchestrator.workflows.push(WorkflowConfig {
                        id: workflow_id,
                        version: 1,
                        inputs: serde_yaml::Value::Sequence(Vec::new()),
                        limits: None,
                        steps: vec![WorkflowStepConfig {
                            id: "step_1".to_string(),
                            step_type: "agent_task".to_string(),
                            agent: selector_agent,
                            prompt: default_step_prompt("agent_task"),
                            workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                            next: None,
                            on_approve: None,
                            on_reject: None,
                            outputs: default_step_output_contract("agent_task"),
                            output_files: default_step_output_files("agent_task"),
                            limits: None,
                        }],
                    });
                    status = "workflow added".to_string();
                }
            }
            KeyCode::Char('d') => {
                if workflow_ids.is_empty() {
                    status = "no workflows to delete".to_string();
                    continue;
                }
                let Some(orchestrator) = bootstrap.orchestrator_configs.get_mut(orchestrator_id)
                else {
                    return Err("orchestrator missing".to_string());
                };
                if orchestrator.workflows.len() <= 1 {
                    status = "at least one workflow must remain".to_string();
                    continue;
                }
                orchestrator.workflows.retain(|w| w.id != selected_workflow);
                if orchestrator.default_workflow == selected_workflow {
                    if let Some(next) = orchestrator.workflows.first() {
                        orchestrator.default_workflow = next.id.clone();
                    }
                }
                selected = selected.saturating_sub(1);
                status = "workflow removed".to_string();
            }
            _ => {}
        }
    }
}

fn workflow_detail_menu_rows(
    bootstrap: &SetupBootstrap,
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

fn workflow_inputs_as_csv(inputs: &serde_yaml::Value) -> String {
    match inputs {
        serde_yaml::Value::Sequence(values) => {
            let parts: Vec<String> = values
                .iter()
                .filter_map(|value| value.as_str().map(|v| v.trim().to_string()))
                .filter(|v| !v.is_empty())
                .collect();
            if parts.is_empty() {
                "<none>".to_string()
            } else {
                parts.join(",")
            }
        }
        _ => "<none>".to_string(),
    }
}

fn parse_csv_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn output_files_as_csv(output_files: Option<&BTreeMap<String, String>>) -> String {
    let Some(output_files) = output_files else {
        return "<none>".to_string();
    };
    if output_files.is_empty() {
        return "<none>".to_string();
    }
    output_files
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_output_files(raw: &str) -> Result<BTreeMap<String, String>, String> {
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
        output_files.insert(key.to_string(), value.to_string());
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
    bootstrap: &mut SetupBootstrap,
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
                    let cfg = bootstrap
                        .orchestrator_configs
                        .get_mut(orchestrator_id)
                        .ok_or_else(|| "orchestrator missing".to_string())?;
                    cfg.default_workflow = current_workflow_id.clone();
                    status = "default workflow updated".to_string();
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        if cfg
                            .workflows
                            .iter()
                            .any(|w| w.id == next_id && w.id != current_workflow_id)
                        {
                            status = "workflow id already exists".to_string();
                            continue;
                        }
                        if let Some(workflow) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == current_workflow_id)
                        {
                            workflow.id = next_id.clone();
                            if cfg.default_workflow == current_workflow_id {
                                cfg.default_workflow = next_id.clone();
                            }
                            current_workflow_id = next_id;
                            status = "workflow id updated".to_string();
                        } else {
                            status = "workflow no longer exists".to_string();
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
                            Ok(v) if v >= 1 => {
                                if let Some(cfg) =
                                    bootstrap.orchestrator_configs.get_mut(orchestrator_id)
                                {
                                    if let Some(workflow) = cfg
                                        .workflows
                                        .iter_mut()
                                        .find(|w| w.id == current_workflow_id)
                                    {
                                        workflow.version = v;
                                        status = "workflow version updated".to_string();
                                    }
                                }
                            }
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
                        if let Some(cfg) = bootstrap.orchestrator_configs.get_mut(orchestrator_id) {
                            if let Some(workflow) = cfg
                                .workflows
                                .iter_mut()
                                .find(|w| w.id == current_workflow_id)
                            {
                                let parsed = parse_csv_values(&value);
                                workflow.inputs = serde_yaml::Value::Sequence(
                                    parsed.into_iter().map(serde_yaml::Value::String).collect(),
                                );
                                status = "workflow inputs updated".to_string();
                            }
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        let Some(workflow) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == current_workflow_id)
                        else {
                            status = "workflow no longer exists".to_string();
                            continue;
                        };
                        if value.trim().is_empty() {
                            if let Some(limits) = workflow.limits.as_mut() {
                                limits.max_total_iterations = None;
                                if limits.run_timeout_seconds.is_none() {
                                    workflow.limits = None;
                                }
                            }
                            status = "max_total_iterations cleared".to_string();
                        } else {
                            match value.trim().parse::<u32>() {
                                Ok(v) if v >= 1 => {
                                    let limits =
                                        workflow.limits.get_or_insert(WorkflowLimitsConfig {
                                            max_total_iterations: None,
                                            run_timeout_seconds: None,
                                        });
                                    limits.max_total_iterations = Some(v);
                                    status = "max_total_iterations updated".to_string();
                                }
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        let Some(workflow) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == current_workflow_id)
                        else {
                            status = "workflow no longer exists".to_string();
                            continue;
                        };
                        if value.trim().is_empty() {
                            if let Some(limits) = workflow.limits.as_mut() {
                                limits.run_timeout_seconds = None;
                                if limits.max_total_iterations.is_none() {
                                    workflow.limits = None;
                                }
                            }
                            status = "run_timeout_seconds cleared".to_string();
                        } else {
                            match value.trim().parse::<u64>() {
                                Ok(v) if v >= 1 => {
                                    let limits =
                                        workflow.limits.get_or_insert(WorkflowLimitsConfig {
                                            max_total_iterations: None,
                                            run_timeout_seconds: None,
                                        });
                                    limits.run_timeout_seconds = Some(v);
                                    status = "run_timeout_seconds updated".to_string();
                                }
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
    let outputs = step
        .outputs
        .as_ref()
        .map(|values| {
            if values.is_empty() {
                "<none>".to_string()
            } else {
                values.join(",")
            }
        })
        .unwrap_or_else(|| "<none>".to_string());
    let max_retries = step
        .limits
        .as_ref()
        .and_then(|limits| limits.max_retries)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    vec![
        field_row("Step ID", Some(step.id.clone())),
        field_row("Step Type", Some(step.step_type.clone())),
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
            Some(output_files_as_csv(step.output_files.as_ref())),
        ),
        field_row("Max Retries", Some(max_retries)),
    ]
}

fn run_workflow_steps_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
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
                    let cfg = bootstrap
                        .orchestrator_configs
                        .get_mut(orchestrator_id)
                        .ok_or_else(|| "orchestrator missing".to_string())?;
                    let selector_agent = cfg.selector_agent.clone();
                    let Some(workflow) = cfg.workflows.iter_mut().find(|w| w.id == workflow_id)
                    else {
                        status = "workflow no longer exists".to_string();
                        continue;
                    };
                    if workflow.steps.iter().any(|step| step.id == step_id) {
                        status = "step id already exists".to_string();
                        continue;
                    }
                    workflow.steps.push(WorkflowStepConfig {
                        id: step_id,
                        step_type: "agent_task".to_string(),
                        agent: selector_agent,
                        prompt: default_step_prompt("agent_task"),
                        workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                        next: None,
                        on_approve: None,
                        on_reject: None,
                        outputs: default_step_output_contract("agent_task"),
                        output_files: default_step_output_files("agent_task"),
                        limits: None,
                    });
                    status = "step added".to_string();
                }
            }
            KeyCode::Char('d') => {
                if step_ids.is_empty() {
                    status = "no steps to delete".to_string();
                    continue;
                }
                let cfg = bootstrap
                    .orchestrator_configs
                    .get_mut(orchestrator_id)
                    .ok_or_else(|| "orchestrator missing".to_string())?;
                let Some(workflow) = cfg.workflows.iter_mut().find(|w| w.id == workflow_id) else {
                    status = "workflow no longer exists".to_string();
                    continue;
                };
                if workflow.steps.len() <= 1 {
                    status = "at least one step must remain".to_string();
                    continue;
                }
                workflow.steps.retain(|step| step.id != selected_step);
                selected = selected.saturating_sub(1);
                status = "step removed".to_string();
            }
            _ => {}
        }
    }
}

fn run_workflow_step_detail_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        let Some(workflow) = cfg.workflows.iter_mut().find(|w| w.id == workflow_id)
                        else {
                            status = "workflow no longer exists".to_string();
                            continue;
                        };
                        if workflow
                            .steps
                            .iter()
                            .any(|step| step.id == next_id && step.id != current_step_id)
                        {
                            status = "step id already exists".to_string();
                            continue;
                        }
                        if let Some(step) = workflow
                            .steps
                            .iter_mut()
                            .find(|step| step.id == current_step_id)
                        {
                            step.id = next_id.clone();
                            current_step_id = next_id;
                            status = "step id updated".to_string();
                        }
                    }
                }
                1 => {
                    let cfg = bootstrap
                        .orchestrator_configs
                        .get_mut(orchestrator_id)
                        .ok_or_else(|| "orchestrator missing".to_string())?;
                    if let Some(step) = cfg
                        .workflows
                        .iter_mut()
                        .find(|w| w.id == workflow_id)
                        .and_then(|workflow| {
                            workflow
                                .steps
                                .iter_mut()
                                .find(|step| step.id == current_step_id)
                        })
                    {
                        step.step_type = if step.step_type == "agent_task" {
                            "agent_review".to_string()
                        } else {
                            "agent_task".to_string()
                        };
                        step.prompt = default_step_prompt(&step.step_type);
                        step.outputs = default_step_output_contract(&step.step_type);
                        step.output_files = default_step_output_files(&step.step_type);
                        status = "step type toggled".to_string();
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        if let Some(step) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == workflow_id)
                            .and_then(|workflow| {
                                workflow
                                    .steps
                                    .iter_mut()
                                    .find(|step| step.id == current_step_id)
                            })
                        {
                            step.agent = value.trim().to_string();
                            status = "step agent updated".to_string();
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        if let Some(step) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == workflow_id)
                            .and_then(|workflow| {
                                workflow
                                    .steps
                                    .iter_mut()
                                    .find(|step| step.id == current_step_id)
                            })
                        {
                            step.prompt = value;
                            status = "step prompt updated".to_string();
                        }
                    }
                }
                4 => {
                    let cfg = bootstrap
                        .orchestrator_configs
                        .get_mut(orchestrator_id)
                        .ok_or_else(|| "orchestrator missing".to_string())?;
                    if let Some(step) = cfg
                        .workflows
                        .iter_mut()
                        .find(|w| w.id == workflow_id)
                        .and_then(|workflow| {
                            workflow
                                .steps
                                .iter_mut()
                                .find(|step| step.id == current_step_id)
                        })
                    {
                        step.workspace_mode = match step.workspace_mode {
                            WorkflowStepWorkspaceMode::OrchestratorWorkspace => {
                                WorkflowStepWorkspaceMode::RunWorkspace
                            }
                            WorkflowStepWorkspaceMode::RunWorkspace => {
                                WorkflowStepWorkspaceMode::AgentWorkspace
                            }
                            WorkflowStepWorkspaceMode::AgentWorkspace => {
                                WorkflowStepWorkspaceMode::OrchestratorWorkspace
                            }
                        };
                        status = "step workspace_mode toggled".to_string();
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        if let Some(step) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == workflow_id)
                            .and_then(|workflow| {
                                workflow
                                    .steps
                                    .iter_mut()
                                    .find(|step| step.id == current_step_id)
                            })
                        {
                            step.next = if value.trim().is_empty() {
                                None
                            } else {
                                Some(value.trim().to_string())
                            };
                            status = "step next updated".to_string();
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        if let Some(step) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == workflow_id)
                            .and_then(|workflow| {
                                workflow
                                    .steps
                                    .iter_mut()
                                    .find(|step| step.id == current_step_id)
                            })
                        {
                            step.on_approve = if value.trim().is_empty() {
                                None
                            } else {
                                Some(value.trim().to_string())
                            };
                            status = "step on_approve updated".to_string();
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        if let Some(step) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == workflow_id)
                            .and_then(|workflow| {
                                workflow
                                    .steps
                                    .iter_mut()
                                    .find(|step| step.id == current_step_id)
                            })
                        {
                            step.on_reject = if value.trim().is_empty() {
                                None
                            } else {
                                Some(value.trim().to_string())
                            };
                            status = "step on_reject updated".to_string();
                        }
                    }
                }
                8 => {
                    let current = step
                        .outputs
                        .as_ref()
                        .map(|outputs| outputs.join(","))
                        .unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Outputs",
                        "Comma-separated output keys (empty clears):",
                        &current,
                    )? {
                        let parsed = parse_csv_values(&value);
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        if let Some(step) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == workflow_id)
                            .and_then(|workflow| {
                                workflow
                                    .steps
                                    .iter_mut()
                                    .find(|step| step.id == current_step_id)
                            })
                        {
                            step.outputs = if parsed.is_empty() {
                                None
                            } else {
                                Some(parsed)
                            };
                            status = "step outputs updated".to_string();
                        }
                    }
                }
                9 => {
                    let current = output_files_as_csv(step.output_files.as_ref());
                    let initial = if current == "<none>" { "" } else { &current };
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Output Files",
                        "Comma-separated key=path mappings (empty clears):",
                        initial,
                    )? {
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        if value.trim().is_empty() {
                            if let Some(step) = cfg
                                .workflows
                                .iter_mut()
                                .find(|w| w.id == workflow_id)
                                .and_then(|workflow| {
                                    workflow
                                        .steps
                                        .iter_mut()
                                        .find(|step| step.id == current_step_id)
                                })
                            {
                                step.output_files = None;
                                status = "step output_files cleared".to_string();
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
                        if let Some(step) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == workflow_id)
                            .and_then(|workflow| {
                                workflow
                                    .steps
                                    .iter_mut()
                                    .find(|step| step.id == current_step_id)
                            })
                        {
                            step.output_files = Some(parsed);
                            status = "step output_files updated".to_string();
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
                        let cfg = bootstrap
                            .orchestrator_configs
                            .get_mut(orchestrator_id)
                            .ok_or_else(|| "orchestrator missing".to_string())?;
                        if let Some(step) = cfg
                            .workflows
                            .iter_mut()
                            .find(|w| w.id == workflow_id)
                            .and_then(|workflow| {
                                workflow
                                    .steps
                                    .iter_mut()
                                    .find(|step| step.id == current_step_id)
                            })
                        {
                            if value.trim().is_empty() {
                                if let Some(limits) = step.limits.as_mut() {
                                    limits.max_retries = None;
                                }
                                step.limits = None;
                                status = "step max_retries cleared".to_string();
                                continue;
                            }
                            match value.trim().parse::<u32>() {
                                Ok(parsed) if parsed >= 1 => {
                                    let limits = step
                                        .limits
                                        .get_or_insert(StepLimitsConfig { max_retries: None });
                                    limits.max_retries = Some(parsed);
                                    status = "step max_retries updated".to_string();
                                }
                                _ => status = "max_retries must be >= 1".to_string(),
                            }
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
    bootstrap: &mut SetupBootstrap,
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
                    let cfg = bootstrap
                        .orchestrator_configs
                        .get_mut(orchestrator_id)
                        .ok_or_else(|| "orchestrator missing".to_string())?;
                    if cfg.agents.contains_key(&agent_id) {
                        status = "agent id already exists".to_string();
                        continue;
                    }
                    cfg.agents.insert(
                        agent_id.clone(),
                        AgentConfig {
                            provider: bootstrap.provider.clone(),
                            model: bootstrap.model.clone(),
                            private_workspace: Some(PathBuf::from(format!("agents/{agent_id}"))),
                            can_orchestrate_workflows: false,
                            shared_access: Vec::new(),
                        },
                    );
                    status = "agent added".to_string();
                }
            }
            KeyCode::Char('d') => {
                if agent_ids.is_empty() {
                    status = "no agents to delete".to_string();
                    continue;
                }
                let cfg = bootstrap
                    .orchestrator_configs
                    .get_mut(orchestrator_id)
                    .ok_or_else(|| "orchestrator missing".to_string())?;
                if cfg.agents.len() <= 1 {
                    status = "at least one agent must remain".to_string();
                    continue;
                }
                cfg.agents.remove(&selected_agent);
                if cfg.selector_agent == selected_agent {
                    if let Some(next) = cfg.agents.keys().next() {
                        cfg.selector_agent = next.clone();
                    }
                }
                selected = selected.saturating_sub(1);
                status = "agent removed".to_string();
            }
            _ => {}
        }
    }
}

fn run_agent_detail_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
    config_exists: bool,
    orchestrator_id: &str,
    agent_id: &str,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status = "Enter to edit selected option. Esc back.".to_string();
    loop {
        let rows = agent_detail_menu_rows(bootstrap, orchestrator_id, agent_id);
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
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => match selected {
                0 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .and_then(|cfg| cfg.agents.get(agent_id))
                        .map(|a| a.provider.clone())
                        .unwrap_or_else(|| "anthropic".to_string());
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Agent Provider",
                        "provider (anthropic|openai):",
                        &current,
                    )? {
                        match parse_provider(value.trim()) {
                            Ok(provider) => {
                                if let Some(cfg) =
                                    bootstrap.orchestrator_configs.get_mut(orchestrator_id)
                                {
                                    if let Some(agent) = cfg.agents.get_mut(agent_id) {
                                        agent.provider = provider;
                                        status = "agent provider updated".to_string();
                                    }
                                }
                            }
                            Err(_) => status = "provider must be anthropic or openai".to_string(),
                        }
                    }
                }
                1 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .and_then(|cfg| cfg.agents.get(agent_id))
                        .map(|a| a.model.clone())
                        .unwrap_or_else(|| bootstrap.model.clone());
                    if let Some(value) =
                        prompt_line_tui(terminal, "Agent Model", "model:", &current)?
                    {
                        if value.trim().is_empty() {
                            status = "model must be non-empty".to_string();
                        } else if let Some(cfg) =
                            bootstrap.orchestrator_configs.get_mut(orchestrator_id)
                        {
                            if let Some(agent) = cfg.agents.get_mut(agent_id) {
                                agent.model = value.trim().to_string();
                                status = "agent model updated".to_string();
                            }
                        }
                    }
                }
                2 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .and_then(|cfg| cfg.agents.get(agent_id))
                        .and_then(|a| a.private_workspace.as_ref())
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Agent Private Workspace",
                        "private workspace (empty clears):",
                        &current,
                    )? {
                        if let Some(cfg) = bootstrap.orchestrator_configs.get_mut(orchestrator_id) {
                            if let Some(agent) = cfg.agents.get_mut(agent_id) {
                                if value.trim().is_empty() {
                                    agent.private_workspace = None;
                                } else {
                                    agent.private_workspace = Some(PathBuf::from(value.trim()));
                                }
                                status = "agent private workspace updated".to_string();
                            }
                        }
                    }
                }
                3 => {
                    let current = bootstrap
                        .orchestrator_configs
                        .get(orchestrator_id)
                        .and_then(|cfg| cfg.agents.get(agent_id))
                        .map(|a| a.shared_access.join(","))
                        .unwrap_or_default();
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Agent Shared Access",
                        "Comma-separated shared workspace keys:",
                        &current,
                    )? {
                        if let Some(cfg) = bootstrap.orchestrator_configs.get_mut(orchestrator_id) {
                            if let Some(agent) = cfg.agents.get_mut(agent_id) {
                                agent.shared_access = value
                                    .split(',')
                                    .map(|v| v.trim().to_string())
                                    .filter(|v| !v.is_empty())
                                    .collect();
                                status = "agent shared_access updated".to_string();
                            }
                        }
                    }
                }
                4 => {
                    if let Some(cfg) = bootstrap.orchestrator_configs.get_mut(orchestrator_id) {
                        if let Some(agent) = cfg.agents.get_mut(agent_id) {
                            agent.can_orchestrate_workflows = !agent.can_orchestrate_workflows;
                            status = "agent orchestration capability toggled".to_string();
                        }
                    }
                }
                _ => {
                    if let Some(cfg) = bootstrap.orchestrator_configs.get_mut(orchestrator_id) {
                        cfg.selector_agent = agent_id.to_string();
                        if let Some(agent) = cfg.agents.get_mut(agent_id) {
                            agent.can_orchestrate_workflows = true;
                        }
                        status = "selector agent updated".to_string();
                    }
                }
            },
            _ => {}
        }
    }
}

fn agent_detail_menu_rows(
    bootstrap: &SetupBootstrap,
    orchestrator_id: &str,
    agent_id: &str,
) -> Vec<SetupFieldRow> {
    let (provider, model, private_workspace, shared_access, can_orchestrate, is_selector) =
        bootstrap
            .orchestrator_configs
            .get(orchestrator_id)
            .and_then(|cfg| {
                cfg.agents.get(agent_id).map(|agent| {
                    (
                        agent.provider.clone(),
                        agent.model.clone(),
                        agent
                            .private_workspace
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "<none>".to_string()),
                        if agent.shared_access.is_empty() {
                            "<none>".to_string()
                        } else {
                            agent.shared_access.join(",")
                        },
                        if agent.can_orchestrate_workflows {
                            "yes".to_string()
                        } else {
                            "no".to_string()
                        },
                        if cfg.selector_agent == agent_id {
                            "yes".to_string()
                        } else {
                            "no".to_string()
                        },
                    )
                })
            })
            .unwrap_or_else(|| {
                (
                    "<missing>".to_string(),
                    "<missing>".to_string(),
                    "<none>".to_string(),
                    "<none>".to_string(),
                    "no".to_string(),
                    "no".to_string(),
                )
            });

    vec![
        field_row("Provider", Some(provider)),
        field_row("Model", Some(model)),
        field_row("Private Workspace", Some(private_workspace)),
        field_row("Shared Access", Some(shared_access)),
        field_row("Can Orchestrate Workflows", Some(can_orchestrate)),
        field_row("Set As Selector Agent", Some(is_selector)),
    ]
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

fn draw_setup_ui(frame: &mut Frame<'_>, config_exists: bool, selected: usize, status: &str) {
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
        Line::from(if config_exists {
            "Mode: existing setup (edit + apply)"
        } else {
            "Mode: first-time setup"
        }),
    ])
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, chunks[0]);

    let mut items = Vec::with_capacity(SETUP_MENU_ITEMS.len());
    for (idx, label) in SETUP_MENU_ITEMS.iter().enumerate() {
        let text = label.to_string();
        let mut item = ListItem::new(Line::from(Span::raw(text)));
        if idx == selected {
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
        Line::from("Up/Down move | Enter open | Esc cancel"),
        Line::from(format!("Status: {status}")),
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
        assert_eq!(parsed.get("result"), Some(&"output/result.md".to_string()));
        assert_eq!(parsed.get("summary"), Some(&"out/summary.md".to_string()));
        assert!(parse_output_files("missing_equals").is_err());
    }

    #[test]
    fn workflow_inputs_as_csv_handles_empty_sequence() {
        assert_eq!(
            workflow_inputs_as_csv(&serde_yaml::Value::Sequence(Vec::new())),
            "<none>"
        );
    }
}
