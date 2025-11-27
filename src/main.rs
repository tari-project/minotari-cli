use std::{
    env,
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
use minotari::util::encrypt_with_password;
use minotari::{
    api::accounts::LockFundsRequest,
    daemon,
    db::{self, get_accounts, get_balance, init_db},
    models::WalletEvent,
    scan::{self, ScanError},
    transactions::{
        lock_amount::LockAmount,
        one_sided_transaction::{OneSidedTransaction, Recipient},
    },
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

use minotari::cli::{Cli, Commands};

use minotari::tapplets::tapplet_command_handler;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    // Initialize tracing with environment variable support
    // Set RUST_LOG=lightweight_wallet_libs=warn,minotari=info to see warnings from the library
    tracing_subscriber::fmt()
        .with_env_filter(
            env::var("RUST_LOG").unwrap_or_else(|_| "lightweight_wallet_libs=info,minotari=info".to_string()),
        )
        .init();

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
                public_view_key.clone(),
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
                    "public_view_key": hex::encode(public_view_key.as_bytes()),
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
            if batch_size <= 1 {
                // Batch size 1 doesn't work for some reason. It scans but never
                // progresses
                println!("Batch size must be greater than 1.");
                return Ok(());
            }
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
        Commands::Tapplet { tapplet_subcommand } => {
            tapplet_command_handler(tapplet_subcommand).await?;
            Ok(())
        },
    }
}

async fn handle_balance(database_file: &str, account_name: Option<&str>) -> Result<(), anyhow::Error> {
    let pool = init_db(database_file).await?;
    let mut conn = pool.acquire().await?;
    let accounts = get_accounts(&mut conn, account_name, false).await?;
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
    let account = account
        .try_into_parent()
        .map_err(|e| anyhow!("Invalid account type: {}", e))?;

    let amount = recipients.iter().map(|r| r.amount).sum();
    let num_outputs = recipients.len();
    let fee_per_gram = MicroMinotari(5);
    let estimated_output_size = None;

    let lock_amount = LockAmount::new(pool.clone());
    let locked_funds = lock_amount
        .lock(
            &account,
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
    let account = db::get_parent_account_by_name(&mut conn, &account_name)
        .await?
        .ok_or_else(|| anyhow!("Account not found: {}", account_name))?;
    let lock_amount = LockAmount::new(pool.clone());
    let result = lock_amount
        .lock(
            &account,
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

    let (nonce, encrypted_view_key, encrypted_spend_key) =
        encrypt_with_password(password, &view_key_bytes, spend_key_bytes)?;

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
