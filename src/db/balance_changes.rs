use log::debug;
use rusqlite::{Connection, named_params};
use serde::Deserialize;
use serde_rusqlite::from_rows;
use crate::db::error::{WalletDbError, WalletDbResult};
use crate::log::mask_amount;
use crate::models::BalanceChange;

pub fn insert_balance_change(conn: &Connection, change: &BalanceChange) -> WalletDbResult<()> {
    debug!(
        target: "audit",
        account_id = change.account_id,
        credit = &*mask_amount(change.balance_credit),
        debit = &*mask_amount(change.balance_debit);
        "DB: Inserting balance change"
    );

    let balance_credit = change.balance_credit.as_u64() as i64;
    let balance_debit = change.balance_debit.as_u64() as i64;
    let effective_height = change.effective_height as i64;
    let claimed_fee = change.claimed_fee.map(|v| v.as_u64() as i64);
    let claimed_amount = change.claimed_amount.map(|v| v.as_u64() as i64);

    conn.execute(
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
         VALUES (
            :account_id,
            :caused_by_output_id,
            :caused_by_input_id,
            :description,
            :balance_credit,
            :balance_debit,
            :effective_date,
            :effective_height,
            :claimed_recipient_address,
            :claimed_sender_address,
            :memo_parsed,
            :memo_hex,
            :claimed_fee,
            :claimed_amount
         )
        "#,
        named_params! {
            ":account_id": change.account_id,
            ":caused_by_output_id": change.caused_by_output_id,
            ":caused_by_input_id": change.caused_by_input_id,
            ":description": change.description,
            ":balance_credit": balance_credit,
            ":balance_debit": balance_debit,
            ":effective_date": change.effective_date,
            ":effective_height": effective_height,
            ":claimed_recipient_address": change.claimed_recipient_address.map(|v| v.to_base58()),
            ":claimed_sender_address": change.claimed_sender_address.map(|v| v.to_base58()),
            ":memo_parsed": change.memo_parsed,
            ":memo_hex": change.memo_hex,
            ":claimed_fee": claimed_fee,
            ":claimed_amount": claimed_amount,
        },
    )?;

    Ok(())
}

pub fn get_all_balance_changes_by_account_id(conn: &Connection, account_id: i64) -> WalletDbResult<Vec<BalanceChange>> {
    debug!(
        account_id = account_id;
        "DB: Fetching all balance changes"
    );

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT 
            account_id,
            caused_by_output_id,
            caused_by_input_id,
            description,
            balance_credit,
            balance_debit,
            REPLACE(effective_date, ' ', 'T') as effective_date,
            effective_height,
            claimed_recipient_address,
            claimed_sender_address,
            memo_parsed,
            memo_hex,
            claimed_fee,
            claimed_amount
        FROM balance_changes
        WHERE account_id = :account_id
        ORDER BY effective_height ASC, id ASC
        "#,
    )?;

    let rows = stmt.query(named_params! { ":account_id": account_id })?;
    let results: Vec<BalanceChange> = from_rows::<BalanceChange>(rows).collect::<Result<Vec<_>, _>>()?;

    Ok(results)
}

#[derive(Debug, Default, Deserialize)]
pub struct BalanceAggregates {
    pub total_credits: Option<i64>,
    pub total_debits: Option<i64>,
    pub max_height: Option<i64>,
    pub max_date: Option<chrono::NaiveDateTime>,
}

pub fn get_balance_aggregates_for_account(conn: &Connection, account_id: i64) -> WalletDbResult<BalanceAggregates> {
    let mut stmt = conn.prepare_cached(
        r#"
            SELECT
              SUM(balance_credit) as total_credits,
              SUM(balance_debit) as total_debits,
              MAX(effective_height) as max_height,
              REPLACE(MAX(effective_date), ' ', 'T') as max_date
            FROM balance_changes
            WHERE account_id = :account_id
        "#,
    )?;

    let rows = stmt.query(named_params! { ":account_id": account_id })?;
    let result = from_rows::<BalanceAggregates>(rows)
        .next()
        .ok_or_else(|| WalletDbError::Unexpected("Aggregate query returned no rows".to_string()))??;
    Ok(result)
}
