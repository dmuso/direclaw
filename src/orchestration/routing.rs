use crate::commands;
use crate::config::Settings;
use crate::orchestration::run_store::WorkflowRunStore;
use crate::orchestration::selector::{FunctionArgSchema, FunctionSchema};
use crate::orchestrator::OrchestratorError;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCall {
    pub function_id: String,
    pub args: Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct FunctionRegistry {
    allowed: BTreeSet<String>,
    catalog: BTreeMap<String, FunctionSchema>,
    run_store: Option<WorkflowRunStore>,
    settings: Option<Settings>,
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self::new(Vec::<String>::new())
    }
}

impl FunctionRegistry {
    fn v1_catalog() -> BTreeMap<String, FunctionSchema> {
        commands::V1_FUNCTIONS
            .iter()
            .map(|def| {
                let args = def
                    .args
                    .iter()
                    .map(|arg| {
                        (
                            arg.name.to_string(),
                            FunctionArgSchema {
                                arg_type: arg.arg_type.into(),
                                required: arg.required,
                                description: arg.description.to_string(),
                            },
                        )
                    })
                    .collect();
                (
                    def.function_id.to_string(),
                    FunctionSchema {
                        function_id: def.function_id.to_string(),
                        description: def.description.to_string(),
                        args,
                        read_only: def.read_only,
                    },
                )
            })
            .collect()
    }

    fn normalize_allowlist<I>(
        function_ids: I,
        catalog: &BTreeMap<String, FunctionSchema>,
    ) -> BTreeSet<String>
    where
        I: IntoIterator<Item = String>,
    {
        function_ids
            .into_iter()
            .filter(|id| catalog.contains_key(id))
            .collect()
    }

    pub fn new<I>(function_ids: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let catalog = Self::v1_catalog();
        Self {
            allowed: Self::normalize_allowlist(function_ids, &catalog),
            catalog,
            run_store: None,
            settings: None,
        }
    }

    pub fn with_run_store<I>(function_ids: I, run_store: WorkflowRunStore) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let catalog = Self::v1_catalog();
        Self {
            allowed: Self::normalize_allowlist(function_ids, &catalog),
            catalog,
            run_store: Some(run_store),
            settings: None,
        }
    }

    pub fn with_context<I>(
        function_ids: I,
        run_store: Option<WorkflowRunStore>,
        settings: Option<Settings>,
    ) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let catalog = Self::v1_catalog();
        Self {
            allowed: Self::normalize_allowlist(function_ids, &catalog),
            catalog,
            run_store,
            settings,
        }
    }

    pub fn v1_defaults(run_store: WorkflowRunStore, settings: &Settings) -> Self {
        let catalog = Self::v1_catalog();
        let allowed = catalog.keys().cloned().collect();
        Self {
            allowed,
            catalog,
            run_store: Some(run_store),
            settings: Some(settings.clone()),
        }
    }

    pub fn contains(&self, function_id: &str) -> bool {
        self.allowed.contains(function_id)
    }

    pub fn available_function_ids(&self) -> Vec<String> {
        self.allowed.iter().cloned().collect()
    }

    pub fn available_function_schemas(&self) -> Vec<FunctionSchema> {
        self.allowed
            .iter()
            .filter_map(|id| self.catalog.get(id))
            .cloned()
            .collect()
    }

    fn validate_args(
        &self,
        call: &FunctionCall,
        schema: &FunctionSchema,
    ) -> Result<(), OrchestratorError> {
        for key in call.args.keys() {
            if !schema.args.contains_key(key) {
                return Err(OrchestratorError::UnknownFunctionArg {
                    function_id: call.function_id.clone(),
                    arg: key.clone(),
                });
            }
        }
        for (arg, arg_schema) in &schema.args {
            match call.args.get(arg) {
                Some(value) => {
                    if !arg_schema.arg_type.matches(value) {
                        return Err(OrchestratorError::InvalidFunctionArgType {
                            function_id: call.function_id.clone(),
                            arg: arg.clone(),
                            expected: arg_schema.arg_type.to_string(),
                        });
                    }
                }
                None if arg_schema.required => {
                    return Err(OrchestratorError::MissingFunctionArg { arg: arg.clone() });
                }
                None => {}
            }
        }
        Ok(())
    }

    pub fn invoke(&self, call: &FunctionCall) -> Result<Value, OrchestratorError> {
        if !self.contains(&call.function_id) {
            return Err(OrchestratorError::UnknownFunction {
                function_id: call.function_id.clone(),
            });
        }
        let schema = self.catalog.get(&call.function_id).ok_or_else(|| {
            OrchestratorError::UnknownFunction {
                function_id: call.function_id.clone(),
            }
        })?;
        self.validate_args(call, schema)?;

        commands::execute_function_invocation(
            &call.function_id,
            &call.args,
            commands::FunctionExecutionContext {
                run_store: self.run_store.as_ref(),
                settings: self.settings.as_ref(),
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResolutionInput {
    pub explicit_run_id: Option<String>,
    pub inbound_workflow_run_id: Option<String>,
    pub channel_profile_id: Option<String>,
    pub conversation_id: Option<String>,
}

pub fn resolve_status_run_id(
    input: &StatusResolutionInput,
    active_conversation_runs: &BTreeMap<(String, String), String>,
) -> Option<String> {
    if let Some(explicit) = input
        .explicit_run_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return Some(explicit.clone());
    }

    if let Some(inbound) = input
        .inbound_workflow_run_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return Some(inbound.clone());
    }

    let key = (
        input.channel_profile_id.as_ref()?.to_string(),
        input.conversation_id.as_ref()?.to_string(),
    );
    active_conversation_runs.get(&key).cloned()
}
