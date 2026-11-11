use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub version: String,
    pub description: String,
    pub entrypoint: String,
    #[serde(default)]
    pub arguments: Vec<Argument>,
    #[serde(default)]
    pub options: BTreeMap<String, OptionSpec>,
    #[serde(default)]
    pub examples: Vec<Example>,
    #[serde(default)]
    pub exactly_one_of: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Argument {
    pub name: String,
    pub help: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionSpec {
    pub short: Option<char>,
    pub help: String,
    pub default: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub command: String,
}

pub const RESERVED: &[&str] = &[
    "help",
    "init",
    "list",
    "register",
    "remove",
    "unregister",
    "config",
];

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

impl Tool {
    pub fn parse(text: &str) -> Result<Self> {
        let tool: Self = toml::from_str(text)?;
        ensure!(
            valid_name(&tool.name) && !RESERVED.contains(&tool.name.as_str()),
            "invalid or reserved tool name"
        );
        Ok(tool)
    }
}
