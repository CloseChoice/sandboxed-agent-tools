use anyhow::{Result, ensure};
use clap::Parser;
// todo: this is not safe agent tools ->
use sandboxed_agent_tools::{
    cli::{self, Action, Cli},
    config::{self, Configuration},
    registry,
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let home = config::home()?;
    let cwd = std::env::current_dir()?;
    match cli.command {
        Action::Init { force } => {
            let path = config::project_config(&cwd);
            ensure!(
                force || !path.exists(),
                "project config already exists; use --force (-f) to overwrite"
            );
            config::save(&path, &Configuration::default())?;
            println!("Created {}", path.display());
        }
        Action::Register { source, force } => {
            registry::register(
                &source,
                force,
                &home,
                &cwd,
                &mut std::io::stdin().lock(),
                &mut std::io::stdout().lock(),
            )
            .await?
        }
        Action::List => {
            for name in registry::registered_tool_names(&home)? {
                println!("{name}");
            }
        }
        Action::Unregister { tool } => {
            let config = Configuration::effective(&home, &cwd)?;
            registry::unregister(&config.resolve(&tool), &home, &cwd)?;
            println!("Unregistered {tool}");
        }
        Action::Config { tool } => {
            let config = Configuration::effective(&home, &cwd)?;
            if let Some(tool) = tool {
                let name = config.resolve(&tool);
                println!(
                    "{}",
                    toml::to_string_pretty(&config.tools.get(&name).cloned().unwrap_or_default())?
                );
            } else {
                println!("{}", toml::to_string_pretty(&config)?);
            }
        }
        Action::Run(args) => {
            let config = Configuration::effective(&home, &cwd)?;
            let name = config.resolve(&args[0]);
            let mut settings = config.tools.get(&name).cloned().unwrap_or_default();
            let tool_args = cli::apply_permission_overrides(&args[1..], &mut settings)?;
            registry::load(&home, &name)?
                .run(&tool_args, &settings, &home, &cwd)
                .await?;
        }
    }
    Ok(())
}
