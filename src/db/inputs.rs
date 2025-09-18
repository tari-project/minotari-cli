use sqlx::SqlitePool;

pub async fn insert_input(
    pool: &SqlitePool,
    account_id: i64,
    output_id: i64,
    mined_in_block_height: u64,
    mined_in_block_hash: &[u8],
    mined_timestamp: u64,
) -> Result<i64, sqlx::Error> {
    let timestamp = chrono::NaiveDateTime::from_timestamp_opt(mined_timestamp as i64, 0)
        .ok_or_else(|| {
            sqlx::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Invalid mined timestamp: {}", mined_timestamp),
            ))
        })?
        .to_string();
    let mined_in_block_height = mined_in_block_height as i64;
    let id = sqlx::query!(
        r#"
       INSERT INTO inputs (account_id, output_id, mined_in_block_height, mined_in_block_hash, mined_timestamp)
       VALUES (?, ?, ?, ?, ?) 
            RETURNING id
        "#,
        account_id,
        output_id,
        mined_in_block_height,
        mined_in_block_hash,
        timestamp
    )
    .fetch_one(pool)
    .await?.id;

    Ok(id)
}
