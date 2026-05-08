// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use anyhow::{Context, anyhow};
use tari_transaction_components::key_manager::{
    KeyManager, TransactionKeyManagerInterface,
    wallet_types::{SeedWordsWallet, WalletType},
};

use crate::{
    db::init_db,
    migrate::{ConsoleDb, MigrationOptions, run_migration},
};

pub async fn handle_migrate_from_console_wallet(
    source_db: PathBuf,
    source_password: String,
    account_name: String,
    dry_run: bool,
    allow_partial_import: bool,
    password: String,
    database_path: PathBuf,
) -> anyhow::Result<()> {
    if source_db == database_path {
        return Err(anyhow!("Source and destination database paths must be different"));
    }

    let result = tokio::task::spawn_blocking(move || {
        let (console_db, cipher_seed) =
            ConsoleDb::open(&source_db, &source_password).context("Failed to open source console wallet")?;
        let seed_wallet = SeedWordsWallet::construct_new(cipher_seed.clone())
            .map_err(|e| anyhow!("Failed to reconstruct source seed wallet: {}", e))?;
        let wallet = WalletType::SeedWords(seed_wallet);
        let key_manager = KeyManager::new(wallet).map_err(|e| anyhow!("Failed to build source key manager: {}", e))?;
        let account_view_key = key_manager.get_private_view_key();
        let dest_pool = init_db(database_path).context("Failed to initialize destination DB")?;

        run_migration(
            &console_db,
            &cipher_seed,
            &dest_pool,
            MigrationOptions {
                account_name: &account_name,
                password: &password,
                dry_run,
                allow_partial_import,
                account_view_key: &account_view_key,
            },
        )
    })
    .await
    .map_err(|e| anyhow!("Migration task failed: {}", e))?;

    result.map(|_| ())
}
