/// Code to parse cli inputs
///
/// The sat cli has the following concepts:
/// Basic sat commands
/// sat <command> (e.g. list, config, register, etc.)
/// And tool calling
/// sat <toolname> <toolarg1> <toolarg2> --<toolkwarg1> <toolkwargvalue1> ... --
/// --<read/write/allow-http/deny_http> <path or uri>
use crate::config::Settings;
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
    /// Create project configuration.
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
#[command(
    name = "sat permissions",
    about = "Tool permissions"
)]
struct Permissions {
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", value_name="PATH")]
    read: Option<Vec<String>>,
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", value_name="PATH")]
    write: Option<Vec<String>>,
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", value_name="GLOB")]
    deny_read: Option<Vec<String>>,
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", value_name="GLOB")]
    deny_write: Option<Vec<String>>,
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", conflicts_with="deny_http",value_name="HOSTNAME")]
    allow_http: Option<Vec<String>>,
    #[arg(long, action=ArgAction::Append, num_args=0..=1, default_missing_value="", conflicts_with="allow_http",value_name="HOSTNAME")]
    deny_http: Option<Vec<String>>,
}

pub fn invocation(args: &[String], settings: &mut Settings) -> Result<Vec<String>> {
    //
    // check if we have non-tool-specific parameters (e.g. read/write, etc.) after '--'
    let Some(separator) = args.iter().position(|a| a == "--") else {
        return Ok(args.to_vec());
    };
    let permissions = Permissions::try_parse_from(
        std::iter::once("permissions".into()).chain(args[separator + 1..].iter().cloned()),
    )?;

    fn clean(v: Vec<String>) -> Vec<String> {
        v.into_iter().filter(|s| !s.is_empty()).collect()
    }
    for (override_, target) in [
        (permissions.read, &mut settings.file_permissions.read),
        (permissions.write, &mut settings.file_permissions.write),
        (
            permissions.deny_read,
            &mut settings.file_permissions.deny_read,
        ),
        (
            permissions.deny_write,
            &mut settings.file_permissions.deny_write,
        ),
    ] {
        if let Some(v) = override_ {
            *target = clean(v);
        }
    }
    if let Some(v) = permissions.allow_http {
        settings.http_permissions.allow_http = Some(clean(v));
        settings.http_permissions.deny_http = None;
    }
    if let Some(v) = permissions.deny_http {
        settings.http_permissions.deny_http = Some(clean(v));
        settings.http_permissions.allow_http = None;
    }
    settings.http_permissions.validate()?;
    Ok(args[..separator].to_vec())
}


#[cfg(test)]
pub mod test {
    fn test_invocation() -> bool {
        // todo: generate string list in the fashion of that clippy generates
        //
        // invocation
        true
    }
}
