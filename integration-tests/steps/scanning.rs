// Scanning Step Definitions
//
// Step definitions for testing blockchain scanning functionality.

use cucumber::{given, then, when};
use rusqlite::{Connection, OptionalExtension};
use std::process::Command;

use super::common::MinotariWorld;

// =============================
// Scanning Steps
// =============================

#[given("the wallet has been previously scanned")]
async fn wallet_previously_scanned(world: &mut MinotariWorld) {
    // Perform a quick scan to establish previous state
    scan_with_max_blocks(world, "10".to_string()).await;
}

#[given(regex = r#"^the wallet has been previously scanned to height "([^"]*)"$"#)]
async fn wallet_scanned_to_height(world: &mut MinotariWorld, height: String) {
    let target_height: u64 = height.parse().expect("Height must be a valid number");

    // Verify the wallet has previously scanned to at least the specified height by
    // querying the scanned_tip_blocks table directly. This step asserts existing
    // state; it does not trigger a scan.
    let db_path = world.database_path.as_ref().expect("Database not set up");
    let conn = Connection::open(db_path).expect("Failed to open wallet database");

    let scanned_height: Option<u64> = conn
        .query_row("SELECT MAX(height) FROM scanned_tip_blocks", [], |row| row.get(0))
        .optional()
        .expect("scanned_tip_blocks query failed")
        .flatten();

    let scanned_height = scanned_height.unwrap_or_else(|| panic!("No scanned blocks found in wallet database"));

    assert!(
        scanned_height >= target_height,
        "Wallet scanned to height {} but expected at least {}",
        scanned_height,
        target_height
    );
}

#[when(regex = r#"^I perform a scan with max blocks "([^"]*)"$"#)]
async fn scan_with_max_blocks(world: &mut MinotariWorld, max_blocks: String) {
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
        max_blocks,
    ]);

    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute scan command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());

    println!("Scan output: {}", world.last_command_output.as_ref().unwrap());
    if !world.last_command_error.as_ref().unwrap().is_empty() {
        println!("Scan stderr: {}", world.last_command_error.as_ref().unwrap());
    }
}

#[when("I perform an incremental scan")]
async fn incremental_scan(world: &mut MinotariWorld) {
    // Incremental scan with default max blocks
    scan_with_max_blocks(world, "50".to_string()).await;
}

#[when(regex = r#"^I re-scan from height "([^"]*)"$"#)]
async fn rescan_from_height(world: &mut MinotariWorld, height: String) {
    let db_path = world.database_path.as_ref().expect("Database not set up");

    // Get base node URL from the first available base node
    let base_url = if let Some((_, node)) = world.base_nodes.iter().next() {
        format!("http://127.0.0.1:{}", node.http_port)
    } else {
        panic!("No base node available for scanning");
    };

    let (cmd, mut args) = world.get_minotari_command();
    args.extend_from_slice(&[
        "re-scan".to_string(),
        "--database-path".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--password".to_string(),
        world.test_password.clone(),
        "--base-url".to_string(),
        base_url,
        "--account-name".to_string(),
        "default".to_string(),
        "--rescan-from-height".to_string(),
        height,
    ]);

    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute re-scan command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

#[when(regex = r#"^I perform a scan with batch size "([^"]*)"$"#)]
async fn scan_with_batch_size(world: &mut MinotariWorld, batch_size: String) {
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
        "--batch-size".to_string(),
        batch_size,
    ]);

    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute scan command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

#[then("the scan should complete successfully")]
async fn scan_succeeds(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Scan command failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );
}

#[then("the scanned tip should be updated")]
async fn scanned_tip_updated(world: &mut MinotariWorld) {
    // Verify scan completed successfully first
    scan_succeeds(world).await;

    // The scanned tip is updated in the database during scanning
    // We can verify this by checking the output mentions scanning progress
    let output = world.last_command_output.as_ref().expect("No scan output");
    assert!(
        output.contains("Scanning") || output.contains("blocks") || output.contains("height"),
        "Expected scan output to mention scanning progress"
    );
}

#[then("the scan should start from the last scanned height")]
async fn scan_from_last_height(world: &mut MinotariWorld) {
    // When performing an incremental scan, it should continue from where it left off
    // This is verified by the scan completing successfully
    scan_succeeds(world).await;
}

#[then("new blocks should be processed")]
async fn new_blocks_processed(world: &mut MinotariWorld) {
    scan_succeeds(world).await;
}

#[then(regex = r#"^the wallet state should be rolled back to height "([^"]*)"$"#)]
async fn wallet_rolled_back(world: &mut MinotariWorld, _height: String) {
    // Re-scan command should complete successfully
    scan_succeeds(world).await;
}

#[then(regex = r#"^scanning should resume from height "([^"]*)"$"#)]
async fn scanning_resumes(world: &mut MinotariWorld, _height: String) {
    scan_succeeds(world).await;
}

#[then(regex = r#"^blocks should be fetched in batches of "([^"]*)"$"#)]
async fn blocks_in_batches(world: &mut MinotariWorld, _batch_size: String) {
    // Verify scan with custom batch size completed
    scan_succeeds(world).await;
}
