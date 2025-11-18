use std::{
    fs::{self, create_dir_all},
    path::Path,
};

use anyhow::anyhow;
use blake2::{Blake2s256, Digest};
use chacha20poly1305::{
    AeadCore, Key, KeyInit, XChaCha20Poly1305,
    aead::{Aead, OsRng},
};
use clap::{Parser, Subcommand};
use minotari::{
    daemon, db,
    db::{get_accounts, get_balance, init_db},
    models::WalletEvent,
    scan,
    scan::ScanError,
    transactions::one_sided_transaction::{OneSidedTransaction, Recipient},
};
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

#[derive(Parser)]
#[command(name = "tari")]
#[command(about = "Tari wallet CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new address, and returns a file with the
    /// seed words, address, birthday, private view key and public spend key,
    /// optionally encrypting the file with a password
    CreateAddress {
        #[arg(short, long, help = "Password to encrypt the wallet file")]
        password: Option<String>,
        #[arg(short, long, help = "Path to the output file", default_value = "data/output.json")]
        output_file: String,
    },
    /// Scan the blockchain for transactions
    Scan {
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        #[arg(
            short = 'u',
            long,
            default_value = "https://rpc.tari.com",
            help = "The base URL of the Tari HTTP API"
        )]
        base_url: String,
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        #[arg(
            short,
            long,
            help = "Optional account name to scan. If not provided, all accounts will be used"
        )]
        account_name: Option<String>,
        #[arg(short = 'n', long, help = "Maximum number of blocks to scan", default_value_t = 50)]
        max_blocks_to_scan: u64,
        #[arg(long, help = "Batch size for scanning", default_value_t = 1)]
        batch_size: u64,
    },
    /// Run the daemon to continuously scan the blockchain
    Daemon {
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        #[arg(
            short = 'u',
            long,
            default_value = "https://rpc.tari.com",
            help = "The base URL of the Tari HTTP API"
        )]
        base_url: String,
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        #[arg(long, help = "Batch size for scanning", default_value_t = 100)]
        batch_size: u64,
        #[arg(short, long, help = "Interval between scans in seconds", default_value_t = 60)]
        scan_interval_secs: u64,
        #[arg(long, help = "Port for the API server", default_value_t = 9000)]
        api_port: u16,
        #[arg(long, help = "The Tari network to connect to", default_value_t = Network::MainNet)]
        network: Network,
    },
    /// Show wallet balance
    Balance {
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        #[arg(
            short,
            long,
            help = "Optional account name to show balance for. If not provided, all accounts will be used"
        )]
        account_name: Option<String>,
    },
    /// Import a wallet from a view key
    ImportViewKey {
        #[arg(short, long, alias = "view_key", help = "The view key in hex format")]
        view_private_key: String,
        #[arg(short, long, alias = "spend_key", help = "The spend public key in hex format")]
        spend_public_key: String,
        #[arg(short, long, help = "Password to encrypt the wallet file")]
        password: String,
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        #[arg(short, long, help = "The wallet birthday (block height)", default_value = "0")]
        birthday: u16,
    },
    /// Create an unsigned transaction
    CreateUnsignedTransaction {
        #[arg(short, long, help = "Name of the account to send from")]
        account_name: String,
        #[arg(
            short,
            long,
            help = "Recipient address, amount and optional payment id (e.g., address::amount or address::amount::payment_id). Can be specified multiple times."
        )]
        recipient: Vec<String>,
        #[arg(
            short,
            long,
            help = "Path to the output file for the unsigned transaction",
            default_value = "data/unsigned_transaction.json"
        )]
        output_file: String,
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        #[arg(long, help = "Optional idempotency key")]
        idempotency_key: Option<String>,
        #[arg(long, help = "Optional seconds to lock UTXOs", default_value_t = 86400)]
        seconds_to_lock: u64,
        #[arg(long, help = "The Tari network to connect to", default_value_t = Network::MainNet)]
        network: Network,
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
            let events = scan(
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
            // Add scanning logic here
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
        let micro_tari_balance = credits.saturating_sub(debits) as f64;
        let tari_balance = micro_tari_balance / 1_000_000.0;
        println!(
            "Balance at height {}({}): {} microTari ({} Tari)",
            agg_result.max_height.unwrap_or(0),
            agg_result.max_date.unwrap_or_else(|| "N/A".to_string()),
            micro_tari_balance,
            tari_balance
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

    let one_sided_tx = OneSidedTransaction::new(pool.clone(), network, password.clone());
    let result = one_sided_tx
        .create_unsigned_transaction(account, recipients, idempotency_key, seconds_to_lock)
        .await
        .map_err(|e| anyhow!("Failed to create unsigned transaction: {}", e))?;

    create_dir_all(Path::new(&output_file).parent().unwrap())?;
    fs::write(output_file, serde_json::to_string_pretty(&result)?)?;

    println!("Unsigned transaction written to file.");
    Ok(())
}

async fn scan(
    password: &str,
    base_url: &str,
    database_file: &str,
    account_name: Option<&str>,
    max_blocks: u64,
    batch_size: u64,
) -> Result<Vec<WalletEvent>, ScanError> {
    scan::scan(password, base_url, database_file, account_name, max_blocks, batch_size).await
}

async fn init_with_view_key(
    view_private_key: &str,
    spend_public_key: &str,
    password: &str,
    database_file: &str,
    birthday: u16,
) -> Result<(), anyhow::Error> {
    let view_key_bytes = hex::decode(view_private_key)?;
    let spend_key_bytes = hex::decode(spend_public_key)?;

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
    let encrypted_view_key = cipher.encrypt(&nonce, view_key_bytes.as_ref())?;
    let encrypted_spend_key = cipher.encrypt(&nonce, spend_key_bytes.as_ref())?;

    // create a hash of the viewkey to determine duplicate wallets
    let view_key_hash = hash_view_key(&view_key_bytes);
    let pool = init_db(database_file).await?;
    let mut conn = pool.acquire().await?;
    db::create_account(
        &mut conn,
        "default",
        &encrypted_view_key,
        &encrypted_spend_key,
        &nonce,
        &view_key_hash,
        birthday as i64,
    )
    .await?;

    Ok(())
}

fn hash_view_key(view_key: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2s256::new();
    hasher.update(b"view_key_hash");
    hasher.update(view_key);
    hasher.finalize().to_vec()
}
