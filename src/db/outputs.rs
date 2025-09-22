use lightweight_wallet_libs::transaction_components::WalletOutput;
use sqlx::SqlitePool;

pub async fn insert_output(
    pool: &SqlitePool,
    account_id: i64,
    output_hash: Vec<u8>,
    output: &WalletOutput,
    block_height: u64,
    block_hash: &[u8],
    mined_timestamp: u64,
) -> Result<i64, sqlx::Error> {
    let output_json = serde_json::to_string(&output).map_err(|e| {
        sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to serialize output to JSON: {}", e),
        ))
    })?;
    let mined_timestamp = chrono::NaiveDateTime::from_timestamp_opt(mined_timestamp as i64, 0)
        .ok_or_else(|| {
            sqlx::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Invalid mined timestamp: {}", mined_timestamp),
            ))
        })?
        .to_string();
    let block_height = block_height as i64;
    let value = output.value().as_u64() as i64;
    let output_id = sqlx::query!(
        r#"
       INSERT INTO outputs (account_id, output_hash, mined_in_block_height, mined_in_block_hash, value, mined_timestamp, wallet_output_json)
       VALUES (?, ?, ?, ?, ?, ?, ?) 
         RETURNING id
        "#,
        account_id,
        output_hash,
        block_height,
        block_hash,
        value,
        mined_timestamp,
        output_json
           )
    .fetch_one(pool)
    .await?.id;

    Ok(output_id)
}

pub async fn get_output_info_by_hash(
    pool: &SqlitePool,
    output_hash: &[u8],
) -> Result<Option<(i64, u64)>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id, value
        FROM outputs
        WHERE output_hash = ?
        "#,
        output_hash
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| (r.id, r.value as u64)))
}

pub struct OutputInfo {
    pub id: i64,
    pub value: u64,
}
