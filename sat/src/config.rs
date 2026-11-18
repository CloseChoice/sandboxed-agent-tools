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
    /// Per-invocation read patterns and original roots for grant-relative exclusions.
    #[serde(skip)]
    pub read_globs: Vec<String>,
    #[serde(skip)]
    pub replaced_grants: Vec<String>,
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
            read_globs: vec![],
            replaced_grants: vec![],
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

/// Snapshot the currently effective settings into the project configuration.
/// Existing project restrictions win over global settings, including with force.
pub fn initialize_project(home: &Path, cwd: &Path, force: bool) -> Result<PathBuf> {
    let path = project_config(cwd);
    ensure!(
        force || !path.exists(),
        "project config already exists; use --force (-f) to overwrite"
    );
    let mut config = Configuration::effective(home, cwd)?;
    for name in crate::registry::registered_tool_names(home)? {
        // Package suggestions are untrusted; an unconfigured tool gets no authority.
        config.tools.entry(name).or_default();
    }
    save(&path, &config)?;
    Ok(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn register_fixture(home: &Path, name: &str) {
        let directory = home.join("tools").join(name);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("tool.toml"), format!("name = '{name}'")).unwrap();
        // Init must not treat package suggestions as approved authority.
        std::fs::write(
            directory.join("defaults.toml"),
            "[file_permissions]\nread = ['$PWD']\nwrite = ['$PWD']\n",
        )
        .unwrap();
    }

    fn serialized(config: &Configuration) -> String {
        toml::to_string(config).unwrap()
    }

    #[test]
    fn init_copies_global_settings_and_explicitly_denies_unconfigured_tools() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        register_fixture(&home, "python");
        register_fixture(&home, "jaq");
        let settings = ToolSettings {
            alias: Some("py".into()),
            file_permissions: FilePermissions {
                read: vec!["$PWD/inputs".into()],
                write: vec!["$PWD/output".into()],
                deny_read: vec!["**/secret*".into()],
                deny_write: vec!["**/protected*".into()],
            },
            http_permissions: HttpPermissions {
                allow_http: Some(vec!["api.example.com".into()]),
                deny_http: None,
            },
            ..Default::default()
        };
        let mut global = Configuration::default();
        global.tools.insert("python".into(), settings);
        save(&home.join("config.toml"), &global).unwrap();
        let global_before = std::fs::read(home.join("config.toml")).unwrap();

        let path = initialize_project(&home, &cwd, false).unwrap();
        let snapshot = Configuration::read(&path).unwrap();
        let mut expected = global;
        expected.tools.insert("jaq".into(), ToolSettings::default());
        assert_eq!(serialized(&snapshot), serialized(&expected));
        assert_eq!(
            snapshot.tools["python"].file_permissions.write,
            ["$PWD/output"]
        );
        assert!(snapshot.tools["jaq"].file_permissions.write.is_empty());
        assert_eq!(
            std::fs::read(home.join("config.toml")).unwrap(),
            global_before
        );
        assert_eq!(
            serialized(&Configuration::effective(&home, &cwd).unwrap()),
            serialized(&expected)
        );
    }

    #[test]
    fn init_refuses_overwrite_and_force_preserves_project_restrictions() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("project");
        register_fixture(&home, "python");
        register_fixture(&home, "jaq");
        let mut broad = ToolSettings::default();
        broad.file_permissions.write.push("$PWD".into());
        let mut global = Configuration::default();
        global.tools.insert("python".into(), broad);
        global.tools.insert("jaq".into(), ToolSettings::default());
        save(&home.join("config.toml"), &global).unwrap();
        let mut local = Configuration::default();
        local.tools.insert("python".into(), ToolSettings::default());
        let path = project_config(&cwd);
        save(&path, &local).unwrap();
        let original = std::fs::read(&path).unwrap();

        assert!(initialize_project(&home, &cwd, false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        initialize_project(&home, &cwd, true).unwrap();
        let snapshot = Configuration::read(&path).unwrap();
        assert!(snapshot.tools["python"].file_permissions.write.is_empty());
        assert!(snapshot.tools.contains_key("jaq"));
    }

    #[test]
    fn invalid_global_settings_do_not_create_a_project_config() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("project");
        register_fixture(&home, "python");
        let mut settings = ToolSettings::default();
        settings.file_permissions.deny_read.push("[".into());
        let mut global = Configuration::default();
        global.tools.insert("python".into(), settings);
        save(&home.join("config.toml"), &global).unwrap();
        assert!(initialize_project(&home, &cwd, false).is_err());
        assert!(!project_config(&cwd).exists());
    }
}
