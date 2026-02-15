use crate::shared::ids::validate_identifier_value;
pub use crate::shared::ids::{AgentId, OrchestratorId, StepId, WorkflowId};
use crate::shared::serde_ext::parse_via_string;
use serde::de::Error as _;
use serde::ser::Serializer;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct WorkflowInputKey(String);

impl WorkflowInputKey {
    pub fn parse(raw: &str) -> Result<Self, String> {
        Ok(Self(normalize_workflow_input_key(raw)?))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkflowInputKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<'de> Deserialize<'de> for WorkflowInputKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        parse_via_string(deserializer, "workflow input key", Self::parse)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct WorkflowInputs(Vec<WorkflowInputKey>);

impl WorkflowInputs {
    pub fn parse_keys<I, S>(keys: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut values = Vec::new();
        let mut seen = HashSet::new();
        for raw in keys {
            let key = WorkflowInputKey::parse(raw.as_ref())?;
            if seen.insert(key.as_str().to_string()) {
                values.push(key);
            }
        }
        Ok(Self(values))
    }

    pub fn as_slice(&self) -> &[WorkflowInputKey] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for WorkflowInputs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match value {
            serde_yaml::Value::Null => Ok(Self::default()),
            serde_yaml::Value::String(raw) => Self::parse_keys([raw]).map_err(D::Error::custom),
            serde_yaml::Value::Sequence(values) => {
                let mut keys = Vec::new();
                for value in values {
                    let raw = value.as_str().ok_or_else(|| {
                        D::Error::custom("workflow inputs must be a sequence of string keys")
                    })?;
                    keys.push(raw.to_string());
                }
                Self::parse_keys(keys).map_err(D::Error::custom)
            }
            _ => Err(D::Error::custom(
                "workflow inputs must be a sequence of string keys",
            )),
        }
    }
}

pub fn normalize_workflow_input_key(raw: &str) -> Result<String, String> {
    let normalized = raw.trim();
    validate_identifier_value("workflow input key", normalized)?;
    Ok(normalized.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputKey {
    pub name: String,
    pub required: bool,
}

impl OutputKey {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("output key must be non-empty".to_string());
        }
        let (name, required) = if let Some(optional) = trimmed.strip_suffix('?') {
            (optional.trim(), false)
        } else {
            (trimmed, true)
        };
        if name.is_empty() {
            return Err("output key must be non-empty".to_string());
        }
        if name.contains('?') {
            return Err("output key may only contain optional marker as trailing `?`".to_string());
        }
        validate_identifier_value("output key", name)?;
        Ok(Self {
            name: name.to_string(),
            required,
        })
    }

    pub fn parse_output_file_key(raw: &str) -> Result<Self, String> {
        let parsed = Self::parse(raw)?;
        if !parsed.required {
            return Err(
                "output_files keys must not include optional marker `?`; declare optionality only in `outputs`"
                    .to_string(),
            );
        }
        Ok(parsed)
    }

    pub fn as_str(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Display for OutputKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.required {
            self.name.fmt(f)
        } else {
            write!(f, "{}?", self.name)
        }
    }
}

impl std::borrow::Borrow<str> for OutputKey {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Serialize for OutputKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for OutputKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        parse_via_string(deserializer, "output key", Self::parse)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PathTemplate(String);

impl PathTemplate {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let normalized = raw.trim();
        if normalized.is_empty() {
            return Err("path template must be non-empty".to_string());
        }
        Ok(Self(normalized.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PathTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<'de> Deserialize<'de> for PathTemplate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        parse_via_string(deserializer, "path template", Self::parse)
    }
}

pub type OutputContractKey = OutputKey;

pub fn parse_output_contract_key(raw: &str) -> Result<OutputContractKey, String> {
    OutputKey::parse(raw)
}
