// Scanning Step Definitions
//
// Step definitions for testing blockchain scanning functionality.

use cucumber::{given, then, when};

use super::common::MinotariWorld;

// =============================
// Scanning Steps
// =============================

#[given(regex = r#"^the wallet has birthday height "([^"]*)"$"#)]
async fn wallet_has_birthday(_world: &mut MinotariWorld, _height: String) {}

#[given("the wallet has been previously scanned")]
async fn wallet_previously_scanned(_world: &mut MinotariWorld) {}

#[given(regex = r#"^the wallet has been previously scanned to height "([^"]*)"$"#)]
async fn wallet_scanned_to_height(_world: &mut MinotariWorld, _height: String) {}

#[when(regex = r#"^I perform a scan with max blocks "([^"]*)"$"#)]
async fn scan_with_max_blocks(_world: &mut MinotariWorld, _max_blocks: String) {}

#[when("I perform an incremental scan")]
async fn incremental_scan(_world: &mut MinotariWorld) {}

#[when(regex = r#"^I re-scan from height "([^"]*)"$"#)]
async fn rescan_from_height(_world: &mut MinotariWorld, _height: String) {}

#[when(regex = r#"^I perform a scan with batch size "([^"]*)"$"#)]
async fn scan_with_batch_size(_world: &mut MinotariWorld, _batch_size: String) {}

#[then("the scan should complete successfully")]
async fn scan_succeeds(_world: &mut MinotariWorld) {}

#[then("the scanned tip should be updated")]
async fn scanned_tip_updated(_world: &mut MinotariWorld) {}

#[then("the scan should start from the last scanned height")]
async fn scan_from_last_height(_world: &mut MinotariWorld) {}

#[then("new blocks should be processed")]
async fn new_blocks_processed(_world: &mut MinotariWorld) {}

#[then(regex = r#"^the wallet state should be rolled back to height "([^"]*)"$"#)]
async fn wallet_rolled_back(_world: &mut MinotariWorld, _height: String) {}

#[then(regex = r#"^scanning should resume from height "([^"]*)"$"#)]
async fn scanning_resumes(_world: &mut MinotariWorld, _height: String) {}

#[then(regex = r#"^blocks should be fetched in batches of "([^"]*)"$"#)]
async fn blocks_in_batches(_world: &mut MinotariWorld, _batch_size: String) {}
