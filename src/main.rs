use std::sync::Arc;

use blake2::{Blake2s256, Digest};
use chacha20poly1305::{
    AeadCore, Key, KeyInit, XChaCha20Poly1305,
    aead::{Aead, OsRng},
};
use clap::{Command, Parser, Subcommand};
use tari_common::configuration::Network;
use tari_common_types::{
    seeds::{
        cipher_seed::CipherSeed,
        mnemonic::{Mnemonic, MnemonicLanguage},
    },
    tari_address::{TariAddress, TariAddressFeatures},
    wallet_types::WalletType,
};
use tari_crypto::{compressed_key::CompressedKey, ristretto::RistrettoPublicKey};
use tari_transaction_components::{
    crypto_factories::CryptoFactories,
    key_manager::{
        TransactionKeyManagerInterface, TransactionKeyManagerWrapper, memory_key_manager::MemoryKeyManagerBackend,
    },
};
use tari_utilities::byte_array::ByteArray;

use minotari::cli::{Cli, Commands};
use minotari::{
    daemon, db,
    db::{get_accounts, get_balance, init_db},
    models::WalletEvent,
    scan,
    scan::ScanError,
};

use minotari::tapplets::tapplet_command_handler;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    match cli.command {
        Commands::CreateAddress { password, output_file } => {
            println!("Creating new address...");
            let seeds = CipherSeed::random();
            let birthday = seeds.birthday();
            let seed_words = seeds.to_mnemonic(MnemonicLanguage::English, None)?.join(" ");
            let key_manager: TransactionKeyManagerWrapper<MemoryKeyManagerBackend> = TransactionKeyManagerWrapper::new(
                Some(seeds),
                CryptoFactories::default(),
                Arc::new(WalletType::DerivedKeys),
            )
            .await?;

            let view_key = key_manager.get_private_view_key().await?;
            let spend_key = key_manager.get_spend_key().await?;

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
                let key = Key::from_slice(password.as_bytes());
                let cipher = XChaCha20Poly1305::new(key);

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
        Commands::Tapplet { tapplet_subcommand } => {
            tapplet_command_handler(tapplet_subcommand).await?;
            Ok(())
        },
    }
}

async fn handle_balance(database_file: &str, account_name: Option<&str>) -> Result<(), anyhow::Error> {
    let db = init_db(database_file).await?;
    let accounts = get_accounts(&db, account_name).await?;
    for account in accounts {
        let agg_result = get_balance(&db, account.id).await?;
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
    let key = Key::from_slice(password.as_bytes());
    let cipher = XChaCha20Poly1305::new(key);

    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let encrypted_view_key = cipher.encrypt(&nonce, view_key_bytes.as_ref())?;
    let encrypted_spend_key = cipher.encrypt(&nonce, spend_key_bytes.as_ref())?;

    // create a hash of the viewkey to determine duplicate wallets
    let view_key_hash = hash_view_key(&view_key_bytes);
    let db = init_db(database_file).await?;
    db::create_account(
        &db,
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
