use blake2::{Blake2s256, Digest};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, ChaChaPoly1305, KeyInit, aead::OsRng};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use clap::{Parser, Subcommand};
use lightweight_wallet_libs::BlockchainScanner;
use lightweight_wallet_libs::{
    HttpBlockchainScanner, KeyManagerBuilder, ScanConfig, ScannerBuilder,
};
use std::env::current_dir;

use crate::db::{AccountRow, get_accounts, init_db};

mod db;

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
            )
            .await
        }
        Commands::Scan {
            password,
            base_url,
            database_file,
            account_name,
        } => {
            println!("Scanning blockchain...");
            scan(
                &password,
                &base_url,
                &database_file,
                account_name.as_deref(),
            )
            .await
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
) -> Result<(), anyhow::Error> {
    let db = init_db(database_file).await?;
    for account in get_accounts(&db, account_name).await? {
        println!("Found account: {:?}", account);

        // load wallet file
        // let (view_key, spend_key) = decrypt_keys("data/wallet.json", password)?;
        let (view_key, spend_key) = decrypt_keys(&account, password)?;

        println!("Decrypted view key: {:x?}", view_key);
        println!("Decrypted spend key: {:x?}", spend_key);

        // let scanner = ScannerBuilder::default().build()?;
        let key_manager = KeyManagerBuilder::default().try_build().await?;
        let mut scanner =
            HttpBlockchainScanner::new(base_url.to_string(), key_manager.clone()).await?;

        let scan_config = ScanConfig::default().with_start_end_heights(0, 100);
        let outputs = scanner.scan_blocks(scan_config).await?;

        dbg!(outputs);
        // let mut scanner = ScannerBuilder::default()
        // .with_key_manager(key_manager)
        // .with_base_node_client(base_url)
        // .build()
        // .await?;
        // scanner.scan().await?;
        println!("Scan complete.");
    }
    Ok(())
}

fn decrypt_keys(
    account_row: &AccountRow,
    password: &str,
) -> Result<(Vec<u8>, Vec<u8>), anyhow::Error> {
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

    Ok((view_key, spend_key))
}

async fn init_with_view_key(
    view_private_key: &str,
    spend_public_key: &str,
    password: &str,
    database_file: &str,
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
