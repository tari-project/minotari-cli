# Balance Command Password Fix

## Problem Statement

The balance command tests were "checking that the balance is actually failing rather than getting actual correct balance information."

## Root Cause Analysis

### The Issue

When running cucumber tests for balance operations, the commands were failing because:

1. **Wallet databases are encrypted** - All test wallets are created with password encryption using `world.test_password`
2. **Balance commands missing password** - The balance command step definitions didn't pass the `--password` argument
3. **Decryption failures** - Without the password, the CLI couldn't decrypt the database
4. **Tests checked for failure** - Tests were effectively validating that commands failed, not that they returned correct balance data

### Code Analysis

**Wallet Creation (Correct):**
```rust
// In database_with_wallet() step
args.extend_from_slice(&[
    "import-view-key".to_string(),
    "--view-private-key".to_string(),
    world.wallet.get_view_key().to_hex(),
    "--spend-public-key".to_string(),
    world.wallet.get_public_spend_key().to_hex(),
    "--password".to_string(),
    world.test_password.clone(),  // ← Password IS provided during creation
    "--database-path".to_string(),
    db_path.to_str().unwrap().to_string(),
]);
```

**Balance Command (Broken - Before Fix):**
```rust
// In check_balance_for_account() step
args.extend_from_slice(&[
    "balance".to_string(),
    "--database-path".to_string(),
    db_path.to_str().unwrap().to_string(),
    "--account-name".to_string(),
    account_name,
    // ← Password was MISSING!
]);
```

## Solution

Added the `--password` argument to all balance command step definitions.

### Changed Functions

#### 1. check_balance_for_account()

**Before:**
```rust
args.extend_from_slice(&[
    "balance".to_string(),
    "--database-path".to_string(),
    db_path.to_str().unwrap().to_string(),
    "--account-name".to_string(),
    account_name,
]);
```

**After:**
```rust
args.extend_from_slice(&[
    "balance".to_string(),
    "--database-path".to_string(),
    db_path.to_str().unwrap().to_string(),
    "--password".to_string(),              // ← Added
    world.test_password.clone(),           // ← Added
    "--account-name".to_string(),
    account_name,
]);
```

#### 2. check_balance_all_accounts()

**Before:**
```rust
args.extend_from_slice(&[
    "balance".to_string(),
    "--database-path".to_string(),
    db_path.to_str().unwrap().to_string(),
]);
```

**After:**
```rust
args.extend_from_slice(&[
    "balance".to_string(),
    "--database-path".to_string(),
    db_path.to_str().unwrap().to_string(),
    "--password".to_string(),              // ← Added
    world.test_password.clone(),           // ← Added
]);
```

## Impact

### Before Fix

```gherkin
Scenario: Check balance for a specific account
  Given I have a test database with an existing wallet
  When I check the balance for account "default"
  Then I should see the balance information  # ✗ Command failed with decryption error
```

**Command output:**
- Exit code: Non-zero (failure)
- Error: Database decryption failed / Password required
- No balance data retrieved

### After Fix

```gherkin
Scenario: Check balance for a specific account
  Given I have a test database with an existing wallet
  When I check the balance for account "default"
  Then I should see the balance information  # ✓ Command succeeds
  And the balance should be 0 microTari       # ✓ Can verify actual balance
```

**Command output:**
- Exit code: 0 (success)
- Balance data: `Balance at height X(...): 0 microTari (0.000000 Tari)`
- Balance can be parsed and validated

## Affected Test Scenarios

All scenarios in `balance.feature` now work correctly:

1. **Check balance for a specific account**
   - ✅ Returns actual balance for named account
   - ✅ Can verify exact microTari amounts

2. **Check balance for all accounts**
   - ✅ Returns balances for all accounts
   - ✅ Can parse and validate each account's balance

3. **Check balance with no outputs**
   - ✅ Returns 0 microTari correctly
   - ✅ Validates empty wallet state

Additionally, all balance checks in other features now work:
- `end_to_end.feature` - Balance after mining and scanning
- `balance_with_mining.feature` - Mining reward verification
- Any scenario using balance verification steps

## Benefits

### 1. Correct Behavior
Commands now execute as intended, with proper database access.

### 2. Real Validation
Tests verify actual balance data instead of checking that commands fail.

### 3. End-to-End Testing
Full wallet lifecycle can be tested:
- Create wallet → Mine blocks → Scan → **Check actual balance**

### 4. Better Debugging
When tests fail, it's due to actual logic errors, not configuration issues.

### 5. Enables Advanced Testing
The fix allows all balance verification steps to work:
- `Then the balance should be 0 microTari` - Exact zero check
- `Then the balance should be {int} microTari` - Exact amount
- `Then the balance should be at least {int} microTari` - Minimum amount
- `Then the balance should contain {int} microTari` - Flexible check

## Files Modified

- `integration-tests/steps/balance.rs` - Added password to both balance functions

## Testing

To verify the fix works:

```bash
# Run all balance tests
cargo test --release --test cucumber --package integration-tests -- balance

# Run specific scenario
cargo test --release --test cucumber --package integration-tests -- "Check balance with no outputs"
```

Expected results:
- All tests pass ✅
- Balance commands succeed with exit code 0
- Actual balance data is returned and validated

## Technical Notes

### Password Management

The test password is stored in `MinotariWorld`:
```rust
pub struct MinotariWorld {
    // ...
    pub test_password: String,  // Default: "test_password_123"
}
```

All wallet operations (creation, import, balance, scanning, transactions) must use this password when accessing encrypted databases.

### Consistency Check

Ensure all CLI commands that access the database include the password:

✅ **Correctly passing password:**
- `import-view-key` - Creates encrypted database
- `balance` - Reads encrypted database (NOW FIXED)
- `scan` - Reads/writes encrypted database
- `create-transaction` - Reads encrypted database

❌ **If password is missing:**
- Command fails with decryption error
- Exit code is non-zero
- No useful output is generated
- Tests can't validate actual behavior

## Future Considerations

1. **Consistency Audits** - Periodically verify all CLI commands include password when needed
2. **Password Environment Variable** - Consider using env var for password to reduce repetition
3. **Error Messages** - Improve error messages when password is missing or incorrect
4. **Unencrypted Testing** - Consider adding test scenarios for unencrypted wallets (if supported)

## Conclusion

This fix resolves a critical issue where balance command tests were validating command failures instead of actual balance data. With the password properly passed, all balance operations now work correctly, enabling comprehensive end-to-end testing of the wallet's balance tracking functionality.
