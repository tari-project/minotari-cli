use sqlx::SqlitePool;

pub async fn create_account(
    pool: &SqlitePool,
    friendly_name: &str,
    encryptd_view_private_key: &[u8],
    encrypted_spend_public_key: &[u8],
    cipher_nonce: &[u8],
    unencrypted_view_key_hash: &[u8],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO accounts (friendly_name, encrypted_view_private_key, encrypted_spend_public_key, cipher_nonce, unencrypted_view_key_hash)
        VALUES (?, ?, ?, ?, ?)
        "#,
friendly_name, encryptd_view_private_key, encrypted_spend_public_key, cipher_nonce, unencrypted_view_key_hash
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_account_by_name(
    pool: &SqlitePool,
    friendly_name: &str,
) -> Result<Option<AccountRow>, sqlx::Error> {
    let row = sqlx::query_as!(
        AccountRow,
        r#"
        SELECT id, friendly_name, encrypted_view_private_key, encrypted_spend_public_key, cipher_nonce, unencrypted_view_key_hash
        FROM accounts
        WHERE friendly_name = ?
        "#,
        friendly_name
    )
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

#[derive(sqlx::FromRow, Debug)]
pub struct AccountRow {
    pub id: i64,
    pub friendly_name: String,
    pub encrypted_view_private_key: Vec<u8>,
    pub encrypted_spend_public_key: Vec<u8>,
    pub cipher_nonce: Vec<u8>,
    pub unencrypted_view_key_hash: Option<Vec<u8>>,
}
