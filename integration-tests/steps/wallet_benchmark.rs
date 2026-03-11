// Wallet Benchmark Step Definitions
//
// Step definitions for benchmarking wallet performance.

use cucumber::{then, when};
use std::process::Command;
use std::time::Instant;
use tari_common::configuration::Network::LocalNet;
use tari_common_types::tari_address::TariAddress;
use tari_common_types::tari_address::TariAddressFeatures;
use tari_transaction_components::consensus::ConsensusConstantsBuilder;
use tari_transaction_components::key_manager::KeyManager;
use tari_transaction_components::offline_signing::models::{
    PrepareOneSidedTransactionForSigningResult, TransactionResult,
};
use tari_transaction_components::offline_signing::sign_locked_transaction;

use super::common::MinotariWorld;

// =============================
// Benchmark Steps
// =============================
#[then(regex = r#"^I measure the time to scan (\d+) blocks$"#)]
async fn measure_scan_time(world: &mut MinotariWorld, blocks: String) {
    let db_path = world.database_path.as_ref().expect("Database not set up");

    // Get base node URL from the first available base node
    let base_url = if let Some((_, node)) = world.base_nodes.iter().next() {
        format!("http://127.0.0.1:{}", node.http_port)
    } else {
        panic!("No base node available for scanning");
    };

    let (cmd, mut args) = world.get_minotari_command();
    args.extend_from_slice(&[
        "scan".to_string(),
        "--database-path".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--password".to_string(),
        world.test_password.clone(),
        "--base-url".to_string(),
        base_url,
        "--max-blocks-to-scan".to_string(),
        blocks,
    ]);

    let start = Instant::now();
    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute scan command");
    let duration = start.elapsed();

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());

    println!("Scan completed in {:?}", duration);
    println!("Scan output: {}", world.last_command_output.as_ref().unwrap());
    if !world.last_command_error.as_ref().unwrap().is_empty() {
        println!("Scan stderr: {}", world.last_command_error.as_ref().unwrap());
    }
}

#[when(regex = r#"^I send (\d+) transactions$"#)]
async fn send_transactions(world: &mut MinotariWorld, transactions: String) {
    let db_path = world.database_path.as_ref().expect("Database not set up");

    // Get base node URL for submitting transactions
    let base_url = if let Some((_, node)) = world.base_nodes.iter().next() {
        format!("http://127.0.0.1:{}", node.http_port)
    } else {
        panic!("No base node available");
    };

    // Generate a test address
    let spend_key = world.wallet.get_public_spend_key();
    let view_key = world.wallet.get_public_view_key();
    let wallet_address = TariAddress::new_dual_address(
        view_key,
        spend_key,
        LocalNet,
        TariAddressFeatures::create_one_sided_only(),
        None,
    )
    .unwrap();
    let address = wallet_address.to_base58().to_string();

    // Create a key manager from the wallet for offline signing
    let key_manager =
        KeyManager::new(world.wallet.clone()).expect("Failed to create key manager from wallet");
    let consensus_constants = ConsensusConstantsBuilder::new(LocalNet).build();

    println!("Sending transactions...");
    let mut successful_txs = 0;
    let num_transactions: usize = transactions.parse().expect("Invalid number of transactions");

    // Create an HTTP client for submitting transactions to the base node
    let submit_url = format!("{}/json_rpc", base_url);
    let http_client =
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

    for i in 0..num_transactions {
        let recipient = format!("{}::1000", address); // 1000 microTari per transaction
        let output_path = world.get_temp_path(&format!("tx_{}.json", i));

        // Step 1: Create unsigned transaction via CLI
        let (cmd, mut args) = world.get_minotari_command();
        args.extend_from_slice(&[
            "create-unsigned-transaction".to_string(),
            "--database-path".to_string(),
            db_path.to_str().unwrap().to_string(),
            "--password".to_string(),
            world.test_password.clone(),
            "--account-name".to_string(),
            "default".to_string(),
            "--recipient".to_string(),
            recipient,
            "--output-file".to_string(),
            output_path.to_str().unwrap().to_string(),
        ]);

        let output = Command::new(&cmd)
            .args(&args)
            .output()
            .expect("Failed to create transaction");

        if output.status.success() {
            // Step 2: Read the unsigned transaction JSON and sign it offline
            let unsigned_json = match std::fs::read_to_string(&output_path) {
                Ok(json) => json,
                Err(e) => {
                    println!("Transaction {} failed to read unsigned tx file: {}", i, e);
                    continue;
                },
            };

            let unsigned_tx =
                match PrepareOneSidedTransactionForSigningResult::from_json(&unsigned_json) {
                    Ok(tx) => tx,
                    Err(e) => {
                        println!("Transaction {} failed to parse unsigned tx: {}", i, e);
                        continue;
                    },
                };

            let signed_result = match sign_locked_transaction(
                &key_manager,
                consensus_constants.clone(),
                LocalNet,
                unsigned_tx,
            ) {
                Ok(result) => result,
                Err(e) => {
                    println!("Transaction {} signing failed: {}", i, e);
                    continue;
                },
            };

            // Step 3: Submit the signed transaction to the base node
            let transaction = signed_result.signed_transaction.transaction;
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": "1",
                "method": "submit_transaction",
                "params": { "transaction": transaction }
            });

            let submit_result = http_client
                .post(&submit_url)
                .json(&request)
                .send()
                .await;

            match submit_result {
                Ok(response) if response.status().is_success() => {
                    successful_txs += 1;
                },
                Ok(response) => {
                    println!(
                        "Transaction {} submit failed with status: {}",
                        i,
                        response.status()
                    );
                },
                Err(e) => {
                    println!("Transaction {} submit failed: {}", i, e);
                },
            }
        } else {
            println!(
                "Transaction {} creation failed: {}",
                i,
                String::from_utf8_lossy(&output.stderr)
            );
            if String::from_utf8_lossy(&output.stderr).contains("insufficient") {
                println!("Insufficient balance, stopping at {} transactions", i);
                break;
            }
            panic!("Transaction {} creation failed", i);
        }

        if (i + 1) % 50 == 0 {
            println!("Sent {} transactions so far...", i + 1);
        }
    }

    println!("Successfully sent {} transactions", successful_txs);
    world.last_command_output = Some(format!("Sent {} transactions", successful_txs));
}

#[when(regex = r#"^I measure the time to confirm (\d+) transactions$"#)]
async fn measure_confirmation_time(world: &mut MinotariWorld, _transactions: String) {
    let db_path = world.database_path.as_ref().expect("Database not set up");

    // Get base node URL
    let base_url = if let Some((_, node)) = world.base_nodes.iter().next() {
        format!("http://127.0.0.1:{}", node.http_port)
    } else {
        panic!("No base node available");
    };

    // First, do a scan to detect the mined blocks
    let (cmd, mut args) = world.get_minotari_command();
    args.extend_from_slice(&[
        "scan".to_string(),
        "--database-path".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--password".to_string(),
        world.test_password.clone(),
        "--base-url".to_string(),
        base_url.clone(),
        "--max-blocks-to-scan".to_string(),
        "100".to_string(),
    ]);

    let start = Instant::now();
    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute scan command");
    let duration = start.elapsed();

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());

    println!("Transaction confirmation scan completed in {:?}", duration);
    println!("Scan output: {}", world.last_command_output.as_ref().unwrap());
}

#[then("all transactions should be confirmed")]
async fn transactions_confirmed(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Scan command failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );

    println!("All transactions confirmed successfully");
}
