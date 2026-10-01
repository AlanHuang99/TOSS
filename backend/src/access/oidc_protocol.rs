//! OpenID Connect protocol exchange and identity normalization.

use super::account_policy::PLACEHOLDER_FEDERATED_DISPLAY_NAME;
use super::auth_settings_model::AuthSettings;
use super::oidc_claims::extract_groups_from_id_token;
use super::oidc_policy::discovery_issuer;
use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreIdTokenClaims, CoreProviderMetadata,
    CoreRequestTokenError,
};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, LocalizedClaim, Nonce, RedirectUrl,
    Scope, TokenResponse,
};
use reqwest::redirect::Policy;
use std::ops::Deref;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum OidcProtocolError {
    #[error("OIDC is disabled")]
    Disabled,
    #[error("OIDC configuration is incomplete")]
    IncompleteConfiguration,
    #[error("OIDC issuer URL is invalid")]
    InvalidIssuer {
        #[source]
        source: url::ParseError,
    },
    #[error("OIDC redirect URI is invalid")]
    InvalidRedirectUri {
        #[source]
        source: url::ParseError,
    },
    #[error("could not initialize OIDC HTTP client")]
    ClientInitialization {
        #[source]
        source: reqwest::Error,
    },
    #[error("OIDC provider discovery failed")]
    ProviderUnavailable {
        #[source]
        source: openidconnect::DiscoveryError<openidconnect::HttpClientError<reqwest::Error>>,
    },
    #[error("OIDC token request could not be configured")]
    TokenRequestConfiguration {
        #[source]
        source: openidconnect::ConfigurationError,
    },
    #[error("OIDC token exchange failed")]
    TokenExchangeFailed {
        #[source]
        source: CoreRequestTokenError<openidconnect::HttpClientError<reqwest::Error>>,
    },
    #[error("OIDC token response did not include an ID token")]
    MissingIdToken,
    #[error("OIDC ID token verification failed")]
    IdTokenVerificationFailed {
        #[source]
        source: openidconnect::ClaimsVerificationError,
    },
}

pub(crate) struct OidcConfiguration {
    issuer: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    groups_claim: String,
}

impl OidcConfiguration {
    pub(crate) fn from_auth_settings(settings: AuthSettings) -> Result<Self, OidcProtocolError> {
        if !settings.allow_oidc {
            return Err(OidcProtocolError::Disabled);
        }
        let issuer = settings.oidc_issuer.unwrap_or_default();
        let client_id = settings.oidc_client_id.unwrap_or_default();
        let client_secret = settings.oidc_client_secret.unwrap_or_default();
        let redirect_uri = settings.oidc_redirect_uri.unwrap_or_default();
        if issuer.trim().is_empty() || client_id.trim().is_empty() || redirect_uri.trim().is_empty()
        {
            return Err(OidcProtocolError::IncompleteConfiguration);
        }
        Ok(Self {
            issuer,
            client_id,
            client_secret,
            redirect_uri,
            groups_claim: settings.oidc_groups_claim,
        })
    }
}

pub(crate) struct AuthenticatedOidcIdentity {
    pub email: String,
    pub display_name: String,
    pub subject: String,
    pub issuer: String,
    pub username_seed: String,
    pub groups: Vec<String>,
}

struct VerifiedOidcTokens {
    claims: CoreIdTokenClaims,
    raw_id_token: String,
}

struct IdentityProfile {
    email: String,
    display_name: String,
    username_seed: String,
}

pub(crate) async fn validate_provider_discovery(
    discovery_url: &str,
) -> Result<(), OidcProtocolError> {
    let issuer = discovery_issuer(discovery_url)
        .map_err(|source| OidcProtocolError::InvalidIssuer { source })?;
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(Policy::none())
        .build()
        .map_err(|source| OidcProtocolError::ClientInitialization { source })?;
    CoreProviderMetadata::discover_async(issuer, &http_client)
        .await
        .map_err(|source| OidcProtocolError::ProviderUnavailable { source })?;
    Ok(())
}

pub(crate) async fn authorization_url(
    configuration: &OidcConfiguration,
    state_token: &str,
    nonce_token: &str,
) -> Result<String, OidcProtocolError> {
    let (provider_metadata, _http_client, redirect_uri) = discover_provider(configuration).await?;
    let client = CoreClient::from_provider_metadata(
        provider_metadata,
        ClientId::new(configuration.client_id.clone()),
        Some(ClientSecret::new(configuration.client_secret.clone())),
    )
    .set_redirect_uri(redirect_uri);
    let csrf_secret = state_token.to_string();
    let nonce_secret = nonce_token.to_string();
    let (authorize_url, _csrf, _nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            move || CsrfToken::new(csrf_secret),
            move || Nonce::new(nonce_secret),
        )
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .url();
    Ok(authorize_url.to_string())
}

pub(crate) async fn authenticate_callback(
    configuration: &OidcConfiguration,
    authorization_code: &str,
    nonce: String,
) -> Result<AuthenticatedOidcIdentity, OidcProtocolError> {
    let verified_tokens =
        exchange_verified_tokens(configuration, authorization_code, nonce).await?;
    Ok(resolve_identity(configuration, verified_tokens))
}

async fn exchange_verified_tokens(
    configuration: &OidcConfiguration,
    authorization_code: &str,
    nonce: String,
) -> Result<VerifiedOidcTokens, OidcProtocolError> {
    let (provider_metadata, http_client, redirect_uri) = discover_provider(configuration).await?;
    let client = CoreClient::from_provider_metadata(
        provider_metadata,
        ClientId::new(configuration.client_id.clone()),
        Some(ClientSecret::new(configuration.client_secret.clone())),
    )
    .set_redirect_uri(redirect_uri);
    let token_request = client
        .exchange_code(AuthorizationCode::new(authorization_code.to_string()))
        .map_err(|source| OidcProtocolError::TokenRequestConfiguration { source })?;
    let tokens = token_request
        .request_async(&http_client)
        .await
        .map_err(|source| OidcProtocolError::TokenExchangeFailed { source })?;
    let id_token = tokens.id_token().ok_or(OidcProtocolError::MissingIdToken)?;
    let claims: CoreIdTokenClaims = id_token
        .claims(&client.id_token_verifier(), &Nonce::new(nonce))
        .map_err(|source| OidcProtocolError::IdTokenVerificationFailed { source })?
        .clone();
    let raw_id_token = id_token.to_string();
    Ok(VerifiedOidcTokens {
        claims,
        raw_id_token,
    })
}

fn resolve_identity(
    configuration: &OidcConfiguration,
    verified_tokens: VerifiedOidcTokens,
) -> AuthenticatedOidcIdentity {
    let issuer = verified_tokens.claims.issuer().url().to_string();
    let subject = verified_tokens.claims.subject().as_str().to_string();
    let profile = identity_profile(&verified_tokens.claims, &subject);
    let groups =
        extract_groups_from_id_token(verified_tokens.raw_id_token, &configuration.groups_claim);
    AuthenticatedOidcIdentity {
        email: profile.email,
        display_name: profile.display_name,
        subject,
        issuer,
        username_seed: profile.username_seed,
        groups,
    }
}

struct DisplayNameClaims<'claims> {
    name: Option<&'claims str>,
    given_name: Option<&'claims str>,
    family_name: Option<&'claims str>,
    preferred_username: Option<&'claims str>,
    email: Option<&'claims str>,
}

fn identity_profile(claims: &CoreIdTokenClaims, subject: &str) -> IdentityProfile {
    let email = claims
        .email()
        .map(|value| value.to_string())
        .unwrap_or_else(|| format!("{subject}@oidc.local"));
    let display_name = display_name_from_claims(&DisplayNameClaims {
        name: localized_claim_value(claims.name()),
        given_name: localized_claim_value(claims.given_name()),
        family_name: localized_claim_value(claims.family_name()),
        preferred_username: claims.preferred_username().map(|value| value.as_str()),
        email: claims.email().map(|value| value.as_str()),
    });
    let username_seed = claims
        .preferred_username()
        .map(|value| value.to_string())
        .unwrap_or_else(|| username_seed_from_email(&email));
    IdentityProfile {
        email,
        display_name,
        username_seed,
    }
}

/// Prefers the full `name` claim, then the given and family names, then the
/// preferred username, and finally the local part of the email claim.
fn display_name_from_claims(claims: &DisplayNameClaims<'_>) -> String {
    if let Some(name) = claims.name.and_then(non_empty_claim) {
        return name.to_string();
    }
    let full_name = [claims.given_name, claims.family_name]
        .into_iter()
        .flatten()
        .filter_map(non_empty_claim)
        .collect::<Vec<_>>()
        .join(" ");
    if !full_name.is_empty() {
        return full_name;
    }
    claims
        .preferred_username
        .and_then(non_empty_claim)
        .or_else(|| {
            claims
                .email
                .and_then(|email| email.split_once('@'))
                .and_then(|(local_part, _)| non_empty_claim(local_part))
        })
        .unwrap_or(PLACEHOLDER_FEDERATED_DISPLAY_NAME)
        .to_string()
}

/// Returns the untagged claim value, or else the localized value with the
/// lowest language tag so the choice does not depend on map ordering.
fn localized_claim_value<T>(claim: Option<&LocalizedClaim<T>>) -> Option<&str>
where
    T: Deref<Target = String>,
{
    let claim = claim?;
    claim
        .get(None)
        .and_then(|value| non_empty_claim(value))
        .or_else(|| {
            claim
                .iter()
                .filter_map(|(language, value)| Some((language?, non_empty_claim(value)?)))
                .min_by(|left, right| left.0.cmp(right.0))
                .map(|(_, value)| value)
        })
}

fn non_empty_claim(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

async fn discover_provider(
    configuration: &OidcConfiguration,
) -> Result<(CoreProviderMetadata, reqwest::Client, RedirectUrl), OidcProtocolError> {
    let issuer = discovery_issuer(&configuration.issuer)
        .map_err(|source| OidcProtocolError::InvalidIssuer { source })?;
    let http_client = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .map_err(|source| OidcProtocolError::ClientInitialization { source })?;
    let provider_metadata = CoreProviderMetadata::discover_async(issuer, &http_client)
        .await
        .map_err(|source| OidcProtocolError::ProviderUnavailable { source })?;
    let redirect_uri = RedirectUrl::new(configuration.redirect_uri.clone())
        .map_err(|source| OidcProtocolError::InvalidRedirectUri { source })?;
    Ok((provider_metadata, http_client, redirect_uri))
}

fn username_seed_from_email(email: &str) -> String {
    email
        .split('@')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("oidc-user")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        display_name_from_claims, identity_profile, localized_claim_value,
        username_seed_from_email, DisplayNameClaims, PLACEHOLDER_FEDERATED_DISPLAY_NAME,
    };
    use chrono::{Duration, Utc};
    use openidconnect::core::CoreIdTokenClaims;
    use openidconnect::{
        Audience, EmptyAdditionalClaims, EndUserEmail, EndUserFamilyName, EndUserGivenName,
        EndUserName, EndUserUsername, IssuerUrl, LanguageTag, LocalizedClaim, StandardClaims,
        SubjectIdentifier,
    };

    fn claims(
        standard_claims: StandardClaims<openidconnect::core::CoreGenderClaim>,
    ) -> Result<CoreIdTokenClaims, url::ParseError> {
        Ok(CoreIdTokenClaims::new(
            IssuerUrl::new("https://identity.example.test".to_string())?,
            vec![Audience::new("toss".to_string())],
            Utc::now() + Duration::minutes(5),
            Utc::now(),
            standard_claims,
            EmptyAdditionalClaims {},
        ))
    }

    fn subject() -> SubjectIdentifier {
        SubjectIdentifier::new("subject-1".to_string())
    }

    fn empty_display_name_claims() -> DisplayNameClaims<'static> {
        DisplayNameClaims {
            name: None,
            given_name: None,
            family_name: None,
            preferred_username: None,
            email: None,
        }
    }

    #[test]
    fn username_seed_prefers_the_email_local_part() {
        assert_eq!(username_seed_from_email("ada@example.com"), "ada");
        assert_eq!(username_seed_from_email("@example.com"), "oidc-user");
    }

    #[test]
    fn display_name_follows_the_standard_claim_fallback_order() {
        let mut claims = DisplayNameClaims {
            name: Some("  Ada Lovelace "),
            given_name: Some("Augusta"),
            family_name: Some("King"),
            preferred_username: Some("ada"),
            email: Some("countess@example.test"),
        };
        assert_eq!(display_name_from_claims(&claims), "Ada Lovelace");

        claims.name = Some("   ");
        assert_eq!(display_name_from_claims(&claims), "Augusta King");

        claims.family_name = None;
        assert_eq!(display_name_from_claims(&claims), "Augusta");

        claims.given_name = Some(" ");
        claims.family_name = Some("King");
        assert_eq!(display_name_from_claims(&claims), "King");

        claims.family_name = None;
        assert_eq!(display_name_from_claims(&claims), "ada");

        claims.preferred_username = Some("");
        assert_eq!(display_name_from_claims(&claims), "countess");

        claims.email = Some(" @example.test");
        assert_eq!(
            display_name_from_claims(&claims),
            PLACEHOLDER_FEDERATED_DISPLAY_NAME
        );
        assert_eq!(
            display_name_from_claims(&empty_display_name_claims()),
            PLACEHOLDER_FEDERATED_DISPLAY_NAME
        );
    }

    #[test]
    fn localized_names_prefer_the_untagged_value_then_the_lowest_language_tag() {
        let untagged = LocalizedClaim::from_iter([
            (
                Some(LanguageTag::new("zh-CN".to_string())),
                EndUserName::new("Localized".to_string()),
            ),
            (None, EndUserName::new("Untagged".to_string())),
        ]);
        assert_eq!(localized_claim_value(Some(&untagged)), Some("Untagged"));

        let localized_only = LocalizedClaim::from_iter([
            (
                Some(LanguageTag::new("zh-CN".to_string())),
                EndUserName::new("Second".to_string()),
            ),
            (
                Some(LanguageTag::new("en".to_string())),
                EndUserName::new(" First ".to_string()),
            ),
            (
                Some(LanguageTag::new("de".to_string())),
                EndUserName::new(" ".to_string()),
            ),
        ]);
        assert_eq!(localized_claim_value(Some(&localized_only)), Some("First"));
        assert_eq!(localized_claim_value::<EndUserName>(None), None);
    }

    #[test]
    fn identity_profile_reads_names_from_verified_claims() -> Result<(), url::ParseError> {
        let named = claims(
            StandardClaims::new(subject())
                .set_name(Some(LocalizedClaim::from(EndUserName::new(
                    "Ada Lovelace".to_string(),
                ))))
                .set_preferred_username(Some(EndUserUsername::new("ada".to_string()))),
        )?;
        let profile = identity_profile(&named, "subject-1");
        assert_eq!(profile.display_name, "Ada Lovelace");
        assert_eq!(profile.username_seed, "ada");
        assert_eq!(profile.email, "subject-1@oidc.local");

        let split_name = claims(
            StandardClaims::new(subject())
                .set_given_name(Some(LocalizedClaim::from(EndUserGivenName::new(
                    "Ada".to_string(),
                ))))
                .set_family_name(Some(LocalizedClaim::from(EndUserFamilyName::new(
                    "Lovelace".to_string(),
                )))),
        )?;
        assert_eq!(
            identity_profile(&split_name, "subject-1").display_name,
            "Ada Lovelace"
        );

        let email_only = claims(
            StandardClaims::new(subject())
                .set_email(Some(EndUserEmail::new("ada@example.test".to_string()))),
        )?;
        let profile = identity_profile(&email_only, "subject-1");
        assert_eq!(profile.display_name, "ada");
        assert_eq!(profile.username_seed, "ada");

        let anonymous = claims(StandardClaims::new(subject()))?;
        assert_eq!(
            identity_profile(&anonymous, "subject-1").display_name,
            PLACEHOLDER_FEDERATED_DISPLAY_NAME
        );
        Ok(())
    }
}
