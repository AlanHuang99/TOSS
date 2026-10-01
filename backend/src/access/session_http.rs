use super::authenticated_user_id;
use super::display_name::DisplayName;
use super::session::{self, AuthenticatedUser, IssueSessionCommand, IssueSessionError};
use crate::app_state::AppState;
use crate::audit::record_event;
use crate::http_response::ApiError;
use crate::protocol::ApiErrorCode;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::Utc;
use sqlx::PgPool;
use std::env;
use uuid::Uuid;

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct SessionResponse {
    pub session_token: String,
    pub user_id: Uuid,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct AuthMeResponse {
    pub user_id: Uuid,
    pub email: String,
    pub username: String,
    pub display_name: String,
    pub session_expires_at: chrono::DateTime<Utc>,
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub(crate) struct UpdateAuthMeInput {
    pub display_name: String,
}

pub(crate) async fn auth_me(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<AuthMeResponse>, ApiError> {
    let user_id = authenticated_user_id(&state.db, &headers, &jar).await?;
    let user = match session::authenticated_user(&state.db, user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => return Err(authenticated_user_missing()),
        Err(database_error) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
                "Failed to load the authenticated user",
            )
            .with_diagnostic("authenticated user lookup failed", database_error));
        }
    };
    Ok(Json(auth_me_response(user)))
}

pub(crate) async fn update_auth_me(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(input): Json<UpdateAuthMeInput>,
) -> Result<Json<AuthMeResponse>, ApiError> {
    let user_id = authenticated_user_id(&state.db, &headers, &jar).await?;
    let display_name = DisplayName::parse(&input.display_name)?;
    let user = match session::update_display_name(&state.db, user_id, &display_name).await {
        Ok(Some(user)) => user,
        Ok(None) => return Err(authenticated_user_missing()),
        Err(database_error) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
                "Failed to update the display name",
            )
            .with_diagnostic("display name update failed", database_error));
        }
    };
    record_event(
        &state.db,
        Some(user_id),
        "profile.display_name.update",
        serde_json::json!({"display_name": user.display_name}),
    )
    .await;
    Ok(Json(auth_me_response(user)))
}

fn auth_me_response(user: AuthenticatedUser) -> AuthMeResponse {
    AuthMeResponse {
        user_id: user.id,
        email: user.email,
        username: user.username,
        display_name: user.display_name,
        session_expires_at: Utc::now() + chrono::Duration::hours(12),
    }
}

fn authenticated_user_missing() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        ApiErrorCode::AuthRequired,
        "Authentication required",
    )
}

pub(crate) async fn auth_logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> axum::response::Response {
    if let Some(token) = jar
        .get("typst_session")
        .map(|cookie| cookie.value().to_string())
    {
        if let Err(database_error) = session::revoke_session(&state.db, &token).await {
            tracing::error!(%database_error, "session revocation failed");
        }
    }
    let jar = jar.remove(Cookie::from("typst_session"));
    (jar, StatusCode::NO_CONTENT).into_response()
}

pub(crate) async fn issue_session_response(
    db: &PgPool,
    headers: &HeaderMap,
    user_id: Uuid,
) -> Result<axum::response::Response, IssueSessionError> {
    let token = issue_session_for_request(db, headers, user_id).await?;
    let session_cookie = session_cookie(token.clone());
    let mut jar = CookieJar::new();
    jar = jar.add(session_cookie);
    Ok((
        jar,
        Json(SessionResponse {
            session_token: token,
            user_id,
        }),
    )
        .into_response())
}

pub(crate) async fn issue_session_for_request(
    db: &PgPool,
    headers: &HeaderMap,
    user_id: Uuid,
) -> Result<String, IssueSessionError> {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|header| header.to_str().ok())
        .unwrap_or("unknown");
    let ip_address = headers
        .get("x-forwarded-for")
        .and_then(|header| header.to_str().ok())
        .unwrap_or("unknown");
    session::issue_session(
        db,
        IssueSessionCommand {
            user_id,
            user_agent,
            ip_address,
        },
    )
    .await
}

pub(crate) fn session_cookie(token: String) -> Cookie<'static> {
    Cookie::build(("typst_session", token))
        .path("/")
        .http_only(true)
        .secure(auth_cookie_secure())
        .same_site(SameSite::Lax)
        .build()
}

pub(crate) fn auth_cookie_secure() -> bool {
    env::var("COOKIE_SECURE")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use crate::server::test_support::{TestApp, TestError};
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    #[tokio::test]
    async fn display_name_updates_are_validated_and_return_the_current_user(
    ) -> Result<(), TestError> {
        let Some(app) = TestApp::start().await? else {
            return Ok(());
        };
        let user_id = app.insert_user("profile").await?;
        let session = app.session_for(user_id).await?;

        let updated = app
            .send(
                Method::PATCH,
                "/v1/auth/me",
                Some(&session),
                Some(json!({"display_name": "  Ada Lovelace  "})),
            )
            .await?;
        assert_eq!(updated.status, StatusCode::OK);
        assert_eq!(updated.field("display_name"), "Ada Lovelace");
        assert_eq!(updated.field("user_id"), &user_id.to_string());

        let current = app
            .send(Method::GET, "/v1/auth/me", Some(&session), None)
            .await?;
        assert_eq!(current.status, StatusCode::OK);
        assert_eq!(current.field("display_name"), "Ada Lovelace");
        assert_eq!(current.field("email"), updated.field("email"));

        for invalid in [" ", "Ada\nLovelace", &"a".repeat(65)] {
            let rejected = app
                .send(
                    Method::PATCH,
                    "/v1/auth/me",
                    Some(&session),
                    Some(json!({"display_name": invalid})),
                )
                .await?;
            assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
            assert_eq!(rejected.code(), Some("auth_display_name_invalid"));
        }

        let anonymous = app
            .send(
                Method::PATCH,
                "/v1/auth/me",
                None,
                Some(json!({"display_name": "Anonymous"})),
            )
            .await?;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
        Ok(())
    }
}
