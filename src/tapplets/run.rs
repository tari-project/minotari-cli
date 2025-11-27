use std::{collections::HashMap, path::PathBuf};

use dialoguer::{Input, Select};
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

pub async fn run_interactive(
    account_name: String,
    name: String,
    method: Option<String>,
    args: Vec<String>,
    cache_directory: String,
    database_file: String,
    password: String,
) -> Result<(), anyhow::Error> {
    let installed_dir = PathBuf::from(&cache_directory).join("installed");
    let tapplet_path = installed_dir.join(&name);

    if !tapplet_path.exists() {
        println!("Tapplet '{}' is not installed.", name);
        return Err(anyhow::anyhow!("Tapplet not installed"));
    }

    // Load the tapplet configuration to get available methods
    let config = tari_tapplet_lib::parse_tapplet_file(tapplet_path.join("manifest.toml"))?;

    // Determine the method to run
    let selected_method = if let Some(m) = method {
        // Method was provided via CLI
        if !config.api.methods.contains(&m) {
            println!("Error: Method '{}' not found in tapplet.", m);
            println!("Available methods: {}", config.api.methods.join(", "));
            return Err(anyhow::anyhow!("Method not found"));
        }
        m
    } else {
        // Interactive method selection
        if config.api.methods.is_empty() {
            println!("Error: Tapplet '{}' has no available methods.", name);
            return Err(anyhow::anyhow!("No methods available"));
        }

        println!("\nAvailable methods for tapplet '{}':", name);

        // Build selection items with descriptions
        let method_items: Vec<String> = config
            .api
            .methods
            .iter()
            .map(|method_name| {
                if let Some(method_def) = config.api.method_definitions.get(method_name) {
                    format!("{} - {}", method_name, method_def.description)
                } else {
                    method_name.clone()
                }
            })
            .collect();

        let selection = Select::new()
            .with_prompt("Select a method to run")
            .items(&method_items)
            .default(0)
            .interact()
            .map_err(|e| anyhow::anyhow!("Failed to get method selection: {}", e))?;

        config.api.methods[selection].clone()
    };

    // Parse arguments from CLI or prompt interactively
    let mut args_map = HashMap::new();

    // First, parse any arguments provided via CLI
    for arg in args.iter().filter(|a| !a.is_empty()) {
        let parts: Vec<&str> = arg.splitn(2, '=').collect();
        if parts.len() == 2 {
            args_map.insert(parts[0].to_string(), parts[1].to_string());
        } else {
            println!("Warning: Ignoring invalid argument format: {}", arg);
        }
    }

    // Check if we need to prompt for missing parameters
    if let Some(method_def) = config.api.method_definitions.get(&selected_method) {
        if !method_def.params.is_empty() {
            println!("\nMethod '{}' requires the following parameters:", selected_method);

            for (param_name, param_def) in &method_def.params {
                // Skip if already provided via CLI
                if args_map.contains_key(param_name) {
                    continue;
                }

                // Prompt for the parameter
                let prompt_text = format!(
                    "{} ({}) - {}",
                    param_name, param_def.param_type, param_def.description
                );

                let value: String = Input::new()
                    .with_prompt(&prompt_text)
                    .interact_text()
                    .map_err(|e| anyhow::anyhow!("Failed to get parameter '{}': {}", param_name, e))?;

                args_map.insert(param_name.clone(), value);
            }
        }
    }

    // Run the tapplet
    println!("\nRunning method '{}' on tapplet '{}'", selected_method, name);

    run_lua(
        &account_name,
        &database_file,
        &password,
        &name,
        &selected_method,
        args_map,
        cache_directory.into(),
    )
    .await
}
