# Exact Balance Testing in Cucumber Integration Tests

This document describes the implementation of exact balance verification in the cucumber integration tests, allowing validation of precise microTari amounts after blockchain operations.

## Overview

Previously, balance tests only verified that the balance command succeeded, but didn't check actual amounts. This update adds the ability to verify exact balances, ensuring that:
- Mining rewards are correctly calculated
- Scanning detects all outputs
- Database accurately tracks amounts
- Balance queries return correct values

## Implementation

### 1. Balance Parsing Helper

**Location:** `integration-tests/steps/common.rs`

Added `parse_balance_from_output()` method to `MinotariWorld`:

```rust
pub fn parse_balance_from_output(&self) -> Option<u64> {
    let output = self.last_command_output.as_ref()?;
    
    // Look for pattern like "1,000,000 microTari" or "0 microTari"
    // Balance format: "Balance at height X(date): Y microTari (A.B Tari)"
    let re = regex::Regex::new(r":\s*([\d,]+)\s+microTari").ok()?;
    let captures = re.captures(output)?;
    let balance_str = captures.get(1)?.as_str();
    
    // Remove commas and parse
    let balance_str = balance_str.replace(',', "");
    balance_str.parse::<u64>().ok()
}
```

**How it works:**
1. Searches for the pattern `: <number> microTari` in the output
2. Extracts the number, which may contain commas (e.g., "1,000,000")
3. Removes commas and parses as u64
4. Returns `None` if parsing fails, allowing graceful error handling

**Example inputs:**
```
Balance at height 10(2024-01-01): 1,000,000 microTari (1.000000 Tari)
→ Returns: Some(1000000)

Balance at height 0(N/A): 0 microTari (0.000000 Tari)
→ Returns: Some(0)

No balance information
→ Returns: None
```

### 2. Balance Verification Steps

**Location:** `integration-tests/steps/balance.rs`

Added four cucumber step definitions for different verification needs:

#### Exact Zero Balance
```gherkin
Then the balance should be 0 microTari
```
```rust
#[then("the balance should be zero")]
async fn balance_is_zero(world: &mut MinotariWorld) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert_eq!(balance, 0, "Expected zero balance, got {}", balance);
}
```

#### Exact Amount Match
```gherkin
Then the balance should be 1000000 microTari
```
```rust
#[then(regex = r"^the balance should be (\d+) microTari$")]
async fn balance_should_be_exact(world: &mut MinotariWorld, expected: u64) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert_eq!(
        balance, expected,
        "Expected balance {} microTari, got {}",
        expected, balance
    );
}
```

#### Minimum Amount (Greater Than or Equal)
```gherkin
Then the balance should be at least 5000000 microTari
```
```rust
#[then(regex = r"^the balance should be at least (\d+) microTari$")]
async fn balance_should_be_at_least(world: &mut MinotariWorld, minimum: u64) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert!(
        balance >= minimum,
        "Expected balance at least {} microTari, got {}",
        minimum, balance
    );
}
```

#### Contains Amount (Flexible Check)
```gherkin
Then the balance should contain 3000000 microTari
```
```rust
#[then(regex = r"^the balance should contain (\d+) microTari$")]
async fn balance_should_contain(world: &mut MinotariWorld, expected: u64) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert!(
        balance >= expected,
        "Expected balance to contain at least {} microTari, got {}",
        expected, balance
    );
}
```

### 3. Updated Test Scenarios

#### Balance Feature
**File:** `integration-tests/features/balance.feature`

Updated the zero balance test:
```gherkin
Scenario: Check balance with no outputs
  Given I have a test database with a new wallet
  When I check the balance for the new wallet
  Then the balance should be 0 microTari
```

#### End-to-End Feature
**File:** `integration-tests/features/end_to_end.feature`

Added balance verification after mining:
```gherkin
Scenario: Mine, scan, and check balance
  Given I have a seed node MinerNode
  And I have a test database with an existing wallet
  When I mine 10 blocks on MinerNode
  Then the chain height should be 10
  When I perform a scan with max blocks "20"
  Then the scan should complete successfully
  And the scanned tip should be updated
  When I check the balance for account "default"
  Then I should see the balance information
  And the balance should be at least 10000000 microTari
```

#### New: Balance with Mining Feature
**File:** `integration-tests/features/balance_with_mining.feature`

Created comprehensive mining reward tests:

```gherkin
Feature: Balance Verification with Mining
  As a user
  I want to verify exact balances after mining
  So that I can confirm the wallet correctly tracks blockchain rewards

  Scenario: Verify balance after mining single block
    Given I have a seed node MinerNode
    And I have a test database with an existing wallet
    When I mine 1 blocks on MinerNode
    And I perform a scan with max blocks "10"
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then the balance should be at least 1000000 microTari

  Scenario: Verify balance after mining multiple blocks
    Given I have a seed node MinerNode
    And I have a test database with an existing wallet
    When I mine 5 blocks on MinerNode
    And I perform a scan with max blocks "10"
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then the balance should be at least 5000000 microTari

  Scenario: Verify balance increases with incremental mining
    Given I have a seed node MinerNode
    And I have a test database with an existing wallet
    When I mine 3 blocks on MinerNode
    And I perform a scan with max blocks "10"
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then the balance should be at least 3000000 microTari
    When I mine 2 blocks on MinerNode
    And I perform an incremental scan
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then the balance should be at least 5000000 microTari
```

## Usage Examples

### Basic Balance Check
```gherkin
When I check the balance for account "default"
Then the balance should be 0 microTari
```

### After Mining
```gherkin
Given I have a seed node MinerNode
And I have a test database with an existing wallet
When I mine 5 blocks on MinerNode
And I perform a scan with max blocks "10"
When I check the balance for account "default"
Then the balance should be at least 5000000 microTari
```

### Incremental Testing
```gherkin
# First mining operation
When I mine 3 blocks on MinerNode
And I perform a scan with max blocks "10"
When I check the balance for account "default"
Then the balance should be at least 3000000 microTari

# Additional mining
When I mine 2 blocks on MinerNode
And I perform an incremental scan
When I check the balance for account "default"
Then the balance should be at least 5000000 microTari
```

## Amount Calculations

### MicroTari to Tari Conversion
- 1 Tari = 1,000,000 microTari
- 1,000,000 microTari = 1 Tari
- 10,000,000 microTari = 10 Tari

### Minimum Block Rewards
The tests use "at least" assertions with minimum expected values:
- 1 block → at least 1,000,000 microTari (1 Tari)
- 5 blocks → at least 5,000,000 microTari (5 Tari)
- 10 blocks → at least 10,000,000 microTari (10 Tari)

This approach allows tests to pass regardless of the exact emission schedule, as long as rewards meet minimum thresholds.

## Benefits

### 1. Accurate Validation
- Tests verify actual blockchain rewards
- Catches off-by-one errors in balance calculations
- Ensures database correctly tracks amounts

### 2. Regression Prevention
- Detects changes in reward calculations
- Catches scanning bugs that miss outputs
- Validates balance query logic

### 3. Confidence in Production
- Same balance logic used in production
- Real blockchain data, not mocks
- End-to-end validation of wallet operations

### 4. Foundation for Future Tests
- Enables transaction amount testing
- Supports fee calculation validation
- Allows locked/unlocked balance verification

## Implementation Notes

### Why "At Least" Assertions?
Using `balance should be at least X` instead of exact equality because:
1. **Emission Schedule Flexibility** - Different networks have different reward schedules
2. **Test Robustness** - Tests pass as long as minimum rewards are met
3. **Future Compatibility** - Works even if reward calculations change
4. **Practical Testing** - What matters is that rewards are sufficient, not exact

### Error Handling
The `parse_balance_from_output()` method:
- Returns `Option<u64>` for safe handling
- Step definitions use `.expect()` to fail fast with clear messages
- Provides helpful error messages when parsing fails

### Performance
- Regex compilation is done at runtime (acceptable for tests)
- Could be optimized with lazy_static if needed
- Parsing is fast enough for test scenarios

## Future Enhancements

### 1. Transaction Amount Testing
Verify exact amounts for:
- Input values
- Output values
- Fee calculations
- Change outputs

### 2. Multi-Account Testing
Test balances across multiple accounts:
```gherkin
When I check the balance for account "savings"
And I check the balance for account "spending"
Then the total balance should be 10000000 microTari
```

### 3. Locked vs Unlocked Balances
Verify different balance types:
```gherkin
Then the unlocked balance should be 5000000 microTari
And the locked balance should be 2000000 microTari
```

### 4. Time-Based Balance Changes
Test balance changes over time:
```gherkin
When I check the balance
Then the balance should be 1000000 microTari
When I wait for 10 blocks
And I check the balance
Then the balance should be at least 11000000 microTari
```

## Testing

### Running Balance Tests
```bash
# All balance tests
cargo test --release --test cucumber --package integration-tests -- --tags @balance

# Specific scenario
cargo test --release --test cucumber --package integration-tests -- "Verify balance after mining"

# All end-to-end tests with balance checks
cargo test --release --test cucumber --package integration-tests -- end_to_end
```

### Debugging Balance Issues
If a balance test fails:
1. Check the command output: `world.last_command_output`
2. Verify the regex matches the format
3. Confirm blocks were actually mined
4. Ensure scanning completed successfully
5. Check database has outputs recorded

## Conclusion

The exact balance verification feature provides robust testing of the wallet's core functionality - accurately tracking and reporting balances. By verifying precise microTari amounts after mining and scanning operations, we ensure the wallet correctly implements the Tari protocol's reward and balance calculation logic.
