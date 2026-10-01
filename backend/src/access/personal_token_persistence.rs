//! Personal access token persistence.

use super::personal_token_model::{PersonalAccessTokenInfo, PersonalAccessTokenScope};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub(crate) struct InsertPersonalAccessTokenRecord<'a> {
    pub id: Uuid,
    pub user_id: Uuid,
    pub label: &'a str,
    pub token_prefix: &'a str,
    pub token_fingerprint: &'a [u8],
    pub scopes: &'a [PersonalAccessTokenScope],
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub(crate) struct ActivePersonalAccessToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub scopes: Vec<PersonalAccessTokenScope>,
    pub last_used_at: Option<DateTime<Utc>>,
}

fn decode_scopes(values: Vec<String>) -> Result<Vec<PersonalAccessTokenScope>, sqlx::Error> {
    values
        .into_iter()
        .map(|value| {
            value.parse().map_err(|()| {
                sqlx::Error::Decode(format!("unknown personal access token scope {value}").into())
            })
        })
        .collect()
}

fn encode_scopes(scopes: &[PersonalAccessTokenScope]) -> Vec<String> {
    scopes.iter().map(ToString::to_string).collect()
}

pub(crate) async fn list(
    db: &PgPool,
    user_id: Uuid,
) -> Result<Vec<PersonalAccessTokenInfo>, sqlx::Error> {
    let rows = sqlx::query(
        "select id, label, token_prefix, scopes, created_at, expires_at, last_used_at, revoked_at
         from personal_access_tokens
         where user_id = $1
         order by created_at desc",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(PersonalAccessTokenInfo {
                id: row.try_get("id")?,
                label: row.try_get("label")?,
                token_prefix: row.try_get("token_prefix")?,
                scopes: decode_scopes(row.try_get("scopes")?)?,
                created_at: row.try_get("created_at")?,
                expires_at: row.try_get("expires_at")?,
                last_used_at: row.try_get("last_used_at")?,
                revoked_at: row.try_get("revoked_at")?,
            })
        })
        .collect()
}

pub(crate) async fn insert(
    db: &PgPool,
    record: InsertPersonalAccessTokenRecord<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into personal_access_tokens (id, user_id, label, token_prefix, token_fingerprint, scopes, created_at, expires_at, last_used_at, revoked_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, null, null)",
    )
    .bind(record.id)
    .bind(record.user_id)
    .bind(record.label)
    .bind(record.token_prefix)
    .bind(record.token_fingerprint)
    .bind(encode_scopes(record.scopes))
    .bind(record.created_at)
    .bind(record.expires_at)
    .execute(db)
    .await?;
    Ok(())
}

pub(crate) async fn revoke(
    db: &PgPool,
    token_id: Uuid,
    user_id: Uuid,
    revoked_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "update personal_access_tokens
         set revoked_at = $3
         where id = $1 and user_id = $2 and revoked_at is null",
    )
    .bind(token_id)
    .bind(user_id)
    .bind(revoked_at)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn find_active(
    db: &PgPool,
    token_fingerprint: &[u8],
) -> Result<Option<ActivePersonalAccessToken>, sqlx::Error> {
    let row = sqlx::query(
        "select id, user_id, scopes, last_used_at
         from personal_access_tokens
         where token_fingerprint = $1
           and revoked_at is null
           and (expires_at is null or expires_at > now())",
    )
    .bind(token_fingerprint)
    .fetch_optional(db)
    .await?;
    row.map(|row| {
        Ok(ActivePersonalAccessToken {
            id: row.try_get("id")?,
            user_id: row.try_get("user_id")?,
            scopes: decode_scopes(row.try_get("scopes")?)?,
            last_used_at: row.try_get("last_used_at")?,
        })
    })
    .transpose()
}

pub(crate) async fn record_use(
    db: &PgPool,
    token_id: Uuid,
    used_at: DateTime<Utc>,
    stale_before: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update personal_access_tokens
         set last_used_at = $2
         where id = $1
           and (last_used_at is null or last_used_at < $3)",
    )
    .bind(token_id)
    .bind(used_at)
    .bind(stale_before)
    .execute(db)
    .await?;
    Ok(())
}
