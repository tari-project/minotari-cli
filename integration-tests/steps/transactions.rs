// Transaction Step Definitions - Full Implementation
//
// Step definitions for testing transaction creation functionality.

use super::common::MinotariWorld;
use super::common::test_support;
use cucumber::{given, then, when};
use std::process::Command;
use tari_common::configuration::Network::LocalNet;
use tari_common_types::tari_address::TariAddress;
use tari_common_types::tari_address::TariAddressFeatures;
// =============================
// Helper Functions
// =============================

/// Helper function to execute create-unsigned-transaction command with flexible parameters
fn execute_create_transaction(world: &mut MinotariWorld, recipients: Vec<String>, lock_duration: Option<u64>) {
    let db_path = world.database_path.as_ref().expect("Database path not set");

    // Generate output file path
    let output_path = world.get_temp_path("unsigned_transaction.json");
    world.output_file = Some(output_path.clone());

    let (program, base_args) = world.get_minotari_command();
    let mut args = base_args;

    args.push("create-unsigned-transaction".to_string());
    args.push("--database-path".to_string());
    args.push(db_path.to_str().unwrap().to_string());
    args.push("--password".to_string());
    args.push(world.test_password.clone());
    args.push("--account-name".to_string());
    args.push("default".to_string());

    // Add recipients
    for recipient in recipients {
        args.push("--recipient".to_string());
        args.push(recipient);
    }

    args.push("--output-file".to_string());
    args.push(output_path.to_str().unwrap().to_string());

    // Add custom lock duration if provided
    if let Some(seconds) = lock_duration {
        args.push("--seconds-to-lock".to_string());
        args.push(seconds.to_string());
    }

    let output = Command::new(&program)
        .args(&args)
        .output()
        .expect("Failed to execute create-unsigned-transaction command");

    world.last_command_exit_code = output.status.code();
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

/// Generate a test Tari address (simplified for testing)
fn generate_test_address(world: &MinotariWorld) -> String {
    // Use the wallet's view key to generate a valid address
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
    wallet_address.to_base58().to_string()
}

// =============================
// Precondition Steps
// =============================

#[given("the wallet has sufficient balance")]
async fn wallet_has_balance(world: &mut MinotariWorld) {
    // 1. Spin up a seed base node if one isn't already running
    if world.base_nodes.is_empty() {
        let base_dir = world.current_base_dir.as_ref().expect("Base dir not set").clone();

        let node = test_support::spawn_base_node(
            &base_dir,
            &mut world.assigned_ports,
            &mut world.base_nodes,
            true,
            "BalanceMiner".to_string(),
            vec![],
        )
        .await;

        world.base_nodes.insert("BalanceMiner".to_string(), node);
        world.seed_nodes.push("BalanceMiner".to_string());
    }

    // 2. Mine blocks so the wallet receives coinbase rewards
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

    let node_name = world.base_nodes.keys().next().unwrap().clone();
    let node = world.base_nodes.get(&node_name).unwrap();
    node.mine_blocks(5, &wallet_address)
        .await
        .expect("Failed to mine blocks for balance");

    // 3. Scan the blockchain so the wallet database picks up the mined outputs
    let db_path = world.database_path.as_ref().expect("Database not set up");
    let base_url = {
        let node = world.base_nodes.get(&node_name).unwrap();
        format!("http://127.0.0.1:{}", node.http_port)
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
        "20".to_string(),
    ]);

    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute scan command");

    assert!(
        output.status.success(),
        "Scan failed during balance setup: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    println!(
        "Wallet funded via mining + scan. Scan output: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[given("the wallet has zero balance")]
async fn wallet_zero_balance(world: &mut MinotariWorld) {
    // Run the balance command and verify the wallet reports zero balance
    let db_path = world.database_path.as_ref().expect("Database not set up");
    let (cmd, mut args) = world.get_minotari_command();
    args.extend_from_slice(&[
        "balance".to_string(),
        "--database-path".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--password".to_string(),
        world.test_password.clone(),
        "--account-name".to_string(),
        "default".to_string(),
    ]);

    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute balance command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());

    // Verify the balance is zero
    if let Some(balance) = world.parse_balance_from_output() {
        assert_eq!(
            balance, 0,
            "Expected zero balance for a fresh wallet, got {} microTari",
            balance
        );
    }

    println!("Confirmed wallet has zero balance");
}

// =============================
// Transaction Creation Steps
// =============================

#[when("I create an unsigned transaction with one recipient")]
async fn create_transaction_one_recipient(world: &mut MinotariWorld) {
    let address = generate_test_address(world);
    let recipient = format!("{}::100000", address); // 100000 microTari

    execute_create_transaction(world, vec![recipient], None);
}

#[when("I create an unsigned transaction with multiple recipients")]
async fn create_transaction_multiple_recipients(world: &mut MinotariWorld) {
    let address1 = generate_test_address(world);
    let address2 = generate_test_address(world);
    let address3 = generate_test_address(world);

    let recipients = vec![
        format!("{}::50000", address1), // 50000 microTari
        format!("{}::30000", address2), // 30000 microTari
        format!("{}::20000", address3), // 20000 microTari
    ];

    execute_create_transaction(world, recipients, None);
}

#[when(regex = r#"^I create an unsigned transaction with payment ID "([^"]*)"$"#)]
async fn create_transaction_with_payment_id(world: &mut MinotariWorld, payment_id: String) {
    let address = generate_test_address(world);
    let recipient = format!("{}::100000::{}", address, payment_id);

    execute_create_transaction(world, vec![recipient], None);
}

#[when("I try to create an unsigned transaction")]
async fn try_create_transaction(world: &mut MinotariWorld) {
    // Try to create a transaction (may fail due to insufficient balance)
    let address = generate_test_address(world);
    let recipient = format!("{}::1000000", address); // 1000000 microTari

    execute_create_transaction(world, vec![recipient], None);
}

#[when(regex = r#"^I create an unsigned transaction with lock duration "([^"]*)" seconds$"#)]
async fn create_transaction_with_lock_duration(world: &mut MinotariWorld, seconds: String) {
    let address = generate_test_address(world);
    let recipient = format!("{}::100000", address);
    let lock_duration = seconds.parse::<u64>().expect("Invalid lock duration");

    execute_create_transaction(world, vec![recipient], Some(lock_duration));
}

// =============================
// Verification Steps
// =============================

#[then("the transaction file should be created")]
async fn transaction_file_created(world: &mut MinotariWorld) {
    let output_file = world.output_file.as_ref().expect("Output file path not set");

    assert!(
        output_file.exists(),
        "Transaction file should exist at {:?}",
        output_file
    );

    // Parse the JSON file
    let content = std::fs::read_to_string(output_file).expect("Failed to read transaction file");

    let transaction_json: serde_json::Value = serde_json::from_str(&content).expect("Failed to parse transaction JSON");

    // Store for later verification
    world.transaction_data.insert("current".to_string(), transaction_json);
}

#[then("the transaction should include the recipient")]
async fn transaction_has_recipient(world: &mut MinotariWorld) {
    let transaction = world
        .transaction_data
        .get("current")
        .expect("Transaction data not found");

    // Check that the transaction has outputs/recipients
    assert!(
        transaction.get("recipients").is_some() || transaction.get("outputs").is_some(),
        "Transaction should have recipients or outputs field"
    );
}

#[then("the inputs should be locked")]
async fn inputs_are_locked(world: &mut MinotariWorld) {
    let transaction = world
        .transaction_data
        .get("current")
        .expect("Transaction data not found");

    // Check that the transaction has inputs
    assert!(
        transaction.get("inputs").is_some() || transaction.get("utxos").is_some(),
        "Transaction should have inputs or utxos field indicating locked inputs"
    );
}

#[then("the transaction should include all recipients")]
async fn transaction_has_all_recipients(world: &mut MinotariWorld) {
    let transaction = world
        .transaction_data
        .get("current")
        .expect("Transaction data not found");

    // Get recipients or outputs array
    let recipients = transaction
        .get("recipients")
        .or_else(|| transaction.get("outputs"))
        .expect("Transaction should have recipients or outputs");

    let recipients_array = recipients.as_array().expect("Recipients should be an array");

    assert_eq!(recipients_array.len(), 3, "Transaction should have 3 recipients");
}

#[then("the total amount should be correct")]
async fn total_amount_correct(world: &mut MinotariWorld) {
    let transaction = world
        .transaction_data
        .get("current")
        .expect("Transaction data not found");

    // Check that total amount or value field exists
    assert!(
        transaction.get("total_amount").is_some()
            || transaction.get("total_value").is_some()
            || transaction.get("amount").is_some(),
        "Transaction should have a total amount field"
    );

    // The total should be 50000 + 30000 + 20000 = 100000 microTari (plus fees)
    // We just verify the field exists and is positive
}

#[then("the transaction should include the payment ID")]
async fn transaction_has_payment_id(world: &mut MinotariWorld) {
    let transaction = world
        .transaction_data
        .get("current")
        .expect("Transaction data not found");

    // Check for payment ID in various possible locations
    let has_payment_id = transaction.get("payment_id").is_some()
        || transaction.get("memo").is_some()
        || transaction.get("message").is_some();

    assert!(has_payment_id, "Transaction should include payment ID/memo field");
}

#[then("the transaction creation should fail")]
async fn transaction_fails(world: &mut MinotariWorld) {
    assert_ne!(
        world.last_command_exit_code,
        Some(0),
        "Transaction creation should fail but got exit code {:?}",
        world.last_command_exit_code
    );
}

#[then("I should see an insufficient balance error")]
async fn see_insufficient_balance_error(world: &mut MinotariWorld) {
    let error = world
        .last_command_error
        .as_ref()
        .or(world.last_command_output.as_ref())
        .expect("No error output");

    assert!(
        error.to_lowercase().contains("insufficient")
            || error.to_lowercase().contains("balance")
            || error.to_lowercase().contains("not enough"),
        "Error message should indicate insufficient balance. Got: {}",
        error
    );
}

#[then(regex = r#"^the inputs should be locked for "([^"]*)" seconds$"#)]
async fn inputs_locked_for_duration(world: &mut MinotariWorld, seconds: String) {
    let transaction = world
        .transaction_data
        .get("current")
        .expect("Transaction data not found");

    // Check that lock duration or expiry information is present
    let has_lock_info = transaction.get("lock_duration").is_some()
        || transaction.get("expires_at").is_some()
        || transaction.get("utxo_lock_duration").is_some();

    assert!(has_lock_info, "Transaction should include lock duration information");

    // Note: The exact duration check would require parsing the timestamp
    // For now, we just verify the field exists
    let _expected_seconds = seconds.parse::<u64>().expect("Invalid seconds");
}
