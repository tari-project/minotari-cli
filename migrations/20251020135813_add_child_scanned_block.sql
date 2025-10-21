-- Add migration script here
-- SQLite doesn't support ALTER COLUMN, so we need to recreate the table

-- Step 1: Create new table with nullable account_id and new child_account_id column
CREATE TABLE scanned_tip_blocks_new (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER,
    child_account_id INTEGER,
    hash Blob NOT NULL,
    height INTEGER NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (account_id) REFERENCES accounts(id),
    FOREIGN KEY (child_account_id) REFERENCES child_accounts(id)
);

-- Step 2: Copy data from old table
INSERT INTO scanned_tip_blocks_new (id, account_id, hash, height, created_at)
SELECT id, account_id, hash, height, created_at
FROM scanned_tip_blocks;

-- Step 3: Drop old table
DROP TABLE scanned_tip_blocks;

-- Step 4: Rename new table
ALTER TABLE scanned_tip_blocks_new RENAME TO scanned_tip_blocks;

