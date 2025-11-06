use crate::db::balance_changes::insert_balance_change;
use crate::models::BalanceChange;
use chrono::{DateTime, Utc};
use sqlx::SqliteConnection;

pub async fn insert_input(
    conn: &mut SqliteConnection,
    account_id: i64,
    output_id: i64,
    mined_in_block_height: u64,
    mined_in_block_hash: &[u8],
    mined_timestamp: u64,
) -> Result<(i64, bool), sqlx::Error> {
    let timestamp = DateTime::<Utc>::from_timestamp(mined_timestamp as i64, 0)
        .ok_or_else(|| {
            sqlx::Error::Io(std::io::Error::other(format!(
                "Invalid mined timestamp: {}",
                mined_timestamp
            )))
        })?
        .to_string();
    let mined_in_block_height = mined_in_block_height as i64;
    let insert_result = sqlx::query!(
        r#"
       INSERT OR IGNORE INTO inputs (account_id, output_id, mined_in_block_height, mined_in_block_hash, mined_timestamp)
       VALUES (?, ?, ?, ?, ?) 
        "#,
        account_id,
        output_id,
        mined_in_block_height,
        mined_in_block_hash,
        timestamp
    )
    .execute(&mut *conn)
    .await?;

    let rows_affected = insert_result.rows_affected();

    // Now fetch the ID, which is guaranteed to exist
    let input_id = sqlx::query!(
        r#"
        SELECT id FROM inputs WHERE output_id = ? AND deleted_at IS NULL
        "#,
        output_id
    )
    .fetch_one(&mut *conn)
    .await?
    .id;

    Ok((input_id, rows_affected > 0))
}

pub async fn soft_delete_inputs_from_height(
    conn: &mut SqliteConnection,
    account_id: i64,
    height: u64,
) -> Result<(), sqlx::Error> {
    let height_i64 = height as i64;
    let now = Utc::now().naive_utc().to_string();

    let inputs_with_output_values = sqlx::query!(
        r#"
        SELECT i.id as input_id, o.id as output_id, o.value as output_value
        FROM inputs i
        JOIN outputs o ON i.output_id = o.id
        WHERE i.account_id = ?
          AND i.mined_in_block_height >= ?
          AND i.deleted_at IS NULL
        "#,
        account_id,
        height_i64
    )
    .fetch_all(&mut *conn)
    .await?;

    for row in inputs_with_output_values {
        let balance_change = BalanceChange {
            account_id,
            caused_by_output_id: Some(row.output_id),
            caused_by_input_id: Some(row.input_id),
            description: format!("Reversal: Input spent as input (reorg at height {})", height),
            balance_credit: row.output_value as u64, // Reversing a debit, so credit the value back
            balance_debit: 0,
            effective_date: Utc::now().naive_utc(),
            effective_height: height,
            claimed_recipient_address: None,
            claimed_sender_address: None,
            memo_parsed: None,
            memo_hex: None,
            claimed_fee: None,
            claimed_amount: None,
        };
        insert_balance_change(conn, &balance_change).await?;
    }

    sqlx::query!(
        r#"
        UPDATE inputs
        SET deleted_at = ?, deleted_in_block_height = ?
        WHERE account_id = ? AND mined_in_block_height >= ? AND deleted_at IS NULL
        "#,
        now,
        height_i64,
        account_id,
        height_i64
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}
