//! Transactional Workspace project deletion.

use super::{assets_persistence, lock_project_content_exclusively, projects_persistence};
use crate::access::{
    ensure_project_role_for_user, lock_project_access_mutation, AccessNeed,
    ProjectAuthorizationError,
};
use crate::object_cleanup::enqueue_object_deletions;
use sqlx::PgPool;
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

pub(super) struct DeleteProjectCommand {
    pub project_id: Uuid,
    pub actor_user_id: Uuid,
}

/// State that outlives the committed row deletion and is removed afterwards.
pub(super) struct DeletedProject {
    pub name: String,
    pub object_keys: Vec<String>,
    pub repository_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum DeleteProjectPersistenceStage {
    Begin,
    LockAccess,
    LockContent,
    LockAssets,
    LoadRepository,
    EnqueueObjects,
    Delete,
    Commit,
}

#[derive(Debug, Error)]
pub(super) enum DeleteProjectError {
    #[error("project was not found")]
    ProjectNotFound,
    #[error(transparent)]
    Authorization(#[from] ProjectAuthorizationError),
    #[error("project deletion failed during {stage:?} for project {project_id}")]
    Persistence {
        stage: DeleteProjectPersistenceStage,
        project_id: Uuid,
        #[source]
        source: sqlx::Error,
    },
}

/// Deletes the project row and everything that cascades from it, and queues
/// its stored objects for deletion in the same transaction. The caller holds
/// the Versioning project lock and removes files after the commit.
pub(super) async fn delete_project(
    db: &PgPool,
    command: DeleteProjectCommand,
) -> Result<DeletedProject, DeleteProjectError> {
    let project_id = command.project_id;
    let persistence_error = |stage, source| DeleteProjectError::Persistence {
        stage,
        project_id,
        source,
    };
    let mut transaction = db
        .begin()
        .await
        .map_err(|source| persistence_error(DeleteProjectPersistenceStage::Begin, source))?;
    lock_project_access_mutation(&mut transaction, project_id)
        .await
        .map_err(|source| persistence_error(DeleteProjectPersistenceStage::LockAccess, source))?;
    // Access cannot change while the mutation lock is held, so this confirms
    // that the caller is still the owner when the deletion commits.
    ensure_project_role_for_user(db, command.actor_user_id, project_id, AccessNeed::Manage).await?;
    lock_project_content_exclusively(&mut transaction, project_id)
        .await
        .map_err(|source| persistence_error(DeleteProjectPersistenceStage::LockContent, source))?;
    let object_keys = assets_persistence::lock_project_object_keys(&mut transaction, project_id)
        .await
        .map_err(|source| persistence_error(DeleteProjectPersistenceStage::LockAssets, source))?;
    let repository_path = crate::versioning::recorded_repository_path(&mut transaction, project_id)
        .await
        .map_err(|source| {
            persistence_error(DeleteProjectPersistenceStage::LoadRepository, source)
        })?;
    enqueue_object_deletions(&mut transaction, &object_keys)
        .await
        .map_err(|source| {
            persistence_error(DeleteProjectPersistenceStage::EnqueueObjects, source)
        })?;
    let Some(name) = projects_persistence::delete(&mut transaction, project_id)
        .await
        .map_err(|source| persistence_error(DeleteProjectPersistenceStage::Delete, source))?
    else {
        return Err(DeleteProjectError::ProjectNotFound);
    };
    transaction
        .commit()
        .await
        .map_err(|source| persistence_error(DeleteProjectPersistenceStage::Commit, source))?;
    Ok(DeletedProject {
        name,
        object_keys,
        repository_path,
    })
}
