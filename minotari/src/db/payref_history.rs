use crate::db::error::WalletDbResult;
use log::{debug, warn};
use rusqlite::{Connection, OptionalExtension, named_params};
use tari_common_types::transaction::TxId;

/// Save an old payref to the history table before it is cleared during a reorg.
/// This allows lookups by stale payrefs to still resolve to the correct transaction.
///
/// `TxId` is a `u64`; SQLite does not support unsigned 64-bit integers, so the
/// id is stored as the wrapping `i64` (`TxId::as_i64_wrapped`) and converted
/// back on read. This matches how `completed_transactions.id` is persisted.
pub fn save_payref_history(
    conn: &Connection,
    account_id: i64,
    transaction_id: TxId,
    old_payref: &str,
    output_hash: Option<&str>,
) -> WalletDbResult<()> {
    debug!(
        account_id = account_id,
        transaction_id = transaction_id.to_string().as_str(),
        old_payref = old_payref;
        "DB: Saving payref to history"
    );

    conn.execute(
        r#"
        INSERT INTO payref_history (account_id, transaction_id, old_payref, output_hash)
        VALUES (:account_id, :transaction_id, :old_payref, :output_hash)
        "#,
        named_params! {
            ":account_id": account_id,
            ":transaction_id": transaction_id.as_i64_wrapped(),
            ":old_payref": old_payref,
            ":output_hash": output_hash,
        },
    )?;

    Ok(())
}

/// Look up a transaction id by a historical (stale) payref.
/// Returns the most-recently-saved match, if any.
pub fn get_transaction_id_by_historical_payref(
    conn: &Connection,
    account_id: i64,
    payref: &str,
) -> WalletDbResult<Option<TxId>> {
    debug!(
        account_id = account_id,
        payref = payref;
        "DB: Looking up historical payref"
    );

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT transaction_id
        FROM payref_history
        WHERE account_id = :account_id AND old_payref = :payref
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )?;

    let result = stmt
        .query_row(
            named_params! {
                ":account_id": account_id,
                ":payref": payref
            },
            |row| row.get::<_, i64>(0),
        )
        .optional()?;

    #[allow(clippy::cast_sign_loss)]
    Ok(result.map(|id| TxId::from(id as u64)))
}

/// Save historical payrefs for displayed transactions being reorganized.
/// Called before the displayed transaction payref column gets overwritten on re-mine.
pub fn save_displayed_transaction_payrefs_before_reorg(
    conn: &Connection,
    account_id: i64,
    reorg_height: u64,
) -> WalletDbResult<u64> {
    warn!(
        account_id = account_id,
        height = reorg_height;
        "DB: Saving displayed transaction payrefs before reorg"
    );

    #[allow(clippy::cast_possible_wrap)]
    let height = reorg_height as i64;

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, payref
        FROM displayed_transactions
        WHERE account_id = :account_id
          AND payref IS NOT NULL
          AND block_height >= :height
        "#,
    )?;

    // `displayed_transactions.id` is the stringified form of the `TxId`.
    let rows = stmt.query_map(
        named_params! {
            ":account_id": account_id,
            ":height": height
        },
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;

    let mut count = 0u64;
    for row in rows {
        let (tx_id_str, payref_json) = row?;
        let tx_id = match tx_id_str.parse::<u64>().map(TxId::from) {
            Ok(id) => id,
            Err(e) => {
                warn!(
                    id = tx_id_str.as_str(),
                    error = e.to_string().as_str();
                    "DB: Skipping displayed tx with unparsable id while saving payref history"
                );
                continue;
            },
        };
        // payref column stores a JSON array of payref strings
        if let Ok(payrefs) = serde_json::from_str::<Vec<String>>(&payref_json) {
            for payref in payrefs {
                if !payref.is_empty() {
                    save_payref_history(conn, account_id, tx_id, &payref, None)?;
                    count += 1;
                }
            }
        }
    }

    Ok(count)
}

/// Save historical payrefs for all completed transactions being reset during a reorg.
/// Called before `reset_mined_completed_transactions_from_height` clears the payrefs.
pub fn save_completed_transaction_payrefs_before_reorg(
    conn: &Connection,
    account_id: i64,
    reorg_height: u64,
) -> WalletDbResult<u64> {
    warn!(
        account_id = account_id,
        height = reorg_height;
        "DB: Saving completed transaction payrefs before reorg"
    );

    #[allow(clippy::cast_possible_wrap)]
    let height = reorg_height as i64;

    let status_unconfirmed = "mined_unconfirmed".to_string();
    let status_confirmed = "mined_confirmed".to_string();

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, sent_payref, sent_output_hash
        FROM completed_transactions
        WHERE account_id = :account_id
          AND sent_payref IS NOT NULL
          AND (status = :status_unconfirmed OR status = :status_confirmed)
          AND mined_height >= :height
        "#,
    )?;

    // `completed_transactions.id` is stored as the wrapping i64 of the TxId.
    let rows = stmt.query_map(
        named_params! {
            ":account_id": account_id,
            ":status_unconfirmed": status_unconfirmed,
            ":status_confirmed": status_confirmed,
            ":height": height
        },
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )?;

    let mut count = 0u64;
    for row in rows {
        let (tx_id_i64, payref, output_hash) = row?;
        #[allow(clippy::cast_sign_loss)]
        let tx_id = TxId::from(tx_id_i64 as u64);
        save_payref_history(conn, account_id, tx_id, &payref, output_hash.as_deref())?;
        count += 1;
    }

    Ok(count)
}
