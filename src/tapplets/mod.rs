use crate::cli::TappletCommand;
use anyhow::Result;

mod default_registries;
mod fetch;
mod install;
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
        TappletCommand::List => {
            // Logic to list tapplets
            println!("Listing all tapplets...");
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
    }
    Ok(())
}
