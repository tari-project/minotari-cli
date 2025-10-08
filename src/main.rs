use blake2::{Blake2s256, Digest};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{AeadCore, KeyInit, aead::OsRng};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use clap::{Parser, Subcommand};
use lightweight_wallet_libs::BlockchainScanner;
// use lightweight_wallet_libs::transaction_components::{Network, TransactionKeyManagerInterface};
use lightweight_wallet_libs::{HttpBlockchainScanner, KeyManagerBuilder, ScanConfig};
use std::sync::Arc;
use std::time::Instant;
use tari_common::configuration::Network;
use tari_common_types::seeds::cipher_seed::CipherSeed;
use tari_common_types::seeds::mnemonic::{Mnemonic, MnemonicLanguage};
use tari_common_types::seeds::seed_words;
use tari_common_types::tari_address::{TariAddress, TariAddressFeatures};
use tari_common_types::wallet_types::WalletType;
use tari_crypto::compressed_key::CompressedKey;
use tari_crypto::keys::PublicKey;
use tari_crypto::ristretto::{RistrettoPublicKey, RistrettoSecretKey};
use tari_transaction_components::crypto_factories::CryptoFactories;
use tari_transaction_components::key_manager::TransactionKeyManagerInterface;
use tari_transaction_components::key_manager::TransactionKeyManagerWrapper;
use tari_transaction_components::key_manager::memory_key_manager::MemoryKeyManagerBackend;

use crate::db::{AccountRow, delete_old_scanned_tip_blocks, get_accounts, init_db};
use crate::models::{BalanceChange, WalletEvent};
use tari_utilities::byte_array::ByteArray;
mod db;
mod models;

pub const BIRTHDAY_GENESIS_FROM_UNIX_EPOCH: u64 = 1640995200;
pub const MAINNET_GENESIS_DATE: u64 = 1746489644; // 6 May 2025

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
        #[arg(
            short,
            long,
            help = "Path to the output file",
            default_value = "data/output.json"
        )]
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
        #[arg(
            short,
            long,
            help = "Path to the database file",
            default_value = "data/wallet.db"
        )]
        database_file: String,
        #[arg(
            short,
            long,
            help = "Optional account name to scan. If not provided, all accounts will be used"
        )]
        account_name: Option<String>,
        #[arg(
            short = 'n',
            long,
            help = "Maximum number of blocks to scan",
            default_value_t = 50
        )]
        max_blocks_to_scan: u64,
        #[arg(long, help = "Batch size for scanning", default_value_t = 1)]
        batch_size: u64,
    },
    /// Show wallet balance
    Balance {
        #[arg(
            short,
            long,
            help = "Path to the database file",
            default_value = "data/wallet.db"
        )]
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
        #[arg(
            short,
            long,
            alias = "spend_key",
            help = "The spend public key in hex format"
        )]
        spend_public_key: String,
        #[arg(short, long, help = "Password to encrypt the wallet file")]
        password: String,
        #[arg(
            short,
            long,
            help = "Path to the database file",
            default_value = "data/wallet.db"
        )]
        database_file: String,
        #[arg(
            short,
            long,
            help = "The wallet birthday (block height)",
            default_value = "0"
        )]
        birthday: u16,
    },
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    match cli.command {
        Commands::CreateAddress {
            password,
            output_file,
        } => {
            println!("Creating new address...");
            let seeds = CipherSeed::random();
            let birthday = seeds.birthday();
            let seed_words = seeds
                .to_mnemonic(MnemonicLanguage::English, None)?
                .join(" ");
            let key_manager: TransactionKeyManagerWrapper<MemoryKeyManagerBackend> =
                TransactionKeyManagerWrapper::new(
                    Some(seeds),
                    CryptoFactories::default(),
                    Arc::new(WalletType::DerivedKeys),
                )
                .await?;

            let view_key = key_manager.get_private_view_key().await?;
            let spend_key = key_manager.get_spend_key().await?;

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
                let key = Key::from_slice(password.as_bytes());
                let cipher = XChaCha20Poly1305::new(key);

                let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
                let encrypted_view_key = cipher.encrypt(&nonce, view_key.as_bytes())?;
                let encrypted_spend_key = cipher.encrypt(&nonce, spend_key.pub_key.as_bytes())?;

                let encrypted_seed_words =
                    cipher.encrypt(&nonce, seed_words.reveal().as_bytes())?;

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
        }
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
        }
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
        }
        Commands::Balance {
            database_file,
            account_name,
        } => {
            println!("Fetching balance...");
            let _ = handle_balance(&database_file, account_name.as_deref()).await;
            Ok(())
        }
    }
}

async fn handle_balance(
    database_file: &str,
    account_name: Option<&str>,
) -> Result<(), anyhow::Error> {
    let db = init_db(database_file).await?;
    let accounts = get_accounts(&db, account_name).await?;
    for account in accounts {
        struct AccountBalance {
            total_credits: Option<i64>,
            total_debits: Option<i64>,
            max_height: Option<i64>,
            max_date: Option<String>,
        }
        println!("Account: {}", account.friendly_name);
        let agg_result = sqlx::query_as!(
            AccountBalance,
            r#"
            SELECT 
              SUM(balance_credit) as "total_credits: _",
              Sum(balance_debit) as "total_debits: _",
              max(effective_height) as "max_height: _",
              strftime('%Y-%m-%d %H:%M:%S', max(effective_date))  as "max_date: _"
            FROM balance_changes
            WHERE account_id = ?
            "#,
            account.id
        )
        .fetch_one(&db)
        .await?;

        // if let Some(agg) = agg_result {
        let micro_tari_balance =
            (agg_result.total_credits.unwrap_or(0) - agg_result.total_debits.unwrap_or(0)) as f64;
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
) -> Result<Vec<WalletEvent>, anyhow::Error> {
    let db = init_db(database_file).await?;
    let mut result = vec![];
    for account in get_accounts(&db, account_name).await? {
        println!("Found account: {:?}", account);

        let (view_key, spend_key) = decrypt_keys(&account, password)?;
        let key_manager = KeyManagerBuilder::default()
            .with_view_key_and_spend_key(view_key, spend_key, account.birthday as u16)
            .try_build()
            .await?;
        let mut scanner =
            HttpBlockchainScanner::new(base_url.to_string(), key_manager.clone()).await?;

        let last_blocks = db::get_scanned_tip_blocks_by_account(&db, account.id).await?;

        let mut start_height = 0;
        if last_blocks.is_empty() {
            println!(
                "No previously scanned blocks found for account {}, starting from genesis.",
                account.friendly_name
            );
        } else {
            println!(
                "Found {} previously scanned blocks for account {}",
                last_blocks.len(),
                account.friendly_name
            );
            let reorged_blocks = check_for_reorgs(&mut scanner, &db, &last_blocks).await?;
            if reorged_blocks.len() == last_blocks.len() {
                println!("All previously scanned blocks have been reorged, starting from genesis.");

                todo!("Need to remove outputs that are no longer valid.");
            } else if !reorged_blocks.is_empty() {
                println!(
                    "Removed {} reorged blocks from the database.",
                    reorged_blocks.len()
                );
                start_height = (reorged_blocks.iter().map(|b| b.height).min().unwrap_or(0) as u64)
                    .saturating_sub(1);
            } else {
                println!("No reorgs detected.");
                start_height = last_blocks[0].height as u64 + 1;
            }
        }
        if start_height == 0 {
            let birthday_day =
                (account.birthday as u64) * 24 * 60 * 60 + BIRTHDAY_GENESIS_FROM_UNIX_EPOCH;
            println!(
                "Calculating birthday day from birthday {} to be {} (unix epoch)",
                account.birthday, birthday_day
            );
            let estimate_birthday_block = (birthday_day.saturating_sub(MAINNET_GENESIS_DATE)) / 120; // 2 minute blocks
            println!(
                "Estimating birthday block height from birthday {} to be {}",
                account.birthday, estimate_birthday_block
            );
            start_height = estimate_birthday_block;
        }
        let mut total_scanned = 0;
        loop {
            if total_scanned >= max_blocks {
                break;
            }
            let max_remaining = max_blocks - total_scanned;
            let scan_config = ScanConfig::default()
                .with_start_height(start_height)
                .with_end_height(start_height.saturating_add(max_remaining.min(batch_size)));
            println!(
                "Scanning blocks {} to {:?}...",
                scan_config.start_height, scan_config.end_height
            );
            let timer = Instant::now();
            let scanned_blocks = scanner.scan_blocks(scan_config).await?;
            println!(
                "Scan took {:?}, on average: {} per second. Total outputs found: {}",
                timer.elapsed(),
                scanned_blocks.len() as f64 / timer.elapsed().as_secs_f64(),
                scanned_blocks
                    .iter()
                    .map(|b| b.wallet_outputs.len())
                    .sum::<usize>()
            );

            total_scanned += scanned_blocks.len() as u64;
            if scanned_blocks.is_empty() {
                println!("No more blocks to scan.");
                break;
            }
            start_height = scanned_blocks.last().unwrap().height + 1;
            println!(
                "Scanned {} blocks, total scanned: {}",
                scanned_blocks.len(),
                total_scanned
            );
            for scanned_block in &scanned_blocks {
                // Deleted inputs
                for input in &scanned_block.inputs {
                    if let Some((output_id, value)) =
                        db::get_output_info_by_hash(&db, input.as_slice()).await?
                    {
                        let (input_id, inserted_new_input) = db::insert_input(
                            &db,
                            account.id,
                            output_id,
                            scanned_block.height as u64,
                            scanned_block.block_hash.as_slice(),
                            scanned_block.mined_timestamp,
                        )
                        .await?;

                        if inserted_new_input {
                            let balance_change = BalanceChange {
                                account_id: account.id,
                                caused_by_output_id: None,
                                caused_by_input_id: Some(input_id),
                                description: format!("Output spent as input"),
                                balance_credit: 0,
                                balance_debit: value,
                                effective_date: chrono::NaiveDateTime::from_timestamp(
                                    scanned_block.mined_timestamp as i64,
                                    0,
                                ),
                                effective_height: scanned_block.height as u64,
                                claimed_recipient_address: None,
                                claimed_sender_address: None,
                                memo_hex: None,
                                memo_parsed: None,
                                claimed_fee: None,
                                claimed_amount: None,
                            };
                            db::insert_balance_change(&db, &balance_change).await?;
                        }
                    }
                }

                // println!(
                //     "Scanned block at height: {}, hash: {:x?}",
                //     scanned_block.height, scanned_block.block_hash
                // );
                // dbg!(&scanned_block);
                for (hash, output) in &scanned_block.wallet_outputs {
                    println!(
                        "Detected output with amount {} at height {}",
                        output.value(),
                        scanned_block.height
                    );

                    // Extract memo information
                    let payment_info = output.payment_id();
                    let memo_bytes = payment_info.get_payment_id();
                    let memo_parsed = if memo_bytes.is_empty() {
                        None
                    } else {
                        Some(String::from_utf8_lossy(&memo_bytes).to_string())
                    };
                    let memo_hex = if memo_bytes.is_empty() {
                        None
                    } else {
                        Some(hex::encode(&memo_bytes))
                    };

                    let event = models::WalletEvent {
                        id: 0,
                        account_id: account.id,
                        event_type: models::WalletEventType::OutputDetected {
                            hash: hash.clone(),
                            block_height: scanned_block.height,
                            block_hash: scanned_block.block_hash.to_vec(),
                            memo_parsed: memo_parsed.clone(),
                            memo_hex: memo_hex.clone(),
                        },
                        description: format!(
                            "Detected output with amount {} at height {}",
                            output.value(),
                            scanned_block.height
                        ),
                    };
                    result.push(event.clone());
                    let (output_id, inserted_new_output) = db::insert_output(
                        &db,
                        account.id,
                        hash.to_vec().clone(),
                        output,
                        scanned_block.height,
                        scanned_block.block_hash.as_slice(),
                        scanned_block.mined_timestamp,
                        memo_parsed.clone(),
                        memo_hex.clone(),
                    )
                    .await?;

                    if inserted_new_output {
                        db::insert_wallet_event(&db, account.id, &event).await?;

                        // parse balance changes.
                        let balance_changes = parse_balance_changes(
                            account.id,
                            output_id,
                            scanned_block.mined_timestamp,
                            scanned_block.height,
                            &output,
                        );
                        for change in balance_changes {
                            db::insert_balance_change(&db, &change).await?;
                        }
                    }
                }
                db::insert_scanned_tip_block(
                    &db,
                    account.id,
                    scanned_block.height as i64,
                    scanned_block.block_hash.as_slice(),
                )
                .await?;

                // Check for outputs that should be confirmed (6 block confirmations)
                let unconfirmed_outputs = db::get_unconfirmed_outputs(
                    &db,
                    account.id,
                    scanned_block.height,
                    6, // 6 block confirmations required
                )
                .await?;

                for (output_hash, original_height, memo_parsed, memo_hex) in unconfirmed_outputs {
                    let confirmation_event = models::WalletEvent {
                        id: 0,
                        account_id: account.id,
                        event_type: models::WalletEventType::OutputConfirmed {
                            hash: output_hash.clone(),
                            block_height: original_height,
                            confirmation_height: scanned_block.height,
                            memo_parsed,
                            memo_hex,
                        },
                        description: format!(
                            "Output confirmed at height {} (originally at height {})",
                            scanned_block.height, original_height
                        ),
                    };

                    result.push(confirmation_event.clone());
                    db::insert_wallet_event(&db, account.id, &confirmation_event).await?;

                    // Mark the output as confirmed in the database
                    db::mark_output_confirmed(
                        &db,
                        &output_hash,
                        scanned_block.height,
                        scanned_block.block_hash.as_slice(),
                    )
                    .await?;

                    println!(
                        "Output {:?} confirmed at height {} (originally at height {})",
                        hex::encode(&output_hash),
                        scanned_block.height,
                        original_height
                    );
                }
            }

            println!("Batch took {:?}.", timer.elapsed());
            println!("deleting old scanned tip blocks...");
            delete_old_scanned_tip_blocks(&db, account.id, 50).await?;

            println!("Cleanup took {:?}.", timer.elapsed());
        }

        println!("Scan complete.");
    }
    Ok(result)
}

fn parse_balance_changes(
    account_id: i64,
    output_id: i64,
    chain_timestamp: u64,
    chain_height: u64,
    output: &lightweight_wallet_libs::transaction_components::WalletOutput,
) -> Vec<models::BalanceChange> {
    // Coinbases.
    if output.features().is_coinbase() {
        let effective_date = chrono::NaiveDateTime::from_timestamp(chain_timestamp as i64, 0);
        let balance_change = models::BalanceChange {
            account_id,
            caused_by_output_id: Some(output_id),
            caused_by_input_id: None,
            description: "Coinbase output found in blockchain scan".to_string(),
            balance_credit: output.value().as_u64(),
            balance_debit: 0,
            effective_date,
            effective_height: chain_height,
            claimed_recipient_address: None,
            memo_hex: None,
            memo_parsed: None,
            claimed_sender_address: None,
            claimed_fee: None,
            claimed_amount: None,
        };
        return vec![balance_change];
    }

    let mut changes = vec![];
    let effective_date = chrono::NaiveDateTime::from_timestamp(chain_timestamp as i64, 0);
    let payment_info = output.payment_id();
    let memo_bytes = payment_info.get_payment_id();
    let memo = String::from_utf8_lossy(&memo_bytes);
    let memo_hex = hex::encode(payment_info.get_payment_id());
    let claimed_recipient_address = payment_info.get_recipient_address().map(|s| s.to_base58());
    let claimed_sender_address = payment_info.get_sender_address().map(|s| s.to_base58());
    let claimed_fee = payment_info.get_fee().map(|v| v.0);
    let claimed_amount = payment_info.get_amount().map(|v| v.0);

    let balance_change = models::BalanceChange {
        account_id,
        caused_by_output_id: Some(output_id),
        caused_by_input_id: None,
        description: "Output found in blockchain scan".to_string(),
        balance_credit: output.value().as_u64(),
        balance_debit: 0,
        effective_date,
        effective_height: chain_height,
        claimed_recipient_address: claimed_recipient_address,
        claimed_sender_address: claimed_sender_address,
        memo_parsed: Some(memo.to_string()),
        memo_hex: Some(memo_hex),
        claimed_fee,
        claimed_amount,
    };
    changes.push(balance_change);
    changes
}

/// Returns (removed_blocks, added_blocks   )
async fn check_for_reorgs(
    scanner: &mut HttpBlockchainScanner<TransactionKeyManagerWrapper<MemoryKeyManagerBackend>>,
    db: &sqlx::SqlitePool,
    last_blocks_in_desc: &[models::ScannedTipBlock],
) -> Result<Vec<models::ScannedTipBlock>, anyhow::Error> {
    let mut removed_blocks = vec![];
    for block in last_blocks_in_desc {
        let chain_block = scanner.get_header_by_height(block.height).await?;
        if let Some(chain_block) = chain_block {
            if chain_block.hash == block.hash {
                // Block matches, no reorg at this height.
                break;
            } else {
                println!(
                    "REORG DETECTED at height {}, updating record.",
                    block.height
                );
                removed_blocks.push(block.clone());
                // If the block hash has changed, delete the old record.
                sqlx::query!(
                    r#"
                    DELETE FROM scanned_tip_blocks
                    WHERE id = ?
                    "#,
                    block.id
                )
                .execute(db)
                .await?;
            }
        } else {
            println!(
                "Block at height {} no longer exists in the chain, reorg detected.",
                block.height
            );
            removed_blocks.push(block.clone());
            // Handle the reorg as needed (e.g., delete affected records, rescan, etc.).
            continue;
        }

        // Fetch the block from the blockchain to verify its hash.
        // For simplicity, we'll just print out the block info here.
        println!(
            "Verifying block at height: {}, hash: {:x?}",
            block.height, block.hash
        );
        // In a real implementation, you would fetch the block from the blockchain
        // and compare its hash to `block.hash`. If they differ, a reorg has occurred.
        // Handle the reorg as needed (e.g., delete affected records, rescan, etc.).
    }
    Ok(removed_blocks)
}

fn decrypt_keys(
    account_row: &AccountRow,
    password: &str,
) -> Result<(RistrettoSecretKey, CompressedKey<RistrettoPublicKey>), anyhow::Error> {
    let password = if password.len() < 32 {
        format!("{:0<32}", password)
    } else {
        password[..32].to_string()
    };
    let key = Key::from_slice(password.as_bytes());
    let cipher = XChaCha20Poly1305::new(key);

    let nonce = XNonce::clone_from_slice(account_row.cipher_nonce.as_ref());

    let view_key = cipher.decrypt(&nonce, account_row.encrypted_view_private_key.as_ref())?;
    let spend_key = cipher.decrypt(&nonce, account_row.encrypted_spend_public_key.as_ref())?;

    let view_key =
        RistrettoSecretKey::from_canonical_bytes(&view_key).map_err(|e| anyhow::anyhow!(e))?;
    let spend_key = CompressedKey::<RistrettoPublicKey>::from_canonical_bytes(&spend_key)
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok((view_key, spend_key))
}

async fn init_with_view_key(
    view_private_key: &str,
    spend_public_key: &str,
    password: &str,
    database_file: &str,
    birthday: u16,
) -> Result<(), anyhow::Error> {
    // if std::fs::metadata(database_file).is_ok() {
    //     return Err(anyhow::anyhow!(
    //         "Wallet already exists. Aborting import to avoid overwriting existing wallet."
    //     ));
    // }
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

    // let wallet_data = serde_json::json!({
    //     "encrypted_view_key": hex::encode(encrypted_view_key),
    //     "encrypted_spend_key": hex::encode(encrypted_spend_key),
    //     "nonce": hex::encode(nonce),
    // };

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

    // std::fs::write(
    //     "data/wallet.json",
    //     serde_json::to_string_pretty(&wallet_data)?,
    // )?;

    Ok(())
}

fn hash_view_key(view_key: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2s256::new();
    hasher.update(b"view_key_hash");
    hasher.update(view_key);
    hasher.finalize().to_vec()
}
