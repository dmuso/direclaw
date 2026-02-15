use direclaw::shared::serde_ext::parse_via_string;
use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, PartialEq, Eq)]
struct NonEmpty(String);

impl NonEmpty {
    fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("must not be empty".to_string());
        }
        Ok(Self(trimmed.to_string()))
    }
}

impl<'de> Deserialize<'de> for NonEmpty {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        parse_via_string(deserializer, "non-empty value", Self::parse)
    }
}

#[test]
fn shared_serde_ext_module_parses_typed_value_from_string() {
    let parsed: NonEmpty = serde_yaml::from_str("\"  hello  \"").expect("parse non-empty");
    assert_eq!(parsed, NonEmpty("hello".to_string()));
}

#[test]
fn shared_serde_ext_module_reports_formatted_parse_error() {
    let err = serde_yaml::from_str::<NonEmpty>("\"   \"").expect_err("empty should fail");
    assert!(err.to_string().contains("invalid non-empty value"));
    assert!(err.to_string().contains("must not be empty"));
}
