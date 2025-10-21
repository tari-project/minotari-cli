use sqlx::SqliteConnection;

use crate::models::ScannedTipBlock;

struct ScannedTipBlockRow {
    pub id: i64,
    pub account_id: i64,
    pub height: i64,
    pub hash: Vec<u8>,
}

pub async fn get_scanned_tip_blocks_by_account(
    conn: &mut SqliteConnection,
    account_id: Option<i64>,
    _child_account_id: Option<i64>, // Deprecated parameter, kept for API compatibility
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
    .fetch_all(&mut *conn)
    .await?;

    Ok(row
        .into_iter()
        .map(|r| ScannedTipBlock {
            id: r.id,
            account_id: Some(r.account_id),
            child_account_id: None, // Deprecated field, always None now
            height: r.height as u64,
            hash: r.hash,
        })
        .collect())
}

pub async fn insert_scanned_tip_block(
    conn: &mut SqliteConnection,
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
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn delete_old_scanned_tip_blocks(
    conn: &mut SqliteConnection,
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
    .execute(&mut *conn)
    .await?;

    Ok(())
}
