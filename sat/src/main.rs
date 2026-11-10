use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use sandboxed_agent_tools::{home, metadata, runtime::Tool};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "sat",
    version,
    about = "Package and run sandboxed component tools",
    subcommand_required = true
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Embed metadata and optional permanent policy in a component.
    Pack {
        component: PathBuf,
        #[arg(long)]
        metadata: PathBuf,
        #[arg(long)]
        policy: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Trust and register a local package. Does not execute it.
    Install {
        component: PathBuf,
        #[arg(long)]
        replace: bool,
    },
    /// List installed tools.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Search installed tool names and descriptions.
    Search {
        query: String,
        #[arg(long)]
        json: bool,
    },
    /// Extract embedded metadata, policy, or WIT from an installed tool.
    Inspect {
        tool: String,
        #[arg(long, conflicts_with = "policy")]
        wit: bool,
        #[arg(long)]
        policy: bool,
    },
    /// Build and register an isolated WASI Python environment.
    PythonVenv {
        #[command(subcommand)]
        command: PythonVenvAction,
    },
    /// Open the localhost execution dashboard, or manage opt-in history.
    Dashboard {
        #[command(subcommand)]
        command: Option<DashboardAction>,
        /// Local port for the dashboard (0 chooses an available port).
        #[arg(long, default_value_t = 7331)]
        port: u16,
        /// Print the URL without opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    #[command(external_subcommand)]
    Run(Vec<String>),
}

#[derive(Subcommand)]
enum DashboardAction {
    /// Start recording tool invocations, including complete command arguments.
    Enable,
    /// Stop recording; keep existing history.
    Disable,
    /// Delete the history database; keep the current recording preference.
    Reset,
}

#[derive(Subcommand)]
enum PythonVenvAction {
    /// Install wheel-only Python requirements into a new WASI component.
    Install {
        /// Requirements, e.g. 'numpy==2.2.0' (repeatable).
        #[arg(required = true)]
        requirements: Vec<String>,
        /// Name under which SAT registers the resulting Python tool.
        #[arg(long)]
        name: String,
        /// Registry name from SAT_HOME/python-registries.toml.
        #[arg(long, default_value = "pypi")]
        registry: String,
        /// Replace an existing registration with this name.
        #[arg(long)]
        replace: bool,
    },
}

#[derive(Serialize, Deserialize)]
struct Registration {
    name: String,
    version: String,
    description: String,
    sha256: String,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn tool_dir(name: &str) -> Result<PathBuf> {
    ensure!(metadata::valid_name(name), "invalid tool name");
    Ok(home()?.join(name))
}
fn installed(name: &str) -> Result<Vec<u8>> {
    let dir = tool_dir(name)?;
    let record: Registration = serde_json::from_slice(
        &fs::read(dir.join("registration.json"))
            .with_context(|| format!("tool {name} is not installed"))?,
    )?;
    let bytes = fs::read(dir.join("tool.wasm"))?;
    ensure!(
        record.name == name && record.sha256 == digest(&bytes),
        "installed package integrity check failed for {name}"
    );
    ensure!(
        metadata::Package::read(&bytes)?.tool.name == name,
        "registered tool name mismatch"
    );
    Ok(bytes)
}
fn list(query: &str, json_output: bool) -> Result<()> {
    let root = home()?;
    let mut records = Vec::<Registration>::new();
    if root.exists() {
        for entry in fs::read_dir(root)? {
            let path = entry?.path().join("registration.json");
            if path.is_file() {
                let record: Registration = serde_json::from_slice(&fs::read(path)?)?;
                if format!("{} {}", record.name, record.description)
                    .to_lowercase()
                    .contains(&query.to_lowercase())
                {
                    records.push(record);
                }
            }
        }
    }
    records.sort_by(|a, b| a.name.cmp(&b.name));
    if json_output {
        println!("{}", serde_json::to_string_pretty(&records)?);
    } else {
        for r in records {
            println!("{} {}  {}", r.name, r.version, r.description);
        }
    }
    Ok(())
}

fn install_bytes(bytes: Vec<u8>, replace: bool) -> Result<()> {
    let tool = Tool::load(&bytes)?;
    let info = &tool.package.tool;
    let dir = tool_dir(&info.name)?;
    ensure!(
        !dir.exists() || replace,
        "already installed; use --replace to explicitly trust this replacement"
    );
    fs::create_dir_all(&dir)?;
    let record = Registration {
        name: info.name.clone(),
        version: info.version.clone(),
        description: info.description.clone(),
        sha256: digest(&bytes),
    };
    fs::write(dir.join("tool.wasm"), bytes)?;
    fs::write(
        dir.join("registration.json"),
        serde_json::to_vec_pretty(&record)?,
    )?;
    println!("Installed {} {}", info.name, info.version);
    Ok(())
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run_cli().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if let Some(status) =
                error.downcast_ref::<sandboxed_agent_tools::runtime::CommandExit>()
            {
                std::process::ExitCode::from(status.0)
            } else {
                eprintln!("Error: {error:?}");
                std::process::ExitCode::FAILURE
            }
        }
    }
}

async fn run_cli() -> Result<()> {
    match Cli::parse().command {
        Action::Pack {
            component,
            metadata,
            policy,
            output,
        } => {
            let text = fs::read_to_string(metadata)?;
            let policy = policy.map(fs::read_to_string).transpose()?;
            let bytes = metadata::embed(fs::read(component)?, &text, policy.as_deref())?;
            Tool::load(&bytes).context("invalid package")?;
            fs::write(&output, &bytes)?;
            println!("{}  sha256:{}", output.display(), digest(&bytes));
        }
        Action::Install { component, replace } => {
            install_bytes(fs::read(component)?, replace)?;
        }
        Action::List { json } => list("", json)?,
        Action::Search { query, json } => list(&query, json)?,
        Action::Inspect { tool, wit, policy } => {
            let bytes = installed(&tool)?;
            let package = metadata::Package::read(&bytes)?;
            if wit {
                print!("{}", metadata::wit(&bytes)?);
            } else if policy {
                print!(
                    "{}",
                    package
                        .policy_text
                        .as_deref()
                        .unwrap_or("# No embedded package restrictions.\n")
                );
            } else {
                print!("{}", package.tool_text);
            }
        }
        Action::PythonVenv { command } => match command {
            PythonVenvAction::Install {
                requirements,
                name,
                registry,
                replace,
            } => install_bytes(
                sandboxed_agent_tools::python_venv::build(&requirements, &name, &registry)?,
                replace,
            )?,
        },
        Action::Dashboard {
            command,
            port,
            no_open,
        } => {
            let root = home()?;
            match command {
                Some(DashboardAction::Enable) => {
                    sandboxed_agent_tools::history::set_enabled(&root, true)?;
                    println!(
                        "Execution history enabled. Complete arguments are stored locally in {}.",
                        root.join("executions.sqlite3").display()
                    );
                }
                Some(DashboardAction::Disable) => {
                    sandboxed_agent_tools::history::set_enabled(&root, false)?;
                    println!("Execution history disabled. Existing history is retained.");
                }
                Some(DashboardAction::Reset) => {
                    sandboxed_agent_tools::history::reset(&root)?;
                    println!("Execution history database deleted. Recording preference unchanged.");
                }
                None => sandboxed_agent_tools::dashboard::serve(&root, port, !no_open)?,
            }
        }
        Action::Run(args) => {
            let started = std::time::Instant::now();
            let invocation = match home().and_then(|root| {
                sandboxed_agent_tools::history::Invocation::begin(&root, &args[0], &args[1..])
            }) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("Could not record execution history: {error:#}");
                    None
                }
            };
            let result = async {
                let bytes = installed(&args[0])?;
                Tool::load(&bytes)?.run(&args[1..]).await
            }
            .await;
            if let Some(invocation) = invocation {
                let code = result.as_ref().err().map_or(0, |error| {
                    error
                        .downcast_ref::<sandboxed_agent_tools::runtime::CommandExit>()
                        .map_or(1, |code| code.0)
                });
                let message = result.as_ref().err().map(|error| format!("{error:#}"));
                if let Err(error) = invocation.finish(started.elapsed(), code, message.as_deref()) {
                    eprintln!("Could not finish execution history: {error:#}");
                }
            }
            result?;
        }
    }
    Ok(())
}
