# Cucumber BDD Integration Tests

This directory contains Behavior-Driven Development (BDD) integration tests for the Minotari CLI wallet using the Cucumber framework.

## Overview

The test suite covers all major functionality of the Minotari CLI wallet:

- **Wallet Creation**: Creating new wallets with/without encryption
- **Wallet Import**: Importing wallets using view/spend keys or seed words
- **Balance Operations**: Checking wallet balances
- **Blockchain Scanning**: Scanning for transactions, re-scanning, and incremental scans
- **Transaction Creation**: Creating unsigned one-sided transactions
- **Fund Locking**: Locking UTXOs for pending transactions
- **Daemon Mode**: Running the wallet daemon with REST API

## Structure

```
tests/cucumber/
├── features/           # Gherkin feature files
│   ├── wallet_creation.feature
│   ├── wallet_import.feature
│   ├── balance.feature
│   ├── scanning.feature
│   ├── transactions.feature
│   ├── fund_locking.feature
│   └── daemon.feature
├── steps/              # Step definitions (Rust code)
│   ├── common.rs       # Shared world and utilities
│   ├── wallet_creation.rs
│   ├── wallet_import.rs
│   ├── balance.rs
│   ├── scanning.rs
│   ├── transactions.rs
│   ├── fund_locking.rs
│   └── daemon.rs
└── fixtures/           # Test data and fixtures (if needed)
```

## Running the Tests

### Run all integration tests

```bash
cargo test --test cucumber_integration
```

### Run tests with output

```bash
cargo test --test cucumber_integration -- --nocapture
```

### Run specific feature file

You can filter tests by feature name:

```bash
cargo test --test cucumber_integration -- wallet_creation
```

## Writing New Tests

### 1. Create a Feature File

Create a new `.feature` file in `tests/cucumber/features/`:

```gherkin
Feature: My New Feature
  As a user
  I want to do something
  So that I achieve some goal

  Scenario: Do something specific
    Given some precondition
    When I perform an action
    Then I should see the expected result
```

### 2. Implement Step Definitions

Create or update a step definition file in `tests/cucumber/steps/`:

```rust
use cucumber::{given, when, then};
use super::common::World;

#[given("some precondition")]
async fn setup_precondition(world: &mut World) {
    // Setup code
}

#[when("I perform an action")]
async fn perform_action(world: &mut World) {
    // Action code
}

#[then("I should see the expected result")]
async fn verify_result(world: &mut World) {
    // Assertion code
}
```

### 3. Register the Module

Add your new module to `tests/cucumber/steps/mod.rs`:

```rust
pub mod my_new_feature;
```

## Test World

The `MinotariWorld` struct in `common.rs` provides shared state across steps:

- `temp_dir`: Temporary directory for test files
- `database_path`: Path to test database
- `output_file`: Path to output files (wallet.json, transactions, etc.)
- `wallet_data`: Parsed wallet JSON data
- `last_command_output`: stdout from last command
- `last_command_error`: stderr from last command
- `last_command_exit_code`: Exit code from last command
- `daemon_handle`: Handle to running daemon process
- `api_port`: Port number for API server

## Dependencies

The integration tests use the following crates:

- `cucumber` - BDD testing framework
- `tempfile` - Temporary file/directory management
- `serial_test` - Sequential test execution
- `tokio` - Async runtime
- `reqwest` - HTTP client for API testing

## Notes

- Tests run sequentially (`max_concurrent_scenarios(1)`) to avoid port conflicts when testing daemon mode
- Each test gets its own temporary directory that is cleaned up automatically
- Tests use a consistent test password and keys for deterministic behavior
- Some tests may require a running Tari node to fully execute (scanning scenarios)
- Database operations are tested using SQLite in temporary directories

## Troubleshooting

### Tests fail to connect to node

Most scanning tests will gracefully handle connection failures to real nodes. This is expected in the test environment. The tests verify that the commands execute correctly even if they can't connect to a blockchain node.

### Port already in use

If daemon tests fail with port conflicts, ensure no other instances are running:

```bash
pkill -f minotari
```

### Database locked errors

Ensure previous test runs have cleaned up properly. Delete any stale test databases:

```bash
find /tmp -name "test_wallet.db" -delete
```

## Future Enhancements

Potential improvements to the test suite:

1. Mock blockchain node for more deterministic scanning tests
2. Test fixtures with pre-populated wallets and outputs
3. Performance benchmarks for scanning operations
4. Integration with CI/CD pipeline
5. Code coverage reporting
6. Parallel test execution where safe
