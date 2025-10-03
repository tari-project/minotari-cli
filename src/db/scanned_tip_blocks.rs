use sqlx::SqlitePool;

use crate::models::ScannedTipBlock;

struct ScannedTipBlockRow {
    pub id: i64,
    pub account_id: i64,
    pub height: i64,
    pub hash: Vec<u8>,
}

pub async fn get_scanned_tip_blocks_by_account(
    pool: &SqlitePool,
    account_id: i64,
) -> Result<Vec<ScannedTipBlock>, sqlx::Error> {
    let row = sqlx::query_as!(
        ScannedTipBlockRow,
        r#"
        SELECT id, account_id, height, hash
        FROM scanned_tip_blocks
        WHERE account_id = ?
        ORDER BY height DESC
        "#,
        account_id
    )
    .fetch_all(pool)
    .await?;

    Ok(row
        .into_iter()
        .map(|r| ScannedTipBlock {
            id: r.id,
            account_id: r.account_id,
            height: r.height as u64,
            hash: r.hash,
        })
        .collect())
}

pub async fn insert_scanned_tip_block(
    pool: &SqlitePool,
    account_id: i64,
    height: i64,
    hash: &[u8],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT OR IGNORE INTO scanned_tip_blocks (account_id, height, hash)
        VALUES (?, ?, ?)
        "#,
        account_id,
        height,
        hash
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn delete_old_scanned_tip_blocks(
    pool: &SqlitePool,
    account_id: i64,
    keep_last_n: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        DELETE FROM scanned_tip_blocks
        WHERE account_id = ?
        AND id NOT IN (
            SELECT id FROM scanned_tip_blocks
            WHERE account_id = ?
            ORDER BY height DESC
            LIMIT ?
        )
        "#,
        account_id,
        account_id,
        keep_last_n
    )
    .execute(pool)
    .await?;

    Ok(())
}
