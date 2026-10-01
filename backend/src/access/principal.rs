use super::personal_token::{authenticate_personal_access_token, is_personal_access_token};
use super::personal_token_model::PersonalAccessTokenScope;
use super::session_persistence;
use axum::http::{header, HeaderMap};
use axum_extra::extract::cookie::CookieJar;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::env;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub(crate) enum RequestAuthenticationError {
    #[error("authentication is required")]
    Required,
    #[error("personal access tokens cannot perform this operation")]
    PersonalAccessTokenRefused,
    #[error("request principal lookup failed")]
    Store(#[source] sqlx::Error),
}

/// The credential that authenticated a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestCredential {
    Session,
    PersonalAccessToken,
    DevelopmentHeader,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestPrincipal {
    pub user_id: Uuid,
    pub credential: RequestCredential,
}

impl RequestPrincipal {
    /// Returns the user for credential management and site administration,
    /// which a personal access token must not be able to perform.
    pub(crate) fn privileged_user_id(self) -> Result<Uuid, RequestAuthenticationError> {
        if self.credential == RequestCredential::PersonalAccessToken {
            return Err(RequestAuthenticationError::PersonalAccessTokenRefused);
        }
        Ok(self.user_id)
    }
}

fn actor_user_id(headers: &HeaderMap) -> Option<Uuid> {
    let allow_dev_header = env::var("AUTH_DEV_HEADER_ENABLED")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !allow_dev_header {
        return None;
    }
    headers
        .get("x-user-id")
        .and_then(|header| header.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|value| value.trim().to_string())
}

pub(super) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            if key.trim() == name {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

pub(super) fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|header| header.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn session_user_id(
    db: &PgPool,
    token: &str,
) -> Result<Option<Uuid>, RequestAuthenticationError> {
    let token_fingerprint = Sha256::digest(token.as_bytes());
    session_persistence::session_user_id(db, token_fingerprint.as_ref())
        .await
        .map_err(RequestAuthenticationError::Store)
}

/// Bearer credentials carrying the personal-token prefix are validated only as
/// personal access tokens with the `api` scope; other bearer values and the
/// session cookie are looked up as sessions.
pub(crate) async fn request_principal(
    db: &PgPool,
    headers: &HeaderMap,
) -> Result<Option<RequestPrincipal>, RequestAuthenticationError> {
    if let Some(user_id) = actor_user_id(headers) {
        return Ok(Some(RequestPrincipal {
            user_id,
            credential: RequestCredential::DevelopmentHeader,
        }));
    }
    if let Some(token) = bearer_token(headers) {
        if is_personal_access_token(&token) {
            if let Some(user_id) =
                authenticate_personal_access_token(db, &token, PersonalAccessTokenScope::Api)
                    .await
                    .map_err(RequestAuthenticationError::Store)?
            {
                return Ok(Some(RequestPrincipal {
                    user_id,
                    credential: RequestCredential::PersonalAccessToken,
                }));
            }
        } else if let Some(user_id) = session_user_id(db, &token).await? {
            return Ok(Some(RequestPrincipal {
                user_id,
                credential: RequestCredential::Session,
            }));
        }
    }
    if let Some(token) = cookie_value(headers, "typst_session") {
        if let Some(user_id) = session_user_id(db, &token).await? {
            return Ok(Some(RequestPrincipal {
                user_id,
                credential: RequestCredential::Session,
            }));
        }
    }
    Ok(None)
}

pub(crate) async fn request_user_id(
    db: &PgPool,
    headers: &HeaderMap,
) -> Result<Option<Uuid>, RequestAuthenticationError> {
    Ok(request_principal(db, headers)
        .await?
        .map(|principal| principal.user_id))
}

pub(crate) async fn required_request_user_id(
    db: &PgPool,
    headers: &HeaderMap,
) -> Result<Uuid, RequestAuthenticationError> {
    request_user_id(db, headers)
        .await?
        .ok_or(RequestAuthenticationError::Required)
}

pub(crate) async fn authenticated_principal(
    db: &PgPool,
    headers: &HeaderMap,
    jar: &CookieJar,
) -> Result<RequestPrincipal, RequestAuthenticationError> {
    if let Some(principal) = request_principal(db, headers).await? {
        return Ok(principal);
    }
    if let Some(token) = jar
        .get("typst_session")
        .map(|cookie| cookie.value().to_string())
    {
        if let Some(user_id) = session_user_id(db, &token).await? {
            return Ok(RequestPrincipal {
                user_id,
                credential: RequestCredential::Session,
            });
        }
    }
    Err(RequestAuthenticationError::Required)
}

pub(crate) async fn authenticated_user_id(
    db: &PgPool,
    headers: &HeaderMap,
    jar: &CookieJar,
) -> Result<Uuid, RequestAuthenticationError> {
    authenticated_principal(db, headers, jar)
        .await
        .map(|principal| principal.user_id)
}
