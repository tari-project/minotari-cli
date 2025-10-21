-- Drop the child_account_id column from scanned_tip_blocks
-- All child account references are now in the unified account_id column
ALTER TABLE scanned_tip_blocks DROP COLUMN child_account_id;

-- Drop the child_accounts table as all data has been migrated to accounts table
DROP TABLE child_accounts;
