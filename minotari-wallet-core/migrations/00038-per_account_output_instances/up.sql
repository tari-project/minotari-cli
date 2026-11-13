-- An output hash can be on chain more than once (re-created after it was
-- spent), and one output can belong to two accounts. An active row is one
-- instance of an output in one account: identified by the block it was mined in.
-- Accounts scanned together get a shared output for the first matching account
-- only; accounts scanned separately each store it.
DROP INDEX IF EXISTS idx_outputs_output_hash_active;
CREATE UNIQUE INDEX idx_outputs_account_hash_block_active
    ON outputs(account_id, output_hash, mined_in_block_hash) WHERE deleted_at IS NULL;

-- tx_id is derived from the account's view key, so two accounts with the same
-- view key derive the same tx_id for an output they share. Known follow-up:
-- displayed_transactions.id is that tx_id and is unique across accounts, so such
-- accounts overwrite each other's history entry for a shared output. Only
-- accounts without a fingerprint or Ledger accounts sharing a view key can.
DROP INDEX IF EXISTS idx_outputs_tx_id_active;
CREATE UNIQUE INDEX idx_outputs_account_tx_id_active ON outputs(account_id, tx_id) WHERE deleted_at IS NULL;

-- Scans now check for an existing credit or debit before every one they write.
CREATE INDEX idx_balance_changes_caused_by_output ON balance_changes(caused_by_output_id)
    WHERE caused_by_output_id IS NOT NULL;
CREATE INDEX idx_balance_changes_caused_by_input ON balance_changes(caused_by_input_id)
    WHERE caused_by_input_id IS NOT NULL;
