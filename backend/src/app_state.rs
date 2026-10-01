//! Process-wide runtime composition passed to HTTP transport handlers.

use crate::access::OidcProviderDefaults;
use crate::collaboration::CollaborationContext;
use crate::distribution::{AiAssistantConfig, DistributionConfig, FrontendFeature};
use crate::document_processing::DocumentProcessingContext;
use crate::external_repositories::{
    ExternalGitGateway, ExternalGitProviderRegistry, ProviderInstanceId,
};
use crate::object_storage::ObjectStorage;
use crate::process_lifecycle::DrainSignal;
use crate::versioning::VersioningContext;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct AppState {
    pub db: PgPool,
    pub oidc_defaults: OidcProviderDefaults,
    pub external_git_providers: ExternalGitProviderRegistry,
    pub data_dir: PathBuf,
    pub git_storage_dir: PathBuf,
    pub typst_builtin_dir: PathBuf,
    pub storage: Option<ObjectStorage>,
    pub distribution: Arc<DistributionConfig>,
    pub frontend_features: Arc<Vec<FrontendFeature>>,
    pub ai_assistant: Arc<Option<AiAssistantConfig>>,
    pub spa_index_html: Arc<[u8]>,
    pub collaboration: CollaborationContext,
    pub versioning: VersioningContext,
    pub processing: DocumentProcessingContext,
    pub drain: DrainSignal,
}

impl AppState {
    pub(crate) fn external_git_gateway(
        &self,
        provider_id: &ProviderInstanceId,
    ) -> ExternalGitGateway<'_> {
        ExternalGitGateway::new(
            &self.db,
            self.external_git_providers.get(provider_id),
            self.drain.clone(),
        )
    }
}

#[cfg(test)]
impl AppState {
    /// Composes the Community distribution over a test database without object
    /// storage, external providers, optional features, or background owners.
    pub(crate) fn for_tests(db: PgPool, data_dir: PathBuf) -> Result<Self, String> {
        let distribution = DistributionConfig::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../distributions/community/toss.json"),
        )?;
        let processing = crate::document_processing::ProcessingConfig::from_config(
            crate::document_processing::ProcessingConfigFile::default(),
            &data_dir,
        )?;
        let external_git_providers = ExternalGitProviderRegistry::from_providers(Vec::new())
            .map_err(|instance_id| format!("duplicate external Git provider {instance_id}"))?;
        let drain = DrainSignal::idle();
        Ok(Self {
            oidc_defaults: OidcProviderDefaults {
                provider_id: "oidc".to_string(),
                provider_display_name: "OpenID Connect".to_string(),
                issuer: String::new(),
                client_id: String::new(),
                client_secret: String::new(),
                redirect_uri: String::new(),
                groups_claim: "groups".to_string(),
            },
            external_git_providers,
            git_storage_dir: data_dir.join("git"),
            typst_builtin_dir: data_dir.join("builtin"),
            storage: None,
            distribution: Arc::new(distribution),
            frontend_features: Arc::new(Vec::new()),
            ai_assistant: Arc::new(None),
            spa_index_html: Arc::from(Vec::new()),
            collaboration: CollaborationContext::new(db.clone(), drain.clone()),
            versioning: VersioningContext::default(),
            processing: DocumentProcessingContext::new(db.clone(), None, processing),
            drain,
            data_dir,
            db,
        })
    }
}
