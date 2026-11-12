-- An output hash can be on chain more than once (re-created after it was
-- spent), and one output can belong to two accounts. An active row is one
-- instance of an output in one account: identified by the block it was mined in.
DROP INDEX IF EXISTS idx_outputs_output_hash_active;
CREATE UNIQUE INDEX idx_outputs_account_hash_block_active
    ON outputs(account_id, output_hash, mined_in_block_hash) WHERE deleted_at IS NULL;

-- tx_id is derived from the account's view key, so two accounts with the same
-- view key derive the same tx_id for an output they share.
DROP INDEX IF EXISTS idx_outputs_tx_id_active;
CREATE UNIQUE INDEX idx_outputs_account_tx_id_active ON outputs(account_id, tx_id) WHERE deleted_at IS NULL;
