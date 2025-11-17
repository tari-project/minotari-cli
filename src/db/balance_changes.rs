use std::sync::LazyLock;

use chrono::NaiveDateTime;
use sqlx::SqliteConnection;
use tokio::sync::{
    Mutex,
    mpsc::{Receiver, Sender},
};

use crate::models::BalanceChange;

pub static BALANCE_CHANGE_CHANNEL: LazyLock<(Sender<BalanceChange>, Mutex<Receiver<BalanceChange>>)> =
    LazyLock::new(|| {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        (tx, Mutex::new(rx))
    });

pub async fn insert_balance_change(conn: &mut SqliteConnection, change: &BalanceChange) -> Result<(), sqlx::Error> {
    let balance_credit = change.balance_credit as i64;
    let balance_debit = change.balance_debit as i64;
    let effective_height = change.effective_height as i64;
    let claimed_fee = change.claimed_fee.map(|v| v as i64);
    let claimed_amount = change.claimed_amount.map(|v| v as i64);
    let effective_date = change.effective_date.format("%Y-%m-%d %H:%M:%S").to_string();
    let description = &change.description;
    sqlx::query!(
        r#"
       INSERT INTO balance_changes (
         account_id, 
         caused_by_output_id, 
         caused_by_input_id, 
         description, 
         balance_credit, 
         balance_debit, 
         effective_date, 
         effective_height, 
         claimed_recipient_address,
         claimed_sender_address,
         memo_parsed,
         memo_hex,
         claimed_fee,
         claimed_amount)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,?,?)
        "#,
        change.account_id,
        change.caused_by_output_id,
        change.caused_by_input_id,
        description,
        balance_credit,
        balance_debit,
        effective_date,
        effective_height,
        change.claimed_recipient_address,
        change.claimed_sender_address,
        change.memo_parsed,
        change.memo_hex,
        claimed_fee,
        claimed_amount
    )
    .execute(&mut *conn)
    .await?;

    // Notify listeners about the new balance change
    // Do not fail because of notification failure
    let _unused = BALANCE_CHANGE_CHANNEL
        .0
        .send(change.clone())
        .await
        .map_err(|e| sqlx::Error::Protocol(e.to_string()));

    Ok(())
}

pub async fn get_all_balance_changes_by_account_id(
    conn: &mut SqliteConnection,
    account_id: i64,
) -> Result<Vec<BalanceChange>, sqlx::Error> {
    let rows = sqlx::query_as!(
        BalanceChange,
        r#"
        SELECT 
            account_id as "account_id: i64",
            caused_by_output_id as "caused_by_output_id: _",
            caused_by_input_id as "caused_by_input_id: _",
            description,
            balance_credit as "balance_credit: u64",
            balance_debit as "balance_debit: u64",
            effective_date as "effective_date: NaiveDateTime",
            effective_height as "effective_height: u64",
            claimed_recipient_address as "claimed_recipient_address: _",
            claimed_sender_address as "claimed_sender_address: _",
            memo_parsed,
            memo_hex,
            claimed_fee as "claimed_fee: _",
            claimed_amount as "claimed_amount: _"
        FROM balance_changes
        WHERE account_id = ?
        ORDER BY effective_height ASC, id ASC
        "#,
        account_id
    )
    .fetch_all(&mut *conn)
    .await?;

    Ok(rows)
}
