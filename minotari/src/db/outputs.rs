use crate::db::balance_changes::{
    get_balance_change_id_by_output, insert_balance_change, mark_balance_change_as_reversed,
};
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
use tari_common_types::types::PrivateKey;
use tari_transaction_components::MicroMinotari;
use tari_transaction_components::transaction_components::WalletOutput;
use tari_transaction_components::utxo_selection::UtxoValue;
use tari_utilities::ByteArray;

#[allow(clippy::too_many_arguments)]
pub fn insert_output(
    conn: &Connection,
    account_id: i64,
    account_view_key: &PrivateKey,
    output_hash: Vec<u8>,
    output: &WalletOutput,
    block_height: u64,
    block_hash: &FixedHash,
    mined_timestamp: u64,
    memo_parsed: Option<String>,
    memo_hex: Option<String>,
    payment_reference: PaymentReference,
    is_burn: bool,
) -> WalletDbResult<i64> {
    info!(
        target: "audit",
        account_id = account_id,
        value = &*mask_amount(output.value()),
        height = block_height;
        "DB: Inserting output"
    );

    let tx_id = TxId::new_deterministic(account_view_key.as_bytes(), &output.output_hash()).as_i64_wrapped();

    let output_json = serde_json::to_string(&output)?;

    #[allow(clippy::cast_possible_wrap)]
    let mined_timestamp_dt = DateTime::<Utc>::from_timestamp(mined_timestamp as i64, 0)
        .ok_or_else(|| WalletDbError::Decoding(format!("Invalid mined timestamp: {}", mined_timestamp)))?;

    #[allow(clippy::cast_possible_wrap)]
    let block_height = block_height as i64;
    #[allow(clippy::cast_possible_wrap)]
    let value = output.value().as_u64() as i64;
    let payment_reference_hex = hex::encode(payment_reference.as_slice());

    conn.execute(
        r#"
       INSERT INTO outputs (
            account_id,
            tx_id,
            output_hash,
            mined_in_block_height,
            mined_in_block_hash,
            value,
            mined_timestamp,
            wallet_output_json,
            memo_parsed,
            memo_hex,
            payment_reference,
            is_burn
       )
       VALUES (
            :account_id,
            :tx_id,
            :output_hash,
            :block_height,
            :block_hash,
            :value,
            :mined_timestamp,
            :output_json,
            :memo_parsed,
            :memo_hex,
            :payment_reference,
            :is_burn
       )
        "#,
        named_params! {
            ":account_id": account_id,
            ":tx_id": tx_id,
            ":output_hash": output_hash,
            ":block_height": block_height,
            ":block_hash": block_hash.as_slice(),
            ":value": value,
            ":mined_timestamp": mined_timestamp_dt,
            ":output_json": output_json,
            ":memo_parsed": memo_parsed,
            ":memo_hex": memo_hex,
            ":payment_reference": payment_reference_hex,
            ":is_burn": is_burn,
        },
    )?;

    Ok(conn.last_insert_rowid())
}

/// Inserts an output only if one with the same output_hash does not already exist.
/// Used during backfill to avoid duplicate outputs. Outputs inserted here are set
/// to `SpentUnconfirmed` status since they were not found as unspent during fast
/// sync and are therefore known to be spent.
/// Returns `Some(id)` if inserted, `None` if the output already existed.
#[allow(clippy::too_many_arguments)]
pub fn insert_spent_output_if_not_exists(
    conn: &Connection,
    account_id: i64,
    account_view_key: &PrivateKey,
    output_hash: Vec<u8>,
    output: &WalletOutput,
    block_height: u64,
    block_hash: &FixedHash,
    mined_timestamp: u64,
    memo_parsed: Option<String>,
    memo_hex: Option<String>,
    payment_reference: PaymentReference,
    is_burn: bool,
) -> WalletDbResult<Option<i64>> {
    let hash_as_fixed = FixedHash::try_from(output_hash.as_slice())
        .map_err(|e| WalletDbError::Decoding(format!("Invalid output hash: {}", e)))?;
    if get_output_info_by_hash(conn, &hash_as_fixed)?.is_some() {
        return Ok(None);
    }

    let id = insert_output(
        conn,
        account_id,
        account_view_key,
        output_hash,
        output,
        block_height,
        block_hash,
        mined_timestamp,
        memo_parsed,
        memo_hex,
        payment_reference,
        is_burn,
    )?;

    // Mark as SpentUnconfirmed — these outputs were not found as unspent during
    // Phase 1 fast sync, so they are known to be spent. The status will transition
    // to Spent once the backfill processes the corresponding input.
    update_output_status(conn, id, OutputStatus::SpentUnconfirmed)?;

    Ok(Some(id))
}

pub fn get_output_info_by_hash(
    conn: &Connection,
    output_hash: &FixedHash,
) -> WalletDbResult<Option<(i64, TxId, WalletOutput)>> {
    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, tx_id, wallet_output_json
        FROM outputs
        WHERE output_hash = :output_hash AND deleted_at IS NULL
        "#,
    )?;

    let rows = stmt.query(named_params! { ":output_hash": output_hash.as_slice() })?;
    let row: Option<WalletOutputRow> = from_rows(rows).next().transpose()?;
    let data = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    let output: WalletOutput = serde_json::from_str(&data.wallet_output_json)?;

    let tx_id = TxId::from(data.tx_id as u64);

    Ok(Some((data.id, tx_id, output)))
}

pub fn get_output_info_by_hash_for_account(
    conn: &Connection,
    account_id: i64,
    output_hash: &FixedHash,
) -> WalletDbResult<Option<(i64, TxId, WalletOutput)>> {
    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, tx_id, wallet_output_json
        FROM outputs
        WHERE account_id = :account_id AND output_hash = :output_hash AND deleted_at IS NULL
        "#,
    )?;

    let rows = stmt.query(named_params! {
        ":account_id": account_id,
        ":output_hash": output_hash.as_slice(),
    })?;
    let row: Option<WalletOutputRow> = from_rows(rows).next().transpose()?;
    let data = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    let output: WalletOutput = serde_json::from_str(&data.wallet_output_json)?;

    let tx_id = TxId::from(data.tx_id as u64);

    Ok(Some((data.id, tx_id, output)))
}

#[derive(Deserialize)]
pub struct UnconfirmedOutputRow {
    pub output_hash: FixedHash,
    pub mined_in_block_height: i64,
    pub memo_parsed: Option<String>,
    pub memo_hex: Option<String>,
    pub tx_id: i64,
    pub is_burn: i64,
}

pub fn get_unconfirmed_outputs(
    conn: &Connection,
    account_id: i64,
    current_height: u64,
    confirmation_blocks: u64,
) -> WalletDbResult<Vec<UnconfirmedOutputRow>> {
    let min_height_to_confirm = current_height.saturating_sub(confirmation_blocks);
    #[allow(clippy::cast_possible_wrap)]
    let min_height = min_height_to_confirm as i64;

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT output_hash, mined_in_block_height, memo_parsed, memo_hex, tx_id, is_burn
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

    #[allow(clippy::cast_possible_wrap)]
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
    #[allow(clippy::cast_possible_wrap)]
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
        // Find and mark the original balance change as reversed
        let original_balance_change_id = get_balance_change_id_by_output(conn, output_row.id)?;
        if let Some(original_id) = original_balance_change_id {
            mark_balance_change_as_reversed(conn, original_id)?;
        }

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
            is_reversal: true,
            reversal_of_balance_change_id: original_balance_change_id,
            is_reversed: false,
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

/// Marks `SpentUnconfirmed` outputs that have a matching input as `Spent`.
/// Returns the number of outputs marked as spent.
pub fn resolve_spent_unconfirmed_with_inputs(conn: &Connection, account_id: i64) -> WalletDbResult<u64> {
    let spent_unconfirmed = OutputStatus::SpentUnconfirmed.to_string();
    let spent = OutputStatus::Spent.to_string();

    let marked_spent = conn.execute(
        r#"
        UPDATE outputs
        SET status = :spent_status
        WHERE account_id = :account_id
          AND status = :spent_unconfirmed_status
          AND deleted_at IS NULL
          AND id IN (
              SELECT output_id FROM inputs WHERE deleted_at IS NULL
          )
        "#,
        named_params! {
            ":spent_status": spent,
            ":account_id": account_id,
            ":spent_unconfirmed_status": spent_unconfirmed,
        },
    )? as u64;

    info!(
        target: "audit",
        account_id = account_id,
        marked_spent = marked_spent;
        "Resolved SpentUnconfirmed outputs with matching inputs"
    );

    Ok(marked_spent)
}

/// Returned data for an unresolved SpentUnconfirmed output.
pub struct UnresolvedSpentOutput {
    pub output_id: i64,
    pub output_hash: Vec<u8>,
    pub mined_in_block_height: u64,
    pub value: u64,
}

/// Returns info for remaining `SpentUnconfirmed` outputs that have no matching input.
/// These need to be verified against the base node.
pub fn get_unresolved_spent_unconfirmed_outputs(
    conn: &Connection,
    account_id: i64,
) -> WalletDbResult<Vec<UnresolvedSpentOutput>> {
    let spent_unconfirmed = OutputStatus::SpentUnconfirmed.to_string();

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, output_hash, mined_in_block_height, value
        FROM outputs
        WHERE account_id = :account_id
          AND status = :status
          AND deleted_at IS NULL
        ORDER BY mined_in_block_height
        "#,
    )?;

    let rows = stmt.query_map(
        named_params! {
            ":account_id": account_id,
            ":status": spent_unconfirmed,
        },
        |row| {
            Ok(UnresolvedSpentOutput {
                output_id: row.get(0)?,
                output_hash: row.get(1)?,
                mined_in_block_height: row.get::<_, i64>(2).map(|h| h as u64)?,
                value: row.get::<_, i64>(3).map(|v| v as u64)?,
            })
        },
    )?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }

    Ok(results)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbWalletOutput {
    pub id: i64,
    pub tx_id: TxId,
    pub output: WalletOutput,
}

impl UtxoValue for DbWalletOutput {
    fn value(&self) -> MicroMinotari {
        self.output.value()
    }
}

#[derive(Deserialize, Debug)]
struct WalletOutputRow {
    id: i64,
    tx_id: i64,
    wallet_output_json: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DbOutput {
    pub id: i64,
    pub account_id: i64,
    pub output_hash: Vec<u8>,
    pub mined_in_block_hash: Vec<u8>,
    pub mined_in_block_height: i64,
    pub value: i64,
    pub created_at: chrono::NaiveDateTime,
    pub wallet_output_json: Option<String>,
    pub mined_timestamp: chrono::NaiveDateTime,
    pub confirmed_height: Option<i64>,
    pub confirmed_hash: Option<Vec<u8>>,
    pub memo_parsed: Option<String>,
    pub memo_hex: Option<String>,
    pub status: String,
    pub locked_at: Option<chrono::NaiveDateTime>,
    pub locked_by_request_id: Option<String>,
    pub deleted_at: Option<chrono::NaiveDateTime>,
    pub deleted_in_block_height: Option<i64>,
    pub payment_reference: Option<String>,
}

impl DbOutput {
    pub fn to_wallet_output(&self) -> WalletDbResult<WalletOutput> {
        let output_str = self
            .wallet_output_json
            .as_ref()
            .ok_or_else(|| WalletDbError::Unexpected("Output JSON is null".to_string()))?;
        let output: WalletOutput = serde_json::from_str(output_str)?;
        Ok(output)
    }
}

pub fn get_output_by_id(conn: &Connection, output_id: i64) -> WalletDbResult<Option<DbOutput>> {
    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, account_id, output_hash, mined_in_block_hash, mined_in_block_height,
               value, created_at, wallet_output_json, mined_timestamp, confirmed_height,
               confirmed_hash, memo_parsed, memo_hex, status, locked_at, locked_by_request_id,
               deleted_at, deleted_in_block_height, payment_reference
        FROM outputs
        WHERE id = :id
        "#,
    )?;

    let rows = stmt.query(named_params! { ":id": output_id })?;
    let output: Option<DbOutput> = from_rows(rows).next().transpose()?;
    Ok(output)
}

pub fn fetch_unspent_outputs(
    conn: &Connection,
    account_id: i64,
    min_height: u64,
) -> WalletDbResult<Vec<DbWalletOutput>> {
    let unspent_status = OutputStatus::Unspent.to_string();
    #[allow(clippy::cast_possible_wrap)]
    let min_height_i64 = min_height as i64;

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, tx_id, wallet_output_json
        FROM outputs
        WHERE account_id = :account_id
          AND status = :unspent_status
          AND mined_in_block_height <= :min_height
          AND deleted_at IS NULL
          AND is_burn = 0
        ORDER BY value DESC
        "#,
    )?;

    let rows = stmt.query(
        named_params! { ":account_id": account_id, ":unspent_status": unspent_status, ":min_height": min_height_i64 },
    )?;
    let raw_rows: Vec<WalletOutputRow> = from_rows(rows).collect::<Result<Vec<_>, _>>()?;

    let mut outputs = Vec::new();
    for row in raw_rows {
        let output: WalletOutput = serde_json::from_str(&row.wallet_output_json)?;
        outputs.push(DbWalletOutput {
            id: row.id,
            tx_id: TxId::from(row.tx_id as u64),
            output,
        });
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
    let mut stmt =
        conn.prepare_cached("SELECT id, tx_id, wallet_output_json FROM outputs WHERE locked_by_request_id = :req_id")?;

    let rows = stmt.query(named_params! { ":req_id": locked_by_request_id })?;
    let raw_rows: Vec<WalletOutputRow> = from_rows(rows).collect::<Result<Vec<_>, _>>()?;

    let mut outputs = Vec::new();
    for row in raw_rows {
        let output: WalletOutput = serde_json::from_str(&row.wallet_output_json)?;
        outputs.push(DbWalletOutput {
            id: row.id,
            tx_id: TxId::from(row.tx_id as u64),
            output,
        });
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
pub fn get_output_totals_for_account(
    conn: &Connection,
    account_id: i64,
) -> WalletDbResult<(MicroMinotari, MicroMinotari, MicroMinotari)> {
    let locked_status = OutputStatus::Locked.to_string();

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT
            COALESCE(SUM(CASE WHEN status = :locked THEN value ELSE 0 END), 0) as locked_val,
            COALESCE(SUM(CASE WHEN confirmed_height IS NULL THEN value ELSE 0 END), 0) as unconfirmed_val,
            COALESCE(SUM(CASE WHEN status = :locked AND confirmed_height IS NULL THEN value ELSE 0 END), 0) as locked_and_unconfirmed_val
        FROM outputs
        WHERE account_id = :account_id AND deleted_at IS NULL AND is_burn = 0
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
        (result.locked_val as u64).into(),
        (result.unconfirmed_val as u64).into(),
        (result.locked_and_unconfirmed_val as u64).into(),
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
    #[allow(clippy::cast_possible_wrap)]
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

pub fn get_total_unspent_balance(conn: &Connection, account_id: i64) -> WalletDbResult<u64> {
    let unspent_status = OutputStatus::Unspent.to_string();

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT COALESCE(SUM(value), 0)
        FROM outputs
        WHERE account_id = :account_id
          AND status = :unspent_status
          AND deleted_at IS NULL
          AND is_burn = 0
        "#,
    )?;

    let total = stmt.query_row(
        named_params! {
            ":account_id": account_id,
            ":unspent_status": unspent_status
        },
        |row| row.get(0),
    )?;

    Ok(total)
}
