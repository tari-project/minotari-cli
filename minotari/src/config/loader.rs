use std::{fs, fs::File, io::Write, path::Path, str::FromStr};

use anyhow::{Context, Result};
use config::{Config, Environment};
use log::{info, trace};
use tari_common::configuration::{ConfigOverrideProvider, Network};

struct WalletConfigOverrides;

impl ConfigOverrideProvider for WalletConfigOverrides {
    fn get_config_property_overrides(&self, network: &Network) -> Vec<(String, String)> {
        let network_str = network.to_string();
        vec![
            ("wallet.override_from".to_string(), network_str.clone()),
            ("wallet.network".to_string(), network_str),
        ]
    }
}

pub fn get_default_config() -> &'static str {
    include_str!("../../config/config.toml")
}

pub fn load_configuration(path: &Path, cli_network: Option<Network>) -> Result<Config> {
    if !path.exists() {
        let sources = get_default_config();
        write_config_to(path, sources).context("Could not create default config")?;
        info!(path:% = path.display(); "Created new configuration file");
    }

    let filename = path.to_str().context("Invalid config file path")?;

    let cfg = Config::builder()
        .add_source(config::File::with_name(filename))
        .add_source(Environment::with_prefix("TARI").prefix_separator("_").separator("__"))
        .build()
        .context("Could not build initial config")?;

    let network = if let Some(val) = cli_network {
        val
    } else {
        match cfg.get_string("network") {
            Ok(network_str) => Network::from_str(&network_str).context("Invalid network")?,
            Err(config::ConfigError::NotFound(_)) => Network::default(),
            Err(e) => return Err(e).context("Could not get network configuration"),
        }
    };

    let overrides_provider = WalletConfigOverrides;
    let overrides_list = overrides_provider.get_config_property_overrides(&network);

    if overrides_list.is_empty() {
        return Ok(cfg);
    }

    let mut builder = Config::builder().add_source(cfg);
    for (key, value) in overrides_list {
        trace!("Set override: ({key}, {value})");
        builder = builder
            .set_override(key.as_str(), value.as_str())
            .context("Could not override config property")?;
    }

    builder.build().context("Could not build final config")
}

pub fn write_config_to(path: &Path, source: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create parent directories")?;
    };

    let mut file = File::create(path).context("Failed to create config file")?;
    file.write_all(source.as_bytes())
        .context("Failed to write config content")?;
    file.write_all(b"\n").context("Failed to write newline")?;
    Ok(())
}
