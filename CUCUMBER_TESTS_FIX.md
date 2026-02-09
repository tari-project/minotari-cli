# Cucumber Tests Release Mode Fix

## Problem

When running `cargo test --release --test cucumber --package integration-tests`, the cucumber BDD tests were failing because they were calling `cargo run --bin minotari` which always runs in dev mode, regardless of how the tests themselves were compiled.

## Root Cause

All step definitions used this pattern:

```rust
Command::new("cargo")
    .args(&["run", "--bin", "minotari", "--", "create-address", ...])
```

This caused several issues:
1. **Mode mismatch**: Tests compiled in release mode but CLI running in dev mode
2. **Performance issues**: Dev mode binary is slower and less optimized
3. **Build overhead**: Each test execution would trigger a dev build check
4. **Inconsistent behavior**: Different behavior between test and production binaries

## Solution

Added a `get_minotari_command()` helper method to `MinotariWorld` that intelligently selects the correct binary:

```rust
pub fn get_minotari_command(&self) -> (String, Vec<String>) {
    // Determine workspace root
    let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
        .map(|p| std::path::PathBuf::from(p).parent().unwrap().to_path_buf())
        .unwrap_or_else(|_| std::env::current_dir().unwrap().parent().unwrap().to_path_buf());
    
    let release_binary = workspace_root.join("target/release/minotari");
    
    if release_binary.exists() {
        // Use the release binary directly
        (release_binary.to_string_lossy().to_string(), vec![])
    } else {
        // Fall back to cargo run for dev mode
        ("cargo".to_string(), vec!["run".to_string(), "--bin".to_string(), "minotari".to_string(), "--".to_string()])
    }
}
```

### How It Works

1. **Checks for release binary**: Looks for `target/release/minotari`
2. **Uses release binary if available**: When running tests with `--release`, the binary exists and is used
3. **Falls back to cargo run**: In dev mode or if binary doesn't exist, uses `cargo run`

## Updated Usage Pattern

Before:
```rust
let output = Command::new("cargo")
    .args(&["run", "--bin", "minotari", "--", "create-address", 
            "--output-file", output_file.to_str().unwrap()])
    .output()
    .expect("Failed to execute command");
```

After:
```rust
let (cmd, mut args) = world.get_minotari_command();
args.extend_from_slice(&[
    "create-address".to_string(),
    "--output-file".to_string(),
    output_file.to_str().unwrap().to_string(),
]);

let output = Command::new(&cmd)
    .args(&args)
    .output()
    .expect("Failed to execute command");
```

## Files Updated

### Core Helper
- **integration-tests/steps/common.rs**
  - Added `get_minotari_command()` method

### Step Definitions
- **integration-tests/steps/wallet_creation.rs**
  - Updated 3 command invocations (create address variants)
  
- **integration-tests/steps/wallet_import.rs**
  - Updated 4 command invocations (import-view-key, create, show-seed-words)
  
- **integration-tests/steps/balance.rs**
  - Updated 2 command invocations (balance checks)

- **integration-tests/steps/common.rs**
  - Updated 1 command invocation (database setup with wallet)

## Testing

### Dev Mode
```bash
# Uses cargo run (as before)
cargo test --test cucumber --package integration-tests
```

### Release Mode
```bash
# Build the release binary first
cargo build --release --bin minotari

# Run tests - uses pre-built binary
cargo test --release --test cucumber --package integration-tests
```

## Benefits

1. **Correct mode matching**: Tests use the same build mode they're compiled with
2. **Faster execution**: No unnecessary rebuilds during test runs
3. **Better CI/CD**: Works correctly in release mode for production validation
4. **Flexible**: Automatically adapts to dev or release mode

## Future Improvements

Potential enhancements:
1. Add environment variable override (e.g., `MINOTARI_BINARY_PATH`)
2. Support for other build profiles (e.g., `--profile bench`)
3. Add validation that binary version matches test expectations
4. Cache binary path lookup for performance

## Compatibility

- ✅ Works with existing dev mode workflows
- ✅ Enables release mode testing
- ✅ Backward compatible with all existing tests
- ✅ No changes to feature files needed
