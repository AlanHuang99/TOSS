---
title: "Identity and access"
summary: "Login authorities, account identity, sessions, project authorization, organizations, sharing, and Git tokens."
status: current
type: architecture
scope: community
audience:
  - backend-contributor
  - operator
  - security-reviewer
  - coding-agent
topics:
  - authentication
  - oidc
  - authorization
  - sharing
related:
  - docs/community/architecture/external-repositories.md
  - docs/community/configuration/external-git.md
  - docs/community/operations/deployment.md
  - docs/community/reference/api.md
code_paths:
  - backend/src/access
  - backend/src/external_repositories/login
  - backend/src/external_repositories/oauth
---

# Identity and access

Identity, application authorization, and repository authorization are distinct
capabilities even when one external-provider callback supplies both an
application session and a repository grant.

## Login authorities

Supported application identities are:

- local accounts with Argon2 password verification and session cookies;
- one generic discovery-based OIDC configuration;
- any configured external repository provider with `login_enabled = true`.

The public auth config exposes an ordered `identity_providers` collection for
the sign-in page. Repository provider configuration is independent: an
instance may permit connection without being a login option.

The platform account is identified by its internal user UUID. It may own many
login methods. A federated login method is the tuple:

```text
(authority_kind, authority_id, subject)
```

Generic OIDC uses normalized issuer as the authority ID. External login uses
the stable provider instance ID. The full tuple maps to exactly one platform
account, and one account may bind at most one subject for an authority.

A new OIDC account takes its display name from the `name` claim, then from
`given_name` and `family_name`, then `preferred_username`, then the local part
of the `email` claim, and finally the placeholder `OIDC User`. When an existing
login identity signs in and the account still carries that placeholder, the
newly derived name replaces it; a name the user has set is never overwritten.
Users change their display name through `PATCH /v1/auth/me`.

Subject is not globally unique, and accounts are never merged by matching
email. A verified email already owned by another account produces an account
link prompt: the user authenticates through an existing method and then
explicitly connects the new provider. The first provider used to create an
account is only its initial login method, not a permanent primary authority.

Removing an external connection also removes its external login method when it
has one. Access rejects removal of the final login method; a local password
counts as a login method independently of federated identities.

## Sessions and credential storage

Application sessions use an HTTP-only cookie. Session, personal access, and
named guest tokens are stored as 32-byte SHA-256 fingerprints; plaintext is
not recoverable. Personal access token plaintext is returned once at creation.

Personal access tokens carry scopes. `git` admits Git smart HTTP and `api`
admits REST requests that send the token as a bearer credential; tokens
created before scopes existed keep only `git`. Personal tokens start with
`tpat_`, which never occurs in session tokens, so Access validates a prefixed
bearer value only as a personal token and checks expiry and revocation on
every request. Principal resolution records which credential authenticated
the request (session, personal token, or development header), and token
management and site administration refuse personal tokens. The last-use time
is written at most once per minute per token.
Project share links are intentionally recoverable and store one canonical token
value so managers can copy an existing link.

Provider OAuth grants are encrypted with the deployment-level external Git key.
Tokens never appear in browser configuration, Git URLs, provider TOML, or
distribution JSON.

## Project authorization

Access computes effective project capability from all applicable sources:

- direct project role;
- organization grant and membership;
- OIDC group-role mapping;
- authenticated share-link redemption;
- named temporary guest session.

Read, write, and manage are separate capabilities. A weaker direct role does
not mask a stronger organization grant. REST and WebSocket boundaries ask the
Access façade; they never rely on UI visibility or query grant tables directly.

Project access changes advance an access epoch and notify realtime clients so
stale connections reauthorize.

## Organizations and sharing

Organizations own memberships and administrators. OIDC group synchronization
may grant organization roles but cannot silently downgrade an existing owner.

A signed-in user redeeming a share link receives a durable project role. A
named temporary guest receives a scoped, expiring project session. Revoking a
link prevents new redemption; it does not silently revoke an authenticated
role already granted through that link. Marking a project as a template
revokes its temporary guest sessions in the template-status transaction.

## Bootstrap administration

The service does not create a default administrator or print a password on
startup. `BOOTSTRAP_ADMIN_EMAILS` is a comma-separated allowlist: when a
matching user successfully registers or signs in, that account is promoted to
site administrator. Production deployments should set the allowlist before the
first intended administrator authenticates, then manage access through normal
policy.

`AUTH_DEV_HEADER_ENABLED` is a development-only identity override and must be
disabled in production.

## Related

- [External repositories](./external-repositories.md)
- [External Git configuration](../configuration/external-git.md)
- [Deployment](../operations/deployment.md)
- [API guide](../reference/api.md)
