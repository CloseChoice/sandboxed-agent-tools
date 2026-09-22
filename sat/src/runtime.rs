// In this file we want to build the functionality to execute other modules via wasmtime
// We should be able to check if a tool is registered, verify a tool via it's wit file and execute
// it
// We'll start with checking if registered, executing and then verifying
// we'll assume that a tool is available as a wasm file and a wit file and is stored in
// WAT_HOME/<tool_name>
use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder};


/// WASI execution target. P2 is the default for component-based tools.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WasiVersion {
    P1,
    #[default]
    P2,
}

/// Filesystem access allowed under a directory grant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DirectoryAccess {
    #[default]
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryGrant {
    pub host_path: PathBuf,
    pub guest_path: String,
    pub access: DirectoryAccess,
}

/// An outbound destination approved by host policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NetworkPolicy {
    #[default]
    Deny,
    /// The runtime must enforce destination restrictions, including redirects.
    /// An empty list permits no destinations.
    AllowEndpoints(Vec<NetworkEndpoint>),
}

/// WASI settings approved by host policy for one invocation.
///
/// Defaults select P2, closed stdin, discarded stdout/stderr, no environment
/// variables or directory grants, and denied network access. Clock and random
/// services are not configured by this type.
///
/// Constructing this value does not enforce permissions. The runtime must check
/// requests against host policy and apply the approved settings to WASI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WasiConfig {
    pub wasi_version: WasiVersion,
    pub inherit_stdin: bool,
    pub inherit_stdout: bool,
    pub inherit_stderr: bool,
    /// Explicit guest environment variables; do not inherit the host environment.
    pub env: Vec<(String, String)>,
    pub directories: Vec<DirectoryGrant>,
    pub network: NetworkPolicy,
}

pub struct SATRuntime {}

impl SATRuntime {
    pub fn new() -> Self {
        return SATRuntime {  };
    }
    /// Check whether a tool directory exists within WAT_HOME.
    /// This does not validate its WASM/WIT files or grant execution authority.
    pub fn verify_path(tool_name: String) -> io::Result<bool> {
        let home = env::var_os("WAT_HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "WAT_HOME is not set or is empty"))?;

        // A registered tool name must be one portable directory name.
        let mut components = Path::new(&tool_name).components();
        if tool_name.contains(['/', '\\', ':'])
            || !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tool name must be a single directory name",
            ));
        }

        let home = Path::new(&home).canonicalize()?;
        let tool_path = match home.join(tool_name).canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if tool_path == home || !tool_path.starts_with(&home) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "tool directory must remain inside WAT_HOME",
            ));
        }

        Ok(tool_path.metadata()?.is_dir())
    }

    /// Execute a tool with approved capabilities and ordered arguments.
    ///
    /// Preserve each argument separately: `--pattern 'fn run'` becomes
    /// two arguments, `--pattern` and `fn run`, without shell quotes.
    /// The configuration selects the WASI version and approved host access.
    /// Execution and capability enforcement are not implemented yet.
    /// todo: -p should be recognized as well
    pub fn execute(
        tool_name: &str,
        config: &WasiConfig,
        tool_args: &[OsString],
    ) -> io::Result<bool> {
        let mut builder = WasiCtxBuilder::new();
        // todo: fix this parsing logic here!
        let args: Vec<&str> = tool_args.map(|arg| arg.to_str().ok_or_else(|| {io::Error("some error")})).collect::<io::Result<_>>()?;
        bulider.arg(tool_name);
        builder.args(args);
        let wasi = builder.build();
        // todo: how to run this?

        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "tool execution is not implemented",
        ))
    }
}
