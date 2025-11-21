-- Add unique constraints to pending_outputs and pending_inputs
-- This prevents duplicate pending transactions from being inserted

-- For pending_outputs, an output_hash should be unique per account for active (non-deleted) entries
CREATE UNIQUE INDEX IF NOT EXISTS idx_pending_outputs_unique_active
ON pending_outputs(account_id, output_hash)
WHERE deleted_at IS NULL;

-- For pending_inputs, the combination of account_id and output_id should be unique for active entries
-- We handle both output_id and pending_output_id cases
CREATE UNIQUE INDEX IF NOT EXISTS idx_pending_inputs_unique_output_active
ON pending_inputs(account_id, output_id)
WHERE deleted_at IS NULL AND output_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_pending_inputs_unique_pending_output_active
ON pending_inputs(account_id, pending_output_id)
WHERE deleted_at IS NULL AND pending_output_id IS NOT NULL;
