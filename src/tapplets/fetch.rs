use std::path::PathBuf;

use tari_tapplet_lib::registry::TappletRegistry;

use crate::tapplets::default_registries::get_default_registries;

pub async fn fetch(cache_directory: PathBuf) -> Result<(), anyhow::Error> {
    println!("Fetching tapplets from all registries...");

    let default_registries = get_default_registries();

    for (name, url) in default_registries {
        let mut registry = TappletRegistry::new(name, url, cache_directory.clone());
        registry.fetch().await?;

        println!("Registy is now at revision:{:?}", registry.revision());
        // let tapplets = registry.get_tapplets().await?;
        // println!("Found {} tapplets in registry '{}':", tapplets.len(), name);
        // for tapplet in tapplets {
        //     println!("- {}: {}", tapplet.name, tapplet.description);
        // }
    }

    Ok(())
}
