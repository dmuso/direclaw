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
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::time::Duration;

pub(super) fn cmd_setup() -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let config_exists = default_global_config_path()
        .map_err(map_config_err)?
        .exists();
    let mut state = load_setup_bootstrap(&paths)?;
    if is_interactive_setup() {
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
    let settings = state.normalize_for_save(config_exists)?;

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
struct SetupState {
    workspaces_path: PathBuf,
    orchestrator_id: String,
    provider: String,
    model: String,
    workflow_template: SetupWorkflowTemplate,
    orchestrators: BTreeMap<String, SettingsOrchestrator>,
    orchestrator_configs: BTreeMap<String, OrchestratorConfig>,
}

enum OrchestrationLimitField {
    MaxTotalIterations,
    DefaultRunTimeoutSeconds,
    DefaultStepTimeoutSeconds,
    MaxStepTimeoutSeconds,
}

impl SetupState {
    fn set_workspaces_path(&mut self, value: PathBuf) {
        self.workspaces_path = value;
    }

    fn set_default_provider(&mut self, provider: String) {
        self.provider = provider;
    }

    fn set_default_model(&mut self, model: String) {
        self.model = model;
    }

    fn set_default_workflow_template(&mut self, template: SetupWorkflowTemplate) {
        self.workflow_template = template;
    }

    fn set_primary_orchestrator(&mut self, orchestrator_id: &str) -> Result<(), String> {
        if !self.orchestrators.contains_key(orchestrator_id) {
            return Err(format!("orchestrator `{orchestrator_id}` does not exist"));
        }
        self.orchestrator_id = orchestrator_id.to_string();
        Ok(())
    }

    fn ensure_minimum_orchestrator(&mut self) {
        if self.orchestrators.is_empty() {
            let id = "main".to_string();
            self.orchestrators.insert(
                id.clone(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            );
            self.orchestrator_configs.insert(
                id.clone(),
                initial_orchestrator_config(
                    &id,
                    &self.provider,
                    &self.model,
                    self.workflow_template,
                ),
            );
            self.orchestrator_id = id;
        }
    }

    fn add_orchestrator(&mut self, orchestrator_id: &str) -> Result<(), String> {
        validate_identifier("orchestrator id", orchestrator_id)?;
        if self.orchestrators.contains_key(orchestrator_id) {
            return Err("orchestrator id already exists".to_string());
        }
        self.orchestrators.insert(
            orchestrator_id.to_string(),
            SettingsOrchestrator {
                private_workspace: None,
                shared_access: Vec::new(),
            },
        );
        self.orchestrator_configs.insert(
            orchestrator_id.to_string(),
            initial_orchestrator_config(
                orchestrator_id,
                &self.provider,
                &self.model,
                self.workflow_template,
            ),
        );
        self.orchestrator_id = orchestrator_id.to_string();
        Ok(())
    }

    fn remove_orchestrator(&mut self, orchestrator_id: &str) -> Result<(), String> {
        if !self.orchestrators.contains_key(orchestrator_id) {
            return Err(format!("orchestrator `{orchestrator_id}` does not exist"));
        }
        if self.orchestrators.len() <= 1 {
            return Err("at least one orchestrator must remain".to_string());
        }
        self.orchestrators.remove(orchestrator_id);
        self.orchestrator_configs.remove(orchestrator_id);
        if self.orchestrator_id == orchestrator_id {
            if let Some(next_id) = self.orchestrators.keys().next() {
                self.orchestrator_id = next_id.clone();
            }
        }
        Ok(())
    }

    fn set_orchestrator_private_workspace(
        &mut self,
        orchestrator_id: &str,
        value: Option<PathBuf>,
    ) -> Result<(), String> {
        let entry = self
            .orchestrators
            .get_mut(orchestrator_id)
            .ok_or_else(|| format!("orchestrator `{orchestrator_id}` missing in setup state"))?;
        entry.private_workspace = value;
        Ok(())
    }

    fn set_orchestrator_shared_access(
        &mut self,
        orchestrator_id: &str,
        shared_access: Vec<String>,
    ) -> Result<(), String> {
        let entry = self
            .orchestrators
            .get_mut(orchestrator_id)
            .ok_or_else(|| format!("orchestrator `{orchestrator_id}` missing in setup state"))?;
        entry.shared_access = shared_access;
        Ok(())
    }

    fn set_orchestrator_default_workflow(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
    ) -> Result<(), String> {
        validate_identifier("workflow id", workflow_id)?;
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| format!("orchestrator `{orchestrator_id}` missing in setup state"))?;
        if !cfg
            .workflows
            .iter()
            .any(|workflow| workflow.id == workflow_id)
        {
            return Err(format!("workflow `{workflow_id}` not found"));
        }
        cfg.default_workflow = workflow_id.to_string();
        Self::validate_orchestrator_invariants(cfg)
    }

    fn set_orchestrator_selection_max_retries(
        &mut self,
        orchestrator_id: &str,
        value: u32,
    ) -> Result<(), String> {
        if value < 1 {
            return Err("selection_max_retries must be >= 1".to_string());
        }
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| format!("orchestrator `{orchestrator_id}` missing in setup state"))?;
        cfg.selection_max_retries = value;
        Ok(())
    }

    fn set_orchestrator_selector_timeout_seconds(
        &mut self,
        orchestrator_id: &str,
        value: u64,
    ) -> Result<(), String> {
        if value < 1 {
            return Err("selector_timeout_seconds must be >= 1".to_string());
        }
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| format!("orchestrator `{orchestrator_id}` missing in setup state"))?;
        cfg.selector_timeout_seconds = value;
        Ok(())
    }

    fn set_orchestrator_workflow_orchestration_limit(
        &mut self,
        orchestrator_id: &str,
        field: OrchestrationLimitField,
        value: Option<u64>,
    ) -> Result<(), String> {
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| format!("orchestrator `{orchestrator_id}` missing in setup state"))?;
        if let Some(parsed) = value {
            if parsed < 1 {
                return Err("value must be >= 1".to_string());
            }
            let orchestration =
                cfg.workflow_orchestration
                    .get_or_insert(WorkflowOrchestrationConfig {
                        max_total_iterations: None,
                        default_run_timeout_seconds: None,
                        default_step_timeout_seconds: None,
                        max_step_timeout_seconds: None,
                    });
            match field {
                OrchestrationLimitField::MaxTotalIterations => {
                    if parsed > u32::MAX as u64 {
                        return Err("max_total_iterations exceeds u32 range".to_string());
                    }
                    orchestration.max_total_iterations = Some(parsed as u32);
                }
                OrchestrationLimitField::DefaultRunTimeoutSeconds => {
                    orchestration.default_run_timeout_seconds = Some(parsed);
                }
                OrchestrationLimitField::DefaultStepTimeoutSeconds => {
                    orchestration.default_step_timeout_seconds = Some(parsed);
                }
                OrchestrationLimitField::MaxStepTimeoutSeconds => {
                    orchestration.max_step_timeout_seconds = Some(parsed);
                }
            }
            return Ok(());
        }

        if let Some(orchestration) = cfg.workflow_orchestration.as_mut() {
            match field {
                OrchestrationLimitField::MaxTotalIterations => {
                    orchestration.max_total_iterations = None
                }
                OrchestrationLimitField::DefaultRunTimeoutSeconds => {
                    orchestration.default_run_timeout_seconds = None
                }
                OrchestrationLimitField::DefaultStepTimeoutSeconds => {
                    orchestration.default_step_timeout_seconds = None
                }
                OrchestrationLimitField::MaxStepTimeoutSeconds => {
                    orchestration.max_step_timeout_seconds = None
                }
            }
            if orchestration.max_total_iterations.is_none()
                && orchestration.default_run_timeout_seconds.is_none()
                && orchestration.default_step_timeout_seconds.is_none()
                && orchestration.max_step_timeout_seconds.is_none()
            {
                cfg.workflow_orchestration = None;
            }
        }
        Ok(())
    }

    fn apply_workflow_template_to_orchestrator(
        &mut self,
        orchestrator_id: &str,
        workflow_template: SetupWorkflowTemplate,
    ) -> Result<(), String> {
        let (provider, model) = provider_model_for_orchestrator(self, orchestrator_id);
        let template =
            initial_orchestrator_config(orchestrator_id, &provider, &model, workflow_template);
        let target = self
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
        Self::validate_orchestrator_invariants(target)
    }

    fn add_workflow(&mut self, orchestrator_id: &str, workflow_id: &str) -> Result<(), String> {
        validate_identifier("workflow id", workflow_id)?;
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        if cfg
            .workflows
            .iter()
            .any(|workflow| workflow.id == workflow_id)
        {
            return Err("workflow id already exists".to_string());
        }
        let selector_agent = cfg.selector_agent.clone();
        cfg.workflows.push(WorkflowConfig {
            id: workflow_id.to_string(),
            version: 1,
            inputs: WorkflowInputs::default(),
            limits: None,
            steps: vec![WorkflowStepConfig {
                id: "step_1".to_string(),
                step_type: WorkflowStepType::AgentTask,
                agent: selector_agent,
                prompt: default_step_scaffold("agent_task"),
                workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                next: None,
                on_approve: None,
                on_reject: None,
                outputs: default_step_output_contract("agent_task"),
                output_files: default_step_output_files("agent_task"),
                limits: None,
            }],
        });
        Self::validate_orchestrator_invariants(cfg)
    }

    fn remove_workflow(&mut self, orchestrator_id: &str, workflow_id: &str) -> Result<(), String> {
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        if !cfg
            .workflows
            .iter()
            .any(|workflow| workflow.id == workflow_id)
        {
            return Err(format!("workflow `{workflow_id}` does not exist"));
        }
        if cfg.workflows.len() <= 1 {
            return Err("at least one workflow must remain".to_string());
        }
        cfg.workflows.retain(|workflow| workflow.id != workflow_id);
        if cfg.default_workflow == workflow_id {
            if let Some(next) = cfg.workflows.first() {
                cfg.default_workflow = next.id.clone();
            }
        }
        Self::validate_orchestrator_invariants(cfg)
    }

    fn rename_workflow(
        &mut self,
        orchestrator_id: &str,
        current_id: &str,
        next_id: &str,
    ) -> Result<(), String> {
        validate_identifier("workflow id", next_id)?;
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        if cfg
            .workflows
            .iter()
            .any(|workflow| workflow.id == next_id && workflow.id != current_id)
        {
            return Err("workflow id already exists".to_string());
        }
        let workflow = cfg
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == current_id)
            .ok_or_else(|| "workflow no longer exists".to_string())?;
        workflow.id = next_id.to_string();
        if cfg.default_workflow == current_id {
            cfg.default_workflow = next_id.to_string();
        }
        Self::validate_orchestrator_invariants(cfg)
    }

    fn set_workflow_version(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        version: u32,
    ) -> Result<(), String> {
        if version < 1 {
            return Err("version must be >= 1".to_string());
        }
        let workflow = self
            .workflow_mut(orchestrator_id, workflow_id)
            .ok_or_else(|| "workflow no longer exists".to_string())?;
        workflow.version = version;
        Ok(())
    }

    fn set_workflow_inputs(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        keys: Vec<String>,
    ) -> Result<(), String> {
        let parsed = WorkflowInputs::parse_keys(keys)?;
        let workflow = self
            .workflow_mut(orchestrator_id, workflow_id)
            .ok_or_else(|| "workflow no longer exists".to_string())?;
        workflow.inputs = parsed;
        Ok(())
    }

    fn set_workflow_max_total_iterations(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        value: Option<u32>,
    ) -> Result<(), String> {
        let workflow = self
            .workflow_mut(orchestrator_id, workflow_id)
            .ok_or_else(|| "workflow no longer exists".to_string())?;
        if let Some(parsed) = value {
            if parsed < 1 {
                return Err("max_total_iterations must be >= 1".to_string());
            }
            let limits = workflow.limits.get_or_insert(WorkflowLimitsConfig {
                max_total_iterations: None,
                run_timeout_seconds: None,
            });
            limits.max_total_iterations = Some(parsed);
        } else if let Some(limits) = workflow.limits.as_mut() {
            limits.max_total_iterations = None;
            if limits.run_timeout_seconds.is_none() {
                workflow.limits = None;
            }
        }
        Ok(())
    }

    fn set_workflow_run_timeout_seconds(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        value: Option<u64>,
    ) -> Result<(), String> {
        let workflow = self
            .workflow_mut(orchestrator_id, workflow_id)
            .ok_or_else(|| "workflow no longer exists".to_string())?;
        if let Some(parsed) = value {
            if parsed < 1 {
                return Err("run_timeout_seconds must be >= 1".to_string());
            }
            let limits = workflow.limits.get_or_insert(WorkflowLimitsConfig {
                max_total_iterations: None,
                run_timeout_seconds: None,
            });
            limits.run_timeout_seconds = Some(parsed);
        } else if let Some(limits) = workflow.limits.as_mut() {
            limits.run_timeout_seconds = None;
            if limits.max_total_iterations.is_none() {
                workflow.limits = None;
            }
        }
        Ok(())
    }

    fn add_step(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
    ) -> Result<(), String> {
        validate_identifier("step id", step_id)?;
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        let selector_agent = cfg.selector_agent.clone();
        let workflow = cfg
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)
            .ok_or_else(|| "workflow no longer exists".to_string())?;
        if workflow.steps.iter().any(|step| step.id == step_id) {
            return Err("step id already exists".to_string());
        }
        workflow.steps.push(WorkflowStepConfig {
            id: step_id.to_string(),
            step_type: WorkflowStepType::AgentTask,
            agent: selector_agent,
            prompt: default_step_scaffold("agent_task"),
            workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
            next: None,
            on_approve: None,
            on_reject: None,
            outputs: default_step_output_contract("agent_task"),
            output_files: default_step_output_files("agent_task"),
            limits: None,
        });
        Self::validate_orchestrator_invariants(cfg)
    }

    fn remove_step(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
    ) -> Result<(), String> {
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        let workflow = cfg
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)
            .ok_or_else(|| "workflow no longer exists".to_string())?;
        if !workflow.steps.iter().any(|step| step.id == step_id) {
            return Err(format!("step `{step_id}` does not exist"));
        }
        if workflow.steps.len() <= 1 {
            return Err("at least one step must remain".to_string());
        }
        workflow.steps.retain(|step| step.id != step_id);
        Self::validate_orchestrator_invariants(cfg)
    }

    fn rename_step(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        current_id: &str,
        next_id: &str,
    ) -> Result<(), String> {
        validate_identifier("step id", next_id)?;
        let workflow = self
            .workflow_mut(orchestrator_id, workflow_id)
            .ok_or_else(|| "workflow no longer exists".to_string())?;
        if workflow
            .steps
            .iter()
            .any(|step| step.id == next_id && step.id != current_id)
        {
            return Err("step id already exists".to_string());
        }
        let step = workflow
            .steps
            .iter_mut()
            .find(|step| step.id == current_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        step.id = next_id.to_string();
        Ok(())
    }

    fn toggle_step_type(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
    ) -> Result<(), String> {
        let step = self
            .step_mut(orchestrator_id, workflow_id, step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        step.step_type = if step.step_type == WorkflowStepType::AgentTask {
            WorkflowStepType::AgentReview
        } else {
            WorkflowStepType::AgentTask
        };
        step.prompt = default_step_scaffold(step.step_type.as_str());
        step.outputs = default_step_output_contract(step.step_type.as_str());
        step.output_files = default_step_output_files(step.step_type.as_str());
        Ok(())
    }

    fn set_step_agent(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
        agent_id: &str,
    ) -> Result<(), String> {
        if agent_id.trim().is_empty() {
            return Err("agent must be non-empty".to_string());
        }
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        if !cfg.agents.contains_key(agent_id) {
            return Err(format!("agent `{agent_id}` does not exist"));
        }
        let workflow = cfg
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)
            .ok_or_else(|| "workflow no longer exists".to_string())?;
        let step = workflow
            .steps
            .iter_mut()
            .find(|step| step.id == step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        step.agent = agent_id.to_string();
        Ok(())
    }

    fn set_step_prompt(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
        prompt: &str,
    ) -> Result<(), String> {
        if prompt.trim().is_empty() {
            return Err("prompt must be non-empty".to_string());
        }
        let step = self
            .step_mut(orchestrator_id, workflow_id, step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        step.prompt = prompt.to_string();
        Ok(())
    }

    fn toggle_step_workspace_mode(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
    ) -> Result<(), String> {
        let step = self
            .step_mut(orchestrator_id, workflow_id, step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        step.workspace_mode = match step.workspace_mode {
            WorkflowStepWorkspaceMode::OrchestratorWorkspace => {
                WorkflowStepWorkspaceMode::RunWorkspace
            }
            WorkflowStepWorkspaceMode::RunWorkspace => WorkflowStepWorkspaceMode::AgentWorkspace,
            WorkflowStepWorkspaceMode::AgentWorkspace => {
                WorkflowStepWorkspaceMode::OrchestratorWorkspace
            }
        };
        Ok(())
    }

    fn set_step_next(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
        next: Option<String>,
    ) -> Result<(), String> {
        let step = self
            .step_mut(orchestrator_id, workflow_id, step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        step.next = next;
        Ok(())
    }

    fn set_step_on_approve(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
        on_approve: Option<String>,
    ) -> Result<(), String> {
        let step = self
            .step_mut(orchestrator_id, workflow_id, step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        step.on_approve = on_approve;
        Ok(())
    }

    fn set_step_on_reject(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
        on_reject: Option<String>,
    ) -> Result<(), String> {
        let step = self
            .step_mut(orchestrator_id, workflow_id, step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        step.on_reject = on_reject;
        Ok(())
    }

    fn set_step_outputs(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
        outputs: Vec<OutputKey>,
    ) -> Result<(), String> {
        if outputs.is_empty() {
            return Err("outputs must be non-empty".to_string());
        }
        let step = self
            .step_mut(orchestrator_id, workflow_id, step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        if let Some(missing) = outputs
            .iter()
            .find(|key| !step.output_files.contains_key(key.as_str()))
        {
            return Err(format!(
                "output_files must include mapping for output `{}`",
                missing
            ));
        }
        step.outputs = outputs;
        Ok(())
    }

    fn set_step_output_files(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
        output_files: BTreeMap<OutputKey, PathTemplate>,
    ) -> Result<(), String> {
        if output_files.is_empty() {
            return Err("output_files must be non-empty".to_string());
        }
        let step = self
            .step_mut(orchestrator_id, workflow_id, step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        if let Some(missing) = step
            .outputs
            .iter()
            .find(|key| !output_files.contains_key(key.as_str()))
        {
            return Err(format!(
                "output_files must include mapping for output `{}`",
                missing
            ));
        }
        step.output_files = output_files;
        Ok(())
    }

    fn set_step_max_retries(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
        max_retries: Option<u32>,
    ) -> Result<(), String> {
        let step = self
            .step_mut(orchestrator_id, workflow_id, step_id)
            .ok_or_else(|| "step no longer exists".to_string())?;
        if let Some(parsed) = max_retries {
            if parsed < 1 {
                return Err("max_retries must be >= 1".to_string());
            }
            let limits = step
                .limits
                .get_or_insert(StepLimitsConfig { max_retries: None });
            limits.max_retries = Some(parsed);
        } else {
            step.limits = None;
        }
        Ok(())
    }

    fn add_agent(&mut self, orchestrator_id: &str, agent_id: &str) -> Result<(), String> {
        validate_identifier("agent id", agent_id)?;
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        if cfg.agents.contains_key(agent_id) {
            return Err("agent id already exists".to_string());
        }
        cfg.agents.insert(
            agent_id.to_string(),
            AgentConfig {
                provider: ConfigProviderKind::parse(&self.provider)
                    .expect("setup provider remains valid"),
                model: self.model.clone(),
                private_workspace: Some(PathBuf::from(format!("agents/{agent_id}"))),
                can_orchestrate_workflows: false,
                shared_access: Vec::new(),
            },
        );
        Self::validate_orchestrator_invariants(cfg)
    }

    fn remove_agent(&mut self, orchestrator_id: &str, agent_id: &str) -> Result<(), String> {
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        if !cfg.agents.contains_key(agent_id) {
            return Err(format!("agent `{agent_id}` does not exist"));
        }
        if cfg.agents.len() <= 1 {
            return Err("at least one agent must remain".to_string());
        }
        cfg.agents.remove(agent_id);
        if cfg.selector_agent == agent_id {
            if let Some(next) = cfg.agents.keys().next() {
                cfg.selector_agent = next.clone();
            }
        }
        Self::validate_orchestrator_invariants(cfg)
    }

    fn set_agent_provider(
        &mut self,
        orchestrator_id: &str,
        agent_id: &str,
        provider: &str,
    ) -> Result<(), String> {
        let parsed = ConfigProviderKind::parse(provider)?;
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        let agent = cfg
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| "agent no longer exists".to_string())?;
        agent.provider = parsed;
        Ok(())
    }

    fn set_agent_model(
        &mut self,
        orchestrator_id: &str,
        agent_id: &str,
        model: &str,
    ) -> Result<(), String> {
        if model.trim().is_empty() {
            return Err("model must be non-empty".to_string());
        }
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        let agent = cfg
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| "agent no longer exists".to_string())?;
        agent.model = model.to_string();
        Ok(())
    }

    fn set_agent_private_workspace(
        &mut self,
        orchestrator_id: &str,
        agent_id: &str,
        workspace: Option<PathBuf>,
    ) -> Result<(), String> {
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        let agent = cfg
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| "agent no longer exists".to_string())?;
        agent.private_workspace = workspace;
        Ok(())
    }

    fn set_agent_shared_access(
        &mut self,
        orchestrator_id: &str,
        agent_id: &str,
        shared_access: Vec<String>,
    ) -> Result<(), String> {
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        let agent = cfg
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| "agent no longer exists".to_string())?;
        agent.shared_access = shared_access;
        Ok(())
    }

    fn toggle_agent_orchestration_capability(
        &mut self,
        orchestrator_id: &str,
        agent_id: &str,
    ) -> Result<(), String> {
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        let is_selector = cfg.selector_agent == agent_id;
        let current = cfg
            .agents
            .get(agent_id)
            .ok_or_else(|| "agent no longer exists".to_string())?
            .can_orchestrate_workflows;
        if is_selector && current {
            return Err("selector agent must keep orchestration capability enabled".to_string());
        }
        let agent = cfg
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| "agent no longer exists".to_string())?;
        agent.can_orchestrate_workflows = !current;
        Ok(())
    }

    fn set_selector_agent(&mut self, orchestrator_id: &str, agent_id: &str) -> Result<(), String> {
        let cfg = self
            .orchestrator_configs
            .get_mut(orchestrator_id)
            .ok_or_else(|| "orchestrator missing".to_string())?;
        if !cfg.agents.contains_key(agent_id) {
            return Err("agent no longer exists".to_string());
        }
        cfg.selector_agent = agent_id.to_string();
        if let Some(agent) = cfg.agents.get_mut(agent_id) {
            agent.can_orchestrate_workflows = true;
        }
        Ok(())
    }

    fn normalize_for_save(&mut self, config_exists: bool) -> Result<Settings, String> {
        self.ensure_minimum_orchestrator();
        self.orchestrators
            .entry(self.orchestrator_id.clone())
            .or_insert(SettingsOrchestrator {
                private_workspace: None,
                shared_access: Vec::new(),
            });
        self.orchestrator_configs
            .entry(self.orchestrator_id.clone())
            .or_insert_with(|| {
                initial_orchestrator_config(
                    &self.orchestrator_id,
                    &self.provider,
                    &self.model,
                    self.workflow_template,
                )
            });
        let mut settings = if config_exists {
            load_settings()?
        } else {
            Settings {
                workspaces_path: self.workspaces_path.clone(),
                shared_workspaces: BTreeMap::new(),
                orchestrators: self.orchestrators.clone(),
                channel_profiles: BTreeMap::new(),
                monitoring: Default::default(),
                channels: BTreeMap::new(),
                auth_sync: AuthSyncConfig::default(),
            }
        };
        settings.workspaces_path = self.workspaces_path.clone();
        settings.orchestrators = self.orchestrators.clone();
        if settings.channel_profiles.is_empty() {
            settings.channel_profiles.insert(
                "local-default".to_string(),
                ChannelProfile {
                    channel: ChannelKind::Local,
                    orchestrator_id: self.orchestrator_id.clone(),
                    slack_app_user_id: None,
                    require_mention_in_channels: None,
                },
            );
        }
        let has_primary_profile = settings
            .channel_profiles
            .values()
            .any(|profile| profile.orchestrator_id == self.orchestrator_id);
        if !has_primary_profile {
            settings.channel_profiles.insert(
                format!("{}-local", self.orchestrator_id),
                ChannelProfile {
                    channel: ChannelKind::Local,
                    orchestrator_id: self.orchestrator_id.clone(),
                    slack_app_user_id: None,
                    require_mention_in_channels: None,
                },
            );
        }
        settings
            .validate(ValidationOptions {
                require_shared_paths_exist: false,
            })
            .map_err(map_config_err)?;
        validate_setup_bootstrap(&settings, self)?;
        Ok(settings)
    }

    fn workflow_mut(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
    ) -> Option<&mut WorkflowConfig> {
        self.orchestrator_configs
            .get_mut(orchestrator_id)?
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)
    }

    fn step_mut(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
        step_id: &str,
    ) -> Option<&mut WorkflowStepConfig> {
        self.workflow_mut(orchestrator_id, workflow_id)?
            .steps
            .iter_mut()
            .find(|step| step.id == step_id)
    }

    fn validate_orchestrator_invariants(cfg: &OrchestratorConfig) -> Result<(), String> {
        if cfg.workflows.is_empty() {
            return Err("at least one workflow must remain".to_string());
        }
        if !cfg
            .workflows
            .iter()
            .any(|workflow| workflow.id == cfg.default_workflow)
        {
            return Err(format!(
                "default workflow `{}` does not exist",
                cfg.default_workflow
            ));
        }
        let mut workflow_ids = BTreeSet::new();
        for workflow in &cfg.workflows {
            if !workflow_ids.insert(workflow.id.clone()) {
                return Err(format!("workflow id `{}` already exists", workflow.id));
            }
            if workflow.steps.is_empty() {
                return Err(format!(
                    "workflow `{}` must include at least one step",
                    workflow.id
                ));
            }
            let mut step_ids = BTreeSet::new();
            for step in &workflow.steps {
                if !step_ids.insert(step.id.clone()) {
                    return Err(format!(
                        "step id `{}` already exists in workflow `{}`",
                        step.id, workflow.id
                    ));
                }
                if !cfg.agents.contains_key(&step.agent) {
                    return Err(format!(
                        "workflow `{}` step `{}` references unknown agent `{}`",
                        workflow.id, step.id, step.agent
                    ));
                }
                if step.outputs.is_empty() {
                    return Err(format!(
                        "workflow `{}` step `{}` requires non-empty outputs",
                        workflow.id, step.id
                    ));
                }
                if step.output_files.is_empty() {
                    return Err(format!(
                        "workflow `{}` step `{}` requires non-empty output_files",
                        workflow.id, step.id
                    ));
                }
                if let Some(missing) = step
                    .outputs
                    .iter()
                    .find(|key| !step.output_files.contains_key(key.as_str()))
                {
                    return Err(format!(
                        "workflow `{}` step `{}` missing output_files mapping for `{}`",
                        workflow.id, step.id, missing
                    ));
                }
            }
        }
        if cfg.agents.is_empty() {
            return Err("at least one agent must remain".to_string());
        }
        if !cfg.agents.contains_key(&cfg.selector_agent) {
            return Err(format!(
                "selector agent `{}` does not exist",
                cfg.selector_agent
            ));
        }
        if let Some(selector) = cfg.agents.get(&cfg.selector_agent) {
            if !selector.can_orchestrate_workflows {
                return Err(format!(
                    "selector agent `{}` must keep orchestration enabled",
                    cfg.selector_agent
                ));
            }
        }
        Ok(())
    }
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
    Ok(ConfigProviderKind::parse(value)?.to_string())
}

fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
    match kind {
        "orchestrator id" => crate::config::OrchestratorId::parse(value).map(|_| ()),
        "workflow id" => crate::config::WorkflowId::parse(value).map(|_| ()),
        "step id" => crate::config::StepId::parse(value).map(|_| ()),
        "agent id" => crate::config::AgentId::parse(value).map(|_| ()),
        _ => Err(format!("unsupported identifier kind `{kind}`")),
    }
}

fn validate_setup_bootstrap(settings: &Settings, bootstrap: &SetupState) -> Result<(), String> {
    if bootstrap.orchestrators.is_empty() {
        return Err("at least one orchestrator must be configured".to_string());
    }
    if !bootstrap
        .orchestrators
        .contains_key(&bootstrap.orchestrator_id)
    {
        return Err(format!(
            "primary orchestrator `{}` is missing from setup registry",
            bootstrap.orchestrator_id
        ));
    }

    for orchestrator_id in settings.orchestrators.keys() {
        validate_identifier("orchestrator id", orchestrator_id)?;
        let config = bootstrap
            .orchestrator_configs
            .get(orchestrator_id)
            .ok_or_else(|| {
                format!("missing orchestrator config for `{orchestrator_id}` in setup registry")
            })?;
        config
            .validate(settings, orchestrator_id)
            .map_err(map_config_err)?;
    }
    for orchestrator_id in bootstrap.orchestrator_configs.keys() {
        if !settings.orchestrators.contains_key(orchestrator_id) {
            return Err(format!(
                "orchestrator config `{orchestrator_id}` exists without settings entry"
            ));
        }
    }
    Ok(())
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
    bootstrap: &mut SetupState,
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
                        bootstrap.set_workspaces_path(PathBuf::from(value.trim()));
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
    bootstrap: &mut SetupState,
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
                    let next_provider = if bootstrap.provider == "anthropic" {
                        "openai".to_string()
                    } else {
                        "anthropic".to_string()
                    };
                    bootstrap.set_default_provider(next_provider);
                    if bootstrap.model == "sonnet" || bootstrap.model == "gpt-5.3-codex" {
                        bootstrap.set_default_model(
                            default_model_for_provider(&bootstrap.provider).to_string(),
                        );
                    }
                    status = format!("provider set to {}", bootstrap.provider);
                } else if let Some(value) =
                    prompt_line_tui(terminal, "Default Model", "Enter model:", &bootstrap.model)?
                {
                    if value.trim().is_empty() {
                        status = "model must be non-empty".to_string();
                    } else {
                        bootstrap.set_default_model(value.trim().to_string());
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

fn provider_model_for_orchestrator(
    bootstrap: &SetupState,
    orchestrator_id: &str,
) -> (String, String) {
    if let Some(cfg) = bootstrap.orchestrator_configs.get(orchestrator_id) {
        if let Some(selector) = cfg.agents.get(&cfg.selector_agent) {
            return (selector.provider.to_string(), selector.model.clone());
        }
        if let Some((_, agent)) = cfg.agents.iter().next() {
            return (agent.provider.to_string(), agent.model.clone());
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
                        .map(|a| a.provider.to_string())
                        .unwrap_or_else(|| "anthropic".to_string());
                    if let Some(value) = prompt_line_tui(
                        terminal,
                        "Agent Provider",
                        "provider (anthropic|openai):",
                        &current,
                    )? {
                        match parse_provider(value.trim()) {
                            Ok(provider) => {
                                match bootstrap.set_agent_provider(
                                    orchestrator_id,
                                    agent_id,
                                    &provider,
                                ) {
                                    Ok(_) => status = "agent provider updated".to_string(),
                                    Err(err) => status = err,
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
                        } else {
                            match bootstrap.set_agent_model(orchestrator_id, agent_id, value.trim())
                            {
                                Ok(_) => status = "agent model updated".to_string(),
                                Err(err) => status = err,
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
                        let workspace = if value.trim().is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(value.trim()))
                        };
                        match bootstrap.set_agent_private_workspace(
                            orchestrator_id,
                            agent_id,
                            workspace,
                        ) {
                            Ok(_) => status = "agent private workspace updated".to_string(),
                            Err(err) => status = err,
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
                        let shared_access = value
                            .split(',')
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty())
                            .collect();
                        match bootstrap.set_agent_shared_access(
                            orchestrator_id,
                            agent_id,
                            shared_access,
                        ) {
                            Ok(_) => status = "agent shared_access updated".to_string(),
                            Err(err) => status = err,
                        }
                    }
                }
                4 => {
                    match bootstrap.toggle_agent_orchestration_capability(orchestrator_id, agent_id)
                    {
                        Ok(_) => status = "agent orchestration capability toggled".to_string(),
                        Err(err) => status = err,
                    }
                }
                _ => match bootstrap.set_selector_agent(orchestrator_id, agent_id) {
                    Ok(_) => status = "selector agent updated".to_string(),
                    Err(err) => status = err,
                },
            },
            _ => {}
        }
    }
}

fn agent_detail_menu_rows(
    bootstrap: &SetupState,
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
                        agent.provider.to_string(),
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
    fn validate_setup_bootstrap_requires_matching_config_registry() {
        let settings = Settings {
            workspaces_path: PathBuf::from("/tmp/workspaces"),
            shared_workspaces: BTreeMap::new(),
            orchestrators: BTreeMap::from_iter([(
                "main".to_string(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            )]),
            channel_profiles: BTreeMap::new(),
            monitoring: Default::default(),
            channels: BTreeMap::new(),
            auth_sync: AuthSyncConfig::default(),
        };

        let bootstrap = SetupState {
            workspaces_path: settings.workspaces_path.clone(),
            orchestrator_id: "main".to_string(),
            provider: "anthropic".to_string(),
            model: "sonnet".to_string(),
            workflow_template: SetupWorkflowTemplate::Minimal,
            orchestrators: settings.orchestrators.clone(),
            orchestrator_configs: BTreeMap::new(),
        };

        let err = validate_setup_bootstrap(&settings, &bootstrap).expect_err("missing config");
        assert!(err.contains("missing orchestrator config"));
    }

    #[test]
    fn validate_setup_bootstrap_accepts_valid_minimal_registry() {
        let settings = Settings {
            workspaces_path: PathBuf::from("/tmp/workspaces"),
            shared_workspaces: BTreeMap::new(),
            orchestrators: BTreeMap::from_iter([(
                "main".to_string(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            )]),
            channel_profiles: BTreeMap::new(),
            monitoring: Default::default(),
            channels: BTreeMap::new(),
            auth_sync: AuthSyncConfig::default(),
        };

        let bootstrap = SetupState {
            workspaces_path: settings.workspaces_path.clone(),
            orchestrator_id: "main".to_string(),
            provider: "anthropic".to_string(),
            model: "sonnet".to_string(),
            workflow_template: SetupWorkflowTemplate::Minimal,
            orchestrators: settings.orchestrators.clone(),
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

        validate_setup_bootstrap(&settings, &bootstrap).expect("valid setup bootstrap");
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
            .normalize_for_save(false)
            .expect_err("invalid default workflow");
        assert!(err.contains("default workflow") || err.contains("missing"));
    }
}
