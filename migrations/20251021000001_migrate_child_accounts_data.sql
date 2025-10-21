-- Migrate data from child_accounts table into accounts table using Single Table Inheritance

-- Insert child accounts into accounts table
INSERT INTO accounts (
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
    a.unencrypted_view_key_hash,
    a.encrypted_view_private_key,
    a.encrypted_spend_public_key,
    a.cipher_nonce,
    a.birthday,
    ca.created_at
FROM child_accounts ca
INNER JOIN accounts a ON a.id = ca.parent_account_id;

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
)
WHERE child_account_id IS NOT NULL;
