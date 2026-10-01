//! Router-level fixtures for HTTP tests that need a migrated test database.
//!
//! Tests return early when neither `TEST_DATABASE_URL` nor `DATABASE_URL` is
//! set, matching the persistence tests in each context.

use super::routes::build_router;
use crate::app_state::AppState;
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use chrono::Utc;
use sqlx::PgPool;
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

pub(crate) type TestError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) struct TestApp {
    pub router: Router,
    pub db: PgPool,
    _data_dir: TempDir,
}

pub(crate) struct TestResponse {
    pub status: StatusCode,
    pub body: serde_json::Value,
}

impl TestResponse {
    pub(crate) fn code(&self) -> Option<&str> {
        self.body.get("code").and_then(serde_json::Value::as_str)
    }

    pub(crate) fn field(&self, name: &str) -> &serde_json::Value {
        self.body.get(name).unwrap_or(&serde_json::Value::Null)
    }
}

impl TestApp {
    pub(crate) async fn start() -> Result<Option<Self>, TestError> {
        let database_url =
            std::env::var("TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"));
        let Ok(database_url) = database_url else {
            return Ok(None);
        };
        let db = PgPool::connect(&database_url).await?;
        sqlx::migrate!("./migrations").run(&db).await?;
        let data_dir = tempfile::tempdir()?;
        let state = AppState::for_tests(db.clone(), data_dir.path().to_path_buf())?;
        Ok(Some(Self {
            router: build_router().with_state(state),
            db,
            _data_dir: data_dir,
        }))
    }

    pub(crate) async fn insert_user(&self, label: &str) -> Result<Uuid, TestError> {
        let user_id = Uuid::new_v4();
        let suffix = user_id.simple().to_string();
        sqlx::query(
            "insert into users (id, email, username, display_name, created_at)
             values ($1, $2, $3, $4, $5)",
        )
        .bind(user_id)
        .bind(format!("{label}-{suffix}@example.test"))
        .bind(format!("{label}-{suffix}"))
        .bind(format!("Test {label}"))
        .bind(Utc::now())
        .execute(&self.db)
        .await?;
        Ok(user_id)
    }

    pub(crate) async fn session_for(&self, user_id: Uuid) -> Result<String, TestError> {
        Ok(crate::access::issue_session_for_request(&self.db, &HeaderMap::new(), user_id).await?)
    }

    pub(crate) async fn send(
        &self,
        method: Method,
        uri: &str,
        bearer: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Result<TestResponse, TestError> {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(token) = bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body)?))?,
            None => request.body(Body::empty())?,
        };
        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await?;
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
            })
        };
        Ok(TestResponse { status, body })
    }
}
