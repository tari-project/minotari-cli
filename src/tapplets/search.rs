use std::path::PathBuf;

use tari_tapplet_lib::{TappletManifest, TappletRegistry};

use crate::tapplets::default_registries::get_default_registries;

pub async fn search_tapplets(query: &str, cache_directory: PathBuf) -> Result<Vec<TappletManifest>, anyhow::Error> {
    println!("Searching for tapplets matching: {}", query);
    let mut results = Vec::new();
    for (reg_name, reg_url) in get_default_registries() {
        println!("Searching in registry: {} ({})", reg_name, reg_url);
        let mut tapplet_registry = TappletRegistry::new(reg_name, reg_url, cache_directory.clone());

        tapplet_registry.load().await?;
        let tapplets = tapplet_registry.search(query)?;
        results.extend(tapplets.into_iter().map(|t| t.clone()).collect::<Vec<_>>());
    }

    Ok(results)
}
