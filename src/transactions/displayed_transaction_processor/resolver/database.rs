use std::collections::HashMap;

use rusqlite::{OptionalExtension, named_params};

use super::{OutputDetails, TransactionDataResolver};
use crate::db::SqlitePool;
use crate::models::{BalanceChange, Id, OutputStatus};
use crate::transactions::ProcessorError;
use crate::transactions::displayed_transaction_processor::parsing::ParsedWalletOutput;

/// Resolver that fetches transaction data from the database.
pub struct DatabaseResolver {
    pool: SqlitePool,
}

impl DatabaseResolver {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl TransactionDataResolver for DatabaseResolver {
    fn get_output_details(&self, change: &BalanceChange) -> Result<Option<OutputDetails>, ProcessorError> {
        let Some(output_id) = change.caused_by_output_id else {
            return Ok(None);
        };

        let conn = self.pool.get().map_err(|e| ProcessorError::DbError(e.into()))?;

        let row = conn
            .query_row(
                r#"
                SELECT output_hash, confirmed_height, status, wallet_output_json
                FROM outputs
                WHERE id = :id AND deleted_at IS NULL
                "#,
                named_params! { ":id": output_id },
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>("output_hash")?,
                        row.get::<_, Option<i64>>("confirmed_height")?,
                        row.get::<_, String>("status")?,
                        row.get::<_, Option<String>>("wallet_output_json")?,
                    ))
                },
            )
            .optional()
            .map_err(|e| ProcessorError::DbError(e.into()))?;

        let Some((output_hash, confirmed_height, status_str, wallet_output_json)) = row else {
            return Ok(None);
        };

        let status = status_str.parse::<OutputStatus>().unwrap_or_else(|_| {
            eprintln!(
                "Warning: Failed to parse output status '{}' for output_id={}, defaulting to Unspent",
                status_str, output_id
            );
            OutputStatus::Unspent
        });

        let (output_type, coinbase_extra, is_coinbase, sent_output_hashes) =
            if let Some(ref json_str) = wallet_output_json {
                if let Some(parsed) = ParsedWalletOutput::from_json(json_str) {
                    (
                        parsed.output_type,
                        parsed.coinbase_extra,
                        parsed.is_coinbase,
                        parsed.sent_output_hashes,
                    )
                } else {
                    ("Unknown".to_string(), None, false, Vec::new())
                }
            } else {
                ("Unknown".to_string(), None, false, Vec::new())
            };

        Ok(Some(OutputDetails {
            hash_hex: hex::encode(output_hash),
            confirmed_height: confirmed_height.map(|h| h as u64),
            status,
            output_type,
            coinbase_extra,
            is_coinbase,
            sent_output_hashes,
        }))
    }

    fn get_input_output_hash(&self, change: &BalanceChange) -> Result<Option<String>, ProcessorError> {
        let Some(input_id) = change.caused_by_input_id else {
            return Ok(None);
        };

        let conn = self.pool.get().map_err(|e| ProcessorError::DbError(e.into()))?;

        let output_hash: Option<Vec<u8>> = conn
            .query_row(
                r#"
                SELECT o.output_hash
                FROM inputs i
                JOIN outputs o ON i.output_id = o.id
                WHERE i.id = :id AND i.deleted_at IS NULL
                "#,
                named_params! { ":id": input_id },
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| ProcessorError::DbError(e.into()))?;

        Ok(output_hash.map(hex::encode))
    }

    fn get_sent_output_hashes(&self, change: &BalanceChange) -> Result<Vec<String>, ProcessorError> {
        if let Some(details) = self.get_output_details(change)? {
            Ok(details.sent_output_hashes)
        } else {
            Ok(Vec::new())
        }
    }

    fn build_output_hash_map(&self) -> Result<HashMap<String, Id>, ProcessorError> {
        let conn = self.pool.get().map_err(|e| ProcessorError::DbError(e.into()))?;

        let mut stmt = conn
            .prepare_cached(
                r#"
                SELECT id, output_hash
                FROM outputs
                WHERE deleted_at IS NULL
                "#,
            )
            .map_err(|e| ProcessorError::DbError(e.into()))?;

        let rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get("id")?;
                let hash: Vec<u8> = row.get("output_hash")?;
                Ok((hex::encode(hash), id))
            })
            .map_err(|e| ProcessorError::DbError(e.into()))?;

        let mut map = HashMap::new();
        for row in rows {
            let (hash, id) = row.map_err(|e| ProcessorError::DbError(e.into()))?;
            map.insert(hash, id);
        }

        Ok(map)
    }
}
