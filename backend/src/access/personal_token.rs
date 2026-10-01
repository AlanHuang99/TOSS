//! Personal access token lifecycle and authentication.

use super::personal_token_model::{
    CreatedPersonalAccessToken, PersonalAccessTokenInfo, PersonalAccessTokenScope,
};
use super::personal_token_persistence;
use chrono::{DateTime, Utc};
use rand::distr::{Alphanumeric, SampleString};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use thiserror::Error;
use tracing::warn;
use uuid::Uuid;

const TOKEN_PREFIX: &str = "tpat_";
/// Successful uses inside this window do not rewrite `last_used_at`, so
/// frequent API and Git requests do not each cause a database write.
const LAST_USED_RESOLUTION_SECONDS: i64 = 60;

/// Session tokens are alphanumeric, so the underscore in the prefix separates
/// the two bearer credential kinds without a database lookup.
pub(crate) fn is_personal_access_token(token: &str) -> bool {
    token.starts_with(TOKEN_PREFIX)
}

pub(crate) async fn list_personal_access_tokens(
    db: &PgPool,
    user_id: Uuid,
) -> Result<Vec<PersonalAccessTokenInfo>, sqlx::Error> {
    personal_token_persistence::list(db, user_id).await
}

#[derive(Debug, Error)]
pub(crate) enum CreatePersonalAccessTokenError {
    #[error("personal access token label is empty")]
    EmptyLabel,
    #[error("personal access token expiration must be in the future")]
    ExpirationNotFuture,
    #[error("personal access token requires at least one scope")]
    EmptyScopes,
    #[error("personal access token {token_id} could not be persisted for user {user_id}")]
    Persistence {
        token_id: Uuid,
        user_id: Uuid,
        #[source]
        source: sqlx::Error,
    },
}

pub(crate) struct CreatePersonalAccessTokenCommand<'value> {
    pub user_id: Uuid,
    pub label: &'value str,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Option<&'value [PersonalAccessTokenScope]>,
}

pub(crate) async fn create_personal_access_token(
    db: &PgPool,
    command: CreatePersonalAccessTokenCommand<'_>,
) -> Result<CreatedPersonalAccessToken, CreatePersonalAccessTokenError> {
    let created_at = Utc::now();
    let user_id = command.user_id;
    let label = validate_creation(command.label, command.expires_at, created_at)?;
    let scopes = normalize_scopes(command.scopes)?;
    let id = Uuid::new_v4();
    let token = format!(
        "{TOKEN_PREFIX}{}",
        Alphanumeric.sample_string(&mut rand::rng(), 40)
    );
    let token_prefix = token.chars().take(12).collect::<String>();
    let token_fingerprint = Sha256::digest(token.as_bytes());
    personal_token_persistence::insert(
        db,
        personal_token_persistence::InsertPersonalAccessTokenRecord {
            id,
            user_id,
            label,
            token_prefix: &token_prefix,
            token_fingerprint: token_fingerprint.as_ref(),
            scopes: &scopes,
            created_at,
            expires_at: command.expires_at,
        },
    )
    .await
    .map_err(|source| CreatePersonalAccessTokenError::Persistence {
        token_id: id,
        user_id,
        source,
    })?;
    Ok(CreatedPersonalAccessToken {
        id,
        label: label.to_string(),
        token,
        token_prefix,
        scopes,
        created_at,
        expires_at: command.expires_at,
    })
}

fn validate_creation(
    label: &str,
    expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
) -> Result<&str, CreatePersonalAccessTokenError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(CreatePersonalAccessTokenError::EmptyLabel);
    }
    if expires_at.is_some_and(|value| value <= created_at) {
        return Err(CreatePersonalAccessTokenError::ExpirationNotFuture);
    }
    Ok(label)
}

/// Omitted scopes keep the Git-only behavior of tokens created before scopes
/// existed. Requested scopes are deduplicated into a canonical order.
fn normalize_scopes(
    requested: Option<&[PersonalAccessTokenScope]>,
) -> Result<Vec<PersonalAccessTokenScope>, CreatePersonalAccessTokenError> {
    let requested = requested.unwrap_or(&[PersonalAccessTokenScope::Git]);
    let scopes = PersonalAccessTokenScope::ALL
        .into_iter()
        .filter(|scope| requested.contains(scope))
        .collect::<Vec<_>>();
    if scopes.is_empty() {
        return Err(CreatePersonalAccessTokenError::EmptyScopes);
    }
    Ok(scopes)
}

#[derive(Debug, Error)]
pub(crate) enum RevokePersonalAccessTokenError {
    #[error("personal access token {token_id} was not found for user {user_id}")]
    NotFound { token_id: Uuid, user_id: Uuid },
    #[error("personal access token {token_id} could not be revoked for user {user_id}")]
    Persistence {
        token_id: Uuid,
        user_id: Uuid,
        #[source]
        source: sqlx::Error,
    },
}

pub(crate) async fn revoke_personal_access_token(
    db: &PgPool,
    user_id: Uuid,
    token_id: Uuid,
) -> Result<(), RevokePersonalAccessTokenError> {
    let revoked = personal_token_persistence::revoke(db, token_id, user_id, Utc::now())
        .await
        .map_err(|source| RevokePersonalAccessTokenError::Persistence {
            token_id,
            user_id,
            source,
        })?;
    if revoked {
        Ok(())
    } else {
        Err(RevokePersonalAccessTokenError::NotFound { token_id, user_id })
    }
}

/// Validates expiry and revocation on every call and returns the token owner
/// only when the token carries `required_scope`.
pub(crate) async fn authenticate_personal_access_token(
    db: &PgPool,
    token: &str,
    required_scope: PersonalAccessTokenScope,
) -> Result<Option<Uuid>, sqlx::Error> {
    let token_fingerprint = Sha256::digest(token.as_bytes());
    let Some(active) =
        personal_token_persistence::find_active(db, token_fingerprint.as_ref()).await?
    else {
        return Ok(None);
    };
    if !active.scopes.contains(&required_scope) {
        return Ok(None);
    }
    let now = Utc::now();
    let stale_before = now - chrono::Duration::seconds(LAST_USED_RESOLUTION_SECONDS);
    if active
        .last_used_at
        .is_none_or(|last_used_at| last_used_at < stale_before)
    {
        if let Err(database_error) =
            personal_token_persistence::record_use(db, active.id, now, stale_before).await
        {
            warn!(%database_error, token_id = %active.id, "personal access token last-used update failed");
        }
    }
    Ok(Some(active.user_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn token_creation_trims_labels_and_requires_future_expiration() {
        let created_at = Utc::now();
        assert_eq!(
            validate_creation(
                "  workstation  ",
                Some(created_at + Duration::hours(1)),
                created_at,
            )
            .ok(),
            Some("workstation")
        );
        assert!(matches!(
            validate_creation("  ", None, created_at),
            Err(CreatePersonalAccessTokenError::EmptyLabel)
        ));
        assert!(matches!(
            validate_creation("workstation", Some(created_at), created_at),
            Err(CreatePersonalAccessTokenError::ExpirationNotFuture)
        ));
    }

    #[test]
    fn token_scopes_default_to_git_and_are_canonical() {
        use PersonalAccessTokenScope::{Api, Git};
        assert_eq!(normalize_scopes(None).ok(), Some(vec![Git]));
        assert_eq!(normalize_scopes(Some(&[Api])).ok(), Some(vec![Api]));
        assert_eq!(
            normalize_scopes(Some(&[Api, Git, Api])).ok(),
            Some(vec![Git, Api])
        );
        assert!(matches!(
            normalize_scopes(Some(&[])),
            Err(CreatePersonalAccessTokenError::EmptyScopes)
        ));
    }

    #[test]
    fn only_prefixed_tokens_are_personal_access_tokens() {
        assert!(is_personal_access_token("tpat_abc"));
        assert!(!is_personal_access_token(
            "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKL"
        ));
    }

    async fn migrated_test_pool() -> Result<Option<PgPool>, Box<dyn std::error::Error + Send + Sync>>
    {
        let database_url =
            std::env::var("TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"));
        let Ok(database_url) = database_url else {
            return Ok(None);
        };
        let pool = PgPool::connect(&database_url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Some(pool))
    }

    async fn insert_user(pool: &PgPool) -> Result<Uuid, sqlx::Error> {
        let user_id = Uuid::new_v4();
        let suffix = user_id.simple().to_string();
        sqlx::query(
            "insert into users (id, email, username, display_name, created_at)
             values ($1, $2, $3, 'Token owner', $4)",
        )
        .bind(user_id)
        .bind(format!("token-{suffix}@example.test"))
        .bind(format!("token-{suffix}"))
        .bind(Utc::now())
        .execute(pool)
        .await?;
        Ok(user_id)
    }

    #[tokio::test]
    async fn authentication_requires_the_scope_and_an_active_token(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use PersonalAccessTokenScope::{Api, Git};
        let Some(pool) = migrated_test_pool().await? else {
            return Ok(());
        };
        let user_id = insert_user(&pool).await?;
        let git_token = create_personal_access_token(
            &pool,
            CreatePersonalAccessTokenCommand {
                user_id,
                label: "Git",
                expires_at: None,
                scopes: None,
            },
        )
        .await?;
        let api_token = create_personal_access_token(
            &pool,
            CreatePersonalAccessTokenCommand {
                user_id,
                label: "API",
                expires_at: Some(Utc::now() + Duration::hours(1)),
                scopes: Some(&[Api]),
            },
        )
        .await?;
        assert_eq!(git_token.scopes, vec![Git]);
        assert_eq!(api_token.scopes, vec![Api]);

        assert_eq!(
            authenticate_personal_access_token(&pool, &git_token.token, Git).await?,
            Some(user_id)
        );
        assert_eq!(
            authenticate_personal_access_token(&pool, &git_token.token, Api).await?,
            None
        );
        assert_eq!(
            authenticate_personal_access_token(&pool, &api_token.token, Api).await?,
            Some(user_id)
        );
        assert_eq!(
            authenticate_personal_access_token(&pool, &api_token.token, Git).await?,
            None
        );

        sqlx::query("update personal_access_tokens set expires_at = $2 where id = $1")
            .bind(api_token.id)
            .bind(Utc::now() - Duration::minutes(1))
            .execute(&pool)
            .await?;
        assert_eq!(
            authenticate_personal_access_token(&pool, &api_token.token, Api).await?,
            None
        );

        revoke_personal_access_token(&pool, user_id, git_token.id).await?;
        assert_eq!(
            authenticate_personal_access_token(&pool, &git_token.token, Git).await?,
            None
        );

        let listed = list_personal_access_tokens(&pool, user_id).await?;
        assert_eq!(listed.len(), 2);
        assert!(listed
            .iter()
            .any(|token| token.id == api_token.id && token.scopes == vec![Api]));
        Ok(())
    }

    #[tokio::test]
    async fn last_use_is_recorded_at_most_once_per_minute(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let Some(pool) = migrated_test_pool().await? else {
            return Ok(());
        };
        let user_id = insert_user(&pool).await?;
        let created = create_personal_access_token(
            &pool,
            CreatePersonalAccessTokenCommand {
                user_id,
                label: "API",
                expires_at: None,
                scopes: Some(&[PersonalAccessTokenScope::Api]),
            },
        )
        .await?;
        let last_used_at = |pool: PgPool| async move {
            sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
                "select last_used_at from personal_access_tokens where id = $1",
            )
            .bind(created.id)
            .fetch_one(&pool)
            .await
        };

        authenticate_personal_access_token(&pool, &created.token, PersonalAccessTokenScope::Api)
            .await?;
        let first_use = last_used_at(pool.clone())
            .await?
            .ok_or("first use was not recorded")?;

        let recent = Utc::now() - Duration::seconds(20);
        sqlx::query("update personal_access_tokens set last_used_at = $2 where id = $1")
            .bind(created.id)
            .bind(recent)
            .execute(&pool)
            .await?;
        authenticate_personal_access_token(&pool, &created.token, PersonalAccessTokenScope::Api)
            .await?;
        let unchanged = last_used_at(pool.clone())
            .await?
            .ok_or("last use vanished")?;
        assert_eq!(unchanged.timestamp_micros(), recent.timestamp_micros());

        let stale = Utc::now() - Duration::minutes(5);
        sqlx::query("update personal_access_tokens set last_used_at = $2 where id = $1")
            .bind(created.id)
            .bind(stale)
            .execute(&pool)
            .await?;
        authenticate_personal_access_token(&pool, &created.token, PersonalAccessTokenScope::Api)
            .await?;
        let refreshed = last_used_at(pool.clone())
            .await?
            .ok_or("last use vanished")?;
        assert!(refreshed > stale + Duration::minutes(4));
        assert!(refreshed >= first_use);
        Ok(())
    }
}
