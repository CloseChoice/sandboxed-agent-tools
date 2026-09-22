use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use safe_agent_tools::{home, metadata, runtime::Tool};
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
    #[command(external_subcommand)]
    Run(Vec<String>),
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

#[tokio::main]
async fn main() -> Result<()> {
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
            let bytes = fs::read(component)?;
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
        Action::Run(args) => {
            let bytes = installed(&args[0])?;
            Tool::load(&bytes)?.run(&args[1..]).await?;
        }
    }
    Ok(())
}
