use chrono::NaiveDateTime;
use sqlx::SqliteConnection;

use crate::models::BalanceChange;

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

#[derive(Debug, Default)]
pub struct BalanceAggregates {
    pub total_credits: Option<i64>,
    pub total_debits: Option<i64>,
    pub max_height: Option<i64>,
    pub max_date: Option<String>,
}

pub async fn get_balance_aggregates_for_account(
    conn: &mut SqliteConnection,
    account_id: i64,
) -> Result<BalanceAggregates, sqlx::Error> {
    let agg_result = sqlx::query_as!(
        BalanceAggregates,
        r#"
            SELECT
              SUM(balance_credit) as "total_credits: _",
              SUM(balance_debit) as "total_debits: _",
              MAX(effective_height) as "max_height: _",
              strftime('%Y-%m-%d %H:%M:%S', MAX(effective_date)) as "max_date: _"
            FROM balance_changes
            WHERE account_id = ?
            "#,
        account_id
    )
    .fetch_one(&mut *conn)
    .await?;

    Ok(agg_result)
}
