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
    let path = save_settings(&settings)?;
    bootstrap
        .orchestrator_configs
        .entry(bootstrap.orchestrator_id.clone())
        .or_insert_with(|| {
            initial_orchestrator_config(
                &bootstrap.orchestrator_id,
                &bootstrap.provider,
                &bootstrap.model,
                bootstrap.bundle,
            )
        });
    let orchestrator_path = save_orchestrator_registry(&settings, &bootstrap.orchestrator_configs)?;
    let prefs = RuntimePreferences {
        provider: Some(bootstrap.provider.clone()),
        model: Some(bootstrap.model.clone()),
    };
    save_preferences(&paths, &prefs)?;
    Ok(format!(
        "setup complete\nconfig={}\nstate_root={}\nworkspace={}\norchestrator={}\nworkflow_bundle={}\nprovider={}\nmodel={}\norchestrator_config={}",
        path.display(),
        paths.root.display(),
        bootstrap.workspaces_path.display(),
        bootstrap.orchestrator_id,
        bootstrap.bundle.as_str(),
        bootstrap.provider,
        bootstrap.model,
        orchestrator_path.display()
    ))
}

#[derive(Debug, Clone, Copy)]
pub(super) enum SetupWorkflowBundle {
    Minimal,
    Engineering,
    Product,
}

impl SetupWorkflowBundle {
    fn as_str(self) -> &'static str {
        match self {
            SetupWorkflowBundle::Minimal => "minimal",
            SetupWorkflowBundle::Engineering => "engineering",
            SetupWorkflowBundle::Product => "product",
        }
    }
}

#[derive(Debug, Clone)]
struct SetupBootstrap {
    workspaces_path: PathBuf,
    orchestrator_id: String,
    provider: String,
    model: String,
    bundle: SetupWorkflowBundle,
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

fn infer_bundle(orchestrator: &OrchestratorConfig) -> SetupWorkflowBundle {
    if orchestrator.agents.contains_key("planner")
        && orchestrator.agents.contains_key("builder")
        && orchestrator.agents.contains_key("reviewer")
    {
        return SetupWorkflowBundle::Engineering;
    }
    if orchestrator.agents.contains_key("researcher") && orchestrator.agents.contains_key("writer")
    {
        return SetupWorkflowBundle::Product;
    }
    SetupWorkflowBundle::Minimal
}

fn load_setup_bootstrap(paths: &StatePaths) -> Result<SetupBootstrap, String> {
    let default_workspace = paths.root.join("workspaces");
    let mut bootstrap = SetupBootstrap {
        workspaces_path: default_workspace,
        orchestrator_id: "main".to_string(),
        provider: "anthropic".to_string(),
        model: "sonnet".to_string(),
        bundle: SetupWorkflowBundle::Minimal,
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
                SetupWorkflowBundle::Minimal,
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
    let registry_path = default_orchestrators_config_path().map_err(map_config_err)?;
    if registry_path.exists() {
        let raw = fs::read_to_string(&registry_path)
            .map_err(|e| format!("failed to read {}: {e}", registry_path.display()))?;
        configs = serde_yaml::from_str::<BTreeMap<String, OrchestratorConfig>>(&raw)
            .map_err(|e| format!("failed to parse {}: {e}", registry_path.display()))?;
    }
    for orchestrator_id in bootstrap.orchestrators.keys() {
        configs.entry(orchestrator_id.clone()).or_insert_with(|| {
            initial_orchestrator_config(
                orchestrator_id,
                &bootstrap.provider,
                &bootstrap.model,
                SetupWorkflowBundle::Minimal,
            )
        });
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
            bootstrap.bundle = infer_bundle(orchestrator);
        }
    }

    Ok(bootstrap)
}

const SETUP_MENU_ITEMS: [&str; 6] = [
    "Workspaces",
    "Orchestrators",
    "Initial Agent Defaults",
    "Workflow Bundle",
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
                3 => {
                    if let Some(message) =
                        run_workflow_bundle_tui(terminal, bootstrap, config_exists)?
                    {
                        status = message;
                    }
                }
                4 => return Ok(SetupExit::Save),
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

fn run_workflow_bundle_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
    config_exists: bool,
) -> Result<Option<String>, String> {
    let mut selected = match bootstrap.bundle {
        SetupWorkflowBundle::Minimal => 0usize,
        SetupWorkflowBundle::Engineering => 1usize,
        SetupWorkflowBundle::Product => 2usize,
    };
    let mut status = "Select a bundle and press Enter. Esc back.".to_string();
    loop {
        let items = vec![
            "minimal".to_string(),
            "engineering".to_string(),
            "product".to_string(),
        ];
        draw_list_screen(
            terminal,
            "Setup > Workflow Bundle",
            config_exists,
            &items,
            selected,
            &status,
            "Up/Down move | Enter select | Esc back",
        )?;
        let ev = event::read().map_err(|e| format!("failed to read workflow bundle input: {e}"))?;
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(Some("Closed Workflow Bundle.".to_string())),
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = std::cmp::min(selected + 1, 2),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                bootstrap.bundle = match selected {
                    0 => SetupWorkflowBundle::Minimal,
                    1 => SetupWorkflowBundle::Engineering,
                    _ => SetupWorkflowBundle::Product,
                };
                status = format!("workflow bundle set to {}", bootstrap.bundle.as_str());
            }
            _ => {}
        }
    }
}

fn run_orchestrator_manager_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    bootstrap: &mut SetupBootstrap,
    config_exists: bool,
) -> Result<Option<String>, String> {
    let mut selected = 0usize;
    let mut status =
        "Enter open orchestrator. a add, d delete, e set primary. Esc back.".to_string();
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
                    bootstrap.bundle,
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
            "Up/Down move | Enter open | a add | d delete | e set primary | Esc back",
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
                                bootstrap.bundle,
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
                5 => {
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
        field_row("Selection Max Retries", Some(selection_max_retries)),
        field_row("Selector Timeout Seconds", Some(selector_timeout_seconds)),
        field_row("Agents", None),
    ]
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
