use std::path::PathBuf;

use tari_common::configuration::bootstrap::prompt;
use tari_tapplet_lib::{TappletRegistry, git_tapplet::GitTapplet, local_folder_tapplet::LocalFolderTapplet};

pub async fn install_from_git(
    registry: Option<String>,
    name: &str,
    cache_directory: PathBuf,
) -> Result<(), anyhow::Error> {
    // Placeholder for install logic
    println!("Install function called");
    if let Some(reg) = registry {
        println!("Installing from registry: {}", reg);
    } else {
        println!("No registry specified, using default.");
        let default_registries = crate::tapplets::default_registries::get_default_registries();
        let mut install_candidates = vec![];
        for (reg_name, url) in default_registries {
            println!("Installing from default registry: {} at {}", reg_name, url);
            let mut tapplet_registry = TappletRegistry::new(reg_name, url, cache_directory.clone());
            tapplet_registry.load().await?;
            let tapplets = tapplet_registry
                .tapplets
                .iter()
                .filter(|t| t.name == name)
                .map(|t| (t.clone(), reg_name.clone()))
                .collect::<Vec<_>>();
            install_candidates.extend(tapplets);
        }

        if install_candidates.is_empty() {
            println!("No tapplet named '{}' found in default registries.", name);
            return Err(anyhow::anyhow!("Tapplet not found"));
        } else if install_candidates.len() > 1 {
            println!(
                "Multiple tapplets named '{}' found in default registries. Please specify a registry.",
                name
            );
            for (tapplet_config, registry_name) in install_candidates {
                println!("- {} from {}", tapplet_config.name, registry_name);
            }
            return Err(anyhow::anyhow!("Multiple tapplets found"));
        } else {
            let (tapplet_config, registry_name) = &install_candidates[0];
            println!("Installing tapplet: {} from {}", tapplet_config.name, registry_name);

            if prompt("Are you sure you want to install") {
                let tapplet = GitTapplet::new(tapplet_config.clone());
                tapplet.install(cache_directory.join("installed"))?;
            } else {
                println!("Installation cancelled.");
                return Ok(());
            }
        }
    }
    Ok(())
}

pub async fn install_from_local(path: PathBuf, cache_directory: PathBuf) -> Result<(), anyhow::Error> {
    // Placeholder for install logic
    println!("Install from local function called");
    println!("Installing tapplet from local path: {:?}", path);
    let tapplet = LocalFolderTapplet::load(path)?;
    tapplet.install(cache_directory.join("installed"))?;
    Ok(())
}
