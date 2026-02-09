# Base Node Integration for Cucumber Tests - Implementation Guide

## Overview

This document outlines the approach to add real base node integration to the Cucumber BDD tests for the minotari-cli wallet.

## Current State

The Cucumber test suite currently has:
- **7 feature files** covering wallet operations, scanning, transactions, etc.
- **All step definitions are stubs** - empty functions that don't test real functionality
- Tests pass but don't validate actual blockchain interactions

## Problem Statement

From the issue: "Look at: https://github.com/tari-project/tari/tree/development/integration_tests - include a base node into the test so that wallet can test sending, balance and all the other stub functions"

## Approach: Mock Base Node Server

### Why Not Full Base Node?

Spawning full Tari base nodes like in the main `tari/integration_tests` would:
- Add significant dependencies (minotari_node, tari_core, etc.)
- Require complex setup and teardown
- Make tests slow and resource-intensive
- Not align with the "lightweight" nature of minotari-cli

### Solution: HTTP Mock Server

Create a lightweight mock HTTP server that simulates the Tari base node REST API:

1. **Mock Server** (`tests/cucumber/src/mock_base_node.rs`)
   - Implements HTTP endpoints the wallet uses:
     - `GET /tip_info` - Chain tip information
     - `GET /base_node/blocks/:height` - Block data
     - `GET /headers/:height` - Block headers
   - Configurable test data (blocks, outputs, UTXOs)
   - Fast startup/shutdown for test isolation

2. **Integration with MinotariWorld**
   - Add `mock_base_node: Option<MockBaseNode>` field
   - Add `base_node_port: Option<u16>` for port management
   - Helper method `get_base_node_url()` for wallet commands

3. **Real Step Implementations**
   - Replace stub functions with actual CLI command execution
   - Pass mock server URL to wallet commands via `--base-url` flag
   - Verify wallet behavior against mock blockchain state

## Implementation Status

### ✅ Completed
- Created `tests/cucumber/src/` directory structure
- Implemented `mock_base_node.rs` with:
  - `MockBaseNode` struct
  - HTTP endpoints for tip_info, blocks, headers
  - Block storage and management
  - Unit tests for mock server
- Created `lib.rs` to export mock module
- Updated `MinotariWorld` to include mock base node fields
- Added dev dependencies: axum, tower, hyper

### 🚧 In Progress
- Fixing build issues (protoc dependency)

### 📋 TODO

#### Phase 1: Complete Infrastructure
- [ ] Fix protoc build dependency
- [ ] Ensure mock server compiles and passes tests
- [ ] Add helper methods to MinotariWorld for base node management

#### Phase 2: Base Node Steps
- [ ] Create `steps/base_node.rs` module
- [ ] Implement "Given I have a base node" step
- [ ] Implement "When I start a base node on port X" step
- [ ] Add port allocation and cleanup logic

#### Phase 3: Scanning Integration
- [ ] Update `steps/scanning.rs` with real implementations:
  - `scan_with_max_blocks` - Actually call CLI scan command
  - `scan_succeeds` - Verify scan completed without errors
  - `scanned_tip_updated` - Query database to check tip height
- [ ] Add test blocks to mock server for scan testing
- [ ] Verify wallet detects outputs correctly

#### Phase 4: Transaction Testing
- [ ] Update `steps/transactions.rs` with real implementations:
  - `create_transaction_one_recipient` - Call create-unsigned-transaction CLI
  - `transaction_file_created` - Check output file exists
  - `transaction_has_recipient` - Parse and validate transaction JSON
- [ ] Add UTXOs to mock server for transaction inputs
- [ ] Test fund locking with real database state

#### Phase 5: Balance Testing
- [ ] Update `steps/balance.rs` with real implementations:
  - `check_balance_for_account` - Call balance CLI command
  - `balance_in_microtari` - Parse and validate output
  - `balance_is_zero` - Verify zero balance state
- [ ] Query database directly to verify balance calculations

#### Phase 6: New Feature Scenarios
- [ ] Create `base_node_integration.feature` with scenarios:
  - Wallet connects to base node
  - Wallet scans blocks from base node
  - Wallet detects outputs on base node
  - Wallet balance updates after scanning
- [ ] Add integration test scenarios combining multiple features

## Example: Implementing a Real Step

### Before (Stub):
```rust
#[when(regex = r#"^I perform a scan with max blocks "([^"]*)"$"#)]
async fn scan_with_max_blocks(_world: &mut MinotariWorld, _max_blocks: String) {}
```

### After (Real Implementation):
```rust
#[when(regex = r#"^I perform a scan with max blocks "([^"]*)"$"#)]
async fn scan_with_max_blocks(world: &mut MinotariWorld, max_blocks: String) {
    let db_path = world.database_path.as_ref().expect("Database not set up");
    let base_url = world.get_base_node_url();
    
    let output = Command::new("cargo")
        .args(&[
            "run", "--bin", "minotari", "--", "scan",
            "--password", &world.test_password,
            "--database-path", db_path.to_str().unwrap(),
            "--base-url", &base_url,
            "--max-blocks-to-scan", &max_blocks,
        ])
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
        "Scan failed: {}", 
        world.last_command_error.as_deref().unwrap_or("")
    );
}
```

## Testing Strategy

1. **Unit Tests**: Test mock server endpoints independently
2. **Integration Tests**: Test CLI commands against mock server
3. **End-to-End Tests**: Complete scenarios from wallet creation to transaction

## Benefits

✅ **Real Testing**: Validates actual CLI behavior, not just command execution  
✅ **Fast**: Mock server starts/stops in milliseconds  
✅ **Deterministic**: Control exact blockchain state for each test  
✅ **Isolated**: Each test gets clean mock server instance  
✅ **Lightweight**: No full node dependencies  
✅ **Maintainable**: Easy to add new test scenarios  

## Next Steps

1. Fix protoc dependency (install protobuf-compiler)
2. Verify mock server builds and tests pass
3. Implement one complete feature (scanning) end-to-end
4. Use as template for other features
5. Document learnings for future contributors

## References

- Tari Integration Tests: https://github.com/tari-project/tari/tree/development/integration_tests
- Lightweight Wallet Libs: https://github.com/tari-project/tari-wallet
- Cucumber Rust: https://cucumber-rs.github.io/cucumber/current/
