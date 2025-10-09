use serde::Serialize;
use sqlx::SqlitePool;
use utoipa::ToSchema;

pub async fn create_account(
    pool: &SqlitePool,
    friendly_name: &str,
    encryptd_view_private_key: &[u8],
    encrypted_spend_public_key: &[u8],
    cipher_nonce: &[u8],
    unencrypted_view_key_hash: &[u8],
    birthday: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO accounts (friendly_name, 
          encrypted_view_private_key, 
          encrypted_spend_public_key, 
          cipher_nonce, 
          unencrypted_view_key_hash,
          birthday)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
        friendly_name,
        encryptd_view_private_key,
        encrypted_spend_public_key,
        cipher_nonce,
        unencrypted_view_key_hash,
        birthday
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_account_by_name(pool: &SqlitePool, friendly_name: &str) -> Result<Option<AccountRow>, sqlx::Error> {
    let row = sqlx::query_as!(
        AccountRow,
        r#"
        SELECT id, 
            friendly_name, 
            encrypted_view_private_key, 
            encrypted_spend_public_key, 
            cipher_nonce, 
            unencrypted_view_key_hash,
            birthday
        FROM accounts
        WHERE friendly_name = ?
        "#,
        friendly_name
    )
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

pub async fn get_accounts(pool: &SqlitePool, friendly_name: Option<&str>) -> Result<Vec<AccountRow>, sqlx::Error> {
    let rows = if let Some(name) = friendly_name {
        sqlx::query_as!(
            AccountRow,
            r#"
            SELECT id, 
              friendly_name, 
              encrypted_view_private_key, 
              encrypted_spend_public_key, 
              cipher_nonce, 
              unencrypted_view_key_hash,
              birthday
            FROM accounts
            WHERE friendly_name = ?
            ORDER BY friendly_name
            "#,
            name
        )
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as!(
            AccountRow,
            r#"
            SELECT id, 
              friendly_name, 
              encrypted_view_private_key, 
              encrypted_spend_public_key, 
              cipher_nonce, 
              unencrypted_view_key_hash,
              birthday
            FROM accounts
            ORDER BY friendly_name
            "#
        )
        .fetch_all(pool)
        .await?
    };

    Ok(rows)
}

#[derive(sqlx::FromRow, Debug)]
pub struct AccountRow {
    pub id: i64,
    pub friendly_name: String,
    pub encrypted_view_private_key: Vec<u8>,
    pub encrypted_spend_public_key: Vec<u8>,
    pub cipher_nonce: Vec<u8>,
    pub unencrypted_view_key_hash: Option<Vec<u8>>,
    pub birthday: i64,
}

#[derive(Debug, Clone, ToSchema, Serialize)]
pub struct AccountBalance {
    pub total_credits: Option<i64>,
    pub total_debits: Option<i64>,
    pub max_height: Option<i64>,
    pub max_date: Option<String>,
}

pub async fn get_balance(pool: &SqlitePool, account_id: i64) -> Result<AccountBalance, sqlx::Error> {
    let agg_result = sqlx::query_as!(
        AccountBalance,
        r#"
            SELECT 
              SUM(balance_credit) as "total_credits: _",
              Sum(balance_debit) as "total_debits: _",
              max(effective_height) as "max_height: _",
              strftime('%Y-%m-%d %H:%M:%S', max(effective_date))  as "max_date: _"
            FROM balance_changes
            WHERE account_id = ?
            "#,
        account_id
    )
    .fetch_one(pool)
    .await?;
    Ok(agg_result)
}
