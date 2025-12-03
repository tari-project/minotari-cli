create table child_accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_account_id INTEGER NOT NULL,
    child_account_name TEXT NOT NULL,
    for_tapplet_name TEXT NOT NULL,
    version TEXT NOT NULL,
    tapplet_pub_key TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (parent_account_id) REFERENCES accounts(id)
);