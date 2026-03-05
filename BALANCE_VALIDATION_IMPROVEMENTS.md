# Balance Validation Improvements

## Overview

This document describes the improvements made to balance validation in the cucumber integration tests. The changes ensure that balance checking steps actually validate the balance content, not just command success.

## Problem Statement

The original balance validation steps in `integration-tests/steps/balance.rs` had a critical weakness: they only checked that the balance command succeeded (exit code 0) without validating the actual balance information in the output.

### What Was Wrong

**Before the fix:**
```rust
#[then("I should see the balance information")]
async fn see_balance_info(world: &mut MinotariWorld) {
    // Only checked command succeeded
    assert_eq!(world.last_command_exit_code, Some(0));
}

#[then("the balance should be displayed in microTari")]
async fn balance_in_microtari(world: &mut MinotariWorld) {
    let output = world.last_command_output.as_ref().expect("No command output");
    // Only checked output not empty - could be any garbage
    assert!(!output.is_empty());
}
```

**Problems:**
1. Tests could pass with malformed balance output
2. No verification that output contained "microTari"
3. No validation of balance format or numeric values
4. Could fail to catch bugs in balance calculation or display
5. Not actually testing what users would see

### Why This Matters

Users rely on balance information to:
- Know how much Tari they own
- Verify transactions completed
- Make financial decisions

If balance output is incorrect or malformed, tests should fail immediately, not pass blindly.

## Solution

Enhanced all three basic balance validation steps to perform comprehensive content validation.

### 1. see_balance_info()

**Purpose:** Verify the balance information is properly displayed

**Enhancements:**
```rust
#[then("I should see the balance information")]
async fn see_balance_info(world: &mut MinotariWorld) {
    // Check 1: Command succeeded
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Balance command failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );
    
    // Check 2: Output contains balance keywords
    let output = world.last_command_output.as_ref().expect("No command output");
    assert!(
        output.contains("microTari"),
        "Balance output should contain 'microTari', got: {}",
        output
    );
    assert!(
        output.contains("Balance at height") || output.contains("balance"),
        "Balance output should contain balance information, got: {}",
        output
    );
    
    // Check 3: Balance can be parsed (format is correct)
    world.parse_balance_from_output().expect(
        "Could not parse balance from output - output format may be incorrect"
    );
}
```

**What it validates:**
- ✅ Command exits successfully
- ✅ Output contains "microTari" keyword
- ✅ Output contains "Balance at height" or "balance"
- ✅ Balance value can be parsed from output
- ✅ Numeric format is correct

### 2. balance_in_microtari()

**Purpose:** Verify the balance is displayed in microTari format

**Enhancements:**
```rust
#[then("the balance should be displayed in microTari")]
async fn balance_in_microtari(world: &mut MinotariWorld) {
    let output = world.last_command_output.as_ref().expect("No command output");
    assert!(!output.is_empty(), "No balance output");
    
    // Check 1: Output specifically mentions microTari
    assert!(
        output.contains("microTari"),
        "Balance output should display amounts in microTari, got: {}",
        output
    );
    
    // Check 2: Numeric balance value can be parsed
    world.parse_balance_from_output().expect(
        "Could not parse numeric balance value - output may not be in correct microTari format"
    );
}
```

**What it validates:**
- ✅ Output is not empty
- ✅ Output contains "microTari" keyword
- ✅ Numeric balance value can be extracted
- ✅ Format matches expected pattern

### 3. see_all_balances()

**Purpose:** Verify balance information for all accounts

**Enhancements:**
```rust
#[then("I should see balance for all accounts")]
async fn see_all_balances(world: &mut MinotariWorld) {
    // Check 1: Command succeeded
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Balance command failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );
    
    // Check 2: Output contains balance information
    let output = world.last_command_output.as_ref().expect("No command output");
    assert!(
        output.contains("microTari"),
        "Balance output should contain 'microTari' for accounts, got: {}",
        output
    );
    
    // Check 3: At least one balance can be parsed
    world.parse_balance_from_output().expect(
        "Could not parse balance from output - output format may be incorrect"
    );
}
```

**What it validates:**
- ✅ Command exits successfully
- ✅ Output contains "microTari" for accounts
- ✅ At least one balance can be parsed
- ✅ Format is correct for account balances

## Expected Balance Format

All validations expect output in this format:
```
Balance at height 123(2024-01-01 12:00:00): 1,000,000 microTari (1.000000 Tari)
```

**Key components:**
- `Balance at height X(date):` - Context header
- `1,000,000` - Numeric value with thousands separators
- `microTari` - Unit identifier
- `(1.000000 Tari)` - Human-readable Tari equivalent

## Parsing Logic

The `parse_balance_from_output()` helper in `common.rs` uses regex to extract the balance:

```rust
pub fn parse_balance_from_output(&self) -> Option<u64> {
    let output = self.last_command_output.as_ref()?;
    
    // Regex pattern matches: ":\s*([\d,]+)\s+microTari"
    let re = regex::Regex::new(r":\s*([\d,]+)\s+microTari").ok()?;
    let captures = re.captures(output)?;
    let balance_str = captures.get(1)?.as_str();
    
    // Remove commas and parse to u64
    let balance_str = balance_str.replace(',', "");
    balance_str.parse::<u64>().ok()
}
```

**What it does:**
1. Searches for pattern `: <number> microTari`
2. Captures the numeric part (with commas)
3. Removes comma separators
4. Parses to u64
5. Returns None if any step fails

## Impact on Tests

### Test Scenarios Affected

**balance.feature:**
- ✅ Scenario: Check balance for a specific account
- ✅ Scenario: Check balance for all accounts
- ✅ Scenario: Check balance with no outputs

All now properly validate balance content.

### Before vs After

**Before:**
```gherkin
Scenario: Check balance for a specific account
  Given I have a test database with an existing wallet
  When I check the balance for account "default"
  Then I should see the balance information  # Only checked exit code 0
```
- ✅ Passes if command returns any output
- ❌ Could miss malformed balance data
- ❌ Could miss incorrect numeric values

**After:**
```gherkin
Scenario: Check balance for a specific account
  Given I have a test database with an existing wallet
  When I check the balance for account "default"
  Then I should see the balance information  # Now validates content
```
- ✅ Verifies microTari format
- ✅ Validates balance keywords
- ✅ Confirms numeric values parseable
- ✅ Catches format errors immediately

## Error Messages

Enhanced error messages help debug failures:

```rust
// Example 1: Missing microTari keyword
assert!(
    output.contains("microTari"),
    "Balance output should contain 'microTari', got: {}", output
);

// Example 2: Unparseable balance
world.parse_balance_from_output().expect(
    "Could not parse balance from output - output format may be incorrect"
);
```

When tests fail, developers see:
- What was expected
- What was actually received
- Clear indication of which validation failed

## Complementary Steps

These basic steps work alongside more specific balance validation steps:

**Already existed (lines 134-168):**
- `balance_is_zero()` - Checks balance == 0
- `balance_should_be_exact()` - Checks exact amount
- `balance_should_be_at_least()` - Checks minimum amount
- `balance_should_contain()` - Checks contains amount

**Now improved (lines 65-132):**
- `see_balance_info()` - Basic format validation
- `balance_in_microtari()` - MicroTari format validation
- `see_all_balances()` - Multi-account validation

## Benefits

### 1. Catch Format Bugs
Tests now fail if balance output format changes unexpectedly.

### 2. Validate Actual Content
Tests verify what users actually see, not just command success.

### 3. Better Error Messages
Clear, actionable error messages help debug issues faster.

### 4. Consistency
All balance steps now use similar validation patterns.

### 5. Production Confidence
More rigorous testing means higher confidence in releases.

## Future Enhancements

Possible improvements for the future:

1. **Multi-Account Parsing**
   - Parse and validate each account's balance separately
   - Verify account names match expectations

2. **Format Variations**
   - Support different output formats (JSON, table, etc.)
   - Validate Tari equivalent calculations

3. **Historical Balance**
   - Verify balance at different heights
   - Check balance history accuracy

4. **Locked vs Unlocked**
   - Separate validation for locked amounts
   - Verify total = unlocked + locked

5. **Error Case Testing**
   - Test invalid account names
   - Test corrupted database scenarios
   - Verify error messages are helpful

## Testing

To verify the improvements:

```bash
# Run all balance tests
cargo test --release --test cucumber --package integration-tests -- balance

# Run specific scenario
cargo test --release --test cucumber --package integration-tests -- "Check balance for a specific account"
```

Expected result: All tests pass with proper validation.

## Conclusion

These improvements transform balance validation from "command didn't crash" to "balance information is correct and properly formatted." This aligns test behavior with user expectations and catches bugs that would otherwise slip through.

The changes are backward compatible - all existing tests continue to work, but now with stronger validation.
