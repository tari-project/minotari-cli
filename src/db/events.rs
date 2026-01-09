use log::info;
use rusqlite::{Connection, named_params};

use crate::WalletEvent;
use crate::db::error::WalletDbResult;

pub fn insert_wallet_event(conn: &Connection, account_id: i64, event: &WalletEvent) -> WalletDbResult<()> {
    info!(
        target: "audit",
        account_id = account_id,
        event_type = &*event.event_type.to_key_string();
        "DB: Inserting wallet event"
    );

    let event_type = &event.event_type.to_key_string();
    let description = &event.description;
    let data_json = serde_json::to_value(&event.event_type)?.to_string();

    conn.execute(
        r#"
       INSERT INTO events (account_id, event_type, description, data_json)
         VALUES (:account_id, :event_type, :description, :data_json)
        "#,
        named_params! {
            ":account_id": account_id,
            ":event_type": event_type,
            ":description": description,
            ":data_json": data_json
        },
    )?;

    Ok(())
}
