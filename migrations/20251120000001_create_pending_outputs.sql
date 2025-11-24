-- Create pending_outputs table
-- This table tracks outputs that are expected but not yet mined/confirmed
CREATE TABLE IF NOT EXISTS pending_outputs (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL,
    output_hash BLOB NOT NULL,
    value INTEGER NOT NULL,
    wallet_output_json TEXT,
    memo_parsed TEXT,
    memo_hex TEXT,
    status TEXT NOT NULL DEFAULT 'PENDING',
    locked_by_request_id TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP,
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);

-- Create indexes for pending_outputs
CREATE INDEX idx_pending_outputs_account_id ON pending_outputs(account_id);
CREATE INDEX idx_pending_outputs_output_hash ON pending_outputs(output_hash);
CREATE INDEX idx_pending_outputs_status ON pending_outputs(status);
