use crate::db::balance_changes::insert_balance_change;
use crate::db::error::{WalletDbError, WalletDbResult};
use crate::models::BalanceChange;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, named_params};
use serde::Deserialize;
use serde_rusqlite::from_rows;

pub fn insert_input(
    conn: &Connection,
    account_id: i64,
    output_id: i64,
    mined_in_block_height: u64,
    mined_in_block_hash: &[u8],
    mined_timestamp: u64,
) -> WalletDbResult<(i64, bool)> {
    let timestamp = DateTime::<Utc>::from_timestamp(mined_timestamp as i64, 0)
        .ok_or_else(|| WalletDbError::Decoding(format!("Invalid mined timestamp: {}", mined_timestamp)))?;

    let mined_in_block_height = mined_in_block_height as i64;

    let rows_affected = conn.execute(
        r#"
       INSERT OR IGNORE INTO inputs (
            account_id,
            output_id,
            mined_in_block_height,
            mined_in_block_hash,
            mined_timestamp
       )
       VALUES (
            :account_id,
            :output_id,
            :block_height,
            :block_hash,
            :timestamp
       ) 
        "#,
        named_params! {
            ":account_id": account_id,
            ":output_id": output_id,
            ":block_height": mined_in_block_height,
            ":block_hash": mined_in_block_hash,
            ":timestamp": timestamp
        },
    )?;

    let input_id: i64 = conn.query_row(
        "SELECT id FROM inputs WHERE output_id = :output_id AND deleted_at IS NULL",
        named_params! { ":output_id": output_id },
        |row| row.get(0),
    )?;

    Ok((input_id, rows_affected > 0))
}

// retrieve output_id
pub fn get_input_details_for_balance_change_by_id(conn: &Connection, input_id: i64) -> WalletDbResult<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        r#"
        SELECT output_id
        FROM inputs
        WHERE id = :id AND deleted_at IS NULL
        "#,
    )?;

    let output_id: Option<i64> = stmt
        .query_row(named_params! { ":id": input_id }, |row| row.get("output_id"))
        .optional()?;

    Ok(output_id)
}

#[derive(Deserialize)]
struct InputToDelete {
    input_id: i64,
    output_id: i64,
    output_value: i64,
}

pub fn soft_delete_inputs_from_height(conn: &Connection, account_id: i64, height: u64) -> WalletDbResult<()> {
    let height_i64 = height as i64;
    let now = Utc::now();

    let inputs_with_output_values = {
        let mut stmt = conn.prepare_cached(
            r#"
            SELECT i.id as input_id, o.id as output_id, o.value as output_value
            FROM inputs i
            JOIN outputs o ON i.output_id = o.id
            WHERE i.account_id = :account_id
              AND i.mined_in_block_height >= :height
              AND i.deleted_at IS NULL
            "#,
        )?;

        let rows = stmt.query(named_params! {
            ":account_id": account_id,
            ":height": height_i64
        })?;

        from_rows::<InputToDelete>(rows).collect::<Result<Vec<_>, _>>()?
    };

    for row in inputs_with_output_values {
        let balance_change = BalanceChange {
            account_id,
            caused_by_output_id: Some(row.output_id),
            caused_by_input_id: Some(row.input_id),
            description: format!("Reversal: Input spent as input (reorg at height {})", height),
            balance_credit: row.output_value as u64, // Reversing a debit, so credit the value back
            balance_debit: 0,
            effective_date: now.naive_utc(),
            effective_height: height,
            claimed_recipient_address: None,
            claimed_sender_address: None,
            memo_parsed: None,
            memo_hex: None,
            claimed_fee: None,
            claimed_amount: None,
        };
        insert_balance_change(conn, &balance_change)?;
    }

    conn.execute(
        r#"
        UPDATE inputs
        SET deleted_at = :now, deleted_in_block_height = :height
        WHERE account_id = :account_id AND mined_in_block_height >= :height AND deleted_at IS NULL
        "#,
        named_params! {
            ":now": now,
            ":height": height_i64,
            ":account_id": account_id
        },
    )?;

    Ok(())
}
