-- Add Single Table Inheritance columns to accounts table
-- This allows child accounts to be stored in the same table as parent accounts
-- Using create-copy-drop-rename pattern for better compatibility

-- Create new table with additional STI columns
CREATE TABLE accounts_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_type TEXT NOT NULL DEFAULT 'parent' CHECK(account_type IN ('parent', 'child')),
    friendly_name TEXT,
    unencrypted_view_key_hash blob ,
    encrypted_view_private_key blob,
    encrypted_spend_public_key blob,
    cipher_nonce blob NOT NULL,
    birthday INTEGER NOT NULL,
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

-- Drop old table
DROP TABLE accounts;

INSERT INTO accounts_new (
    account_type,
    friendly_name,
    parent_account_id,
    for_tapplet_name,
    version,
    tapplet_pub_key,
    -- Copy parent's cryptographic fields (child accounts share parent keys)
    unencrypted_view_key_hash,
    encrypted_view_private_key,
    encrypted_spend_public_key,
    cipher_nonce,
    birthday,
    created_at
)
SELECT
    'child' as account_type,
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

-- Rename new table to accounts
ALTER TABLE accounts_new RENAME TO accounts;

-- Update scanned_tip_blocks to reference the new account IDs
-- Map child_account_id to the new account_id in the unified accounts table
UPDATE scanned_tip_blocks
SET account_id = (
    SELECT a.id
    FROM accounts a
    INNER JOIN child_accounts ca ON ca.child_account_name = a.friendly_name
        AND ca.parent_account_id = a.parent_account_id
        AND a.account_type = 'child'
    WHERE ca.id = scanned_tip_blocks.child_account_id
);


Alter table scanned_tip_blocks
    drop column child_account_id;
    
drop table child_accounts;


-- Add indexes for common queries
CREATE INDEX idx_accounts_parent_account_id ON accounts(parent_account_id);
CREATE INDEX idx_accounts_account_type ON accounts(account_type);
CREATE INDEX idx_accounts_for_tapplet_name ON accounts(for_tapplet_name);
