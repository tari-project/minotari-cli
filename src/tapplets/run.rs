use std::{collections::HashMap, path::PathBuf};

use serde_json::Value;
use tari_tapplet_lib::{LuaTappletHost, TappletConfig, WasmTappletHost, host::MinotariTappletApiV1};

use crate::tapplets::api::MinotariApiProvider;

pub async fn run_wasm(
    name: &str,
    method: &str,
    args: HashMap<String, String>,
    cache_directory: PathBuf,
) -> Result<(), anyhow::Error> {
    let installed_dir = cache_directory.join("installed");
    let tapplet_path = installed_dir.join(name);

    if !tapplet_path.exists() {
        println!("Tapplet '{}' is not installed.", name);
        return Err(anyhow::anyhow!("Tapplet not installed"));
    }

    // Load the tapplet configuration
    let config = tari_tapplet_lib::parse_tapplet_file(tapplet_path.join("manifest.toml"))?;
    let wasm_path = tapplet_path.join(&config.name).with_extension("wasm");

    let mut tapplet = WasmTappletHost::new(config, wasm_path)?;

    println!("Running method '{}' on tapplet '{}'", method, name);

    // Convert HashMap to JSON Value
    let args_json: Value = serde_json::to_value(&args)?;

    tapplet.run(method, args_json)?;

    Ok(())
}

pub async fn run_lua(
    database_file: &str,
    password: &str,
    name: &str,
    method: &str,
    args: HashMap<String, String>,
    cache_directory: PathBuf,
) -> Result<(), anyhow::Error> {
    let installed_dir = cache_directory.join("installed");
    let tapplet_path = installed_dir.join(name);

    if !tapplet_path.exists() {
        println!("Tapplet '{}' is not installed.", name);
        return Err(anyhow::anyhow!("Tapplet not installed"));
    }
    let config = tari_tapplet_lib::parse_tapplet_file(tapplet_path.join("manifest.toml"))?;

    let api = MinotariApiProvider::try_create("default".to_string(), &config, database_file, password).await?;

    // Load the tapplet configuration
    let config = tari_tapplet_lib::parse_tapplet_file(tapplet_path.join("manifest.toml"))?;
    let lua_path = tapplet_path.join(&config.name).with_extension("lua");

    let mut tapplet = LuaTappletHost::new(config, lua_path, api)?;

    println!("Running method '{}' on tapplet '{}'", method, name);

    // Convert HashMap to JSON Value
    let args_json: Value = serde_json::to_value(&args)?;

    let result = tapplet.run(method, args_json)?;
    dbg!("Lua tapplet result: {:?}", result);

    Ok(())
}
