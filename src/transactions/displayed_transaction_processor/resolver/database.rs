use std::collections::HashMap;

use async_trait::async_trait;
use sqlx::SqlitePool;

use super::{OutputDetails, TransactionDataResolver};
use crate::models::{BalanceChange, Id, OutputStatus};
use crate::transactions::ProcessorError;
use crate::transactions::displayed_transaction_processor::parsing::ParsedWalletOutput;

/// Resolver that fetches transaction data from the database.
pub struct DatabaseResolver<'a> {
    pool: &'a SqlitePool,
}

impl<'a> DatabaseResolver<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TransactionDataResolver for DatabaseResolver<'_> {
    async fn get_output_details(&self, change: &BalanceChange) -> Result<Option<OutputDetails>, ProcessorError> {
        let Some(output_id) = change.caused_by_output_id else {
            return Ok(None);
        };

        let mut conn = self.pool.acquire().await?;

        let row = sqlx::query!(
            r#"
            SELECT output_hash, confirmed_height, status, wallet_output_json
            FROM outputs
            WHERE id = ? AND deleted_at IS NULL
            "#,
            output_id
        )
        .fetch_optional(&mut *conn)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let status = row.status.parse::<OutputStatus>().unwrap_or_else(|_| {
            eprintln!(
                "Warning: Failed to parse output status '{}' for output_id={}, defaulting to Unspent",
                row.status, output_id
            );
            OutputStatus::Unspent
        });

        let (output_type, coinbase_extra, is_coinbase, sent_output_hashes) =
            if let Some(ref json_str) = row.wallet_output_json {
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
            hash_hex: hex::encode(&row.output_hash),
            confirmed_height: row.confirmed_height.map(|h| h as u64),
            status,
            output_type,
            coinbase_extra,
            is_coinbase,
            sent_output_hashes,
        }))
    }

    async fn get_input_output_hash(&self, change: &BalanceChange) -> Result<Option<String>, ProcessorError> {
        let Some(input_id) = change.caused_by_input_id else {
            return Ok(None);
        };

        let mut conn = self.pool.acquire().await?;

        let input_row = sqlx::query!(
            r#"
            SELECT output_id
            FROM inputs
            WHERE id = ? AND deleted_at IS NULL
            "#,
            input_id
        )
        .fetch_optional(&mut *conn)
        .await?;

        let Some(input_row) = input_row else {
            return Ok(None);
        };

        let output_row = sqlx::query!(
            r#"
            SELECT output_hash
            FROM outputs
            WHERE id = ?
            "#,
            input_row.output_id
        )
        .fetch_optional(&mut *conn)
        .await?;

        Ok(output_row.map(|r| hex::encode(&r.output_hash)))
    }

    async fn get_sent_output_hashes(&self, change: &BalanceChange) -> Result<Vec<String>, ProcessorError> {
        if let Some(details) = self.get_output_details(change).await? {
            Ok(details.sent_output_hashes)
        } else {
            Ok(Vec::new())
        }
    }

    async fn build_output_hash_map(&self) -> Result<HashMap<String, Id>, ProcessorError> {
        let mut conn = self.pool.acquire().await?;

        let rows = sqlx::query!(
            r#"
            SELECT id, output_hash
            FROM outputs
            WHERE deleted_at IS NULL
            "#
        )
        .fetch_all(&mut *conn)
        .await?;

        let map: HashMap<String, Id> = rows
            .into_iter()
            .map(|row| (hex::encode(&row.output_hash), row.id))
            .collect();

        Ok(map)
    }
}
