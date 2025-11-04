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
        SELECT id FROM inputs WHERE output_id = ?
        "#,
        output_id
    )
    .fetch_one(&mut *conn)
    .await?
    .id;

    Ok((input_id, rows_affected > 0))
}

pub async fn delete_inputs_from_height(
    conn: &mut SqliteConnection,
    account_id: i64,
    height: u64,
) -> Result<(), sqlx::Error> {
    let height = height as i64;
    sqlx::query!(
        r#"
        DELETE FROM inputs
        WHERE account_id = ? AND mined_in_block_height >= ?
        "#,
        account_id,
        height
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}
