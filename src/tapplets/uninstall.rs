use std::path::PathBuf;
use anyhow::Result;

/// Uninstall a tapplet by removing its directory from the cache.
/// This does NOT delete any data stored in the database.
pub async fn uninstall_tapplet(name: &str, cache_directory: PathBuf) -> Result<()> {
    let installed_dir = cache_directory.join("installed");
    let tapplet_path = installed_dir.join(name);

    if !tapplet_path.exists() {
        println!("Tapplet '{}' is not installed.", name);
        return Err(anyhow::anyhow!("Tapplet '{}' not found", name));
    }

    // Confirm it's a directory
    if !tapplet_path.is_dir() {
        println!("Error: '{}' exists but is not a directory.", name);
        return Err(anyhow::anyhow!("Invalid tapplet path"));
    }

    // Remove the tapplet directory
    println!("Removing tapplet directory: {}", tapplet_path.display());
    std::fs::remove_dir_all(&tapplet_path)?;

    println!("✓ Successfully uninstalled tapplet '{}'", name);
    println!("Note: Tapplet data in the database has been preserved.");

    Ok(())
}
