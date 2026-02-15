use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

pub fn parse_via_string<'de, D, T, F>(deserializer: D, kind: &str, parser: F) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    F: FnOnce(&str) -> Result<T, String>,
{
    let raw = String::deserialize(deserializer)?;
    parser(&raw).map_err(|err| D::Error::custom(format!("invalid {kind} `{raw}`: {err}")))
}
