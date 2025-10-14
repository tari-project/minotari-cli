use std::path::PathBuf;

use tari_tapplet_lib::registry::TappletRegistry;

pub async fn fetch(cache_directory: PathBuf) -> Result<(), anyhow::Error> {
    println!("Fetching tapplets from all registries...");

    let default_registries = vec![(
        "Tari Official",
        "https://github.com/tari-project/tapplet-registry/raw/main/registry.json",
    )];

    for (name, url) in default_registries {
        println!("Fetching from registry: {} ({})", name, url);

        let mut registry = TappletRegistry::new(name, url, cache_directory.clone());
        registry.fetch().await?;

        println!("Registy is not at revision:{:?}", registry.revision());
        // let tapplets = registry.get_tapplets().await?;
        // println!("Found {} tapplets in registry '{}':", tapplets.len(), name);
        // for tapplet in tapplets {
        //     println!("- {}: {}", tapplet.name, tapplet.description);
        // }
    }

    Ok(())
}
