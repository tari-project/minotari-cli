use sqlx::SqliteConnection;

use crate::models::WalletEvent;

pub async fn insert_wallet_event(
    conn: &mut SqliteConnection,
    account_id: i64,
    event: &WalletEvent,
) -> Result<(), anyhow::Error> {
    let event_type = &event.event_type.to_key_string();
    let description = &event.description;
    let data_json = serde_json::to_value(&event.event_type)?.to_string();
    sqlx::query!(
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

    Ok(())
}
