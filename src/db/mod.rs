use anyhow::{Context, Result};
use log::{debug, info, warn};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub mod accounts;
pub mod inputs;
pub mod outputs;
pub mod payref_history;

#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let conn = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .context("Failed to open database connection")?;

        let db = Self {
            connection: Arc::new(Mutex::new(conn)),
        };

        db.initialize().context("Failed to initialize database")?;
        Ok(db)
    }

    fn initialize(&self) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        
        // Check if we need to adopt from sqlx
        self.check_and_adopt_sqlx(&conn)?;
        
        // Apply migrations
        self.apply_migrations(&conn)?;
        
        Ok(())
    }

    fn check_and_adopt_sqlx(&self, conn: &Connection) -> Result<()> {
        // Check if _sqlx_migrations table exists
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'"
        )?;
        
        let has_sqlx = stmt.exists([])?;
        
        if has_sqlx {
            info!("Detected existing sqlx database, adopting...");
            
            // Set user_version to current migration version
            let current_version = MIGRATIONS.len() as i32;
            conn.pragma_update(None, "user_version", current_version)?;
            
            // Drop the sqlx migrations table
            conn.execute("DROP TABLE _sqlx_migrations", [])?;
            
            info!("Successfully adopted sqlx database");
        }
        
        Ok(())
    }

    fn apply_migrations(&self, conn: &Connection) -> Result<()> {
        let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| {
            Ok(row.get(0)?)
        })?;

        debug!("Current database version: {}", current_version);
        debug!("Target database version: {}", MIGRATIONS.len());

        if current_version < MIGRATIONS.len() as i32 {
            info!(
                "Applying {} database migrations (from {} to {})",
                MIGRATIONS.len() - current_version as usize,
                current_version,
                MIGRATIONS.len()
            );

            for (i, migration) in MIGRATIONS.iter().enumerate().skip(current_version as usize) {
                let version = i + 1;
                debug!("Applying migration {}: {}", version, migration.description);
                
                conn.execute_batch(migration.sql)
                    .with_context(|| format!("Failed to apply migration {}", version))?;
                
                conn.pragma_update(None, "user_version", version as i32)?;
                
                debug!("Successfully applied migration {}", version);
            }
            
            info!("All migrations applied successfully");
        } else if current_version > MIGRATIONS.len() as i32 {
            warn!(
                "Database version ({}) is newer than expected ({}). This may cause compatibility issues.",
                current_version,
                MIGRATIONS.len()
            );
        } else {
            debug!("Database is up to date");
        }

        Ok(())
    }

    pub fn with_connection<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self.connection.lock().unwrap();
        f(&*conn)
    }
}

struct Migration {
    description: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        description: "Create accounts table",
        sql: r#"
            CREATE TABLE accounts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT UNIQUE NOT NULL,
                view_private_key_encrypted BLOB NOT NULL,
                spend_public_key BLOB NOT NULL,
                birthday INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
        "#,
    },
    Migration {
        description: "Create outputs table",
        sql: r#"
            CREATE TABLE outputs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                commitment BLOB NOT NULL,
                value INTEGER NOT NULL,
                script BLOB NOT NULL,
                sender_offset_public_key BLOB NOT NULL,
                metadata_signature_ephemeral_commitment BLOB NOT NULL,
                metadata_signature_ephemeral_pubkey BLOB NOT NULL,
                metadata_signature_u_a BLOB NOT NULL,
                metadata_signature_u_x BLOB NOT NULL,
                metadata_signature_u_y BLOB NOT NULL,
                encrypted_data BLOB NOT NULL,
                minimum_value_promise INTEGER NOT NULL,
                mined_height INTEGER,
                mined_timestamp INTEGER,
                mined_in_block BLOB,
                spent_height INTEGER,
                spent_timestamp INTEGER,
                spent_in_block BLOB,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (account_id) REFERENCES accounts (id),
                UNIQUE(commitment)
            );
        "#,
    },
    Migration {
        description: "Create inputs table",
        sql: r#"
            CREATE TABLE inputs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                commitment BLOB NOT NULL,
                mined_height INTEGER NOT NULL,
                mined_timestamp INTEGER,
                mined_in_block BLOB,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (account_id) REFERENCES accounts (id),
                UNIQUE(commitment)
            );
        "#,
    },
    Migration {
        description: "Create balance_changes table",
        sql: r#"
            CREATE TABLE balance_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                commitment BLOB NOT NULL,
                change_type TEXT NOT NULL CHECK (change_type IN ('credit', 'debit')),
                amount INTEGER NOT NULL,
                mined_height INTEGER NOT NULL,
                mined_timestamp INTEGER,
                mined_in_block BLOB,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (account_id) REFERENCES accounts (id)
            );
        "#,
    },
    Migration {
        description: "Create wallet_events table",
        sql: r#"
            CREATE TABLE wallet_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                commitment BLOB,
                amount INTEGER,
                block_height INTEGER,
                block_hash BLOB,
                timestamp INTEGER,
                details TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (account_id) REFERENCES accounts (id)
            );
        "#,
    },
    Migration {
        description: "Create scanned_blocks table",
        sql: r#"
            CREATE TABLE scanned_blocks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                height INTEGER NOT NULL,
                block_hash BLOB NOT NULL,
                scanned_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (account_id) REFERENCES accounts (id),
                UNIQUE(account_id, height)
            );
        "#,
    },
    Migration {
        description: "Create payref_history table",
        sql: r#"
            CREATE TABLE payref_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                tx_id BLOB NOT NULL,
                output_hash BLOB NOT NULL,
                old_payref TEXT NOT NULL,
                new_payref TEXT,
                reorg_height INTEGER NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (account_id) REFERENCES accounts (id),
                INDEX(old_payref),
                INDEX(new_payref),
                INDEX(tx_id)
            );
        "#,
    },
];
