# Integration Tests Fix Summary

## Problem

The command `cargo test -p integration-tests` was not running due to several configuration issues in the newly created integration-tests workspace package.

## Root Causes

### 1. Incorrect Feature Path
The test runner in `integration-tests/tests/cucumber.rs` used a relative path `"features/"` which didn't resolve correctly when tests were executed, as the working directory differs from the package root.

### 2. Missing Dependencies
The step definition code imported `tari_transaction_components` and `tari_utilities` crates, but these dependencies were not declared in `integration-tests/Cargo.toml`.

### 3. Missing Build Prerequisite
The package requires `protoc` (Protocol Buffers compiler) to build gRPC dependencies, but this wasn't documented.

### 4. Missing Main Function
With `harness = false` in Cargo.toml, Cargo expects a `main` function as the entry point, not `#[test]` functions. The cucumber test was using `#[tokio::test]` which caused a "main function not found" error.

## Solutions Implemented

### 1. Fixed Feature Path Resolution

**File**: `integration-tests/tests/cucumber.rs`

Changed from relative path to absolute path using `CARGO_MANIFEST_DIR`:

```rust
// Before
steps::MinotariWorld::cucumber()
    .max_concurrent_scenarios(1)
    .run("features/")
    .await;

// After
use std::path::PathBuf;

let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
let features_path = manifest_dir.join("features");

steps::MinotariWorld::cucumber()
    .max_concurrent_scenarios(1)
    .run(features_path)
    .await;
```

**Why this works**: `CARGO_MANIFEST_DIR` is an environment variable set by Cargo that contains the absolute path to the package's directory (where Cargo.toml is located).

### 2. Added Missing Dependencies

**File**: `integration-tests/Cargo.toml`

Added:
```toml
tari_transaction_components = { git = "https://github.com/tari-project/tari/", rev = "766f80ccc20596413ee208311750c11e02a2841d" }
tari_utilities = { version = "0.8", features = ["std"] }
```

These dependencies were already used in the code but not declared, causing compilation errors.

### 3. Documented Prerequisites

**File**: `integration-tests/README.md`

Added Prerequisites section:

```markdown
## Prerequisites

Before building or running the tests, you need to install the Protocol Buffers compiler (`protoc`):

### Ubuntu/Debian
\`\`\`bash
sudo apt-get install protobuf-compiler
\`\`\`

### macOS
\`\`\`bash
brew install protobuf
\`\`\`
```

Also updated:
- Directory structure to reflect workspace layout
- Test running commands to use `cargo test -p integration-tests`
- Code examples to use correct struct names

### 4. Fixed Main Function Requirement

**File**: `integration-tests/tests/cucumber.rs`

Changed from test function to main function:

```rust
// Before - causes "main function not found" error
#[tokio::test]
async fn run_cucumber_tests() {
    // test code
}

// After - proper entry point for harness = false
#[tokio::main]
async fn main() {
    // test code
}
```

**Why this works**: When `harness = false` is set in the `[[test]]` configuration in Cargo.toml, Cargo doesn't use the default test harness. Instead, it expects a `main` function as the entry point. The cucumber framework requires this to provide its own test discovery and execution system.

The `#[tokio::main]` macro:
- Creates an async runtime
- Makes the main function async
- Provides the entry point that Cargo expects with custom harness

## Verification

After these fixes, the integration tests can be run successfully:

```bash
# Install protoc (one-time setup)
sudo apt-get install protobuf-compiler  # Ubuntu/Debian
# or
brew install protobuf  # macOS

# Run tests from workspace root
cargo test -p integration-tests

# Or from integration-tests directory
cd integration-tests
cargo test
```

## Files Changed

1. **integration-tests/tests/cucumber.rs**
   - Added proper path resolution using CARGO_MANIFEST_DIR
   - Added std::path::PathBuf import
   - Changed from `#[tokio::test]` to `#[tokio::main] async fn main()`

2. **integration-tests/Cargo.toml**
   - Added tari_transaction_components dependency
   - Added tari_utilities dependency

3. **integration-tests/README.md**
   - Added Prerequisites section
   - Updated directory structure
   - Fixed test commands
   - Updated examples

## Technical Details

### Custom Test Harness (harness = false)

The `[[test]]` section in Cargo.toml has `harness = false`, which tells Cargo:

1. **Don't use the default test harness** - No automatic test discovery
2. **Expect a main function** - The test file must provide `fn main()`
3. **Custom test runner** - The framework (cucumber) handles test execution

This is necessary for cucumber because:
- Cucumber needs to discover and parse `.feature` files
- It has its own test execution model (scenarios, steps)
- It provides rich test output formatting
- Standard Rust test harness doesn't support BDD-style tests

**Configuration in Cargo.toml:**
```toml
[[test]]
name = "cucumber"
harness = false  # This requires main function
```

**Corresponding code:**
```rust
#[tokio::main]  // Creates async runtime
async fn main() {  // Entry point for harness = false
    steps::MinotariWorld::cucumber()
        .run(features_path)
        .await;
}
```

### Path Resolution in Rust Tests

When Cargo runs tests, the working directory is typically the workspace root or target directory, not the package directory. Therefore, relative paths like `"features/"` don't work reliably.

The solution uses `env!("CARGO_MANIFEST_DIR")` which is:
- Set at compile time by Cargo
- Contains the absolute path to the package root
- Works consistently regardless of where tests are run from

### Dependency Management in Workspaces

In a Cargo workspace, each package has its own `Cargo.toml` and must declare all dependencies it uses, even if they're already declared in other workspace packages. The `integration-tests` package depends on the `minotari` package but also needs direct dependencies on Tari crates used in test code.

## Impact

These fixes enable:
- ✅ Proper execution of `cargo test -p integration-tests`
- ✅ Cucumber tests finding feature files correctly
- ✅ All dependencies resolving properly
- ✅ Clear documentation for new contributors

## Future Improvements

Potential enhancements:
1. Add CI workflow to verify tests can build and run
2. Add more detailed troubleshooting guide
3. Consider adding a build.rs script to check for protoc
4. Add example test output to documentation
