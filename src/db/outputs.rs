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
    memo_parsed: Option<String>,
    memo_hex: Option<String>,
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
       INSERT INTO outputs (account_id, output_hash, mined_in_block_height, mined_in_block_hash, value, mined_timestamp, wallet_output_json, memo_parsed, memo_hex)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id
        "#,
        account_id,
        output_hash,
        block_height,
        block_hash,
        value,
        mined_timestamp,
        output_json,
        memo_parsed,
        memo_hex
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

pub async fn get_unconfirmed_outputs(
    pool: &SqlitePool,
    account_id: i64,
    current_height: u64,
    confirmation_blocks: u64,
) -> Result<Vec<(Vec<u8>, u64, Option<String>, Option<String>)>, sqlx::Error> {
    let min_height_to_confirm = if current_height >= confirmation_blocks {
        current_height - confirmation_blocks
    } else {
        0
    };
    let min_height = min_height_to_confirm as i64;

    let rows = sqlx::query!(
        r#"
        SELECT output_hash, mined_in_block_height, memo_parsed, memo_hex
        FROM outputs o
        WHERE o.account_id = ?
          AND o.mined_in_block_height <= ?
          AND o.confirmed_height IS NULL
        "#,
        account_id,
        min_height
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.output_hash,
                row.mined_in_block_height as u64,
                row.memo_parsed,
                row.memo_hex,
            )
        })
        .collect())
}

pub async fn mark_output_confirmed(
    pool: &SqlitePool,
    output_hash: &[u8],
    confirmed_height: u64,
    confirmed_hash: &[u8],
) -> Result<(), sqlx::Error> {
    let confirmed_height = confirmed_height as i64;
    sqlx::query!(
        r#"
        UPDATE outputs
        SET confirmed_height = ?, confirmed_hash = ?
        WHERE output_hash = ?
        "#,
        confirmed_height,
        confirmed_hash,
        output_hash
    )
    .execute(pool)
    .await?;

    Ok(())
}
