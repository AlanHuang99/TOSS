//! Validated Workspace project descriptions and their update workflow.

use super::projects_persistence;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

const MAX_PROJECT_DESCRIPTION_CHARS: usize = 2000;

/// A trimmed description; an empty value clears it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectDescription(Option<String>);

impl ProjectDescription {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, InvalidProjectDescription> {
        let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self(None));
        };
        if value.chars().count() > MAX_PROJECT_DESCRIPTION_CHARS
            || value
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        {
            return Err(InvalidProjectDescription);
        }
        Ok(Self(Some(value.to_string())))
    }

    pub(crate) fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("project description is invalid")]
pub(crate) struct InvalidProjectDescription;

#[derive(Clone, Copy, Debug)]
enum UpdateProjectDescriptionStage {
    Begin,
    Update,
    Commit,
}

#[derive(Debug, Error)]
#[error("project description update failed during {stage:?} for project {project_id}")]
pub(super) struct UpdateProjectDescriptionPersistenceError {
    stage: UpdateProjectDescriptionStage,
    project_id: Uuid,
    #[source]
    source: sqlx::Error,
}

#[derive(Debug, Error)]
pub(super) enum UpdateProjectDescriptionError {
    #[error("project was not found")]
    ProjectNotFound,
    #[error(transparent)]
    Persistence(#[from] UpdateProjectDescriptionPersistenceError),
}

pub(super) async fn update_project_description(
    db: &PgPool,
    project_id: Uuid,
    description: &ProjectDescription,
) -> Result<(), UpdateProjectDescriptionError> {
    let persistence_error = |stage, source| UpdateProjectDescriptionPersistenceError {
        stage,
        project_id,
        source,
    };
    let mut transaction = db
        .begin()
        .await
        .map_err(|source| persistence_error(UpdateProjectDescriptionStage::Begin, source))?;
    let updated =
        projects_persistence::update_description(&mut transaction, project_id, description)
            .await
            .map_err(|source| persistence_error(UpdateProjectDescriptionStage::Update, source))?;
    if !updated {
        return Err(UpdateProjectDescriptionError::ProjectNotFound);
    }
    transaction
        .commit()
        .await
        .map_err(|source| persistence_error(UpdateProjectDescriptionStage::Commit, source))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{InvalidProjectDescription, ProjectDescription, MAX_PROJECT_DESCRIPTION_CHARS};

    #[test]
    fn descriptions_are_trimmed_and_empty_values_clear_them(
    ) -> Result<(), InvalidProjectDescription> {
        assert_eq!(
            ProjectDescription::parse(Some("  Lecture notes\nweek 1  "))?.as_deref(),
            Some("Lecture notes\nweek 1")
        );
        assert_eq!(ProjectDescription::parse(Some(" \n "))?.as_deref(), None);
        assert_eq!(ProjectDescription::parse(None)?.as_deref(), None);
        Ok(())
    }

    #[test]
    fn oversized_and_control_character_descriptions_are_rejected(
    ) -> Result<(), InvalidProjectDescription> {
        let longest = "界".repeat(MAX_PROJECT_DESCRIPTION_CHARS);
        assert_eq!(
            ProjectDescription::parse(Some(&longest))?.as_deref(),
            Some(longest.as_str())
        );
        assert_eq!(
            ProjectDescription::parse(Some(&format!("{longest}x"))),
            Err(InvalidProjectDescription)
        );
        assert_eq!(
            ProjectDescription::parse(Some("null\0byte")),
            Err(InvalidProjectDescription)
        );
        Ok(())
    }
}
