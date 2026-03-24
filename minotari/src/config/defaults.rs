use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tari_common::{SubConfigPath, configuration::Network};

pub fn default_burn_proofs_dir(network: Network) -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tari")
        .join(network.as_key_str())
        .join("burn_proofs")
}

use crate::cli::{AccountArgs, ApplyArgs, BurnArgs, DaemonArgs, DatabaseArgs, NodeArgs, TransactionArgs};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct WebhookConfig {
    /// The HTTP endpoint to post events to
    pub url: Option<String>,
    /// The secret key used for HMAC signing
    pub secret: Option<String>,
    /// Optional list of event types to send. If None, all events are sent.
    pub send_only_event_types: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WalletConfig {
    pub network: Network,
    pub base_url: String,
    pub database_path: PathBuf,
    pub batch_size: u64,
    pub scan_interval_secs: u64,
    pub api_port: u16,
    pub confirmation_window: u64,
    pub account_name: Option<String>,
    pub webhook: WebhookConfig,
    /// Directory where complete burn proof JSON files are written after a burn transaction is confirmed
    /// and the kernel merkle proof is fetched from the base node.
    /// If not set, defaults to the platform data directory: `<data_dir>/tari/<network>/burn_proofs`.
    pub burn_proofs_dir: Option<PathBuf>,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            network: Network::MainNet,
            base_url: "https://rpc.tari.com".to_string(),
            database_path: PathBuf::from("data/wallet.db"),
            batch_size: 25,
            scan_interval_secs: 60,
            api_port: 9000,
            confirmation_window: 3,
            account_name: None,
            webhook: WebhookConfig::default(),
            burn_proofs_dir: None,
        }
    }
}

impl WalletConfig {
    pub fn effective_burn_proofs_dir(&self) -> PathBuf {
        self.burn_proofs_dir
            .clone()
            .unwrap_or_else(|| default_burn_proofs_dir(self.network))
    }
}

impl SubConfigPath for WalletConfig {
    fn main_key_prefix() -> &'static str {
        "wallet"
    }
}

impl ApplyArgs for WalletConfig {
    fn apply_database(&mut self, args: &DatabaseArgs) {
        if let Some(database_path) = &args.database_path {
            self.database_path = database_path.clone();
        }
    }

    fn apply_node(&mut self, args: &NodeArgs) {
        if let Some(base_url) = &args.base_url {
            self.base_url = base_url.clone();
        }
        if let Some(batch_size) = args.batch_size {
            self.batch_size = batch_size;
        }
    }

    fn apply_account(&mut self, args: &AccountArgs) {
        if let Some(account_name) = &args.account_name {
            self.account_name = Some(account_name.clone());
        }
    }

    fn apply_transaction(&mut self, args: &TransactionArgs) {
        if let Some(confirmation_window) = args.confirmation_window {
            self.confirmation_window = confirmation_window;
        }
    }

    fn apply_burn(&mut self, args: &BurnArgs) {
        if let Some(dir) = &args.burn_proofs_dir {
            self.burn_proofs_dir = Some(dir.clone());
        }
    }

    fn apply_daemon(&mut self, args: &DaemonArgs) {
        if let Some(scan_interval_secs) = args.scan_interval_secs {
            self.scan_interval_secs = scan_interval_secs;
        }
        if let Some(api_port) = args.api_port {
            self.api_port = api_port;
        }
    }
}
