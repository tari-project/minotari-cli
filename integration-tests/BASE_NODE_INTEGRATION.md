# Base Node Integration for Cucumber Tests - Implementation Guide

## Overview

This document outlines the implementation of **actual** Tari base node integration for the Cucumber BDD tests.

## Approach: Real Base Nodes from minotari_node

Following the pattern from `tari/integration_tests`, we spawn actual `minotari_node` processes for testing.

### Why Real Base Nodes?

As requested in the issue, we use actual base nodes from `https://github.com/tari-project/tari/tree/development/applications/minotari_node` instead of mocks. This provides:

- **Real Integration Testing**: Tests actual wallet-node interactions
- **Accurate Behavior**: Tests against real blockchain logic
- **Complete Feature Coverage**: Can test all node features
- **Production Parity**: Same code path as production

## Implementation

### Dependencies Added

From `tari/integration_tests/Cargo.toml`:

```toml
[dev-dependencies]
minotari_app_utilities = { git = "...", rev = "..." }
minotari_node = { git = "...", rev = "...", features = ["metrics"] }
minotari_node_grpc_client = { git = "...", rev = "..." }
tari_common_sqlite = { git = "...", rev = "..." }
tari_comms = { git = "...", rev = "..." }
tari_comms_dht = { git = "...", rev = "..." }
tari_p2p = { git = "...", rev = "..." }
tari_shutdown = { git = "...", rev = "..." }
rand = "0.8"
tonic = "0.13"
indexmap = "1.9"
```

### Core Components

#### 1. BaseNodeProcess (`tests/cucumber/src/base_node_process.rs`)

Manages actual base node processes:
- Spawns `minotari_node` via `run_base_node()`
- Configures LocalNet network for testing
- Manages GRPC, HTTP, and P2P ports
- Handles proper cleanup via `Shutdown` signal
- Provides GRPC client access

Key features:
- Each node gets unique ports (P2P, GRPC, HTTP)
- Temporary directories for data
- Configurable as seed or regular node
- Automatic port release on shutdown

#### 2. MinotariWorld (`tests/cucumber/steps/common.rs`)

Updated to manage actual nodes:
- `base_nodes: IndexMap<String, BaseNodeProcess>` - Running nodes
- `assigned_ports: IndexMap<u64, u64>` - Port tracking
- `current_base_dir: PathBuf` - Base temp directory
- `seed_nodes: Vec<String>` - Seed node names

#### 3. Base Node Steps (`tests/cucumber/steps/base_node.rs`)

Cucumber step definitions:
- `Given I have a seed node {name}` - Start a seed node
- `Given I have a base node {name}` - Start a regular node
- `Given I have a base node {name} connected to all seed nodes` - Start connected node

### Configuration

Base nodes are configured for LocalNet testing:
- **Network**: LocalNet (isolated test network)
- **Transport**: TCP (no Tor for simplicity)
- **Ports**: Dynamically allocated (18000-18499 P2P, 18500-18999 GRPC, 19000-19499 HTTP)
- **Data**: Temporary directories cleaned up after tests
- **DHT**: Configured for fast local discovery
- **Sync**: Fast sync settings for testing

### Usage Example

```gherkin
Feature: Wallet Scanning with Base Node

  Scenario: Scan blocks from base node
    Given I have a seed node Node_A
    And I have a base node Node_B connected to all seed nodes
    And I have a test database with an existing wallet
    When I perform a scan with max blocks "10"
    Then the scan should complete successfully
```

## Implementation Status

### ✅ Completed

- **Dependencies**: Added all required Tari dependencies
- **BaseNodeProcess**: Full implementation from tari/integration_tests
- **Port Management**: Dynamic port allocation and tracking
- **World Structure**: Updated to manage real nodes
- **Base Node Steps**: Step definitions for node startup
- **Cleanup**: Proper shutdown handling via Drop trait

### 🚧 In Progress

- Building and testing the integration
- Verifying node startup
- Testing wallet connectivity

### 📋 Next Steps

1. **Build Verification**
   - Install protoc compiler (required for GRPC)
   - Build with new dependencies
   - Verify no compilation errors

2. **Basic Node Testing**
   - Test single node startup
   - Verify GRPC endpoint accessibility
   - Test node shutdown

3. **Wallet Integration**
   - Connect wallet to real node
   - Test scanning against real blockchain
   - Verify balance updates

4. **Step Implementations**
   - Implement scanning steps with real CLI calls
   - Implement transaction steps
   - Implement balance checking

5. **Feature Scenarios**
   - Create integration test scenarios
   - Test multi-node scenarios
   - Add mining for transaction testing

## Differences from Mock Approach

| Aspect | Mock Server | Real Base Node |
|--------|-------------|----------------|
| Dependencies | Minimal (axum) | Full Tari stack |
| Startup Time | <100ms | ~2-5s |
| Resource Usage | Very light | Moderate |
| Test Accuracy | Simulated | Actual |
| Feature Coverage | Limited | Complete |
| Maintenance | Custom code | Uses production code |

## Benefits

✅ **Authentic Testing**: Real blockchain interactions  
✅ **Production Parity**: Same code as production nodes  
✅ **Complete Features**: Can test all node capabilities  
✅ **Mining Support**: Can mine blocks for transaction testing  
✅ **P2P Testing**: Can test multi-node scenarios  
✅ **Maintained**: Benefits from upstream Tari improvements  

## Testing Strategy

1. **Unit Level**: Individual node startup/shutdown
2. **Integration Level**: Wallet-node communication
3. **Scenario Level**: Complete user workflows
4. **Multi-Node**: Network behaviors (optional)

## Notes

- Tests run on LocalNet (isolated from mainnet/testnet)
- Each test gets fresh nodes and temp directories
- Port conflicts avoided via dynamic allocation
- Cleanup happens automatically via Drop trait
- Can run multiple nodes in same test for complex scenarios

## References

- Tari Integration Tests: https://github.com/tari-project/tari/tree/development/integration_tests
- Minotari Node: https://github.com/tari-project/tari/tree/development/applications/minotari_node
- Cucumber Rust: https://cucumber-rs.github.io/cucumber/current/
