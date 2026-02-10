// Balance Step Definitions
//
// Step definitions for testing balance checking functionality.

use cucumber::{then, when};
use std::process::Command;

use super::common::MinotariWorld;

// =============================
// Balance Steps
// =============================

#[when(regex = r#"^I check the balance for account "([^"]*)"$"#)]
async fn check_balance_for_account(world: &mut MinotariWorld, account_name: String) {
    let db_path = world.database_path.as_ref().expect("Database not set up");
    let (cmd, mut args) = world.get_minotari_command();
    args.extend_from_slice(&[
        "balance".to_string(),
        "--database-path".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--account-name".to_string(),
        account_name,
    ]);

    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

#[when("I check the balance without specifying an account")]
async fn check_balance_all_accounts(world: &mut MinotariWorld) {
    let db_path = world.database_path.as_ref().expect("Database not set up");
    let (cmd, mut args) = world.get_minotari_command();
    args.extend_from_slice(&[
        "balance".to_string(),
        "--database-path".to_string(),
        db_path.to_str().unwrap().to_string(),
    ]);

    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

#[when("I check the balance for the new wallet")]
async fn check_balance_new_wallet(world: &mut MinotariWorld) {
    check_balance_all_accounts(world).await;
}

#[then("I should see the balance information")]
async fn see_balance_info(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Balance command failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );
}

#[then("the balance should be displayed in microTari")]
async fn balance_in_microtari(world: &mut MinotariWorld) {
    let output = world.last_command_output.as_ref().expect("No command output");
    assert!(!output.is_empty(), "No balance output");
}

#[then("I should see balance for all accounts")]
async fn see_all_balances(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Balance command failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );
}

#[then("the balance should be zero")]
async fn balance_is_zero(world: &mut MinotariWorld) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert_eq!(balance, 0, "Expected zero balance, got {}", balance);
}

#[then(regex = r"^the balance should be (\d+) microTari$")]
async fn balance_should_be_exact(world: &mut MinotariWorld, expected: u64) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert_eq!(
        balance, expected,
        "Expected balance {} microTari, got {}",
        expected, balance
    );
}

#[then(regex = r"^the balance should be at least (\d+) microTari$")]
async fn balance_should_be_at_least(world: &mut MinotariWorld, minimum: u64) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert!(
        balance >= minimum,
        "Expected balance at least {} microTari, got {}",
        minimum, balance
    );
}

#[then(regex = r"^the balance should contain (\d+) microTari$")]
async fn balance_should_contain(world: &mut MinotariWorld, expected: u64) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert!(
        balance >= expected,
        "Expected balance to contain at least {} microTari, got {}",
        expected, balance
    );
}
