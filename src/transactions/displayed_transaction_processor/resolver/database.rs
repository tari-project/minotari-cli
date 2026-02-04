use std::collections::HashMap;

use super::{OutputDetails, TransactionDataResolver};
use crate::db::SqlitePool;
use crate::models::{BalanceChange, Id, OutputStatus};
use crate::transactions::ProcessorError;
use log::warn;
use rusqlite::{OptionalExtension, named_params};
use tari_common_types::types::FixedHash;
use tari_transaction_components::transaction_components::WalletOutput;

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
                SELECT output_hash, mined_in_block_height, status, mined_in_block_hash, wallet_output_json
                FROM outputs
                WHERE id = :id AND deleted_at IS NULL
                "#,
                named_params! { ":id": output_id },
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>("output_hash")?,
                        row.get::<_, i64>("mined_in_block_height")?,
                        row.get::<_, String>("status")?,
                        row.get::<_, Vec<u8>>("mined_in_block_hash")?,
                        row.get::<_, Option<String>>("wallet_output_json")?,
                    ))
                },
            )
            .optional()
            .map_err(|e| ProcessorError::DbError(e.into()))?;

        let Some((output_hash, mined_in_block_height, status_str, block_hash, wallet_output_json)) = row else {
            return Ok(None);
        };

        let status = status_str.parse::<OutputStatus>().unwrap_or_else(|_| {
            warn!(
                target: "audit",
                status = &*status_str,
                output_id = output_id;
                "Failed to parse output status, defaulting to Unspent"
            );
            OutputStatus::Unspent
        });
        let json_string =
            wallet_output_json.ok_or_else(|| ProcessorError::MissingError("No wallet output".to_string()))?;
        let output = serde_json::from_str::<WalletOutput>(&json_string)
            .map_err(|e| ProcessorError::ParseError(e.to_string()))?;
        //
        let output_type = output.features().output_type;
        let coinbase_extra = output.features().coinbase_extra.clone();
        let sent_output_hashes = output.payment_id().get_sent_hashes().unwrap_or_default();

        let hash = FixedHash::try_from(output_hash).map_err(|e| ProcessorError::ParseError(e.to_string()))?;
        Ok(Some(OutputDetails {
            hash,
            mined_in_block_height: mined_in_block_height as u64,
            mined_hash: FixedHash::try_from(block_hash).map_err(|e| ProcessorError::ParseError(e.to_string()))?,
            status,
            output_type,
            coinbase_extra,
            sent_output_hashes,
        }))
    }

    fn get_input_output_hash(&self, change: &BalanceChange) -> Result<Option<(FixedHash, FixedHash)>, ProcessorError> {
        let Some(input_id) = change.caused_by_input_id else {
            return Ok(None);
        };

        let conn = self.pool.get().map_err(|e| ProcessorError::DbError(e.into()))?;
        let mut stmt = conn
            .prepare_cached(
                r#"
                SELECT o.output_hash, i.mined_in_block_hash
                FROM inputs i
                JOIN outputs o ON i.output_id = o.id
                WHERE i.id = :id AND i.deleted_at IS NULL
                "#,
            )
            .map_err(|e| ProcessorError::DbError(e.into()))?;

        let result = stmt
            .query_row(named_params! { ":id": input_id }, |row| {
                let output_hash: Vec<u8> = row.get("output_hash")?;
                let mined_hash: Vec<u8> = row.get("mined_in_block_hash")?;

                Ok((output_hash, mined_hash))
            })
            .optional()
            .map_err(|e| ProcessorError::DbError(e.into()))?;

        result
            .map(|(output_hash, mined_hash)| {
                let input = FixedHash::try_from(output_hash).map_err(|e| ProcessorError::ParseError(e.to_string()))?;
                let mined = FixedHash::try_from(mined_hash).map_err(|e| ProcessorError::ParseError(e.to_string()))?;
                Ok((input, mined))
            })
            .transpose()
    }

    fn get_sent_output_hashes(&self, change: &BalanceChange) -> Result<Vec<FixedHash>, ProcessorError> {
        if let Some(details) = self.get_output_details(change)? {
            Ok(details.sent_output_hashes)
        } else {
            Ok(Vec::new())
        }
    }

    fn build_output_hash_map(&self) -> Result<HashMap<FixedHash, Id>, ProcessorError> {
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
                let raw_hash: Vec<u8> = row.get("output_hash")?;
                let hash = FixedHash::try_from(raw_hash).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(e))
                })?;
                Ok((hash, id))
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
