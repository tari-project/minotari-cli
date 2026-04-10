// Payref History Step Definitions
//
// Exercises the payref history fallback lookup wired into the api layer.
// The cucumber world prepares a wallet database, inserts a completed
// transaction together with a stale payref in `payref_history`, starts the
// daemon, and asserts that the api serves the transaction for both the live
// and the historical payref while returning 404 for an unrelated payref.

use cucumber::{given, then, when};
use rusqlite::{Connection, params};

use super::common::{MinotariWorld, database_with_wallet};

fn open_test_connection(world: &MinotariWorld) -> Connection {
    let db_path = world.database_path.as_ref().expect("Database not set up");
    let conn = Connection::open(db_path).expect("Failed to open test database");
    // Disable FK enforcement on this test connection so we can seed
    // `completed_transactions` without also constructing a matching row in
    // `pending_transactions`. FK enforcement is a per-connection pragma in
    // SQLite; the daemon's connection keeps it on.
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("Failed to disable foreign keys on test connection");
    conn
}

fn account_id_for_default(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT id FROM accounts WHERE friendly_name = ?1",
        params!["default"],
        |row| row.get::<_, i64>(0),
    )
    .expect("default account row not found — ensure the wallet was imported first")
}

#[given(regex = r#"^a completed transaction "([^"]*)" exists for the default account with live payref "([^"]*)"$"#)]
async fn seed_completed_transaction(world: &mut MinotariWorld, tx_id: String, live_payref: String) {
    // `database_with_wallet` is idempotent for our purposes (runs import-view-key)
    // and is safe to re-invoke if the feature hasn't already executed it.
    if world.database_path.is_none() {
        database_with_wallet(world).await;
    }

    let tx_id_i64: i64 = tx_id.parse().expect("transaction id must be an integer");

    let conn = open_test_connection(world);
    let account_id = account_id_for_default(&conn);

    conn.execute(
        r#"
        INSERT INTO completed_transactions (
            id, account_id, pending_tx_id, status, kernel_excess,
            sent_payref, serialized_transaction
        )
        VALUES (?1, ?2, ?3, 'mined_confirmed', ?4, ?5, ?6)
        "#,
        params![
            tx_id_i64,
            account_id,
            format!("pending-{}", tx_id_i64),
            Vec::<u8>::new(),
            live_payref,
            Vec::<u8>::new(),
        ],
    )
    .expect("Failed to insert seed completed transaction");
}

#[given(regex = r#"^the completed transaction "([^"]*)" has historical payref "([^"]*)" recorded$"#)]
async fn seed_payref_history(world: &mut MinotariWorld, tx_id: String, old_payref: String) {
    let tx_id_i64: i64 = tx_id.parse().expect("transaction id must be an integer");

    let conn = open_test_connection(world);
    let account_id = account_id_for_default(&conn);

    conn.execute(
        r#"
        INSERT INTO payref_history (account_id, transaction_id, old_payref, output_hash)
        VALUES (?1, ?2, ?3, NULL)
        "#,
        params![account_id, tx_id_i64, old_payref],
    )
    .expect("Failed to insert payref history row");
}

#[when(regex = r#"^I request the completed transaction by payref "([^"]*)" via the API on port "([^"]*)"$"#)]
async fn request_completed_by_payref(world: &mut MinotariWorld, payref: String, port: String) {
    let port_num: u16 = port.parse().expect("Invalid port number");
    let url = format!(
        "http://127.0.0.1:{}/accounts/default/completed_transactions/by_payref/{}",
        port_num, payref
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .send()
        .await
        .expect("Failed to query completed-transaction-by-payref API");

    let status = response.status();
    let body = response.text().await.expect("Failed to read response body");

    world.last_command_output = Some(body);
    world.last_command_exit_code = Some(if status.is_success() {
        0
    } else {
        i32::from(status.as_u16())
    });
}

#[then(regex = r#"^the API should return a completed transaction with id "([^"]*)"$"#)]
async fn assert_returned_completed_tx(world: &mut MinotariWorld, expected_id: String) {
    let body = world
        .last_command_output
        .as_ref()
        .expect("No response body captured from the previous API call");

    let exit = world.last_command_exit_code.unwrap_or(-1);
    assert_eq!(
        exit, 0,
        "Expected 2xx success from API, got exit code {exit}, body: {body}"
    );

    let json: serde_json::Value = serde_json::from_str(body).expect("Response body was not valid JSON");
    let id_value = json.get("id").cloned().expect("API response missing `id` field");

    // CompletedTransactionResponse serializes `id` via tx_id_schema, which may
    // render TxId as either a JSON number or string depending on the schema.
    // Accept both forms.
    let actual_id = match id_value {
        serde_json::Value::Number(n) => n.as_u64().expect("id number was not u64").to_string(),
        serde_json::Value::String(s) => s,
        other => panic!("Unexpected JSON type for id field: {other:?}"),
    };

    assert_eq!(
        actual_id, expected_id,
        "API returned transaction id {actual_id}, expected {expected_id}"
    );
}

#[then("the API should return a 404 for the completed transaction")]
async fn assert_404(world: &mut MinotariWorld) {
    let exit = world
        .last_command_exit_code
        .expect("No exit code captured from the previous API call");
    assert_eq!(
        exit, 404,
        "Expected 404 from API, got exit code {exit}, body: {:?}",
        world.last_command_output
    );
}
