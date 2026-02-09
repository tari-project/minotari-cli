// Fund Locking Step Definitions
//
// Step definitions for testing fund locking functionality.

use cucumber::{then, when};

use super::common::MinotariWorld;

// =============================
// Fund Locking Steps
// =============================

#[when(regex = r#"^I lock funds for amount "([^"]*)" microTari$"#)]
async fn lock_funds_for_amount(_world: &mut MinotariWorld, _amount: String) {}

#[when(regex = r#"^I lock funds with "([^"]*)" outputs$"#)]
async fn lock_funds_with_outputs(_world: &mut MinotariWorld, _num_outputs: String) {}

#[when(regex = r#"^I lock funds with duration "([^"]*)" seconds$"#)]
async fn lock_funds_with_duration(_world: &mut MinotariWorld, _seconds: String) {}

#[when(regex = r#"^I try to lock funds for amount "([^"]*)" microTari$"#)]
async fn try_lock_funds(_world: &mut MinotariWorld, _amount: String) {}

#[when(regex = r#"^I lock funds with fee per gram "([^"]*)" microTari$"#)]
async fn lock_funds_with_fee(_world: &mut MinotariWorld, _fee: String) {}

#[then("the funds should be locked")]
async fn funds_are_locked(_world: &mut MinotariWorld) {}

#[then("the locked funds file should be created")]
async fn locked_funds_file_created(_world: &mut MinotariWorld) {}

#[then("the UTXOs should be marked as locked")]
async fn utxos_marked_locked(_world: &mut MinotariWorld) {}

#[then(regex = r#"^"([^"]*)" UTXOs should be locked$"#)]
async fn n_utxos_locked(_world: &mut MinotariWorld, _num: String) {}

#[then(regex = r#"^the UTXOs should be locked for "([^"]*)" seconds$"#)]
async fn utxos_locked_duration(_world: &mut MinotariWorld, _seconds: String) {}

#[then("the fund locking should fail")]
async fn fund_locking_fails(_world: &mut MinotariWorld) {}

#[then(regex = r#"^the fee calculation should use "([^"]*)" microTari per gram$"#)]
async fn fee_calculation_uses(_world: &mut MinotariWorld, _fee: String) {}
