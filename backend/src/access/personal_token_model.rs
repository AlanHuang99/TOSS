//! Personal access token read contracts and creation outcomes owned by Access.

use crate::text_enum::text_enum;
use chrono::{DateTime, Utc};
use uuid::Uuid;

text_enum! {
    #[schema(rename_all = "snake_case")]
    pub enum PersonalAccessTokenScope {
        Git => "git",
        Api => "api",
    }
}

impl PersonalAccessTokenScope {
    pub(crate) const ALL: [Self; 2] = [Self::Git, Self::Api];
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct PersonalAccessTokenInfo {
    pub id: Uuid,
    pub label: String,
    pub token_prefix: String,
    pub scopes: Vec<PersonalAccessTokenScope>,
    pub created_at: DateTime<Utc>,
    #[schema(required)]
    pub expires_at: Option<DateTime<Utc>>,
    #[schema(required)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[schema(required)]
    pub revoked_at: Option<DateTime<Utc>>,
}

pub(crate) struct CreatedPersonalAccessToken {
    pub id: Uuid,
    pub label: String,
    pub token: String,
    pub token_prefix: String,
    pub scopes: Vec<PersonalAccessTokenScope>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}
