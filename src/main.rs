//! Minotari Wallet CLI Application
//!
//! A command-line interface for managing Tari wallets with support for view-key
//! based operations, blockchain scanning, and transaction creation.
//!
//! # Overview
//!
//! This application provides a lightweight wallet implementation for the Tari
//! cryptocurrency network. It supports:
//!
//! - **Wallet Creation**: Generate new wallets with optional password encryption
//! - **View Key Import**: Import existing wallets using view and spend keys
//! - **Blockchain Scanning**: Detect incoming transactions and track wallet balance
//! - **Transaction Creation**: Build unsigned one-sided transactions
//! - **Fund Locking**: Reserve UTXOs for pending transactions
//! - **Daemon Mode**: Continuous blockchain monitoring with REST API
//!
//! # Security Model
//!
//! The wallet uses view-key based scanning, which allows detecting incoming
//! transactions without exposing spending capability. Sensitive data (view keys,
//! seed words) can be encrypted using XChaCha20-Poly1305 with a user-provided
//! password.
//!
//! # Usage Examples
//!
//! Create a new encrypted wallet:
//! ```bash
//! tari create-address --password "my_secure_password" --output-file wallet.json
//! ```
//!
//! Scan the blockchain for transactions:
//! ```bash
//! tari scan --password "my_password" --database-file wallet.db
//! ```
//!
//! Run the wallet daemon with API server:
//! ```bash
//! tari daemon --password "my_password" --api-port 9000
//! ```
//!
//! # Data Storage
//!
//! - Wallet credentials are stored in JSON files (optionally encrypted)
//! - Transaction and balance data is stored in a SQLite database
//! - Default data directory is `./data/`

use std::{
    fs::{self, create_dir_all},
    path::Path,
};

use anyhow::anyhow;
use chacha20poly1305::{
    AeadCore, Key, KeyInit, XChaCha20Poly1305,
    aead::{Aead, OsRng},
};
use clap::{Parser, Subcommand};
use minotari::{
    api::accounts::LockFundsRequest,
    daemon,
    db::{self, get_accounts, get_balance, init_db},
    models::WalletEvent,
    scan::{self, rollback_from_height, scan::ScanError},
    transactions::{
        fund_locker::FundLocker,
        one_sided_transaction::{OneSidedTransaction, Recipient},
    },
    utils,
};
use num_format::{Locale, ToFormattedString};
use std::str::FromStr;
use tari_common::configuration::Network;
use tari_common_types::{
    seeds::{
        cipher_seed::CipherSeed,
        mnemonic::{Mnemonic, MnemonicLanguage},
    },
    tari_address::{TariAddress, TariAddressFeatures},
};
use tari_crypto::compressed_key::CompressedKey;
use tari_transaction_components::key_manager::KeyManager;
use tari_transaction_components::key_manager::TransactionKeyManagerInterface;
use tari_transaction_components::key_manager::wallet_types::SeedWordsWallet;
use tari_transaction_components::key_manager::wallet_types::WalletType;
use tari_transaction_components::tari_amount::MicroMinotari;
use tari_utilities::byte_array::ByteArray;

/// Command-line interface definition for the Tari wallet.
///
/// This struct is the root of the CLI argument parser, containing all available
/// subcommands for wallet operations. It uses the `clap` crate for argument parsing.
///
/// # Subcommands
///
/// - [`Commands::CreateAddress`] - Generate a new wallet address
/// - [`Commands::ImportViewKey`] - Import an existing wallet by view key
/// - [`Commands::Scan`] - Scan blockchain for transactions
/// - [`Commands::ReScan`] - Re-scan from a specific block height
/// - [`Commands::Daemon`] - Run continuous scanning daemon
/// - [`Commands::Balance`] - Display wallet balance
/// - [`Commands::CreateUnsignedTransaction`] - Build an unsigned transaction
/// - [`Commands::LockFunds`] - Lock UTXOs for a pending transaction
#[derive(Parser)]
#[command(name = "tari")]
#[command(about = "Tari wallet CLI", long_about = None)]
struct Cli {
    /// The subcommand to execute
    #[command(subcommand)]
    command: Commands,
}

/// Available CLI subcommands for wallet operations.
///
/// Each variant represents a distinct operation that can be performed on the wallet.
/// Commands are organized by their primary function: wallet management, blockchain
/// scanning, balance queries, and transaction operations.
///
/// # Wallet Management Commands
///
/// - [`Commands::CreateAddress`] - Generate a brand new wallet
/// - [`Commands::ImportViewKey`] - Import an existing wallet using keys
///
/// # Scanning Commands
///
/// - [`Commands::Scan`] - One-time blockchain scan
/// - [`Commands::ReScan`] - Re-scan from a specific height (useful for recovery)
/// - [`Commands::Daemon`] - Continuous scanning with REST API
///
/// # Query Commands
///
/// - [`Commands::Balance`] - View current wallet balance
///
/// # Transaction Commands
///
/// - [`Commands::CreateUnsignedTransaction`] - Create a transaction for offline signing
/// - [`Commands::LockFunds`] - Reserve UTXOs for pending operations
#[derive(Subcommand)]
enum Commands {
    /// Create a new wallet address with optional encryption.
    ///
    /// Generates a new wallet with:
    /// - Random cipher seed
    /// - Mnemonic seed words (English)
    /// - View key (private) and spend key (public)
    /// - Tari address for receiving funds
    ///
    /// The output file can be encrypted with a password using XChaCha20-Poly1305.
    /// If no password is provided, keys are stored in plaintext (not recommended
    /// for production use).
    ///
    /// # Output Format
    ///
    /// The generated JSON file contains:
    /// - `address`: Base58-encoded Tari address
    /// - `view_key` / `encrypted_view_key`: Private view key
    /// - `spend_key` / `encrypted_spend_key`: Public spend key
    /// - `seed_words` / `encrypted_seed_words`: Mnemonic recovery phrase
    /// - `birthday`: Block height when wallet was created
    /// - `nonce`: (encrypted only) Encryption nonce
    CreateAddress {
        /// Password to encrypt the wallet file (optional but recommended).
        /// If provided, will be padded or truncated to 32 bytes.
        #[arg(short, long, help = "Password to encrypt the wallet file")]
        password: Option<String>,
        /// Path to write the wallet credentials JSON file.
        #[arg(short, long, help = "Path to the output file", default_value = "data/output.json")]
        output_file: String,
    },
    /// Scan the blockchain for incoming transactions.
    ///
    /// Performs a partial scan of the blockchain starting from the last scanned
    /// height, looking for outputs that belong to the wallet. Detected outputs
    /// are recorded in the database and can be viewed with the `balance` command.
    ///
    /// # Scanning Process
    ///
    /// 1. Fetches blocks from the Tari HTTP API
    /// 2. Decrypts output commitments using the view key
    /// 3. Records detected outputs in the SQLite database
    /// 4. Updates the scanned tip height
    ///
    /// # Performance Tuning
    ///
    /// - `max_blocks_to_scan`: Limits scan duration (default: 50)
    /// - `batch_size`: Number of blocks per API request (default: 100)
    Scan {
        /// Password to decrypt the wallet view key from the database.
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        /// Base URL of the Tari HTTP RPC API endpoint.
        #[arg(
            short = 'u',
            long,
            default_value = "https://rpc.tari.com",
            help = "The base URL of the Tari HTTP API"
        )]
        base_url: String,
        /// Path to the SQLite database file storing wallet state.
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        /// Specific account to scan. If omitted, all accounts are scanned.
        #[arg(
            short,
            long,
            help = "Optional account name to scan. If not provided, all accounts will be used"
        )]
        account_name: Option<String>,
        /// Maximum number of blocks to scan in this invocation.
        #[arg(short = 'n', long, help = "Maximum number of blocks to scan", default_value_t = 50)]
        max_blocks_to_scan: u64,
        /// Number of blocks to fetch per API request for efficiency.
        #[arg(long, help = "Batch size for scanning", default_value_t = 100)]
        batch_size: u64,
    },
    /// Re-scan the blockchain from a specific height.
    ///
    /// Rolls back the wallet state to a specified block height and re-scans
    /// from that point. This is useful for:
    ///
    /// - Recovering from database corruption
    /// - Handling blockchain reorganizations
    /// - Debugging missing transactions
    ///
    /// # Warning
    ///
    /// This operation modifies the database by removing outputs detected
    /// after the specified height. Make a backup before re-scanning.
    ReScan {
        /// Password to decrypt the wallet view key.
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        /// Base URL of the Tari HTTP RPC API endpoint.
        #[arg(
            short = 'u',
            long,
            default_value = "https://rpc.tari.com",
            help = "The base URL of the Tari HTTP API"
        )]
        base_url: String,
        /// Path to the SQLite database file.
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        /// Name of the account to re-scan (required).
        #[arg(short, long, help = "Account name to re-scan")]
        account_name: String,
        /// Block height to roll back to before re-scanning.
        #[arg(short = 'r', long, help = "Re-scan from height")]
        rescan_from_height: u64,
        /// Number of blocks to fetch per API request.
        #[arg(long, help = "Batch size for scanning", default_value_t = 100)]
        batch_size: u64,
    },
    /// Run the wallet daemon for continuous blockchain monitoring.
    ///
    /// Starts a long-running process that:
    /// - Continuously scans the blockchain at regular intervals
    /// - Exposes a REST API for wallet operations
    /// - Automatically unlocks expired UTXO locks
    /// - Handles graceful shutdown on Ctrl+C
    ///
    /// # API Endpoints
    ///
    /// The daemon exposes endpoints for:
    /// - Balance queries: `GET /accounts/{name}/balance`
    /// - Fund locking: `POST /accounts/{name}/lock_funds`
    /// - Transaction creation: `POST /accounts/{name}/create_unsigned_transaction`
    ///
    /// API documentation is available at `/swagger-ui/` when the daemon is running.
    ///
    /// # Shutdown
    ///
    /// Press Ctrl+C to initiate graceful shutdown. The daemon will:
    /// 1. Stop accepting new API requests
    /// 2. Complete the current scan cycle
    /// 3. Close database connections
    Daemon {
        /// Password to decrypt wallet credentials.
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        /// Base URL of the Tari HTTP RPC API endpoint.
        #[arg(
            short = 'u',
            long,
            default_value = "https://rpc.tari.com",
            help = "The base URL of the Tari HTTP API"
        )]
        base_url: String,
        /// Path to the SQLite database file.
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        /// Number of blocks to fetch per API request.
        #[arg(long, help = "Batch size for scanning", default_value_t = 100)]
        batch_size: u64,
        /// Seconds to wait between scan cycles.
        #[arg(short, long, help = "Interval between scans in seconds", default_value_t = 60)]
        scan_interval_secs: u64,
        /// TCP port for the REST API server.
        #[arg(long, help = "Port for the API server", default_value_t = 9000)]
        api_port: u16,
        /// Tari network to connect to (MainNet, StageNet, NextNet, LocalNet).
        #[arg(long, help = "The Tari network to connect to", default_value_t = Network::MainNet)]
        network: Network,
    },
    /// Display the wallet balance.
    ///
    /// Shows the current balance for one or all accounts in the wallet.
    /// Balance is calculated as the sum of confirmed outputs minus spent inputs.
    ///
    /// # Output Format
    ///
    /// Displays balance in both microTari (base units) and Tari with proper
    /// formatting and thousand separators for readability.
    Balance {
        /// Path to the SQLite database file.
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        /// Specific account to show balance for. If omitted, shows all accounts.
        #[arg(
            short,
            long,
            help = "Optional account name to show balance for. If not provided, all accounts will be used"
        )]
        account_name: Option<String>,
    },
    /// Import a wallet using view and spend keys.
    ///
    /// Creates a new account in the database using existing cryptographic keys.
    /// This is useful for:
    ///
    /// - Restoring a wallet from backed-up keys
    /// - Creating a watch-only wallet (view key only)
    /// - Importing a wallet generated by another application
    ///
    /// # Key Format
    ///
    /// Both keys should be provided as hex-encoded strings:
    /// - `view_private_key`: 64 hex characters (32 bytes)
    /// - `spend_public_key`: 64 hex characters (32 bytes, compressed)
    ///
    /// # Birthday
    ///
    /// The birthday is the block height when the wallet was created. Setting
    /// this correctly avoids scanning unnecessary historical blocks.
    ImportViewKey {
        /// Private view key in hexadecimal format.
        #[arg(short, long, alias = "view_key", help = "The view key in hex format")]
        view_private_key: String,
        /// Public spend key in hexadecimal format (compressed point).
        #[arg(short, long, alias = "spend_key", help = "The spend public key in hex format")]
        spend_public_key: String,
        /// Password to encrypt the stored credentials.
        #[arg(short, long, help = "Password to encrypt the wallet file")]
        password: String,
        /// Path to the SQLite database file.
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        /// Block height when the wallet was created (for scan optimization).
        #[arg(short, long, help = "The wallet birthday (block height)", default_value = "0")]
        birthday: u16,
    },
    /// Create an unsigned one-sided transaction.
    ///
    /// Builds a transaction that can be signed offline. The transaction sends
    /// funds to one or more recipients using one-sided (non-interactive) payments.
    ///
    /// # Recipient Format
    ///
    /// Recipients are specified as `address::amount` or `address::amount::payment_id`:
    /// - `address`: Base58-encoded Tari address
    /// - `amount`: Amount in microTari
    /// - `payment_id`: Optional memo/reference (max 48 characters)
    ///
    /// # UTXO Locking
    ///
    /// Input UTXOs are automatically locked to prevent double-spending. If the
    /// transaction is not broadcast within `seconds_to_lock`, the UTXOs are
    /// automatically released.
    ///
    /// # Example
    ///
    /// ```bash
    /// tari create-unsigned-transaction \
    ///     --account-name main \
    ///     --recipient "f2ABC...123::1000000" \
    ///     --password secret
    /// ```
    CreateUnsignedTransaction {
        /// Name of the account to spend from.
        #[arg(short, long, help = "Name of the account to send from")]
        account_name: String,
        /// Recipients in `address::amount[::payment_id]` format. Repeatable.
        #[arg(
            short,
            long,
            help = "Recipient address, amount and optional payment id (e.g., address::amount or address::amount::payment_id). Can be specified multiple times."
        )]
        recipient: Vec<String>,
        /// Path to write the unsigned transaction JSON.
        #[arg(
            short,
            long,
            help = "Path to the output file for the unsigned transaction",
            default_value = "data/unsigned_transaction.json"
        )]
        output_file: String,
        /// Password to decrypt wallet credentials.
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        /// Path to the SQLite database file.
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        /// Unique key to prevent duplicate transactions.
        #[arg(long, help = "Optional idempotency key")]
        idempotency_key: Option<String>,
        /// Duration in seconds to lock input UTXOs (default: 24 hours).
        #[arg(long, help = "Optional seconds to lock UTXOs", default_value_t = 86400)]
        seconds_to_lock: u64,
        /// Tari network for address validation and consensus rules.
        #[arg(long, help = "The Tari network to connect to", default_value_t = Network::MainNet)]
        network: Network,
    },
    /// Lock funds (reserve UTXOs) for a pending transaction.
    ///
    /// Reserves a set of UTXOs totaling at least the specified amount plus
    /// estimated fees. Locked UTXOs cannot be used for other transactions
    /// until they are either spent or the lock expires.
    ///
    /// # Use Case
    ///
    /// This is useful when you need to:
    /// - Reserve funds before creating a complex multi-step transaction
    /// - Ensure sufficient funds are available for a future payment
    /// - Coordinate multiple transactions without double-spending
    ///
    /// # Automatic Unlock
    ///
    /// If the locked funds are not spent within `seconds_to_lock_utxos`,
    /// they are automatically unlocked and become available again.
    LockFunds {
        /// Name of the account to lock funds from.
        #[arg(short, long, help = "Name of the account to send from")]
        account_name: String,
        /// Path to write the locked funds details JSON.
        #[arg(
            short,
            long,
            help = "Path to the output file for the unsigned transaction",
            default_value = "data/locked_funds.json"
        )]
        output_file: String,
        /// Path to the SQLite database file.
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        /// Amount to lock in microTari.
        #[arg(short, long, help = "Amount to lock")]
        amount: MicroMinotari,
        /// Number of output UTXOs to create (for splitting).
        #[arg(short, long, help = "Optional number of outputs", default_value = "1")]
        num_outputs: usize,
        /// Fee rate in microTari per gram of transaction weight.
        #[arg(short, long, help = "Optional fee per gram", default_value = "5")]
        fee_per_gram: MicroMinotari,
        /// Estimated size of outputs for fee calculation.
        #[arg(short, long, help = "Optional estimated output size")]
        estimated_output_size: Option<usize>,
        /// Duration in seconds before locked UTXOs are released (default: 24h).
        #[arg(
            short,
            long,
            help = "Optional seconds to lock (will be unlocked if not spent)",
            default_value = "86400"
        )]
        seconds_to_lock_utxos: Option<u64>,
        /// Unique key to prevent duplicate lock operations.
        #[arg(long, help = "Optional idempotency key")]
        idempotency_key: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    match cli.command {
        Commands::CreateAddress { password, output_file } => {
            println!("Creating new address...");
            let seeds = CipherSeed::random();
            let birthday = seeds.birthday();
            let seed_words = seeds.to_mnemonic(MnemonicLanguage::English, None)?.join(" ");
            let seed_wallet = SeedWordsWallet::construct_new(seeds).map_err(|_| anyhow::anyhow!("Invalid seeds"))?;
            let wallet = WalletType::SeedWords(seed_wallet);
            let key_manager = KeyManager::new(wallet)?;

            let view_key = key_manager.get_private_view_key();
            let spend_key = key_manager.get_spend_key();

            let public_view_key = CompressedKey::from_secret_key(&view_key);

            let tari_address = TariAddress::new_dual_address(
                public_view_key,
                spend_key.pub_key.clone(),
                Network::MainNet,
                TariAddressFeatures::create_one_sided_only(),
                None,
            )?;
            println!("New address: {}", tari_address);

            let wallet_data = if let Some(password) = password {
                let password = if password.len() < 32 {
                    format!("{:0<32}", password)
                } else {
                    password[..32].to_string()
                };
                let key_bytes: [u8; 32] = password
                    .as_bytes()
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Password must be 32 bytes"))?;
                let key = Key::from(key_bytes);
                let cipher = XChaCha20Poly1305::new(&key);

                let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
                let encrypted_view_key = cipher.encrypt(&nonce, view_key.as_bytes())?;
                let encrypted_spend_key = cipher.encrypt(&nonce, spend_key.pub_key.as_bytes())?;

                let encrypted_seed_words = cipher.encrypt(&nonce, seed_words.reveal().as_bytes())?;

                serde_json::json!({
                    "address": tari_address.to_base58(),
                    "encrypted_view_key": hex::encode(encrypted_view_key),
                    "encrypted_spend_key": hex::encode(encrypted_spend_key),
                    "encrypted_seed_words": hex::encode(encrypted_seed_words),
                    "nonce": hex::encode(nonce),
                    "birthday": birthday,
                })
            } else {
                serde_json::json!({
                    "address": tari_address.to_base58(),
                    "view_key": hex::encode(view_key.as_bytes()),
                    "spend_key": hex::encode(spend_key.pub_key.as_bytes()),
                    "seed_words": seed_words.reveal().clone(),
                    "birthday": birthday,
                })
            };
            std::fs::create_dir_all(std::path::Path::new(&output_file).parent().unwrap())?;
            std::fs::write(output_file, serde_json::to_string_pretty(&wallet_data)?)?;
            println!("Wallet data written to file.");
            Ok(())
        },
        Commands::ImportViewKey {
            view_private_key,
            spend_public_key,
            password,
            database_file,
            birthday,
        } => {
            println!(
                "Importing wallet with view key: {} and spend key: {}",
                view_private_key, spend_public_key
            );
            init_with_view_key(
                &view_private_key,
                &spend_public_key,
                &password,
                &database_file,
                birthday,
            )
            .await
        },
        Commands::Scan {
            password,
            base_url,
            database_file,
            account_name,
            max_blocks_to_scan,
            batch_size,
        } => {
            println!("Scanning blockchain...");
            let (events, _more_blocks_to_scan) = scan(
                &password,
                &base_url,
                &database_file,
                account_name.as_deref(),
                max_blocks_to_scan,
                batch_size,
            )
            .await?;
            println!("Scan complete. Events: {}", events.len());
            Ok(())
        },
        Commands::ReScan {
            password,
            base_url,
            database_file,
            account_name,
            rescan_from_height,
            batch_size,
        } => {
            println!(
                "Rolling back to block {} and scanning blockchain...",
                rescan_from_height
            );
            let (events, _more_blocks_to_scan) = rescan(
                &password,
                &base_url,
                &database_file,
                &account_name,
                rescan_from_height,
                batch_size,
            )
            .await?;
            println!("Re-scan complete. Events: {}", events.len());
            Ok(())
        },
        Commands::Daemon {
            password,
            base_url,
            database_file,
            batch_size,
            scan_interval_secs,
            api_port,
            network,
        } => {
            println!("Starting Tari wallet daemon...");
            let max_blocks_to_scan = u64::MAX;
            let daemon = daemon::Daemon::new(
                password,
                base_url,
                database_file,
                max_blocks_to_scan,
                batch_size,
                scan_interval_secs,
                api_port,
                network,
            );
            daemon.run().await?;
            Ok(())
        },
        Commands::Balance {
            database_file,
            account_name,
        } => {
            println!("Fetching balance...");
            let _ = handle_balance(&database_file, account_name.as_deref()).await;
            Ok(())
        },
        Commands::CreateUnsignedTransaction {
            account_name,
            recipient,
            output_file,
            password,
            database_file,
            idempotency_key,
            seconds_to_lock,
            network,
        } => {
            println!("Creating unsigned transaction...");
            handle_create_unsigned_transaction(
                recipient,
                database_file,
                account_name,
                network,
                password,
                idempotency_key,
                seconds_to_lock,
                output_file,
            )
            .await
        },
        Commands::LockFunds {
            account_name,
            output_file,
            database_file,
            amount,
            num_outputs,
            fee_per_gram,
            estimated_output_size,
            seconds_to_lock_utxos,
            idempotency_key,
        } => {
            println!("Creating unsigned transaction...");
            let request = LockFundsRequest {
                amount,
                num_outputs: Some(num_outputs),
                fee_per_gram: Some(fee_per_gram),
                estimated_output_size,
                seconds_to_lock_utxos,
                idempotency_key,
            };
            handle_lock_funds(database_file, account_name, output_file, request).await
        },
    }
}

async fn handle_balance(database_file: &str, account_name: Option<&str>) -> Result<(), anyhow::Error> {
    let pool = init_db(database_file).await?;
    let mut conn = pool.acquire().await?;
    let accounts = get_accounts(&mut conn, account_name).await?;
    for account in accounts {
        let agg_result = get_balance(&mut conn, account.id).await?;
        let credits = agg_result.total_credits.unwrap_or(0) as u64;
        let debits = agg_result.total_debits.unwrap_or(0) as u64;
        let micro_tari_balance = credits.saturating_sub(debits);
        let tari_balance = micro_tari_balance / 1_000_000;
        let remainder = micro_tari_balance % 1_000_000;
        println!(
            "Balance at height {}({}): {} microTari ({}.{} Tari)",
            agg_result.max_height.unwrap_or(0),
            agg_result.max_date.unwrap_or_else(|| "N/A".to_string()),
            micro_tari_balance.to_formatted_string(&Locale::en),
            tari_balance.to_formatted_string(&Locale::en),
            remainder.to_formatted_string(&Locale::en),
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_create_unsigned_transaction(
    recipient: Vec<String>,
    database_file: String,
    account_name: String,
    network: Network,
    password: String,
    idempotency_key: Option<String>,
    seconds_to_lock: u64,
    output_file: String,
) -> Result<(), anyhow::Error> {
    let recipients: Result<Vec<Recipient>, anyhow::Error> = recipient
        .into_iter()
        .map(|r_str| {
            let parts: Vec<&str> = r_str.split("::").collect();
            if parts.len() < 2 || parts.len() > 3 {
                return Err(anyhow!(
                    "Invalid recipient format. Expected 'address::amount' or 'address::amount::payment_id'"
                ));
            }
            let address = TariAddress::from_str(parts[0])?;
            let amount = MicroMinotari::from_str(parts[1])?;
            let payment_id = if parts.len() == 3 {
                Some(parts[2].to_string())
            } else {
                None
            };
            Ok(Recipient {
                address,
                amount,
                payment_id,
            })
        })
        .collect();
    let recipients = recipients?;

    let pool = init_db(&database_file).await?;
    let mut conn = pool.acquire().await?;
    let account = db::get_account_by_name(&mut conn, &account_name)
        .await?
        .ok_or_else(|| anyhow!("Account not found: {}", account_name))?;

    let amount = recipients.iter().map(|r| r.amount).sum();
    let num_outputs = recipients.len();
    let fee_per_gram = MicroMinotari(5);
    let estimated_output_size = None;

    let lock_amount = FundLocker::new(pool.clone());
    let locked_funds = lock_amount
        .lock(
            account.id,
            amount,
            num_outputs,
            fee_per_gram,
            estimated_output_size,
            idempotency_key,
            seconds_to_lock,
        )
        .await
        .map_err(|e| anyhow!("Failed to lock funds: {}", e))?;

    let one_sided_tx = OneSidedTransaction::new(pool.clone(), network, password.clone());
    let result = one_sided_tx
        .create_unsigned_transaction(&account, locked_funds, recipients, fee_per_gram)
        .await
        .map_err(|e| anyhow!("Failed to create an unsigned transaction: {}", e))?;

    create_dir_all(Path::new(&output_file).parent().unwrap())?;
    fs::write(output_file, serde_json::to_string_pretty(&result)?)?;

    println!("Unsigned transaction written to file.");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_lock_funds(
    database_file: String,
    account_name: String,
    output_file: String,
    request: LockFundsRequest,
) -> Result<(), anyhow::Error> {
    let pool = init_db(&database_file).await?;
    let mut conn = pool.acquire().await?;
    let account = db::get_account_by_name(&mut conn, &account_name)
        .await?
        .ok_or_else(|| anyhow!("Account not found: {}", account_name))?;
    let lock_amount = FundLocker::new(pool.clone());
    let result = lock_amount
        .lock(
            account.id,
            request.amount,
            request.num_outputs.expect("must be present"),
            request.fee_per_gram.expect("must be present"),
            request.estimated_output_size,
            request.idempotency_key,
            request.seconds_to_lock_utxos.expect("must be present"),
        )
        .await
        .map_err(|e| anyhow!("Failed to lock funds: {}", e))?;

    create_dir_all(Path::new(&output_file).parent().unwrap())?;
    fs::write(output_file, serde_json::to_string_pretty(&result)?)?;

    println!("Locked funds output written to file.");
    Ok(())
}

async fn scan(
    password: &str,
    base_url: &str,
    database_file: &str,
    account_name: Option<&str>,
    max_blocks: u64,
    batch_size: u64,
) -> Result<(Vec<WalletEvent>, bool), ScanError> {
    let mut scanner =
        scan::Scanner::new(password, base_url, database_file, batch_size).mode(scan::ScanMode::Partial { max_blocks });

    if let Some(name) = account_name {
        scanner = scanner.account(name);
    }

    scanner.run().await
}

async fn rescan(
    password: &str,
    base_url: &str,
    database_file: &str,
    account_name: &str,
    rescan_from_height: u64,
    batch_size: u64,
) -> Result<(Vec<WalletEvent>, bool), ScanError> {
    let pool = init_db(database_file).await?;
    let mut conn = pool.acquire().await?;

    let account = db::get_account_by_name(&mut conn, account_name)
        .await?
        .ok_or_else(|| anyhow!("Account not found: {}", account_name))?;
    let _ = rollback_from_height(&mut conn, account.id, rescan_from_height).await?;

    let max_blocks_to_scan = u64::MAX;
    let mut scanner = scan::Scanner::new(password, base_url, database_file, batch_size).mode(scan::ScanMode::Partial {
        max_blocks: max_blocks_to_scan,
    });
    scanner = scanner.account(account_name);
    scanner.run().await
}

async fn init_with_view_key(
    view_private_key: &str,
    spend_public_key: &str,
    password: &str,
    database_file: &str,
    birthday: u16,
) -> Result<(), anyhow::Error> {
    utils::init_with_view_key(
        view_private_key,
        spend_public_key,
        password,
        database_file,
        birthday,
        None,
    )
    .await
}
