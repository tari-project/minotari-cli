use blake2::{Blake2s256, Digest};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::aead::rand_core::le;
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, ChaChaPoly1305, KeyInit, aead::OsRng};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use clap::{Parser, Subcommand};
use lightweight_wallet_libs::{BlockScanResult, BlockchainScanner};
use lightweight_wallet_libs::{
    HttpBlockchainScanner, KeyManagerBuilder, ScanConfig, ScannerBuilder,
};
use std::env::current_dir;
use std::time::Instant;
use tari_crypto::compressed_key::CompressedKey;
use tari_crypto::ristretto::{RistrettoPublicKey, RistrettoSecretKey};

use crate::db::{AccountRow, delete_old_scanned_tip_blocks, get_accounts, init_db};
use crate::models::WalletEvent;
use tari_utilities::byte_array::ByteArray;
mod db;
mod models;

pub const BIRTHDAY_GENESIS_FROM_UNIX_EPOCH: u64 = 1640995200;
pub const MAINNET_GENESIS_DATE: u64 = 1746940800;

#[derive(Parser)]
#[command(name = "tari")]
#[command(about = "Tari wallet CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan the blockchain for transactions
    Scan {
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        #[arg(
            short,
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
    Balance,
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
            println!("Scan complete. Events: {:?}", events);
            Ok(())
            // Add scanning logic here
        }
        Commands::Balance => {
            println!("Fetching balance...");
            // Add balance checking logic here
            Ok(())
        }
    }
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

        // load wallet file
        // let (view_key, spend_key) = decrypt_keys("data/wallet.json", password)?;
        // let (view_key, spend_key) = decrypt_keys(&account, password)?;

        // println!("Decrypted view key: {:x?}", view_key);
        // println!("Decrypted spend key: {:x?}", spend_key);

        // // let scanner = ScannerBuilder::default().build()?;
        // let key_manager = KeyManagerBuilder::default().try_build().await?;
        // let mut scanner =
        //     HttpBlockchainScanner::new(base_url.to_string(), key_manager.clone()).await?;

        // Get last scanned blocks.

        //  Scan for reorgs.

        let mut last_blocks = db::get_scanned_tip_blocks_by_account(&db, account.id).await?;

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
            last_blocks.sort_by(|a, b| b.height.cmp(&a.height));
            let reorged_blocks =
                check_for_reorgs(&last_blocks, &account, password, base_url, &db).await?;
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
            let estimate_birthday_block = (birthday_day - MAINNET_GENESIS_DATE) / 120; // 2 minute blocks
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
            let scan_config = ScanConfig::default()
                .with_start_height(start_height)
                .with_end_height(start_height.saturating_add(max_blocks.min(batch_size)));
            let (view_key, spend_key) = decrypt_keys(&account, password)?;
            let key_manager = KeyManagerBuilder::default()
                .with_view_key_and_spend_key(view_key, spend_key, account.birthday as u16)
                .try_build()
                .await?;
            let mut scanner =
                HttpBlockchainScanner::new(base_url.to_string(), key_manager.clone()).await?;
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
                // println!(
                //     "Scanned block at height: {}, hash: {:x?}",
                //     scanned_block.height, scanned_block.block_hash
                // );
                // dbg!(&scanned_block);
                for (hash, output) in &scanned_block.wallet_outputs {
                    println!(
                        "Detected output with amount {} at height {}",
                        output.value, scanned_block.height
                    );
                    result.push(WalletEvent {
                        id: 0,
                        account_id: account.id,
                        event_type: models::WalletEventType::OutputDetected {
                            hash: hash.clone(),
                            output: output.clone(),
                        },
                        details: format!(
                            "Detected output with amount {} at height {}",
                            output.value, scanned_block.height
                        ),
                    });
                    db::insert_output(
                        &db,
                        account.id,
                        hash.clone(),
                        output,
                        scanned_block.height,
                        &scanned_block.block_hash,
                        scanned_block.mined_timestamp,
                    )
                    .await?;
                }
                db::insert_scanned_tip_block(
                    &db,
                    account.id,
                    scanned_block.height as i64,
                    &scanned_block.block_hash,
                )
                .await?;
            }

            println!("deleting old scanned tip blocks...");
            delete_old_scanned_tip_blocks(&db, account.id, 50).await?;
        }

        println!("Scan complete.");
    }
    Ok(result)
}

/// Returns (removed_blocks, added_blocks   )
async fn check_for_reorgs(
    last_blocks_in_desc: &[models::ScannedTipBlock],
    account: &AccountRow,
    password: &str,
    base_url: &str,
    db: &sqlx::SqlitePool,
) -> Result<Vec<models::ScannedTipBlock>, anyhow::Error> {
    let (view_key, spend_key) = decrypt_keys(&account, password)?;
    let birthday = account.birthday;
    let start_height = if let Some(last_block) = last_blocks_in_desc.last() {
        last_block.height as u64
    } else {
        0
    };

    let end_height = if let Some(last_block) = last_blocks_in_desc.first() {
        last_block.height
    } else {
        0
    };

    let key_manager = KeyManagerBuilder::default()
        .with_view_key_and_spend_key(view_key, spend_key, birthday as u16)
        .try_build()
        .await?;
    let mut scanner = HttpBlockchainScanner::new(base_url.to_string(), key_manager).await?;
    // let scan_config = ScanConfig::default()
    // .with_start_height(start_height)
    // .with_end_height(end_height + 1);
    // let scanned_blocks = scanner.(scan_config).await?;

    let mut removed_blocks = vec![];
    for block in last_blocks_in_desc {
        let chain_block = scanner.get_header_by_height(block.height).await?;
        dbg!(&chain_block);
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

        // if scanned_blocks
        //     .iter()
        //     .any(|b| b.height == block.height && b.block_hash == block.hash)
        // {
        //     // Block matches, no reorg at this height.
        //     continue;
        // }

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
