//! Host-owned permission settings. Missing tools get no authority.

use crate::metadata::{RESERVED, valid_name};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    #[serde(default)]
    pub tools: BTreeMap<String, ToolSettings>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSettings {
    /// Explicit authority over a debugging endpoint, independent of HTTP grants.
    #[serde(
        default,
        rename = "cdp-endpoint",
        skip_serializing_if = "Option::is_none"
    )]
    pub cdp_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub file_permissions: FilePermissions,
    pub http_permissions: HttpPermissions,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct FilePermissions {
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub deny_read: Vec<String>,
    pub deny_write: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct HttpPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_http: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny_http: Option<Vec<String>>,
}

impl Default for HttpPermissions {
    fn default() -> Self {
        Self {
            allow_http: Some(vec![]),
            deny_http: None,
        }
    }
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            cdp_endpoint: None,
            alias: None,
            file_permissions: Default::default(),
            http_permissions: Default::default(),
        }
    }
}

impl ToolSettings {
    pub fn validate(&self, home: &Path, cwd: &Path) -> Result<()> {
        if let Some(endpoint) = &self.cdp_endpoint {
            crate::browser::validate_endpoint(endpoint)?;
        }
        if let Some(alias) = &self.alias {
            ensure!(
                valid_name(alias) && !RESERVED.contains(&alias.as_str()),
                "invalid or reserved alias {alias}"
            );
        }
        self.http_permissions.validate()?;
        crate::access::Access::new(&self.file_permissions, home, cwd)?;
        Ok(())
    }
}

impl Configuration {
    pub fn read(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("invalid configuration {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn effective(home: &Path, cwd: &Path) -> Result<Self> {
        // effective configuration, resolved from project and home sat directories
        let mut global = Self::read(&home.join("config.toml"))?;
        let local = Self::read(&project_config(cwd))?;
        global.tools.extend(local.tools);
        let names = crate::registry::registered_tool_names(home)?;
        global.check_aliases(&names)?;
        for (name, settings) in &global.tools {
            if let Some(endpoint) = &settings.cdp_endpoint {
                crate::browser::validate_endpoint(endpoint)?;
            }
            ensure!(
                valid_name(name) && !RESERVED.contains(&name.as_str()),
                "invalid tool name {name}"
            );
            settings.http_permissions.validate()?;
            crate::access::match_patterns(&settings.file_permissions.deny_read)?;
            crate::access::match_patterns(&settings.file_permissions.deny_write)?;
        }
        Ok(global)
    }

    pub fn check_aliases(&self, names: &[String]) -> Result<()> {
        let mut aliases = std::collections::BTreeSet::new();
        for (name, settings) in &self.tools {
            if let Some(alias) = &settings.alias {
                ensure!(
                    valid_name(alias)
                        && !RESERVED.contains(&alias.as_str())
                        && !names.contains(alias)
                        && !self.tools.contains_key(alias)
                        && aliases.insert(alias),
                    "duplicate, invalid, or conflicting alias {alias} for {name}"
                );
            }
        }
        Ok(())
    }

    pub fn resolve(&self, name: &str) -> String {
        self.tools
            .iter()
            .find(|(_, s)| s.alias.as_deref() == Some(name))
            .map(|(n, _)| n.as_str())
            .unwrap_or(name)
            .to_owned()
    }
}

pub fn project_config(cwd: &Path) -> PathBuf {
    cwd.join(".sat/config.overrides.toml")
}

pub fn home() -> Result<PathBuf> {
    let path = std::env::var_os("SAT_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|p| PathBuf::from(p).join(".sat"))
        })
        .context("SAT_HOME is not set and under HOME no .sat directory was found.")?;
    Ok(std::path::absolute(path)?)
}

/// Write beside the destination and rename, so interrupted writes cannot truncate config.
pub fn save(path: &Path, config: &Configuration) -> Result<()> {
    let parent = path.parent().context("configuration has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    file.write_all(toml::to_string_pretty(config)?.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
