CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);
CREATE TABLE accounts (
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            friendly_name TEXT NOT NULL UNIQUE,
            unencrypted_view_key_hash blob NOT NULL UNIQUE,
            encrypted_view_private_key blob NOT NULL,
            encrypted_spend_public_key blob NOT NULL,
            cipher_nonce blob NOT NULL,
            birthday INTEGER NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
CREATE TABLE sqlite_sequence(name,seq);
CREATE TABLE scanned_tip_blocks (
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            hash Blob NOT NULL,
            height INTEGER NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (account_id) REFERENCES accounts(id)
        );
CREATE TABLE outputs (
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            output_hash blob NOT NULL,
            mined_in_block_hash blob NOT NULL,
            mined_in_block_height INTEGER NOT NULL,
            value INTEGER NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, wallet_output_json TEXT, mined_timestamp TIMESTAMP NOT NULL, confirmed_height INTEGER, confirmed_hash BLOB, memo_parsed TEXT, memo_hex TEXT, status TEXT NOT NULL DEFAULT 'UNSPENT', locked_at TIMESTAMP, locked_by_request_id TEXT, deleted_at TIMESTAMP, deleted_in_block_height INTEGER,
            FOREIGN KEY (account_id) REFERENCES accounts(id)
        );
CREATE TABLE events (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    description TEXT NOT NULL,
    data_json TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);
CREATE TABLE balance_changes (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL,
    caused_by_output_id INTEGER,
    caused_by_input_id INTEGER,
    description TEXT NOT NULL,
    balance_debit INTEGER NOT NULL,
    balance_credit INTEGER NOT NULL,
    effective_date TIMESTAMP NOT NULL,
    effective_height INTEGER NOT NULL,
    claimed_recipient_address TEXT,
    claimed_sender_address TEXT,
    memo_parsed TEXT,
    memo_hex TEXT,
    claimed_fee INTEGER,
    claimed_amount INTEGER,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, caused_by_pending_output_id INTEGER, caused_by_pending_input_id INTEGER,
    FOREIGN KEY (account_id) REFERENCES accounts(id),
    FOREIGN KEY (caused_by_output_id) REFERENCES outputs(id),
    FOREIGN KEY (caused_by_input_id) REFERENCES inputs(id), FOREIGN KEY (caused_by_pending_output_id) REFERENCES pending_outputs(id), FOREIGN KEY (caused_by_pending_input_id) REFERENCES pending_inputs(id)
);
CREATE TABLE inputs (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL,
    output_id INTEGER NOT NULL,
    mined_in_block_hash blob NOT NULL,
    mined_in_block_height INTEGER NOT NULL,
    mined_timestamp TIMESTAMP NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, deleted_at TIMESTAMP, deleted_in_block_height INTEGER,
    FOREIGN KEY (output_id) REFERENCES outputs(id),
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);
CREATE INDEX idx_scanned_tip_blocks_account_height ON scanned_tip_blocks(account_id, height DESC);
CREATE UNIQUE INDEX idx_scanned_tip_blocks_account_height_hash ON scanned_tip_blocks(account_id, height, hash);
CREATE TABLE pending_transactions (
    id TEXT PRIMARY KEY NOT NULL,
    account_id INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL,
    unsigned_tx_json TEXT NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (account_id) REFERENCES accounts(id),
    UNIQUE (account_id, idempotency_key)
);
CREATE TABLE pending_outputs (
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
CREATE TABLE pending_inputs (
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
CREATE INDEX idx_pending_transactions_status_expires_at ON pending_transactions(status, expires_at);
CREATE INDEX idx_pending_outputs_account_id ON pending_outputs(account_id);
CREATE INDEX idx_pending_outputs_output_hash ON pending_outputs(output_hash);
CREATE INDEX idx_pending_outputs_status ON pending_outputs(status);
CREATE INDEX idx_pending_inputs_account_id ON pending_inputs(account_id);
CREATE INDEX idx_pending_inputs_output_id ON pending_inputs(output_id);
CREATE INDEX idx_pending_inputs_pending_output_id ON pending_inputs(pending_output_id);
CREATE INDEX idx_pending_inputs_status ON pending_inputs(status);
CREATE INDEX idx_balance_changes_account_height ON balance_changes(account_id, effective_height);
CREATE UNIQUE INDEX idx_outputs_output_hash_active ON outputs(output_hash) WHERE deleted_at IS NULL;
CREATE INDEX idx_outputs_account_mined_height_active ON outputs(account_id, mined_in_block_height) WHERE deleted_at IS NULL;
CREATE INDEX idx_outputs_status_active ON outputs(status) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX idx_inputs_output_id_unique_active ON inputs(output_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_inputs_account_mined_height_active ON inputs(account_id, mined_in_block_height) WHERE deleted_at IS NULL;
