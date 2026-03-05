# Balance Error Message Fix

## Problem Statement

The `see_balance_info()` and `see_all_balances()` step definitions had misleading error messages that made it appear they were testing for command failure when they were actually testing for command success.

## The Issue

### Confusing Error Messages

Both steps contained assertions like this:

```rust
assert_eq!(
    world.last_command_exit_code,
    Some(0),
    "Balance command failed: {}",
    world.last_command_error.as_deref().unwrap_or("")
);
```

### The Confusion

**What the code does:**
- Checks that `exit_code == 0` (command succeeded)
- This is testing for **SUCCESS**

**What the error message says:**
- "Balance command failed"
- This sounds like we're testing for **FAILURE**

**The problem:**
- The error message is shown when the assertion **FAILS**
- I.e., when the exit code is **NOT** 0
- So it's actually saying "expected success but got failure"
- But the wording makes it sound backwards

### Example Scenario

If a balance command actually fails with exit code 1:

**What happens:**
```
assertion failed: `(left == right)`
  left: `Some(1)`,
 right: `Some(0)`
Balance command failed: Database decryption error
```

**What it looks like:**
- Looks like we're testing that the command fails
- But we're actually testing that it succeeds
- The message is shown because our expectation (success) wasn't met

## The Solution

### New Error Messages

Changed both assertions to use clear, explicit error messages:

```rust
assert_eq!(
    world.last_command_exit_code,
    Some(0),
    "Balance command should succeed but got exit code {:?}. Error: {}",
    world.last_command_exit_code,
    world.last_command_error.as_deref().unwrap_or("")
);
```

### Why This is Better

**Clarity:**
- "should succeed" - clearly states expectation
- "but got exit code {:?}" - shows what actually happened
- No ambiguity about test intent

**Information:**
- Shows both expected and actual values
- Includes error message for debugging
- Makes failures easy to diagnose

## Changed Functions

### 1. see_balance_info()

**Location:** `integration-tests/steps/balance.rs`, lines 65-92

**Before:**
```rust
#[then("I should see the balance information")]
async fn see_balance_info(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Balance command failed: {}",  // ← Confusing
        world.last_command_error.as_deref().unwrap_or("")
    );
    // ... rest of validation
}
```

**After:**
```rust
#[then("I should see the balance information")]
async fn see_balance_info(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Balance command should succeed but got exit code {:?}. Error: {}",  // ← Clear
        world.last_command_exit_code,
        world.last_command_error.as_deref().unwrap_or("")
    );
    // ... rest of validation
}
```

### 2. see_all_balances()

**Location:** `integration-tests/steps/balance.rs`, lines 112-134

**Before:**
```rust
#[then("I should see balance for all accounts")]
async fn see_all_balances(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Balance command failed: {}",  // ← Confusing
        world.last_command_error.as_deref().unwrap_or("")
    );
    // ... rest of validation
}
```

**After:**
```rust
#[then("I should see balance for all accounts")]
async fn see_all_balances(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Balance command should succeed but got exit code {:?}. Error: {}",  // ← Clear
        world.last_command_exit_code,
        world.last_command_error.as_deref().unwrap_or("")
    );
    // ... rest of validation
}
```

## Impact

### Test Behavior

**No functional changes:**
- Tests still work exactly the same way
- Still checking for exit code 0 (success)
- Still validating balance content
- No changes to actual test logic

### Error Messages

**Old error output:**
```
assertion failed: `(left == right)`
  left: `Some(1)`,
 right: `Some(0)`
Balance command failed: Database decryption error
```

**New error output:**
```
assertion failed: `(left == right)`
  left: `Some(1)`,
 right: `Some(0)`
Balance command should succeed but got exit code Some(1). Error: Database decryption error
```

**Improvements:**
- Clearly states expectation (should succeed)
- Shows actual exit code received
- No ambiguity about test intent

### Affected Scenarios

These error messages appear in test failures for:

**balance.feature:**
1. **Check balance for a specific account**
   - Step: "Then I should see the balance information"
   - Uses: `see_balance_info()`

2. **Check balance for all accounts**
   - Step: "Then I should see balance for all accounts"
   - Uses: `see_all_balances()`

## Benefits

### 1. Clear Test Intent

**Before:**
- Error message could be misinterpreted
- Looked like testing for failure
- Confusion about what step does

**After:**
- Explicitly states expectation
- Obvious we're testing for success
- No room for misinterpretation

### 2. Better Debugging

**Before:**
```
Balance command failed: Database decryption error
```
- Hard to tell if this is expected or unexpected
- Is the test checking that it fails?

**After:**
```
Balance command should succeed but got exit code Some(1). Error: Database decryption error
```
- Clear that success was expected
- Shows exact exit code received
- Immediately actionable

### 3. Code Maintenance

**Before:**
- New developers might be confused
- Error message suggests different test intent
- Could lead to incorrect "fixes"

**After:**
- Self-documenting code
- Error message matches test intent
- Less likely to be misunderstood

### 4. Consistency

**Pattern used throughout codebase:**
```rust
assert_eq!(actual, expected, "Expected X but got {:?}", actual);
```

These changes align with common testing patterns.

## No Other Changes Needed

### Other Steps Already Clear

Other balance validation steps don't have this issue:

```rust
#[then("the balance should be zero")]
async fn balance_is_zero(world: &mut MinotariWorld) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert_eq!(balance, 0, "Expected zero balance, got {}", balance);
    // ↑ Clear: expects zero, shows actual if different
}

#[then(regex = r"^the balance should be (\d+) microTari$")]
async fn balance_should_be_exact(world: &mut MinotariWorld, expected: u64) {
    let balance = world.parse_balance_from_output().expect("Could not parse balance");
    assert_eq!(
        balance, expected,
        "Expected balance {} microTari, got {}",  // ↑ Clear
        expected, balance
    );
}
```

These already have clear error messages that state expectations.

## Testing

### Verify the Fix

Run balance tests:
```bash
cargo test --release --test cucumber --package integration-tests -- balance
```

### When Tests Pass

No change in output - tests still pass as before.

### When Tests Fail

Now you'll see clear error messages like:
```
Balance command should succeed but got exit code Some(1). Error: [error message]
```

Instead of the confusing:
```
Balance command failed: [error message]
```

## Related Documentation

- **BALANCE_PASSWORD_FIX.md** - Documents password fix that enables commands to succeed
- **BALANCE_VALIDATION_IMPROVEMENTS.md** - Documents content validation enhancements
- **integration-tests/features/balance.feature** - Feature file with scenarios

## Conclusion

This is a **documentation and clarity fix** with no functional changes. The steps still validate that balance commands succeed and return valid data, but now the error messages clearly communicate this intent.

The change eliminates confusion about what these steps are testing and makes debugging failures much easier by providing clear, explicit error messages.
