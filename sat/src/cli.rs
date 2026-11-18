/// Code to parse cli inputs
///
/// The sat cli has the following concepts:
/// Basic sat commands
/// sat <command> (e.g. list, config, register, etc.)
/// And tool calling
/// sat <toolname> <toolarg1> <toolarg2> --<toolkwarg1> <toolkwargvalue1> ... --
/// --<read/write/allow-http/deny_http> <path or uri>
use crate::config::ToolSettings;
use anyhow::Result;
use clap::{ArgAction, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "sat",
    version,
    about = "Run tools with explicit sandbox permissions"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Action,
}

#[derive(Subcommand)]
pub enum Action {
    /// Run dedicated Chrome with a fresh temporary profile and a bounded optional duration.
    ChromeSession {
        /// Path to Chrome; otherwise search standard installation locations.
        #[arg(long)]
        chrome: Option<std::path::PathBuf>,
        /// Display the browser window (headless by default).
        #[arg(long)]
        headed: bool,
        /// Keep the session alive for this many seconds, even if stdin closes (for agents).
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=3600))]
        duration: Option<u64>,
    },
    /// Snapshot effective permissions for all registered tools into project configuration.
    Init {
        #[arg(short = 'f', long)]
        force: bool,
    },
    /// List registered tools.
    List,
    /// Register a local or remote .tar/.zip archive, reviewing its permissions.
    Register {
        source: String,
        #[arg(short = 'f', long)]
        force: bool,
    },
    /// Delete installed files and settings in global and current-project config.
    #[command(alias = "remove")]
    Unregister { tool: String },
    /// Show effective settings.
    Config { tool: Option<String> },
    #[command(external_subcommand)]
    Run(Vec<String>),
}

#[derive(Parser, Default)]
#[command(name = "sat permissions", about = "Tool permissions")]
struct Permissions {
    /// Exact loopback CDP WebSocket endpoint; grants control of its Chrome instance.
    #[arg(long, value_name = "WEBSOCKET_URL")]
    allow_cdp: Option<String>,
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", value_name="PATH")]
    read: Option<Vec<String>>,
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", value_name="PATH")]
    write: Option<Vec<String>>,
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", conflicts_with="deny_http",value_name="HOSTNAME")]
    allow_http: Option<Vec<String>>,
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", conflicts_with="allow_http",value_name="HOSTNAME")]
    deny_http: Option<Vec<String>>,
}

pub fn apply_permission_overrides(
    args: &[String],
    settings: &mut ToolSettings,
) -> Result<Vec<String>> {
    //
    // check if we have non-tool-specific parameters (e.g. read/write, etc.) after '--'
    let Some(separator) = args.iter().rposition(|a| a == "--").filter(|i| {
        args.get(i + 1).is_some_and(|next| {
            [
                "--read",
                "--write",
                // Reserve the retired flags so clap rejects them even in first position.
                "--deny-read",
                "--deny-write",
                "--allow-http",
                "--deny-http",
                "--allow-cdp",
            ]
            .iter()
            .any(|flag| next == flag || next.starts_with(&format!("{flag}=")))
        })
    }) else {
        return Ok(args.to_vec());
    };
    let permissions = Permissions::try_parse_from(
        std::iter::once("permissions".into()).chain(args[separator + 1..].iter().cloned()),
    )?;
    if let Some(endpoint) = permissions.allow_cdp {
        crate::browser::validate_endpoint(&endpoint)?;
        settings.cdp_endpoint = Some(endpoint);
    }

    fn remove_empty_permission_values(v: Vec<String>) -> Vec<String> {
        v.into_iter().filter(|s| !s.is_empty()).collect()
    }
    let override_read = permissions.read.is_some();
    for (override_, target) in [
        (permissions.read, &mut settings.file_permissions.read),
        (permissions.write, &mut settings.file_permissions.write),
    ] {
        if let Some(v) = override_ {
            settings.replaced_grants.append(target);
            *target = remove_empty_permission_values(v);
        }
    }
    if override_read {
        let (patterns, paths) = std::mem::take(&mut settings.file_permissions.read)
            .into_iter()
            .partition(|value| crate::access::is_read_glob(std::path::Path::new(value)));
        settings.read_globs = patterns;
        settings.file_permissions.read = paths;
    }
    if let Some(v) = permissions.allow_http {
        settings.http_permissions.allow_http = Some(remove_empty_permission_values(v));
        settings.http_permissions.deny_http = None;
    }
    if let Some(v) = permissions.deny_http {
        settings.http_permissions.deny_http = Some(remove_empty_permission_values(v));
        settings.http_permissions.allow_http = None;
    }
    settings.http_permissions.validate()?;
    Ok(args[..separator].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_exclusions_are_configuration_only() {
        let mut settings = ToolSettings::default();
        settings.file_permissions.deny_read.push("**/.env".into());
        settings
            .file_permissions
            .deny_write
            .push("**/*.lock".into());
        for flag in ["--deny-read", "--deny-write"] {
            for args in [vec!["--", flag], vec!["--", "--read", "/file", flag, "*"]] {
                let args = args.into_iter().map(String::from).collect::<Vec<_>>();
                assert!(apply_permission_overrides(&args, &mut settings).is_err());
            }
        }
        assert_eq!(settings.file_permissions.deny_read, ["**/.env"]);
        assert_eq!(settings.file_permissions.deny_write, ["**/*.lock"]);
    }

    #[test]
    fn preserves_upstream_separator_and_extracts_only_host_permissions() {
        let args: Vec<String> = ["-r", ".name", "--", "-input.json"]
            .map(str::to_owned)
            .into();
        let mut settings = ToolSettings::default();
        assert_eq!(
            apply_permission_overrides(&args, &mut settings).unwrap(),
            args
        );
        let with_grants: Vec<String> = args
            .iter()
            .cloned()
            .chain(["--", "--read", "/tmp/input.json"].map(str::to_owned))
            .collect();
        assert_eq!(
            apply_permission_overrides(&with_grants, &mut settings).unwrap(),
            args
        );
        assert_eq!(settings.file_permissions.read, ["/tmp/input.json"]);
    }
}
