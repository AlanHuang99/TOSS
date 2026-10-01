//! HTTP transport for the Workspace project catalog and basic lifecycle.

use super::http_error::project_service_unavailable;
use super::project_catalog::{self, ListProjectsError};
use super::project_creation::{self, CreateProject, CreateProjectError};
use super::project_deletion::{self, DeleteProjectCommand, DeleteProjectError};
use super::project_description::{self, ProjectDescription, UpdateProjectDescriptionError};
use super::project_rename::{self, RenameProjectError};
use super::project_thumbnail::remove_project_thumbnail_file;
use super::{LatexEngine, Project, ProjectName, ProjectType};
use crate::access::{ensure_project_role, required_request_user_id, AccessNeed};
use crate::app_state::AppState;
use crate::audit::record_event;
use crate::http_response::ApiError;
use crate::object_cleanup::delete_queued_objects_now;
use crate::protocol::ApiErrorCode;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use tracing::warn;
use uuid::Uuid;

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct ProjectListResponse {
    pub projects: Vec<Project>,
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub(crate) struct CreateProjectInput {
    pub name: String,
    pub project_type: Option<ProjectType>,
    pub latex_engine: Option<LatexEngine>,
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub(crate) struct UpdateProjectNameInput {
    pub name: String,
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub(crate) struct UpdateProjectDescriptionInput {
    #[schema(required)]
    pub description: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct ListProjectsQuery {
    pub include_archived: Option<bool>,
    pub q: Option<String>,
}

pub(crate) async fn list_projects(
    State(state): State<AppState>,
    Query(query): Query<ListProjectsQuery>,
    headers: HeaderMap,
) -> Result<Json<ProjectListResponse>, ApiError> {
    let actor_user_id = required_request_user_id(&state.db, &headers).await?;
    let projects = project_catalog::list_projects(
        &state.db,
        actor_user_id,
        query.include_archived.unwrap_or(true),
        query.q.as_deref(),
    )
    .await?;
    Ok(Json(ProjectListResponse { projects }))
}

pub(crate) async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateProjectInput>,
) -> Result<Json<Project>, ApiError> {
    let actor_user_id = required_request_user_id(&state.db, &headers).await?;
    let name = ProjectName::parse(&input.name)?;
    let project = project_creation::create_project(
        &state.db,
        &state.distribution,
        CreateProject {
            actor_user_id,
            name: &name,
            project_type: input.project_type.unwrap_or(ProjectType::Typst),
            latex_engine: input.latex_engine.unwrap_or(LatexEngine::Xetex),
        },
    )
    .await?;
    record_event(
        &state.db,
        Some(actor_user_id),
        "project.create",
        serde_json::json!({"project_id": project.id, "name": project.name}),
    )
    .await;
    Ok(Json(project))
}

pub(crate) async fn update_project_name(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(input): Json<UpdateProjectNameInput>,
) -> Result<StatusCode, ApiError> {
    let actor_user_id =
        ensure_project_role(&state.db, &headers, project_id, AccessNeed::Manage).await?;
    let name = ProjectName::parse(&input.name)?;
    project_rename::rename_project(&state.db, project_id, &name).await?;
    record_event(
        &state.db,
        Some(actor_user_id),
        "project.rename",
        serde_json::json!({
            "project_id": project_id,
            "name": name.as_str()
        }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn update_project_description(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(input): Json<UpdateProjectDescriptionInput>,
) -> Result<StatusCode, ApiError> {
    let actor_user_id =
        ensure_project_role(&state.db, &headers, project_id, AccessNeed::Manage).await?;
    let description = ProjectDescription::parse(input.description.as_deref())?;
    project_description::update_project_description(&state.db, project_id, &description).await?;
    record_event(
        &state.db,
        Some(actor_user_id),
        "project.description.update",
        serde_json::json!({
            "project_id": project_id,
            "description": description.as_deref()
        }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes a project for every member. A linked external repository is left
/// unchanged on its provider.
pub(crate) async fn delete_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let actor_user_id =
        ensure_project_role(&state.db, &headers, project_id, AccessNeed::Manage).await?;
    let git_lock = state.versioning.acquire_project_lock(project_id).await;
    let deleted = project_deletion::delete_project(
        &state.db,
        DeleteProjectCommand {
            project_id,
            actor_user_id,
        },
    )
    .await?;
    if let Err(error) =
        crate::versioning::remove_project_repository(project_id, deleted.repository_path.as_deref())
            .await
    {
        warn!(%error, %project_id, "deleted project repository could not be removed");
    }
    drop(git_lock);
    delete_queued_objects_now(&state.db, state.storage.as_ref(), &deleted.object_keys).await;
    if let Err(error) = remove_project_thumbnail_file(&state.data_dir, project_id).await {
        warn!(%error, %project_id, "deleted project thumbnail could not be removed");
    }
    state.collaboration.access_changed(project_id).await;
    record_event(
        &state.db,
        Some(actor_user_id),
        "project.delete",
        serde_json::json!({
            "project_id": project_id,
            "name": deleted.name
        }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

impl From<ListProjectsError> for ApiError {
    fn from(source: ListProjectsError) -> Self {
        project_service_unavailable(source)
    }
}

impl From<CreateProjectError> for ApiError {
    fn from(source: CreateProjectError) -> Self {
        match source {
            CreateProjectError::ProjectTypeDisabled { .. } => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                ApiErrorCode::ProjectTypeDisabled,
                "This project type is disabled in the current deployment",
            ),
            failure @ (CreateProjectError::StarterContentMissing { .. }
            | CreateProjectError::Identity(_)
            | CreateProjectError::Persistence(_)) => project_service_unavailable(failure),
        }
    }
}

impl From<RenameProjectError> for ApiError {
    fn from(source: RenameProjectError) -> Self {
        match source {
            RenameProjectError::ProjectNotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                ApiErrorCode::ProjectNotFound,
                "Project was not found",
            ),
            failure @ RenameProjectError::Persistence(_) => project_service_unavailable(failure),
        }
    }
}

impl From<UpdateProjectDescriptionError> for ApiError {
    fn from(source: UpdateProjectDescriptionError) -> Self {
        match source {
            UpdateProjectDescriptionError::ProjectNotFound => project_not_found(),
            failure @ UpdateProjectDescriptionError::Persistence(_) => {
                project_service_unavailable(failure)
            }
        }
    }
}

impl From<DeleteProjectError> for ApiError {
    fn from(source: DeleteProjectError) -> Self {
        match source {
            DeleteProjectError::ProjectNotFound => project_not_found(),
            DeleteProjectError::Authorization(source) => source.into(),
            failure @ DeleteProjectError::Persistence { .. } => {
                project_service_unavailable(failure)
            }
        }
    }
}

fn project_not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ApiErrorCode::ProjectNotFound,
        "Project was not found",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::{TestApp, TestError};
    use axum::http::Method;
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn invalid_project_names_have_a_semantic_bad_request_response() {
        let error = ApiError::from(super::super::InvalidProjectName);

        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(error.code(), ApiErrorCode::ProjectNameInvalid);
    }

    #[test]
    fn invalid_project_descriptions_have_a_semantic_bad_request_response() {
        let error = ApiError::from(super::super::InvalidProjectDescription);

        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(error.code(), ApiErrorCode::ProjectDescriptionInvalid);
    }

    async fn listed_project(
        app: &TestApp,
        session: &str,
        project_id: Uuid,
    ) -> Result<Option<serde_json::Value>, TestError> {
        let listed = app
            .send(Method::GET, "/v1/projects", Some(session), None)
            .await?;
        Ok(listed
            .field("projects")
            .as_array()
            .ok_or("project list is missing")?
            .iter()
            .find(|project| project.get("id") == Some(&json!(project_id)))
            .cloned())
    }

    #[tokio::test]
    async fn project_descriptions_are_owner_managed_and_listed() -> Result<(), TestError> {
        let Some(app) = TestApp::start().await? else {
            return Ok(());
        };
        let owner = app.insert_user("description-owner").await?;
        let editor = app.insert_user("description-editor").await?;
        let owner_session = app.session_for(owner).await?;
        let editor_session = app.session_for(editor).await?;
        let project_id = app.create_project(&owner_session, "Described").await?;
        app.grant_project_role(project_id, editor, "ReadWrite")
            .await?;
        let path = format!("/v1/projects/{project_id}/description");

        let created = listed_project(&app, &owner_session, project_id)
            .await?
            .ok_or("created project is not listed")?;
        assert_eq!(created.get("description"), Some(&serde_json::Value::Null));

        let updated = app
            .send(
                Method::PATCH,
                &path,
                Some(&owner_session),
                Some(json!({"description": "  Course notes\nWeek 1  "})),
            )
            .await?;
        assert_eq!(updated.status, StatusCode::NO_CONTENT);
        let listed = listed_project(&app, &editor_session, project_id)
            .await?
            .ok_or("shared project is not listed")?;
        assert_eq!(
            listed.get("description"),
            Some(&json!("Course notes\nWeek 1"))
        );

        let forbidden = app
            .send(
                Method::PATCH,
                &path,
                Some(&editor_session),
                Some(json!({"description": "Editor text"})),
            )
            .await?;
        assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
        let oversized = app
            .send(
                Method::PATCH,
                &path,
                Some(&owner_session),
                Some(json!({"description": "x".repeat(2001)})),
            )
            .await?;
        assert_eq!(oversized.status, StatusCode::BAD_REQUEST);
        assert_eq!(oversized.code(), Some("project_description_invalid"));

        let cleared = app
            .send(
                Method::PATCH,
                &path,
                Some(&owner_session),
                Some(json!({"description": "   "})),
            )
            .await?;
        assert_eq!(cleared.status, StatusCode::NO_CONTENT);
        let stored: Option<String> =
            sqlx::query_scalar("select description from projects where id = $1")
                .bind(project_id)
                .fetch_one(&app.db)
                .await?;
        assert_eq!(stored, None);
        Ok(())
    }

    #[tokio::test]
    async fn owners_delete_projects_with_their_rows_files_and_stored_objects(
    ) -> Result<(), TestError> {
        let Some(app) = TestApp::start().await? else {
            return Ok(());
        };
        let owner = app.insert_user("delete-owner").await?;
        let editor = app.insert_user("delete-editor").await?;
        let owner_session = app.session_for(owner).await?;
        let editor_session = app.session_for(editor).await?;
        let project_id = app.create_project(&owner_session, "Disposable").await?;
        app.grant_project_role(project_id, editor, "ReadWrite")
            .await?;

        let asset_id = Uuid::new_v4();
        let object_key = format!("projects/{project_id}/assets/{asset_id}");
        sqlx::query(
            "insert into project_assets (
                 id, project_id, path, content_revision, object_key, content_type,
                 size_bytes, uploaded_by, created_at, inline_data
             ) values ($1, $2, 'figure.png', $1, $3, 'image/png', 4, $4, $5, null)",
        )
        .bind(asset_id)
        .bind(project_id)
        .bind(&object_key)
        .bind(owner)
        .bind(Utc::now())
        .execute(&app.db)
        .await?;
        let repository = app.data_dir().join("git").join(project_id.to_string());
        tokio::fs::create_dir_all(repository.join(".git")).await?;
        tokio::fs::write(repository.join("main.typ"), "= Disposable").await?;
        sqlx::query("update git_repositories set local_path = $2 where project_id = $1")
            .bind(project_id)
            .bind(repository.to_string_lossy().to_string())
            .execute(&app.db)
            .await?;
        let thumbnail = app
            .data_dir()
            .join("thumbnails")
            .join(format!("{project_id}.thumb"));
        tokio::fs::create_dir_all(app.data_dir().join("thumbnails")).await?;
        tokio::fs::write(&thumbnail, b"webp").await?;
        sqlx::query(
            "insert into project_thumbnails (project_id, content_type, updated_by, updated_at)
             values ($1, 'image/webp', $2, $3)",
        )
        .bind(project_id)
        .bind(owner)
        .bind(Utc::now())
        .execute(&app.db)
        .await?;

        let path = format!("/v1/projects/{project_id}");
        let forbidden = app
            .send(Method::DELETE, &path, Some(&editor_session), None)
            .await?;
        assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
        assert!(repository.exists());

        let deleted = app
            .send(Method::DELETE, &path, Some(&owner_session), None)
            .await?;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);

        let remaining = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
            "select
               (select count(*) from projects where id = $1),
               (select count(*) from documents where project_id = $1),
               (select count(*) from project_assets where project_id = $1),
               (select count(*) from project_roles where project_id = $1),
               (select count(*) from git_repositories where project_id = $1),
               (select count(*) from project_thumbnails where project_id = $1)",
        )
        .bind(project_id)
        .fetch_one(&app.db)
        .await?;
        assert_eq!(remaining, (0, 0, 0, 0, 0, 0));
        assert!(!repository.exists());
        assert!(!thumbnail.exists());
        let queued: bool = sqlx::query_scalar(
            "select exists(select 1 from object_deletion_queue where object_key = $1)",
        )
        .bind(&object_key)
        .fetch_one(&app.db)
        .await?;
        assert!(queued);
        let audited: bool = sqlx::query_scalar(
            "select exists(
                 select 1 from audit_events
                 where event_type = 'project.delete'
                   and actor_user_id = $1
                   and payload->>'project_id' = $2
             )",
        )
        .bind(owner)
        .bind(project_id.to_string())
        .fetch_one(&app.db)
        .await?;
        assert!(audited);

        let listed = listed_project(&app, &owner_session, project_id).await?;
        assert!(listed.is_none());
        Ok(())
    }
}
