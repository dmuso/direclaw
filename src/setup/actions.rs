use crate::app::command_support::{ensure_runtime_root, map_config_err};
use crate::config::{
    agent_editable_fields, default_global_config_path, AgentEditableField, ConfigProviderKind,
    OrchestrationLimitField, OutputKey,
};
use crate::setup::navigation::{
    parse_scripted_setup_keys, setup_action_from_key, setup_screen_item_count, setup_transition,
    NavState, SetupAction, SetupNavEffect, SetupScreen,
};
use crate::setup::persistence::{load_setup_bootstrap, persist_setup_state};
use crate::setup::screens::{
    centered_rect, draw_field_screen, draw_list_screen, draw_setup_ui, field_row,
    project_setup_menu_view_model, tail_for_display, workflow_step_menu_rows, SetupFieldRow,
    SETUP_MENU_ITEMS,
};
use crate::setup::state::{
    default_model_for_provider, infer_workflow_template, model_options_for_provider,
    output_files_as_csv, parse_csv_values, parse_output_files, provider_options,
    setup_workflow_template_index, unique_step_id, workflow_inputs_as_csv,
    workflow_template_from_index, workflow_template_options, SetupState,
};
use crate::templates::orchestrator_templates::WorkflowTemplate as SetupWorkflowTemplate;
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
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratatui::Terminal;
use std::collections::BTreeMap;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::time::Duration;

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
    persist_setup_state(&paths, &mut state, config_exists)
}

#[cfg(test)]
fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
    crate::setup::state::validate_identifier(kind, value)
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
    parse_scripted_setup_keys(&raw).map(Some)
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
        let item_count = setup_screen_item_count(nav.screen, SETUP_MENU_ITEMS.len());
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
        let item_count = setup_screen_item_count(nav.screen, SETUP_MENU_ITEMS.len());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SettingsOrchestrator;
    use crate::setup::navigation::SetupNavError;
    use crate::templates::orchestrator_templates::initial_orchestrator_config;
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
        assert_eq!(nav.status_text, "Enter opens a section. Esc cancels setup.");
        assert_eq!(nav.hint_text, "Up/Down move | Enter open | Esc cancel");

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
        assert_eq!(
            workflow_inputs_as_csv(&crate::config::WorkflowInputs::default()),
            "<none>"
        );
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
        assert_eq!(descriptors.len(), 4);
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
        let template =
            crate::config::PathTemplate::parse("outputs/summary.md").expect("path template");

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
