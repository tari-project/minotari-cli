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
