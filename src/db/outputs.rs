use crate::db::balance_changes::insert_balance_change;
use crate::db::error::{WalletDbError, WalletDbResult};
use crate::log::mask_amount;
use crate::models::BalanceChange;
use crate::models::OutputStatus;
use chrono::{DateTime, Utc};
use log::{debug, info, warn};
use rusqlite::{Connection, named_params};
use serde::Deserialize;
use serde_rusqlite::from_rows;
use tari_common_types::payment_reference::PaymentReference;
use tari_common_types::transaction::TxId;
use tari_common_types::types::FixedHash;
use tari_transaction_components::transaction_components::WalletOutput;

#[allow(clippy::too_many_arguments)]
pub fn insert_output(
    conn: &Connection,
    account_id: i64,
    account_view_key: &[u8],
    output_hash: Vec<u8>,
    output: &WalletOutput,
    block_height: u64,
    block_hash: &FixedHash,
    mined_timestamp: u64,
    memo_parsed: Option<String>,
    memo_hex: Option<String>,
    payment_reference: PaymentReference,
) -> WalletDbResult<i64> {
    info!(
        target: "audit",
        account_id = account_id,
        value = &*mask_amount(output.value().as_u64() as i64),
        height = block_height;
        "DB: Inserting output"
    );

    let id = TxId::new_deterministic(account_view_key, &output.output_hash()).as_i64_wrapped();

    let output_json = serde_json::to_string(&output)?;

    let mined_timestamp_dt = DateTime::<Utc>::from_timestamp(mined_timestamp as i64, 0)
        .ok_or_else(|| WalletDbError::Decoding(format!("Invalid mined timestamp: {}", mined_timestamp)))?;

    let block_height = block_height as i64;
    let value = output.value().as_u64() as i64;
    let payment_reference_hex = hex::encode(payment_reference.as_slice());

    let rows_affected = conn.execute(
        r#"
       INSERT INTO outputs (
            id,
            account_id,
            output_hash,
            mined_in_block_height,
            mined_in_block_hash,
            value,
            mined_timestamp,
            wallet_output_json,
            memo_parsed,
            memo_hex,
            payment_reference
       )
       VALUES (
            :id,
            :account_id,
            :output_hash,
            :block_height,
            :block_hash,
            :value,
            :mined_timestamp,
            :output_json,
            :memo_parsed,
            :memo_hex,
            :payment_reference
       )
        "#,
        named_params! {
            ":id": id,
            ":account_id": account_id,
            ":output_hash": output_hash,
            ":block_height": block_height,
            ":block_hash": block_hash.as_slice(),
            ":value": value,
            ":mined_timestamp": mined_timestamp_dt,
            ":output_json": output_json,
            ":memo_parsed": memo_parsed,
            ":memo_hex": memo_hex,
            ":payment_reference": payment_reference_hex,
        },
    )?;

    Ok(id)
}

#[derive(Deserialize)]
struct OutputInfoRow {
    id: i64,
    value: i64,
}

pub fn get_output_info_by_hash(conn: &Connection, output_hash: &FixedHash) -> WalletDbResult<Option<(i64, u64)>> {
    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, value
        FROM outputs
        WHERE output_hash = :output_hash AND deleted_at IS NULL
        "#,
    )?;

    let rows = stmt.query(named_params! { ":output_hash": output_hash.as_slice() })?;
    let result: Option<OutputInfoRow> = from_rows(rows).next().transpose()?;

    Ok(result.map(|r| (r.id, r.value as u64)))
}

#[derive(Deserialize)]
pub struct UnconfirmedOutputRow {
    pub output_hash: FixedHash,
    pub mined_in_block_height: i64,
    pub memo_parsed: Option<String>,
    pub memo_hex: Option<String>,
}

pub fn get_unconfirmed_outputs(
    conn: &Connection,
    account_id: i64,
    current_height: u64,
    confirmation_blocks: u64,
) -> WalletDbResult<Vec<UnconfirmedOutputRow>> {
    let min_height_to_confirm = current_height.saturating_sub(confirmation_blocks);
    let min_height = min_height_to_confirm as i64;

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT output_hash, mined_in_block_height, memo_parsed, memo_hex
        FROM outputs o
        WHERE o.account_id = :account_id
          AND o.mined_in_block_height <= :min_height
          AND o.confirmed_height IS NULL
          AND o.deleted_at IS NULL
        "#,
    )?;

    let rows = stmt.query(named_params! {
        ":account_id": account_id,
        ":min_height": min_height
    })?;

    let result_rows: Vec<UnconfirmedOutputRow> = from_rows(rows).collect::<Result<Vec<_>, _>>()?;
    Ok(result_rows)
}

pub fn mark_output_confirmed(
    conn: &Connection,
    output_hash: &FixedHash,
    confirmed_height: u64,
    confirmed_hash: &[u8],
) -> WalletDbResult<()> {
    info!(
        target: "audit",
        height = confirmed_height;
        "DB: Output Confirmed"
    );

    let confirmed_height = confirmed_height as i64;
    conn.execute(
        r#"
        UPDATE outputs
        SET confirmed_height = :height, confirmed_hash = :hash
        WHERE output_hash = :output_hash
        "#,
        named_params! {
            ":height": confirmed_height,
            ":hash": confirmed_hash,
            ":output_hash": output_hash.to_vec(),
        },
    )?;

    Ok(())
}

#[derive(Deserialize)]
struct OutputToDelete {
    id: i64,
    value: i64,
}

pub fn soft_delete_outputs_from_height(conn: &Connection, account_id: i64, height: u64) -> WalletDbResult<()> {
    warn!(
        target: "audit",
        account_id = account_id,
        height = height;
        "DB: Soft deleting outputs (Reorg)"
    );

    let height_i64 = height as i64;
    let now = Utc::now();

    let outputs_to_delete = {
        let mut stmt = conn.prepare_cached(
            r#"
            SELECT id, value
            FROM outputs
            WHERE account_id = :account_id AND mined_in_block_height >= :height AND deleted_at IS NULL
            "#,
        )?;

        let rows = stmt.query(named_params! {
            ":account_id": account_id,
            ":height": height_i64
        })?;

        from_rows::<OutputToDelete>(rows).collect::<Result<Vec<_>, _>>()?
    };

    for output_row in outputs_to_delete {
        let balance_change = BalanceChange {
            account_id,
            caused_by_output_id: Some(output_row.id),
            caused_by_input_id: None,
            description: format!("Reversal: Output found in blockchain scan (reorg at height {})", height),
            balance_credit: 0.into(),
            balance_debit: (output_row.value as u64).into(),
            effective_date: now.naive_utc(),
            effective_height: height,
            claimed_recipient_address: None,
            claimed_sender_address: None,
            memo_parsed: None,
            memo_hex: None,
            claimed_fee: None,
            claimed_amount: None,
        };
        insert_balance_change(conn, &balance_change)?;
    }

    conn.execute(
        r#"
        UPDATE outputs
        SET deleted_at = :now, deleted_in_block_height = :height, payment_reference = NULL
        WHERE account_id = :account_id AND mined_in_block_height >= :height AND deleted_at IS NULL
        "#,
        named_params! {
            ":now": now,
            ":height": height_i64,
            ":account_id": account_id
        },
    )?;

    Ok(())
}

pub fn update_output_status(conn: &Connection, output_id: i64, status: OutputStatus) -> WalletDbResult<()> {
    debug!(
        output_id = output_id,
        status:% = status;
        "DB: Updating output status"
    );

    let status_str = status.to_string();
    conn.execute(
        r#"
        UPDATE outputs
        SET status = :status
        WHERE id = :id
        "#,
        named_params! {
            ":status": status_str,
            ":id": output_id
        },
    )?;

    Ok(())
}

pub fn lock_output(
    conn: &Connection,
    output_id: i64,
    locked_by_request_id: &str,
    locked_at: DateTime<Utc>,
) -> WalletDbResult<()> {
    info!(
        target: "audit",
        output_id = output_id,
        request_id = locked_by_request_id;
        "DB: Locking output"
    );

    let locked_status = OutputStatus::Locked.to_string();
    let unspent_status = OutputStatus::Unspent.to_string();

    conn.execute(
        r#"
        UPDATE outputs
        SET status = :locked_status, locked_by_request_id = :req_id, locked_at = :locked_at
        WHERE id = :id and status = :unspent_status
        "#,
        named_params! {
            ":locked_status": locked_status,
            ":req_id": locked_by_request_id,
            ":locked_at": locked_at,
            ":id": output_id,
            ":unspent_status": unspent_status,
        },
    )?;

    Ok(())
}

#[derive(Debug)]
pub struct DbWalletOutput {
    pub id: i64,
    pub output: WalletOutput,
}

#[derive(Deserialize)]
struct WalletOutputRow {
    id: i64,
    wallet_output_json: Option<String>,
}

pub fn fetch_unspent_outputs(
    conn: &Connection,
    account_id: i64,
    min_height: u64,
) -> WalletDbResult<Vec<DbWalletOutput>> {
    let unspent_status = OutputStatus::Unspent.to_string();
    let min_height_i64 = min_height as i64;

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, wallet_output_json
        FROM outputs
        WHERE account_id = :account_id
          AND status = :unspent_status
          AND confirmed_height <= :min_height
          AND wallet_output_json IS NOT NULL
          AND deleted_at IS NULL
        ORDER BY value DESC
        "#,
    )?;

    let rows = stmt.query(
        named_params! { ":account_id": account_id, ":unspent_status": unspent_status, ":min_height": min_height_i64 },
    )?;
    let raw_rows: Vec<WalletOutputRow> = from_rows(rows).collect::<Result<Vec<_>, _>>()?;

    let mut outputs = Vec::new();
    for row in raw_rows {
        if let Some(json_str) = row.wallet_output_json {
            let output: WalletOutput = serde_json::from_str(&json_str)?;
            outputs.push(DbWalletOutput { id: row.id, output });
        }
    }
    Ok(outputs)
}

pub fn unlock_outputs_for_request(conn: &Connection, locked_by_request_id: &str) -> WalletDbResult<()> {
    debug!(
        request_id = locked_by_request_id;
        "DB: Unlocking outputs for request"
    );

    let unspent_status = OutputStatus::Unspent.to_string();
    let locked_status = OutputStatus::Locked.to_string();

    conn.execute(
        r#"
        UPDATE outputs
        SET status = :unspent, locked_at = NULL, locked_by_request_id = NULL
        WHERE locked_by_request_id = :req_id AND status = :locked
        "#,
        named_params! {
            ":unspent": unspent_status,
            ":req_id": locked_by_request_id,
            ":locked": locked_status
        },
    )?;

    Ok(())
}

pub fn fetch_outputs_by_lock_request_id(
    conn: &Connection,
    locked_by_request_id: &str,
) -> WalletDbResult<Vec<DbWalletOutput>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, wallet_output_json FROM outputs WHERE locked_by_request_id = :req_id and wallet_output_json IS NOT NULL"
    )?;

    let rows = stmt.query(named_params! { ":req_id": locked_by_request_id })?;
    let raw_rows: Vec<WalletOutputRow> = from_rows(rows).collect::<Result<Vec<_>, _>>()?;

    let mut outputs = Vec::new();
    for row in raw_rows {
        if let Some(json_str) = row.wallet_output_json {
            let output: WalletOutput = serde_json::from_str(&json_str)?;
            outputs.push(DbWalletOutput { id: row.id, output });
        }
    }
    Ok(outputs)
}

#[derive(Deserialize)]
struct OutputTotals {
    locked_val: i64,
    unconfirmed_val: i64,
    locked_and_unconfirmed_val: i64,
}

/// Retrieves the sum of LOCKED values, the sum of UNCONFIRMED values, and the sum of values that are both LOCKED and UNCONFIRMED for an account.
/// Returns (locked_balance, unconfirmed_balance, locked_and_unconfirmed_balance)
pub fn get_output_totals_for_account(conn: &Connection, account_id: i64) -> WalletDbResult<(u64, u64, u64)> {
    let locked_status = OutputStatus::Locked.to_string();

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT
            COALESCE(SUM(CASE WHEN status = :locked THEN value ELSE 0 END), 0) as locked_val,
            COALESCE(SUM(CASE WHEN confirmed_height IS NULL THEN value ELSE 0 END), 0) as unconfirmed_val,
            COALESCE(SUM(CASE WHEN status = :locked AND confirmed_height IS NULL THEN value ELSE 0 END), 0) as locked_and_unconfirmed_val
        FROM outputs
        WHERE account_id = :account_id AND deleted_at IS NULL
        "#,
    )?;

    let rows = stmt.query(named_params! {
        ":locked": locked_status,
        ":account_id": account_id
    })?;

    let result = from_rows::<OutputTotals>(rows)
        .next()
        .ok_or_else(|| WalletDbError::Unexpected("Aggregate query returned no rows".to_string()))??;

    Ok((
        result.locked_val as u64,
        result.unconfirmed_val as u64,
        result.locked_and_unconfirmed_val as u64,
    ))
}

#[derive(Deserialize)]
pub struct ReorgOutputInfo {
    pub output_hash: Vec<u8>,
    pub mined_in_block_height: i64,
    pub locked_by_request_id: Option<String>,
}

pub fn get_active_outputs_from_height(
    conn: &Connection,
    account_id: i64,
    height: u64,
) -> WalletDbResult<Vec<ReorgOutputInfo>> {
    let height_i64 = height as i64;

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT output_hash, mined_in_block_height, locked_by_request_id
        FROM outputs
        WHERE account_id = :account_id 
          AND mined_in_block_height >= :height 
          AND deleted_at IS NULL
        "#,
    )?;

    let rows = stmt.query(named_params! {
        ":account_id": account_id,
        ":height": height_i64
    })?;

    let results = from_rows::<ReorgOutputInfo>(rows).collect::<Result<Vec<_>, _>>()?;

    Ok(results)
}
