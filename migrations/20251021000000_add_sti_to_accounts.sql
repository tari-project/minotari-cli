-- Add Single Table Inheritance columns to accounts table
-- This allows child accounts to be stored in the same table as parent accounts
-- Using create-copy-drop-rename pattern for better compatibility

-- Recreate scanned_tip_blocks without child_account_id and its FK
PRAGMA foreign_keys=off;
-- Create new table with additional STI columns
CREATE TABLE accounts_new (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_type TEXT NOT NULL DEFAULT 'parent' CHECK(account_type IN ('parent', 'child_tapplet', 'child_viewkey')),
    friendly_name TEXT,
    unencrypted_view_key_hash blob ,
    encrypted_view_private_key blob,
    encrypted_spend_public_key blob,
    cipher_nonce blob NOT NULL,
    birthday INTEGER,
    parent_account_id INTEGER,
    for_tapplet_name TEXT,
    version TEXT,
    tapplet_pub_key TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Copy existing data from old table (all existing accounts are parent accounts)
INSERT INTO accounts_new
( id,account_type, friendly_name, unencrypted_view_key_hash, encrypted_view_private_key,
  encrypted_spend_public_key, cipher_nonce, birthday, created_at )
SELECT
    id, 'parent', friendly_name, unencrypted_view_key_hash, encrypted_view_private_key,
    encrypted_spend_public_key, cipher_nonce, birthday, created_at
FROM accounts;

INSERT INTO accounts_new (
    account_type,
    friendly_name,
    parent_account_id,
    for_tapplet_name,
    version,
    tapplet_pub_key,
    -- -- Copy parent's cryptographic fields (child accounts share parent keys)
    -- unencrypted_view_key_hash,
    -- encrypted_view_private_key,
    -- encrypted_spend_public_key,
    -- cipher_nonce,
    -- birthday,
    created_at
)
SELECT
    'child_tapplet' as account_type,
    ca.child_account_name as friendly_name,
    ca.parent_account_id,
    ca.for_tapplet_name,
    ca.version,
    ca.tapplet_pub_key,
    -- Copy parent's cryptographic fields
    -- a.unencrypted_view_key_hash,
    -- a.encrypted_view_private_key,
    -- a.encrypted_spend_public_key,
    -- a.cipher_nonce,
    -- a.birthday,
    ca.created_at
FROM child_accounts ca;



-- Add indexes for common queries
CREATE INDEX idx_accounts_parent_account_id ON accounts_new(parent_account_id);
CREATE INDEX idx_accounts_account_type ON accounts_new(account_type);
CREATE INDEX idx_accounts_for_tapplet_name ON accounts_new(for_tapplet_name);

-- Update scanned_tip_blocks to reference the new account IDs
-- Map child_account_id to the new account_id in the unified accounts table
-- UPDATE scanned_tip_blocks
-- SET account_id = (
--     SELECT a.id
--     FROM accounts_new a
--     INNER JOIN child_accounts ca ON ca.child_account_name = a.friendly_name
--         AND ca.parent_account_id = a.parent_account_id
--         AND a.account_type = 'child_tapplet'
--     WHERE ca.id = scanned_tip_blocks.child_account_id
-- );




CREATE TABLE scanned_tip_blocks_new (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL,
    hash Blob NOT NULL,
    height INTEGER NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (account_id) REFERENCES accounts_new(id)
);

INSERT INTO scanned_tip_blocks_new (id, account_id, hash, height, created_at)
SELECT id, account_id, hash, height, created_at
FROM scanned_tip_blocks;

DROP TABLE scanned_tip_blocks;

ALTER TABLE scanned_tip_blocks_new RENAME TO scanned_tip_blocks;




-- Drop old table
DROP TABLE accounts;
-- Rename new table to accounts
ALTER TABLE accounts_new RENAME TO accounts;

PRAGMA foreign_keys=on;

-- Drop child_accounts table as it's no longer needed
DROP TABLE child_accounts;


