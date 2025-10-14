use std::path::PathBuf;

use anyhow::Result;

pub async fn list_installed_tapplets(cache_directory: PathBuf) -> Result<Vec<String>> {
    let installed_dir = cache_directory.join("installed");

    if !installed_dir.exists() {
        println!("No installed tapplets found.");
        return Ok(vec![]);
    }

    let mut tapplets = Vec::new();

    // Read all directories in the installed folder
    let entries = std::fs::read_dir(&installed_dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Get the tapplet name from the directory name
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                tapplets.push(name.to_string());
            }
        }
    }

    Ok(tapplets)
}
