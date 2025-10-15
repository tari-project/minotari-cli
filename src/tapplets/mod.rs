use std::collections::HashMap;

use crate::cli::TappletCommand;
use anyhow::Result;

mod default_registries;
mod fetch;
mod install;
mod list;
mod run;
mod search;

pub async fn tapplet_command_handler(tapplet_subcommand: TappletCommand) -> Result<()> {
    match tapplet_subcommand {
        TappletCommand::Fetch { cache_directory } => {
            println!("Fetching all tapplets from registries...");
            fetch::fetch(cache_directory.into()).await?;
        },
        TappletCommand::Search { query, cache_directory } => {
            let results = search::search_tapplets(&query, cache_directory.into()).await?;
            println!("Found {} tapplets matching '{}':", results.len(), query);
            for tapplet in results {
                println!("- {}: {}", tapplet.name, tapplet.description);
            }
        },
        TappletCommand::List { cache_directory } => {
            println!("Listing installed tapplets...");
            let tapplets = list::list_installed_tapplets(cache_directory.into()).await?;
            if tapplets.is_empty() {
                println!("No tapplets installed.");
            } else {
                println!("Installed tapplets ({}):", tapplets.len());
                for tapplet in tapplets {
                    println!("  - {}", tapplet);
                }
            }
        },
        TappletCommand::AddRegistry { name, url } => {
            // Logic to add a new tapplet
            println!("Adding tapplet registry: {} at {}", name, url);
        },
        TappletCommand::Install {
            registry,
            cache_directory,
            name,
            path,
        } => {
            if let Some(n) = name {
                println!("Installing tapplet...");
                install::install_from_git(registry, &n, cache_directory.into()).await?;
            } else {
                if let Some(p) = path {
                    println!("Installing tapplet from local path...");
                    install::install_from_local(p.into(), cache_directory.into()).await?;
                } else {
                    println!("Either name or path must be provided for installation.");
                    return Err(anyhow::anyhow!("Name or path required"));
                }
            }
        },
        TappletCommand::Uninstall { name } => {
            // Logic to remove a tapplet
            println!("Uninstalling tapplet: {}", name);
        },
        TappletCommand::Run { name, method, args } => {
            // Logic to run a tapplet
            println!("Running tapplet: {} with method: {} and args: {:?}", name, method, args);
            let mut args_map = HashMap::new();
            for arg in args {
                let parts: Vec<&str> = arg.splitn(2, '=').collect();
                if parts.len() == 2 {
                    args_map.insert(parts[0].to_string(), parts[1].to_string());
                } else {
                    println!("Ignoring invalid argument: {}", arg);
                }
            }
            run::run(&name, &method, args_map, "data/tapplet_cache".into()).await?;
        },
    }
    Ok(())
}
