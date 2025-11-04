use sqlx::SqliteConnection;

use crate::models::ScannedTipBlock;

const RECENT_BLOCKS_TO_KEEP: u64 = 1000;
const OLD_BLOCKS_PRUNING_INTERVAL: u64 = 500;

struct ScannedTipBlockRow {
    pub id: i64,
    pub account_id: i64,
    pub height: i64,
    pub hash: Vec<u8>,
}

pub async fn get_scanned_tip_blocks_by_account(
    conn: &mut SqliteConnection,
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
    .fetch_all(&mut *conn)
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

pub async fn delete_scanned_tip_blocks_from_height(
    conn: &mut SqliteConnection,
    account_id: i64,
    height: u64,
) -> Result<(), sqlx::Error> {
    let height = height as i64;
    sqlx::query!(
        r#"
        DELETE FROM scanned_tip_blocks
        WHERE account_id = ? AND height >= ?
        "#,
        account_id,
        height
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn prune_scanned_tip_blocks(
    conn: &mut SqliteConnection,
    account_id: i64,
    current_tip_height: u64,
) -> Result<(), anyhow::Error> {
    // Keep the last RECENT_BLOCKS_TO_KEEP blocks
    let min_height_for_recent = current_tip_height.saturating_sub(RECENT_BLOCKS_TO_KEEP) as i64;

    // Delete blocks older than min_height_for_recent that are not at the pruning interval
    sqlx::query!(
        r#"
        DELETE FROM scanned_tip_blocks
        WHERE account_id = ?
          AND height < ?
          AND (height % ? != 0)
        "#,
        account_id,
        min_height_for_recent,
        OLD_BLOCKS_PRUNING_INTERVAL as i64
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}
