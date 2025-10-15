use std::{collections::HashMap, path::PathBuf};

use serde_json::Value;
use tari_tapplet_lib::{LuaTappletHost, TappletConfig, WasmTappletHost, host::MinotariTappletApiV1};

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

#[derive(Clone)]
struct MinotariApiProvider {}

impl MinotariTappletApiV1 for MinotariApiProvider {
    fn append_data(&self, slot: &str, value: &str) -> Result<(), anyhow::Error> {
        println!("Appending data to slot '{}': {}", slot, value);
        Ok(())
    }

    fn load_data_entries(&self, slot: &str) -> Result<Vec<String>, anyhow::Error> {
        println!("Loading data entries from slot '{}'", slot);
        Ok(vec!["example_entry_1".to_string(), "example_entry_2".to_string()])
    }
}

pub async fn run_lua(
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

    let api = MinotariApiProvider {};
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
