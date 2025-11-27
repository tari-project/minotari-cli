use std::{collections::HashMap, path::PathBuf};

use serde_json::Value;
use tari_tapplet_lib::{LuaTappletHost, TappletManifest, WasmTappletHost, host::MinotariTappletApiV1};

use crate::tapplets::api::MinotariApiProvider;

fn print_value_as_table(value: &Value, indent: usize) {
    let prefix = "  ".repeat(indent);

    match value {
        Value::Object(map) => {
            for (key, val) in map {
                match val {
                    Value::Object(_) | Value::Array(_) => {
                        println!("{}{}", prefix, key);
                        print_value_as_table(val, indent + 1);
                    },
                    _ => {
                        println!("{}{}  {}", prefix, key, format_value(val));
                    },
                }
            }
        },
        Value::Array(arr) => {
            for (idx, val) in arr.iter().enumerate() {
                match val {
                    Value::Object(_) | Value::Array(_) => {
                        println!("{}[{}]", prefix, idx);
                        print_value_as_table(val, indent + 1);
                    },
                    _ => {
                        println!("{}[{}]  {}", prefix, idx, format_value(val));
                    },
                }
            }
        },
        _ => {
            println!("{}{}", prefix, format_value(value));
        },
    }
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

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
    account_name: &str,
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

    let api = MinotariApiProvider::try_create(
        account_name.to_string(),
        &config,
        database_file.into(),
        password.to_string(),
    )
    .await?;

    // Load the tapplet configuration
    let config = tari_tapplet_lib::parse_tapplet_file(tapplet_path.join("manifest.toml"))?;
    let lua_path = tapplet_path.join(&config.name).with_extension("lua");

    let mut tapplet = LuaTappletHost::new(config, lua_path, api)?;

    println!("Running method '{}' on tapplet '{}'", method, name);

    // Convert HashMap to JSON Value
    let args_json: Value = serde_json::to_value(&args)?;

    let result = tapplet.run(method, args_json).await?;

    println!("\nResult:");
    print_value_as_table(&result, 0);

    Ok(())
}
