# Migration Merge Summary

## Problem Statement

The integration test branch was missing database migrations that exist in the main branch, causing potential schema inconsistencies.

## Analysis

### Branch Divergence

**Main Branch:**
- Location: `migrations/` (root of repository)
- Migration count: 27 (00001-00027)
- Latest migrations: 00026-create_webhook_queue, 00027-make_wallet_output_json_not_null

**Integration Test Branch:**
- Location: `minotari/migrations/` (workspace structure)
- Migration count: 25 (00001-00025)
- Missing: 00026 and 00027

### Root Cause

1. Integration test branch was created from main at commit 4586f7b
2. Main branch continued development and added 2 new migrations
3. Integration test branch underwent workspace restructuring (moved migrations from root to minotari/)
4. The parallel development caused migrations to be in sync by number but different by location
5. When main added migrations 00026-00027, they weren't present in the integration branch

## Solution

Since the branches have diverged structurally (monolithic vs workspace), a simple git merge wouldn't work correctly. The solution was to manually copy the missing migrations from main branch to the workspace structure.

### Steps Taken

1. Fetched main branch
2. Identified missing migrations (00026, 00027)
3. Extracted migration files from main branch
4. Placed them in workspace-compliant path: `minotari/migrations/`
5. Committed and pushed to integration test branch

## Migration Details

### Migration 00026: create_webhook_queue

**Purpose:** Creates infrastructure for webhook delivery tracking and retry logic.

**Content:**
```sql
CREATE TABLE IF NOT EXISTS webhook_queue (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    event_id INTEGER,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    target_url TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_retry_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_error TEXT,
    FOREIGN KEY (event_id) REFERENCES events(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_webhook_queue_pending 
    ON webhook_queue(status, next_retry_at);
```

**Features:**
- Tracks webhook delivery status (pending, success, failed, permanent_failure)
- Supports retry mechanism with attempt counts
- Stores error messages for debugging
- Indexed for efficient worker polling
- Optional link to events table for traceability

**Use Cases:**
- Notifying external systems of wallet events
- Reliable delivery with automatic retries
- Error tracking and debugging
- Webhook delivery monitoring

### Migration 00027: make_wallet_output_json_not_null

**Purpose:** Enforces NOT NULL constraint on wallet_output_json field in outputs table.

**Why Needed:**
SQLite doesn't support `ALTER COLUMN`, so the entire outputs table must be recreated with the new constraint.

**Process:**
1. Drop all dependent indexes
2. Create new table with NOT NULL constraint
3. Copy data from old table to new table
4. Drop old table
5. Rename new table to outputs
6. Recreate all indexes

**Impact:**
- Ensures wallet_output_json field always has a value
- Prevents NULL-related bugs in output processing
- Maintains referential integrity
- No data loss during migration

**Key Constraint:**
```sql
wallet_output_json TEXT NOT NULL
```

## Verification

### Migration Count

Before: 25 migrations
```
minotari/migrations/00001-init/
...
minotari/migrations/00025-add_reversal_flags_to_balance_changes/
```

After: 27 migrations
```
minotari/migrations/00001-init/
...
minotari/migrations/00025-add_reversal_flags_to_balance_changes/
minotari/migrations/00026-create_webhook_queue/ ✅
minotari/migrations/00027-make_wallet_output_json_not_null/ ✅
```

### File Locations

```bash
# Migration 26
minotari/migrations/00026-create_webhook_queue/up.sql (16 lines)

# Migration 27
minotari/migrations/00027-make_wallet_output_json_not_null/up.sql (65 lines)
```

### Verification Commands

```bash
# Count migrations
ls minotari/migrations/ | wc -l
# Expected: 27

# List latest migrations
ls minotari/migrations/ | tail -5
# Expected to include 00026 and 00027

# Check migration content
cat minotari/migrations/00026-create_webhook_queue/up.sql
cat minotari/migrations/00027-make_wallet_output_json_not_null/up.sql
```

## Benefits

### Immediate Benefits

1. **Schema Consistency**
   - Database schema now matches main branch
   - Tests use same schema as production

2. **Webhook Functionality**
   - Webhook queue table available for integration tests
   - Can test webhook delivery scenarios

3. **Data Integrity**
   - NOT NULL constraint ensures data quality
   - Prevents NULL-related bugs

### Long-term Benefits

1. **Reduced Merge Conflicts**
   - Migrations in sync reduces future conflicts
   - Easier to merge updates from main

2. **Test Reliability**
   - Tests run against current schema
   - Catches schema-related issues early

3. **Development Efficiency**
   - Developers don't hit migration errors
   - Smooth onboarding for new contributors

## Future Considerations

### Keeping Migrations in Sync

**Recommended Approach:**
1. Periodically merge main into integration test branch
2. Check for new migrations in main
3. Copy new migrations to workspace structure
4. Test that migrations apply correctly

**Automation Opportunity:**
- Create a script to check for migration differences
- Automate copying of new migrations to workspace structure
- Add to CI/CD pipeline

### Workspace Migration Strategy

When merging branches with different structures:
1. Identify structural differences (monolithic vs workspace)
2. Map file locations between structures
3. Use git show to extract files from source branch
4. Place files in target structure
5. Verify and commit

### Migration Naming Convention

The project follows a clear naming convention:
```
00XXX-description_of_change/
  └── up.sql
```

Where XXX is zero-padded sequential number (001, 002, ..., 026, 027)

This ensures migrations apply in correct order.

## Related Documentation

- [WORKSPACE_MIGRATION.md](WORKSPACE_MIGRATION.md) - Workspace restructuring details
- [INTEGRATION_TESTS_FIX.md](INTEGRATION_TESTS_FIX.md) - Integration test fixes
- [README.md](README.md) - Project overview

## Summary

✅ **Problem Solved:** Missing migrations 00026 and 00027 added  
✅ **Method:** Manual copy from main to workspace structure  
✅ **Verification:** 27 migrations now present (matching main)  
✅ **Impact:** Schema consistency, webhook support, data integrity  
✅ **Documentation:** Complete summary and verification steps  

The migration merge is complete and the integration test branch now has all migrations in sync with the main branch.
