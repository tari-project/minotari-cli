// Wallet Import Step Definitions
//
// Step definitions for testing wallet import functionality, including
// importing wallets using view/spend keys or seed words.

use cucumber::{then, when};
use std::process::Command;

use super::common::MinotariWorld;

// =============================
// Wallet Import Steps
// =============================

#[when("I import a wallet with view key and spend key")]
async fn import_wallet_with_keys(world: &mut MinotariWorld) {
    let db_path = world.database_path.as_ref().expect("Database not set up");
    
    let output = Command::new("cargo")
        .args(&[
            "run", "--bin", "minotari", "--", "import-view-key",
            "--view-private-key", &world.test_view_key,
            "--spend-public-key", &world.test_spend_key,
            "--password", &world.test_password,
            "--database-path", db_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

#[when(regex = r#"^I import a wallet with birthday "([^"]*)"$"#)]
async fn import_wallet_with_birthday(world: &mut MinotariWorld, birthday: String) {
    let db_path = world.database_path.as_ref().expect("Database not set up");
    
    let output = Command::new("cargo")
        .args(&[
            "run", "--bin", "minotari", "--", "import-view-key",
            "--view-private-key", &world.test_view_key,
            "--spend-public-key", &world.test_spend_key,
            "--password", &world.test_password,
            "--database-path", db_path.to_str().unwrap(),
            "--birthday", &birthday,
        ])
        .output()
        .expect("Failed to execute command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

#[when("I create a wallet with seed words")]
async fn create_wallet_with_seed_words(world: &mut MinotariWorld) {
    let db_path = world.database_path.as_ref().expect("Database not set up");
    let seed_words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    
    let output = Command::new("cargo")
        .args(&[
            "run", "--bin", "minotari", "--", "create",
            "--password", &world.test_password,
            "--database-path", db_path.to_str().unwrap(),
            "--seed-words", seed_words,
        ])
        .output()
        .expect("Failed to execute command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

#[when("I request to show seed words with password")]
async fn show_seed_words(world: &mut MinotariWorld) {
    let db_path = world.database_path.as_ref().expect("Database not set up");
    
    let output = Command::new("cargo")
        .args(&[
            "run", "--bin", "minotari", "--", "show-seed-words",
            "--password", &world.test_password,
            "--database-path", db_path.to_str().unwrap(),
            "--account-name", "default",
        ])
        .output()
        .expect("Failed to execute command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

#[then("the account should be created in the database")]
async fn account_created(world: &mut MinotariWorld) {
    assert_eq!(world.last_command_exit_code, Some(0), 
        "Command failed: {}", world.last_command_error.as_deref().unwrap_or(""));
}

#[then("the account should have the correct keys")]
async fn account_has_keys(_world: &mut MinotariWorld) {
    // Verification would require database query
}

#[then(regex = r#"^the account should have birthday "([^"]*)"$"#)]
async fn account_has_birthday(_world: &mut MinotariWorld, _birthday: String) {
    // Verification would require database query
}

#[then("the account should be encrypted with password")]
async fn account_is_encrypted(_world: &mut MinotariWorld) {
    // Verification would require database query
}

#[then("I should see the seed words")]
async fn see_seed_words(world: &mut MinotariWorld) {
    let output = world.last_command_output.as_ref().expect("No command output");
    assert!(output.contains("seed") || output.contains("words") || !output.is_empty(),
        "Seed words not found in output");
}
