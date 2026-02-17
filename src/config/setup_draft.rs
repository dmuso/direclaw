use crate::config::{
    AgentConfig, AuthSyncConfig, ChannelKind, ChannelProfile, ConfigProviderKind,
    OrchestratorConfig, OutputKey, PathTemplate, Settings, SettingsOrchestrator, StepLimitsConfig,
    ValidationOptions, WorkflowConfig, WorkflowInputs, WorkflowLimitsConfig,
    WorkflowOrchestrationConfig, WorkflowStepConfig, WorkflowStepPromptType, WorkflowStepType,
    WorkflowStepWorkspaceMode, WorkflowTag,
};
use crate::templates::orchestrator_templates::{
    initial_orchestrator_config, WorkflowTemplate as SetupWorkflowTemplate,
};
use crate::templates::workflow_step_defaults::{
    default_step_output_contract, default_step_output_files, default_step_output_priority,
    default_step_scaffold,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct SetupDraft {
    pub(crate) workspaces_path: PathBuf,
    pub(crate) orchestrator_id: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) workflow_template: SetupWorkflowTemplate,
    pub(crate) orchestrators: BTreeMap<String, SettingsOrchestrator>,
    pub(crate) orchestrator_configs: BTreeMap<String, OrchestratorConfig>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum OrchestrationLimitField {
    MaxTotalIterations,
    DefaultRunTimeoutSeconds,
    DefaultStepTimeoutSeconds,
    MaxStepTimeoutSeconds,
}

impl SetupDraft {
    pub(crate) fn set_workspaces_path(&mut self, value: PathBuf) {
        self.workspaces_path = value;
    }

    pub(crate) fn set_default_provider(&mut self, provider: String) {
        self.provider = provider;
    }

    pub(crate) fn set_default_model(&mut self, model: String) {
        self.model = model;
    }

    pub(crate) fn set_default_workflow_template(&mut self, template: SetupWorkflowTemplate) {
        self.workflow_template = template;
    }

    pub(crate) fn set_primary_orchestrator(&mut self, orchestrator_id: &str) -> Result<(), String> {
        if !self.orchestrators.contains_key(orchestrator_id) {
            return Err(format!("orchestrator `{orchestrator_id}` does not exist"));
        }
        self.orchestrator_id = orchestrator_id.to_string();
        Ok(())
    }

    pub(crate) fn ensure_minimum_orchestrator(&mut self) {
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

    pub(crate) fn add_orchestrator(&mut self, orchestrator_id: &str) -> Result<(), String> {
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

    pub(crate) fn remove_orchestrator(&mut self, orchestrator_id: &str) -> Result<(), String> {
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

    pub(crate) fn set_orchestrator_private_workspace(
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

    pub(crate) fn set_orchestrator_shared_access(
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

    pub(crate) fn set_orchestrator_default_workflow(
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
        validate_orchestrator_invariants(cfg)
    }

    pub(crate) fn set_orchestrator_selection_max_retries(
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

    pub(crate) fn set_orchestrator_selector_timeout_seconds(
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

    pub(crate) fn set_orchestrator_workflow_orchestration_limit(
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

    pub(crate) fn apply_workflow_template_to_orchestrator(
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
        validate_orchestrator_invariants(target)
    }

    pub(crate) fn add_workflow(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
    ) -> Result<(), String> {
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
            description: format!("{workflow_id} workflow"),
            tags: vec![WorkflowTag::parse(workflow_id)?],
            inputs: WorkflowInputs::default(),
            limits: None,
            steps: vec![WorkflowStepConfig {
                id: "step_1".to_string(),
                step_type: WorkflowStepType::AgentTask,
                agent: selector_agent,
                prompt: default_step_scaffold("agent_task"),
                prompt_type: WorkflowStepPromptType::FileOutput,
                workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                next: None,
                on_approve: None,
                on_reject: None,
                outputs: default_step_output_contract("agent_task"),
                output_files: default_step_output_files("agent_task"),
                final_output_priority: default_step_output_priority("agent_task"),
                limits: None,
            }],
        });
        validate_orchestrator_invariants(cfg)
    }

    pub(crate) fn remove_workflow(
        &mut self,
        orchestrator_id: &str,
        workflow_id: &str,
    ) -> Result<(), String> {
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
        validate_orchestrator_invariants(cfg)
    }

    pub(crate) fn rename_workflow(
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
        validate_orchestrator_invariants(cfg)
    }

    pub(crate) fn set_workflow_version(
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

    pub(crate) fn set_workflow_inputs(
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

    pub(crate) fn set_workflow_max_total_iterations(
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

    pub(crate) fn set_workflow_run_timeout_seconds(
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

    pub(crate) fn add_step(
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
            prompt_type: WorkflowStepPromptType::FileOutput,
            workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
            next: None,
            on_approve: None,
            on_reject: None,
            outputs: default_step_output_contract("agent_task"),
            output_files: default_step_output_files("agent_task"),
            final_output_priority: default_step_output_priority("agent_task"),
            limits: None,
        });
        validate_orchestrator_invariants(cfg)
    }

    pub(crate) fn remove_step(
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
        validate_orchestrator_invariants(cfg)
    }

    pub(crate) fn rename_step(
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

    pub(crate) fn toggle_step_type(
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

    pub(crate) fn set_step_agent(
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

    pub(crate) fn set_step_prompt(
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

    pub(crate) fn toggle_step_workspace_mode(
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

    pub(crate) fn set_step_next(
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

    pub(crate) fn set_step_on_approve(
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

    pub(crate) fn set_step_on_reject(
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

    pub(crate) fn set_step_outputs(
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

    pub(crate) fn set_step_output_files(
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

    pub(crate) fn set_step_max_retries(
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

    pub(crate) fn add_agent(
        &mut self,
        orchestrator_id: &str,
        agent_id: &str,
    ) -> Result<(), String> {
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
        validate_orchestrator_invariants(cfg)
    }

    pub(crate) fn remove_agent(
        &mut self,
        orchestrator_id: &str,
        agent_id: &str,
    ) -> Result<(), String> {
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
        validate_orchestrator_invariants(cfg)
    }

    pub(crate) fn set_agent_provider(
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

    pub(crate) fn set_agent_model(
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

    pub(crate) fn set_agent_private_workspace(
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

    pub(crate) fn set_agent_shared_access(
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

    pub(crate) fn toggle_agent_orchestration_capability(
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

    pub(crate) fn set_selector_agent(
        &mut self,
        orchestrator_id: &str,
        agent_id: &str,
    ) -> Result<(), String> {
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

    pub(crate) fn normalize_for_save(
        &mut self,
        existing_settings: Option<Settings>,
    ) -> Result<Settings, String> {
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

        let mut settings = existing_settings.unwrap_or_else(|| Settings {
            workspaces_path: self.workspaces_path.clone(),
            shared_workspaces: BTreeMap::new(),
            orchestrators: self.orchestrators.clone(),
            channel_profiles: BTreeMap::new(),
            monitoring: Default::default(),
            channels: BTreeMap::new(),
            auth_sync: AuthSyncConfig::default(),
        });

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
            .map_err(|err| err.to_string())?;
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
}

fn validate_orchestrator_invariants(cfg: &OrchestratorConfig) -> Result<(), String> {
    cfg.validate_setup_invariants()
        .map_err(|err| err.to_string())
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

fn validate_setup_bootstrap(settings: &Settings, bootstrap: &SetupDraft) -> Result<(), String> {
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
            .map_err(|err| err.to_string())?;
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

fn provider_model_for_orchestrator(
    bootstrap: &SetupDraft,
    orchestrator_id: &str,
) -> (String, String) {
    bootstrap
        .orchestrator_configs
        .get(orchestrator_id)
        .and_then(|cfg| cfg.agents.get(&cfg.selector_agent))
        .or_else(|| {
            bootstrap
                .orchestrator_configs
                .get(orchestrator_id)
                .and_then(|cfg| cfg.agents.values().next())
        })
        .map(|agent| (agent.provider.to_string(), agent.model.clone()))
        .unwrap_or_else(|| (bootstrap.provider.clone(), bootstrap.model.clone()))
}

fn unique_workflow_id(existing: &BTreeMap<String, WorkflowConfig>, base: &str) -> String {
    if !existing.contains_key(base) {
        return base.to_string();
    }
    for idx in 2..1000 {
        let candidate = format!("{}_{}", base, idx);
        if !existing.contains_key(&candidate) {
            return candidate;
        }
    }
    format!("{}_{}", base, existing.len() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_setup_draft() -> SetupDraft {
        SetupDraft {
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
    fn setup_draft_enforces_orchestrator_and_workflow_invariants() {
        let mut draft = test_setup_draft();
        draft.add_orchestrator("alpha").expect("add orchestrator");
        assert_eq!(draft.orchestrator_id, "alpha");
        assert!(draft.add_orchestrator("alpha").is_err());
        draft
            .remove_orchestrator("alpha")
            .expect("remove orchestrator");
        assert_eq!(draft.orchestrators.len(), 1);
        assert!(draft.remove_orchestrator("main").is_err());

        assert!(draft
            .set_orchestrator_default_workflow("main", "missing")
            .is_err());
        draft.add_workflow("main", "triage").expect("add workflow");
        assert!(draft.add_workflow("main", "triage").is_err());
        draft
            .set_orchestrator_default_workflow("main", "triage")
            .expect("set default");
        draft
            .remove_workflow("main", "default")
            .expect("remove default");
        assert!(draft.remove_workflow("main", "triage").is_err());
    }

    #[test]
    fn setup_draft_enforces_step_output_contract_invariants() {
        let mut draft = test_setup_draft();
        let key = OutputKey::parse("summary").expect("output key");
        let missing_key = OutputKey::parse("missing").expect("output key");
        let template = PathTemplate::parse("outputs/summary.md").expect("path template");

        assert!(draft
            .set_step_outputs("main", "default", "step_1", Vec::new())
            .is_err());
        assert!(draft
            .set_step_output_files("main", "default", "step_1", BTreeMap::new())
            .is_err());

        assert!(draft
            .set_step_outputs("main", "default", "step_1", vec![missing_key.clone()])
            .is_err());

        let valid_files = BTreeMap::from_iter([(key.clone(), template.clone())]);
        draft
            .set_step_outputs("main", "default", "step_1", vec![key.clone()])
            .expect("set outputs");
        draft
            .set_step_output_files("main", "default", "step_1", valid_files)
            .expect("set output files");

        let invalid_files = BTreeMap::from_iter([(missing_key, template)]);
        assert!(draft
            .set_step_output_files("main", "default", "step_1", invalid_files)
            .is_err());
    }

    #[test]
    fn setup_draft_normalize_for_save_rejects_invalid_domain_state() {
        let mut draft = test_setup_draft();
        let cfg = draft
            .orchestrator_configs
            .get_mut("main")
            .expect("main orchestrator config");
        cfg.default_workflow = "missing".to_string();

        let err = draft
            .normalize_for_save(None)
            .expect_err("invalid default workflow");
        assert!(err.contains("default_workflow") || err.contains("missing"));
    }
}
