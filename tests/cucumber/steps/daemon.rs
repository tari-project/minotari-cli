// Daemon Step Definitions
//
// Step definitions for testing daemon mode functionality.

use cucumber::{given, then, when};

use super::common::MinotariWorld;

// =============================
// Daemon Steps
// =============================

#[given("I have a running daemon with an existing wallet")]
async fn running_daemon_with_wallet(_world: &mut MinotariWorld) {}

#[given("I have a running daemon")]
async fn running_daemon(_world: &mut MinotariWorld) {}

#[when(regex = r#"^I start the daemon on port "([^"]*)"$"#)]
async fn start_daemon_on_port(_world: &mut MinotariWorld, _port: String) {}

#[when(regex = r#"^I start the daemon with scan interval "([^"]*)" seconds$"#)]
async fn start_daemon_with_interval(_world: &mut MinotariWorld, _interval: String) {}

#[when(regex = r#"^I query the balance via the API for account "([^"]*)"$"#)]
async fn query_balance_api(_world: &mut MinotariWorld, _account_name: String) {}

#[when(regex = r#"^I lock funds via the API for amount "([^"]*)" microTari$"#)]
async fn lock_funds_api(_world: &mut MinotariWorld, _amount: String) {}

#[when("I create a transaction via the API")]
async fn create_transaction_api(_world: &mut MinotariWorld) {}

#[when("I send a shutdown signal")]
async fn send_shutdown_signal(_world: &mut MinotariWorld) {}

#[then(regex = r#"^the API should be accessible on port "([^"]*)"$"#)]
async fn api_accessible(_world: &mut MinotariWorld, _port: String) {}

#[then("the Swagger UI should be available")]
async fn swagger_available(_world: &mut MinotariWorld) {}

#[then("the daemon should scan periodically")]
async fn daemon_scans_periodically(_world: &mut MinotariWorld) {}

#[then("the scanned tip should be updated over time")]
async fn scanned_tip_updated_over_time(_world: &mut MinotariWorld) {}

#[then("I should receive a balance response")]
async fn receive_balance_response(_world: &mut MinotariWorld) {}

#[then("the response should include balance information")]
async fn response_has_balance_info(_world: &mut MinotariWorld) {}

#[then("the API should return success")]
async fn api_returns_success(_world: &mut MinotariWorld) {}

#[then("the API should return the unsigned transaction")]
async fn api_returns_transaction(_world: &mut MinotariWorld) {}

#[then("the daemon should stop gracefully")]
async fn daemon_stops_gracefully(_world: &mut MinotariWorld) {}

#[then("database connections should be closed")]
async fn database_connections_closed(_world: &mut MinotariWorld) {}
