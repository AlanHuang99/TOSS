use super::personal_token::{
    self, CreatePersonalAccessTokenCommand, CreatePersonalAccessTokenError,
    RevokePersonalAccessTokenError,
};
use super::{authenticated_principal, PersonalAccessTokenInfo, PersonalAccessTokenScope};
use crate::app_state::AppState;
use crate::audit::record_event;
use crate::http_response::ApiError;
use crate::protocol::ApiErrorCode;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct PersonalAccessTokenListResponse {
    pub tokens: Vec<PersonalAccessTokenInfo>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct CreatePatInput {
    pub label: String,
    pub expires_at: Option<String>,
    pub scopes: Option<Vec<PersonalAccessTokenScope>>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct CreatePatResponse {
    pub id: Uuid,
    pub label: String,
    pub token: String,
    pub token_prefix: String,
    pub scopes: Vec<PersonalAccessTokenScope>,
    pub created_at: DateTime<Utc>,
    #[schema(required)]
    pub expires_at: Option<DateTime<Utc>>,
}

pub(crate) async fn list_personal_access_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<PersonalAccessTokenListResponse>, ApiError> {
    let user_id = authenticated_principal(&state.db, &headers, &jar)
        .await?
        .privileged_user_id()?;
    let tokens = personal_token::list_personal_access_tokens(&state.db, user_id)
        .await
        .map_err(|database_error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
                "Failed to load personal access tokens",
            )
            .with_diagnostic("personal access token lookup failed", database_error)
        })?;
    Ok(Json(PersonalAccessTokenListResponse { tokens }))
}

pub(crate) async fn create_personal_access_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(input): Json<CreatePatInput>,
) -> Result<Json<CreatePatResponse>, ApiError> {
    let user_id = authenticated_principal(&state.db, &headers, &jar)
        .await?
        .privileged_user_id()?;
    let expires_at = if let Some(raw) = input.expires_at.as_deref() {
        let parsed = DateTime::parse_from_rfc3339(raw)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|_| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    ApiErrorCode::BadRequest,
                    "Invalid expiry time format. Use an RFC 3339 timestamp",
                )
            })?;
        Some(parsed)
    } else {
        None
    };
    let response = personal_token::create_personal_access_token(
        &state.db,
        CreatePersonalAccessTokenCommand {
            user_id,
            label: &input.label,
            expires_at,
            scopes: input.scopes.as_deref(),
        },
    )
    .await
    .map_err(|error| match error {
        CreatePersonalAccessTokenError::EmptyLabel => ApiError::new(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::BadRequest,
            "Token label is required",
        ),
        CreatePersonalAccessTokenError::ExpirationNotFuture => ApiError::new(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::BadRequest,
            "Token expiration must be in the future",
        ),
        CreatePersonalAccessTokenError::EmptyScopes => ApiError::new(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::BadRequest,
            "Select at least one token scope",
        ),
        failure @ CreatePersonalAccessTokenError::Persistence { .. } => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::InternalError,
            "Failed to create personal access token",
        )
        .with_diagnostic("personal access token creation failed", failure),
    })?;

    record_event(
        &state.db,
        Some(user_id),
        "security.token.create",
        serde_json::json!({
            "token_id": response.id,
            "label": response.label,
            "scopes": response.scopes
        }),
    )
    .await;

    Ok(Json(CreatePatResponse {
        id: response.id,
        label: response.label,
        token: response.token,
        token_prefix: response.token_prefix,
        scopes: response.scopes,
        created_at: response.created_at,
        expires_at: response.expires_at,
    }))
}

pub(crate) async fn revoke_personal_access_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Path(token_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let user_id = authenticated_principal(&state.db, &headers, &jar)
        .await?
        .privileged_user_id()?;
    personal_token::revoke_personal_access_token(&state.db, user_id, token_id)
        .await
        .map_err(|error| match error {
            RevokePersonalAccessTokenError::NotFound { .. } => ApiError::new(
                StatusCode::NOT_FOUND,
                ApiErrorCode::NotFound,
                "Token not found or already revoked",
            ),
            failure @ RevokePersonalAccessTokenError::Persistence { .. } => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
                "Failed to revoke personal access token",
            )
            .with_diagnostic("personal access token revocation failed", failure),
        })?;
    record_event(
        &state.db,
        Some(user_id),
        "security.token.revoke",
        serde_json::json!({"token_id": token_id}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use crate::server::test_support::{TestApp, TestError, TestResponse};
    use axum::http::{Method, StatusCode};
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    fn refused(response: &TestResponse) -> bool {
        response.status == StatusCode::FORBIDDEN
            && response.code() == Some("auth_personal_access_token_refused")
    }

    async fn create_token(
        app: &TestApp,
        session: &str,
        body: serde_json::Value,
    ) -> Result<TestResponse, TestError> {
        app.send(
            Method::POST,
            "/v1/profile/security/tokens",
            Some(session),
            Some(body),
        )
        .await
    }

    fn token_value(response: &TestResponse) -> Result<String, TestError> {
        Ok(response
            .field("token")
            .as_str()
            .ok_or("token response has no plaintext token")?
            .to_string())
    }

    #[tokio::test]
    async fn token_scopes_are_persisted_and_validated() -> Result<(), TestError> {
        let Some(app) = TestApp::start().await? else {
            return Ok(());
        };
        let user_id = app.insert_user("scopes").await?;
        let session = app.session_for(user_id).await?;

        let default_scope = create_token(&app, &session, json!({"label": "Git"})).await?;
        assert_eq!(default_scope.status, StatusCode::OK);
        assert_eq!(default_scope.field("scopes"), &json!(["git"]));

        let both = create_token(
            &app,
            &session,
            json!({"label": "Both", "scopes": ["api", "git", "api"]}),
        )
        .await?;
        assert_eq!(both.status, StatusCode::OK);
        assert_eq!(both.field("scopes"), &json!(["git", "api"]));

        let empty = create_token(&app, &session, json!({"label": "None", "scopes": []})).await?;
        assert_eq!(empty.status, StatusCode::BAD_REQUEST);
        let unknown = create_token(
            &app,
            &session,
            json!({"label": "Admin", "scopes": ["admin"]}),
        )
        .await?;
        assert!(unknown.status.is_client_error());

        let listed = app
            .send(
                Method::GET,
                "/v1/profile/security/tokens",
                Some(&session),
                None,
            )
            .await?;
        assert_eq!(listed.status, StatusCode::OK);
        let tokens = listed
            .field("tokens")
            .as_array()
            .ok_or("token list is missing")?;
        assert_eq!(tokens.len(), 2);
        assert!(tokens
            .iter()
            .any(|token| token.get("scopes") == Some(&json!(["git", "api"]))));
        Ok(())
    }

    #[tokio::test]
    async fn api_tokens_reach_the_rest_api_but_not_token_management_or_administration(
    ) -> Result<(), TestError> {
        let Some(app) = TestApp::start().await? else {
            return Ok(());
        };
        let user_id = app.insert_user("api-token").await?;
        let mut transaction = app.db.begin().await?;
        crate::access::grant_site_admin_membership(&mut transaction, user_id, Utc::now()).await?;
        transaction.commit().await?;
        let session = app.session_for(user_id).await?;

        let api = create_token(&app, &session, json!({"label": "API", "scopes": ["api"]})).await?;
        let api_token = token_value(&api)?;
        let api_token_id = api
            .field("id")
            .as_str()
            .ok_or("token response has no id")?
            .to_string();
        let git = create_token(&app, &session, json!({"label": "Git"})).await?;
        let git_token = token_value(&git)?;

        let projects = app
            .send(Method::GET, "/v1/projects", Some(&api_token), None)
            .await?;
        assert_eq!(projects.status, StatusCode::OK);
        let me = app
            .send(Method::GET, "/v1/auth/me", Some(&api_token), None)
            .await?;
        assert_eq!(me.field("user_id"), &json!(user_id));
        let git_only = app
            .send(Method::GET, "/v1/projects", Some(&git_token), None)
            .await?;
        assert_eq!(git_only.status, StatusCode::UNAUTHORIZED);

        let organization_id = Uuid::new_v4();
        let administration_path =
            format!("/v1/admin/orgs/{organization_id}/oidc-group-role-mappings");
        let token_path = format!("/v1/profile/security/tokens/{api_token_id}");
        for (method, path, body) in [
            (Method::GET, "/v1/profile/security/tokens", None),
            (
                Method::POST,
                "/v1/profile/security/tokens",
                Some(json!({"label": "Escalation", "scopes": ["api"]})),
            ),
            (Method::DELETE, token_path.as_str(), None),
            (Method::GET, "/v1/admin/settings/auth", None),
            (
                Method::PUT,
                "/v1/admin/settings/auth",
                Some(json!({
                    "allow_local_login": true,
                    "allow_local_registration": true,
                    "allow_oidc": false
                })),
            ),
            (
                Method::POST,
                "/v1/organizations",
                Some(json!({"name": "Token organization"})),
            ),
            (Method::GET, administration_path.as_str(), None),
        ] {
            let response = app
                .send(method.clone(), path, Some(&api_token), body)
                .await?;
            assert!(refused(&response), "{method} {path}: {:?}", response.body);
        }

        let administrator = app
            .send(Method::GET, "/v1/admin/settings/auth", Some(&session), None)
            .await?;
        assert_eq!(administrator.status, StatusCode::OK);

        let revoked = app
            .send(Method::DELETE, &token_path, Some(&session), None)
            .await?;
        assert_eq!(revoked.status, StatusCode::NO_CONTENT);
        let after_revocation = app
            .send(Method::GET, "/v1/projects", Some(&api_token), None)
            .await?;
        assert_eq!(after_revocation.status, StatusCode::UNAUTHORIZED);
        Ok(())
    }
}
