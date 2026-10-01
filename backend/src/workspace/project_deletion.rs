//! Transactional Workspace project deletion.

use super::{assets_persistence, lock_project_content_exclusively, projects_persistence};
use crate::access::{ensure_project_role_for_user, AccessNeed, ProjectAuthorizationError};
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
    LockProject,
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
    // The exclusive project-row lock waits for admitted collaboration writes,
    // content mutations, and access-epoch changes, which all lock this row,
    // and holds off new ones until the deletion commits. The access mutation
    // advisory lock is not taken: collaboration writes acquire it after the
    // row lock, so holding it here first could deadlock.
    lock_project_content_exclusively(&mut transaction, project_id)
        .await
        .map_err(|source| persistence_error(DeleteProjectPersistenceStage::LockProject, source))?;
    // Access changes advance the epoch on the locked row, so this confirms
    // that the caller is still the owner when the deletion commits.
    ensure_project_role_for_user(db, command.actor_user_id, project_id, AccessNeed::Manage).await?;
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

#[cfg(test)]
mod tests {
    use super::{delete_project, DeleteProjectCommand};
    use crate::access::{lock_project_access_epoch, ProjectAccessEpochMatch};
    use chrono::Utc;
    use sqlx::PgPool;
    use std::time::Duration;
    use uuid::Uuid;

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

    #[tokio::test]
    async fn deletion_waits_for_an_admitted_collaboration_write_without_deadlock(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let Some(pool) = migrated_test_pool().await? else {
            return Ok(());
        };
        let owner = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let now = Utc::now();
        sqlx::query(
            "insert into users (id, email, username, display_name, created_at)
             values ($1, $2, $3, 'Deletion owner', $4)",
        )
        .bind(owner)
        .bind(format!("{owner}@example.test"))
        .bind(format!("deletion-{}", owner.simple()))
        .bind(now)
        .execute(&pool)
        .await?;
        sqlx::query(
            "insert into projects (id, owner_user_id, name, created_at, project_type)
             values ($1, $2, 'Lock order', $3, 'typst')",
        )
        .bind(project_id)
        .bind(owner)
        .bind(now)
        .execute(&pool)
        .await?;
        sqlx::query(
            "insert into project_roles (project_id, user_id, role, granted_at)
             values ($1, $2, 'Owner', $3)",
        )
        .bind(project_id)
        .bind(owner)
        .bind(now)
        .execute(&pool)
        .await?;

        // A collaboration write locks the project row before it takes the
        // shared access lock for the epoch it was admitted under.
        let mut collaboration_write = pool.begin().await?;
        sqlx::query("select content_epoch from projects where id = $1 for key share")
            .bind(project_id)
            .fetch_one(&mut *collaboration_write)
            .await?;
        let deletion_pool = pool.clone();
        let deletion = tokio::spawn(async move {
            delete_project(
                &deletion_pool,
                DeleteProjectCommand {
                    project_id,
                    actor_user_id: owner,
                },
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!deletion.is_finished());

        let access = tokio::time::timeout(
            Duration::from_secs(5),
            lock_project_access_epoch(&mut collaboration_write, project_id, 0),
        )
        .await??;
        assert_eq!(access, ProjectAccessEpochMatch::Current);
        collaboration_write.commit().await?;

        let deleted = tokio::time::timeout(Duration::from_secs(5), deletion).await???;
        assert_eq!(deleted.name, "Lock order");
        let remaining: i64 = sqlx::query_scalar("select count(*) from projects where id = $1")
            .bind(project_id)
            .fetch_one(&pool)
            .await?;
        assert_eq!(remaining, 0);
        Ok(())
    }
}
