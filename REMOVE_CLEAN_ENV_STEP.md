# Remove "Given I have a clean test environment" Step

## Problem

The cucumber feature files required an explicit "Given I have a clean test environment" step at the beginning of most scenarios. This was boilerplate that added noise to the feature files and required developers to remember to include it.

## Solution

Integrated the temp directory setup directly into the `MinotariWorld::new()` initialization method, making test isolation automatic and implicit rather than explicit.

## Changes Made

### 1. Modified MinotariWorld Initialization

**File:** `integration-tests/steps/common.rs`

**Before:**
```rust
impl MinotariWorld {
    pub fn new() -> Self {
        // ... other setup ...
        Self {
            temp_dir: None,  // Not initialized
            // ... other fields ...
        }
    }
}
```

**After:**
```rust
impl MinotariWorld {
    pub fn new() -> Self {
        // ... other setup ...
        
        // Automatically setup temp directory for test isolation
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        
        Self {
            temp_dir: Some(temp_dir),  // Always initialized
            // ... other fields ...
        }
    }
}
```

### 2. Removed Step Definition

**File:** `integration-tests/steps/common.rs`

Removed the step definition:
```rust
#[given("I have a clean test environment")]
async fn clean_environment(world: &mut MinotariWorld) {
    world.setup_temp_dir();
}
```

Note: The `setup_temp_dir()` method is kept in case manual re-initialization is needed.

### 3. Updated Feature Files

Removed "Given I have a clean test environment" from 8 scenarios across 3 files:

#### wallet_creation.feature (3 scenarios)
- Create a new wallet without encryption
- Create a new wallet with password encryption  
- Create wallet with custom output file

#### wallet_import.feature (4 scenarios)
- Import wallet using view and spend keys
- Import wallet with custom birthday
- Create wallet from seed words
- Show seed words for existing wallet

#### base_node.feature (1 scenario)
- Start a base node

## Example Improvement

### Before
```gherkin
Scenario: Create a new wallet without encryption
  Given I have a clean test environment    # Boilerplate setup
  When I create a new address without a password
  Then the wallet file should be created
  And the wallet should contain a valid address
```

### After
```gherkin
Scenario: Create a new wallet without encryption
  When I create a new address without a password
  Then the wallet file should be created
  And the wallet should contain a valid address
```

## Benefits

1. **Cleaner Feature Files**
   - Removed 8 instances of boilerplate setup step
   - Feature files focus on behavior, not technical setup
   - Easier to read and understand scenarios

2. **Automatic Test Isolation**
   - Every scenario automatically gets a fresh temp directory
   - No risk of forgetting to add the setup step
   - Consistent test isolation across all scenarios

3. **Convention Over Configuration**
   - Setup happens implicitly when World is created
   - Follows BDD best practices of hiding technical details
   - Developers focus on "what" not "how"

4. **Reduced Duplication**
   - DRY principle - setup code in one place
   - Less maintenance burden
   - Easier to modify setup logic in the future

5. **Better Developer Experience**
   - New scenarios don't need boilerplate
   - Feature files are more concise
   - Clear separation between setup and behavior

## Technical Details

### When Does Setup Happen?

The temp directory is created when `MinotariWorld::new()` is called, which happens:
- Once per scenario (due to `#[world(init = Self::new)]` attribute)
- Before any step definitions execute
- Automatically by the cucumber framework

### Cleanup

Cleanup still happens automatically via the `Drop` implementation:
```rust
impl Drop for MinotariWorld {
    fn drop(&mut self) {
        self.cleanup();
    }
}
```

The `TempDir` also has its own `Drop` implementation that removes the directory.

### Manual Re-initialization

If a scenario needs to create a fresh temp directory mid-test, the `setup_temp_dir()` method is still available:
```rust
world.setup_temp_dir(); // Creates new temp directory
```

## Backward Compatibility

This change is backward compatible because:
- The step definition was only used internally in feature files
- No external code depended on the step
- The `setup_temp_dir()` method is still available if needed

## Testing

The change maintains the same behavior:
- All scenarios still get isolated temp directories
- Tests that previously called the step now get automatic setup
- No functional changes to test behavior

## Future Improvements

Similar improvements could be made for:
1. Database setup - could be made automatic when needed
2. Other common setup steps that appear frequently
3. Consider using cucumber hooks (`Before`, `After`) for cross-cutting concerns

## Related

This change follows the pattern established in the cucumber best practices:
- Feature files should describe behavior from user perspective
- Technical setup should be hidden in step definitions or world initialization
- Focus on "what" the system does, not "how" it's tested
