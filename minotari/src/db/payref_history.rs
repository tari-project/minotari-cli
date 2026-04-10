use crate::db::error::WalletDbResult;
use log::{debug, warn};
use rusqlite::{Connection, OptionalExtension, named_params};

/// Save old payrefs to the history table before they are cleared during a reorg.
/// This allows lookups by stale payrefs to still resolve to the correct transaction.
pub fn save_payref_history(
    conn: &Connection,
    account_id: i64,
    transaction_id: &str,
    old_payref: &str,
    output_hash: Option<&str>,
) -> WalletDbResult<()> {
    debug!(
        account_id = account_id,
        transaction_id = transaction_id,
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
            ":transaction_id": transaction_id,
            ":old_payref": old_payref,
            ":output_hash": output_hash,
        },
    )?;

    Ok(())
}

/// Look up a transaction ID by a historical (stale) payref.
pub fn get_transaction_id_by_historical_payref(
    conn: &Connection,
    account_id: i64,
    payref: &str,
) -> WalletDbResult<Option<String>> {
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
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    Ok(result)
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

    let rows = stmt.query_map(
        named_params! {
            ":account_id": account_id,
            ":height": height
        },
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;

    let mut count = 0u64;
    for row in rows {
        let (tx_id, payref_json) = row?;
        // payref column stores a JSON array of payref strings
        if let Ok(payrefs) = serde_json::from_str::<Vec<String>>(&payref_json) {
            for payref in payrefs {
                if !payref.is_empty() {
                    save_payref_history(conn, account_id, &tx_id, &payref, None)?;
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

    let rows = stmt.query_map(
        named_params! {
            ":account_id": account_id,
            ":status_unconfirmed": status_unconfirmed,
            ":status_confirmed": status_confirmed,
            ":height": height
        },
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )?;

    let mut count = 0u64;
    for row in rows {
        let (tx_id, payref, output_hash) = row?;
        save_payref_history(conn, account_id, &tx_id, &payref, output_hash.as_deref())?;
        count += 1;
    }

    Ok(count)
}
