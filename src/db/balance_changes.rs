use sqlx::SqlitePool;

use crate::models::BalanceChange;

pub async fn insert_balance_change(
    pool: &SqlitePool,
    change: &BalanceChange,
) -> Result<(), sqlx::Error> {
    let balance_credit = change.balance_credit as i64;
    let balance_debit = change.balance_debit as i64;
    let effective_date = change
        .effective_date
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let description = &change.description;
    sqlx::query!(
        r#"
       INSERT INTO balance_changes (account_id, caused_by_output_id, caused_by_input_id, description, balance_credit, balance_debit, effective_date)
         VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
        change.account_id,
        change.caused_by_output_id,
        change.caused_by_input_id,
        description,
        balance_credit ,
        balance_debit ,
        effective_date
    )
    .execute(pool)
    .await?;

    Ok(())
}
