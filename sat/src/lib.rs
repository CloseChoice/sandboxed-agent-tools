//! Shared command-line interface for the native SAT host.
//!
//! The host selects a registered tool; each tool interprets its own arguments.
//! Preserve arguments in order instead of guessing option types or converting
//! them to a map: tools may accept flags, repeated options, and positional values.
//! Parsing arguments grants no filesystem or network authority. The runtime must
//! enforce host policy when component execution is implemented.

use std::ffi::OsString;

use clap::Parser;

pub mod runtime;

/// Parse a tool selection and the arguments to pass to that tool.
///
/// ```
/// use clap::Parser;
/// use safe_agent_tools::Args;
///
/// let args = Args::try_parse_from([
///     "sat", "--tool", "sometoolname", "-tool-option1", "adsfasdf",
/// ]).unwrap();
/// assert_eq!(args.tool, "sometoolname");
/// assert_eq!(args.tool_args, ["-tool-option1", "adsfasdf"]);
/// ```
#[derive(Parser, Debug)]
#[command(
    name = "sat",
    version,
    about = "Safe Agent Toolbox",
    long_about = None,
    after_help = "Use -- before tool arguments that overlap with host options: sat --tool NAME -- --help"
)]
pub struct Args {
    /// Registered tool to select
    #[arg(long, value_name = "NAME")]
    pub tool: String,

    /// Arguments forwarded to the tool without interpreting its options
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, value_name = "TOOL_ARGS")]
    pub tool_args: Vec<OsString>,
}
