-- Create pending_inputs table
-- This table tracks inputs (spent outputs) that are expected but not yet mined/confirmed
CREATE TABLE IF NOT EXISTS pending_inputs (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL,
    output_id INTEGER,
    pending_output_id INTEGER,
    status TEXT NOT NULL DEFAULT 'PENDING',
    locked_by_request_id TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP,
    FOREIGN KEY (account_id) REFERENCES accounts(id),
    FOREIGN KEY (output_id) REFERENCES outputs(id),
    FOREIGN KEY (pending_output_id) REFERENCES pending_outputs(id)
);

-- Create indexes for pending_inputs
CREATE INDEX idx_pending_inputs_account_id ON pending_inputs(account_id);
CREATE INDEX idx_pending_inputs_output_id ON pending_inputs(output_id);
CREATE INDEX idx_pending_inputs_pending_output_id ON pending_inputs(pending_output_id);
CREATE INDEX idx_pending_inputs_status ON pending_inputs(status);
