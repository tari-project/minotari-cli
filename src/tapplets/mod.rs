use std::collections::HashMap;

use crate::cli::TappletCommand;
use anyhow::Result;

mod api;
mod default_registries;
mod fetch;
mod install;
mod list;
mod run;
mod search;
mod uninstall;

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
                println!(
                    "- {}: {}",
                    tapplet.name,
                    tapplet.description.as_ref().unwrap_or(&"No description".to_string())
                );
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
            database_file,
            account_name,
            password,
        } => {
            if let Some(n) = name {
                println!("Installing tapplet...");
                install::install_from_git(registry, &n, cache_directory.into(), account_name, &database_file).await?;
            } else {
                if let Some(p) = path {
                    println!("Installing tapplet from local path...");
                    install::install_from_local(
                        p.into(),
                        cache_directory.into(),
                        account_name,
                        &database_file,
                        &password,
                    )
                    .await?;
                } else {
                    println!("Either name or path must be provided for installation.");
                    return Err(anyhow::anyhow!("Name or path required"));
                }
            }
        },
        TappletCommand::Uninstall { name, cache_directory } => {
            println!("Uninstalling tapplet: {}", name);
            uninstall::uninstall_tapplet(&name, cache_directory.into()).await?;
        },
        TappletCommand::Run {
            account_name,
            name,
            method,
            args,
            cache_directory,
            database_file,
            password,
        } => {
            run::run_interactive(
                account_name,
                name,
                method,
                args,
                cache_directory,
                database_file,
                password,
            )
            .await?;
        },
    }
    Ok(())
}
