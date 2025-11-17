use std::sync::LazyLock;

use sqlx::SqliteConnection;
use tokio::sync::{
    Mutex,
    mpsc::{Receiver, Sender},
};

use crate::models::WalletEvent;

pub static EVENT_CHANNEL: LazyLock<(Sender<WalletEvent>, Mutex<Receiver<WalletEvent>>)> = LazyLock::new(|| {
    let (tx, rx) = tokio::sync::mpsc::channel(100);
    (tx, Mutex::new(rx))
});

pub async fn insert_wallet_event(
    conn: &mut SqliteConnection,
    account_id: i64,
    event: &WalletEvent,
) -> Result<(), anyhow::Error> {
    let event_type = &event.event_type.to_key_string();
    let description = &event.description;
    let data_json = serde_json::to_value(&event.event_type)?.to_string();
    let query_result = sqlx::query!(
        r#"
       INSERT INTO events (account_id, event_type, description, data_json)
         VALUES (?, ?, ?, ?)
        "#,
        account_id,
        event_type,
        description,
        data_json
    )
    .execute(&mut *conn)
    .await?;

    // Notify listeners about the new event
    // Do not fail because of notification failure
    let _unused = EVENT_CHANNEL
        .0
        .send(WalletEvent {
            id: query_result.last_insert_rowid(),
            account_id: event.account_id,
            event_type: event.event_type.clone(),
            description: event.description.clone(),
        })
        .await
        .map_err(|e| sqlx::Error::Protocol(e.to_string()));

    Ok(())
}
