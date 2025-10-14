use crate::cli::TappletCommand;
use anyhow::Result;

mod fetch;

pub async fn tapplet_command_handler(tapplet_subcommand: TappletCommand) -> Result<()> {
    match tapplet_subcommand {
        TappletCommand::Fetch { cache_directory } => {
            // Logic to fetch tapplets
            println!("Fetching all tapplets from registries...");
            fetch::fetch(cache_directory.into()).await?;
        },
        TappletCommand::Search { query } => {
            // Logic to search tapplets
            println!("Searching for tapplets matching: {}", query);
        },
        TappletCommand::List => {
            // Logic to list tapplets
            println!("Listing all tapplets...");
        },
        TappletCommand::AddRegistry { name, url } => {
            // Logic to add a new tapplet
            println!("Adding tapplet registry: {} at {}", name, url);
        },
        TappletCommand::Install { .. } => {
            // Logic to remove a tapplet
            println!("Installing tapplet...");
        },
        TappletCommand::Uninstall { name } => {
            // Logic to remove a tapplet
            println!("Uninstalling tapplet: {}", name);
        },
    }
    Ok(())
}
