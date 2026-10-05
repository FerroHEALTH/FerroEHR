// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one server configuration tree — `ferroehr.toml` + `FERROEHR_*` env
//! overrides.
//!
//! No openEHR spec governs configuration; this is entirely our own design.
//! [`FerroEhrConfig`] is the single serde root, each section owned by the crate
//! that consumes it and referenced here, with exactly one loader
//! ([`load`]/[`assemble`]) and no per-subsystem `FERROEHR_*_CONFIG` file
//! pointers.
//!
//! Precedence, lowest first: built-in `Default` impls, the config file, the
//! `FERROEHR_*` environment (`__` for nesting), then `--set key=value`
//! overrides. Two conventional aliases sit below their `FERROEHR_` forms within
//! the env layer, `DATABASE_URL` for `db.url` and `RUST_LOG` for `log.filter`.
//! [`assemble`] is a pure function of `(file, env_map, overrides)` with no
//! process-global env, so the whole test plan runs on injected inputs.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

mod alias;
pub mod loader;

use crate::config::loader::{ConfigError, ConfigErrors};
use crate::config::secret::Secret;
mod strict;

pub mod auth;
pub mod authz;
pub mod deployment;
pub mod management;
pub mod profile;
pub mod secret;
pub mod server;
pub mod smart;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The complete server configuration.
///
/// Every section has a `Default`, so the file may be empty or absent
/// (zero-config boot). `deny_unknown_fields` makes a misspelled top-level
/// table a boot error.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FerroEhrConfig {
    /// `spec_profile` — the openEHR specification generation set the server
    /// runs (`development` | `stable`; default `development`).
    pub spec_profile: profile::SpecProfile,
    /// `deployment_profile` — the declared posture (`sandbox` | `production`;
    /// default `sandbox`). `production` refuses to start while a separation it
    /// asserts is missing and not accepted by name (#3226).
    pub deployment_profile: deployment::DeploymentProfile,
    /// `deployment_accepts` — the [`deployment::DeploymentGap`]s a `production`
    /// deployment runs without, each stated on every boot and on
    /// `/rest/status`. Ignored under `sandbox`, which accepts everything.
    pub deployment_accepts: Vec<deployment::DeploymentGap>,
    /// `[server]` — HTTP listener + REST surface + System-Options identity.
    pub server: server::ServerConfig,
    /// `[db]` — the shared `PostgreSQL` DSN, pool tuning and migration mode.
    pub db: crate::db::DbConfig,
    /// `[storage]` — one DSN per storage domain (`clinical`, `party`,
    /// `linkage`, `audit`), each defaulting to `[db].url`.
    pub storage: crate::db::domain::StorageConfig,
    /// `[log]` — logging.
    pub log: crate::telemetry::config::LogConfig,
    /// `[telemetry]` — OpenTelemetry export.
    pub telemetry: crate::telemetry::config::OtelConfig,
    /// `[auth]` — authentication.
    pub auth: auth::AuthConfig,
    /// `[authz]` — RBAC + ABAC.
    pub authz: authz::AuthzConfig,
    /// `[admin]` — the ADMIN API group.
    pub admin: server::AdminConfig,
    /// `[smart]` — SMART App Launch.
    pub smart: smart::SmartConfig,
    /// `[management]` — the management/observability surface.
    pub management: management::ManagementConfig,
    /// `[signing]` — VERSION signing.
    pub signing: crate::versioning::signature::config::SigningConfig,
    /// `[licence]` — the commercial licence token (optional; the server runs
    /// identically without one).
    pub licence: crate::licence::config::LicenceConfig,
    /// `[query]` — AQL execution knobs.
    pub query: crate::service::query::config::QueryConfig,
    /// `[events]` — contribution-outbox eventing (+ its admin API).
    pub events: crate::extensions::events::config::EventsConfig,
    /// `[fhir]` — the FHIR connector (inbound façade + outbound emitter).
    pub fhir: crate::extensions::fhir::config::FhirConfig,
    /// `[terminology]` — terminology API + external-server validation.
    pub terminology: crate::service::terminology::config::TerminologyConfig,
    /// `[multimedia]` — `DV_MULTIMEDIA` externalization.
    pub multimedia: crate::extensions::multimedia::config::MultimediaConfig,
    /// `[audit]` — the IHE ATNA audit trail / System Log (local Audit Record
    /// Repository + the syslog and FHIR-feed forwarding sinks).
    pub audit: crate::system_log::config::AuditConfig,
    /// `[privacy]` — the clinical-side data-minimisation policy.
    pub privacy: crate::privacy::config::PrivacyConfig,
    /// `[demographic]` — the demographic domain, including national-identifier
    /// protection.
    pub demographic: crate::service::demographic::identifier::config::DemographicConfig,
    /// `[cohort]` — the cross-domain cohort-query allow-list and limits.
    pub cohort: crate::service::linkage::cohort::config::CohortConfig,
}

/// The annotated default template `ferroehr config default` prints — a
/// hand-maintained asset kept in sync with the schema by the template tests.
pub const DEFAULT_TEMPLATE: &str = include_str!("../../assets/ferroehr.default.toml");

impl FerroEhrConfig {
    /// Aggregated semantic validation: every cross-field rule reported at
    /// once, so an operator fixes the config in one iteration.
    ///
    /// # Errors
    /// [`ConfigErrors`] carrying every failing rule.
    pub fn validate(&self) -> Result<(), ConfigErrors> {
        let mut errors = Vec::new();
        self.validate_system_id(&mut errors);
        self.validate_base_path(&mut errors);
        self.validate_mechanisms(&mut errors);
        self.validate_signing(&mut errors);
        self.validate_key_sources(&mut errors);
        self.validate_privacy(&mut errors);
        self.validate_audit(&mut errors);
        self.validate_cohort(&mut errors);
        self.validate_deployment(&mut errors);
        errors.extend(multimedia_endpoint_errors(&self.multimedia));
        // management.port must differ from the server.bind port.
        if let Some(port) = self.management.port
            && server_bind_port(&self.server.bind) == Some(port)
        {
            errors.push(ConfigError::semantic(format!(
                "management.port ({port}) must differ from the server.bind port"
            )));
        }
        validate_terminology(&self.terminology.external, &mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ConfigErrors(errors))
        }
    }

    /// The access-log horizon sits between the floors and the ceilings the
    /// jurisdictions in force declare.
    ///
    /// A retention shorter than a floor erases the log while the chain still
    /// verifies (#3242); a retention longer than a ceiling keeps log data past
    /// the point the law says to delete it (#3346), which `0` — keep forever —
    /// always does. The jurisdictions in force are the ones the active
    /// identifier rules name, and the German ceiling additionally waits for the
    /// deployment to declare itself one of the SGB V § 307 controllers. Where a
    /// floor and a ceiling contradict each other, no horizon satisfies both and
    /// the refusal says so rather than silently preferring one.
    fn validate_audit(&self, errors: &mut Vec<ConfigError>) {
        let store = &self.audit.store;
        if !(self.audit.enabled && store.enabled) {
            return;
        }
        let jurisdictions: std::collections::BTreeSet<&str> = self
            .privacy
            .identifier_scan
            .rules
            .iter()
            .filter_map(|key| crate::privacy::detect::rule(key))
            .map(|rule| rule.jurisdiction)
            .collect();
        let declared = self.audit.store.sgb_v_309_controller;
        let mut floors: Vec<(&str, u32)> = Vec::new();
        let mut ceilings: Vec<(&str, u32)> = Vec::new();
        for jurisdiction in jurisdictions {
            if let Some(floor) = crate::system_log::config::retention_floor_days(jurisdiction) {
                floors.push((jurisdiction, floor));
            }
            if let Some(ceiling) =
                crate::system_log::config::retention_ceiling_days(jurisdiction, declared)
            {
                ceilings.push((jurisdiction, ceiling));
            }
        }
        for &(jurisdiction, floor) in &floors {
            for &(capped, ceiling) in &ceilings {
                if ceiling < floor {
                    errors.push(ConfigError::semantic(format!(
                        "the {capped} access-log retention ceiling of {ceiling} days is below \
                         the {jurisdiction} floor of {floor} days, so no horizon satisfies \
                         both; run the two jurisdictions as separate deployments, or drop the \
                         privacy.identifier_scan rule that does not apply here"
                    )));
                }
            }
            if store.retention_days > 0 && store.retention_days < floor {
                errors.push(ConfigError::semantic(format!(
                    "audit.store.retention_days = {} is below the {jurisdiction} access-log \
                     retention floor of {floor} days (five years, Besluit vaststelling \
                     bewaartermijn logging, https://wetten.overheid.nl/BWBR0042391); keep \
                     records at least that long, or set 0 to keep them forever",
                    store.retention_days
                )));
            }
        }
        for &(jurisdiction, ceiling) in &ceilings {
            if store.retention_days == 0 || store.retention_days > ceiling {
                let horizon = match store.retention_days {
                    0 => "0 (keep forever)".to_owned(),
                    days => days.to_string(),
                };
                errors.push(ConfigError::semantic(format!(
                    "audit.store.retention_days = {horizon} is above the {jurisdiction} \
                     access-log retention ceiling of {ceiling} days (the three-year limitation \
                     period of SGB V § 309 Abs. 1, after which Abs. 3 requires deletion \
                     unverzüglich, https://www.gesetze-im-internet.de/sgb_5/__309.html); set a \
                     horizon at or below it, or clear audit.store.sgb_v_309_controller if this \
                     deployment is not one of the § 307 controllers"
                )));
            }
        }
    }

    /// A cohort predicate is only a selection criterion if the deployment can
    /// actually reach the leaf it names: an unknown key, an archetype that is
    /// not a demographic HRID, or a node that is not an at-code binds a
    /// predicate that would match nothing and report an empty cohort rather
    /// than a misconfiguration. No openEHR spec governs configuration — our
    /// own design; the identifier grammars are BASE `base_types`
    /// `master05-identification_package.adoc` §Archetype Identifiers.
    fn validate_cohort(&self, errors: &mut Vec<ConfigError>) {
        use crate::service::linkage::cohort::config as cohort;
        for (key, binding) in &self.cohort.predicates {
            if !cohort::PREDICATE_KEYS.contains(&key.as_str()) {
                errors.push(ConfigError::semantic(format!(
                    "cohort.predicates names `{key}`, which is not a cohort predicate; the \
                     bindable keys are {}",
                    cohort::PREDICATE_KEYS.join(", ")
                )));
                continue;
            }
            let is_birth_date = binding.kind == cohort::PredicateKind::BirthDate;
            if (key == cohort::BIRTH_DATE_KEY) != is_birth_date {
                errors.push(ConfigError::semantic(format!(
                    "cohort.predicates.{key}.kind = \"{}\" — `{}` is the age-band predicate \
                     and takes kind \"birth_date\"; no other key does",
                    binding.kind.as_str(),
                    cohort::BIRTH_DATE_KEY
                )));
            }
            if !cohort::is_demographic_archetype_hrid(&binding.archetype) {
                errors.push(ConfigError::semantic(format!(
                    "cohort.predicates.{key}.archetype = {:?} is not a demographic archetype \
                     HRID (openEHR-DEMOGRAPHIC-<CLASS>.<concept>.v<n>)",
                    binding.archetype
                )));
            }
            if !cohort::is_at_code(&binding.node) {
                errors.push(ConfigError::semantic(format!(
                    "cohort.predicates.{key}.node = {:?} is not an at-code (at0012)",
                    binding.node
                )));
            }
        }
    }

    /// `deployment_accepts` names each gap once, and only means something
    /// under `production`: a sandbox that lists accepted gaps is describing a
    /// posture it does not enforce, which is the confusion the key exists to
    /// remove.
    fn validate_deployment(&self, errors: &mut Vec<ConfigError>) {
        let mut seen = std::collections::BTreeSet::new();
        for gap in &self.deployment_accepts {
            if !seen.insert(gap.as_str()) {
                errors.push(ConfigError::semantic(format!(
                    "deployment_accepts names `{gap}` twice"
                )));
            }
        }
        if self.deployment_profile == deployment::DeploymentProfile::Sandbox
            && !self.deployment_accepts.is_empty()
        {
            errors.push(ConfigError::semantic(
                "deployment_accepts is set under deployment_profile = \"sandbox\", which accepts \
                 every gap already; the key belongs to a production deployment that runs \
                 without a named separation"
                    .to_owned(),
            ));
        }
    }

    /// The clinical-side privacy policy has to be BUILDABLE, not merely
    /// well-typed: an unknown rule key or an uncompilable pattern would
    /// otherwise leave the scanner silently short a rule while reporting that
    /// the jurisdiction was covered, and a blank pseudonym namespace would
    /// accept an empty `external_ref.namespace` as if it were declared. No
    /// openEHR spec governs configuration — our own design.
    fn validate_privacy(&self, errors: &mut Vec<ConfigError>) {
        if let Err(error) = crate::privacy::PrivacyPolicy::compile(&self.privacy) {
            errors.push(ConfigError::semantic(error.to_string()));
        }
        if self
            .privacy
            .subject_namespaces
            .iter()
            .any(|namespace| namespace.trim().is_empty())
        {
            errors.push(ConfigError::semantic(
                "privacy.subject_namespaces carries a blank entry; every pseudonym namespace is \
                 a name an EHR_STATUS subject reference may claim"
                    .to_owned(),
            ));
        }
    }

    /// `server.system_id` is judged by the SAME validating `UID` constructor
    /// the reader uses.
    ///
    /// It is stamped into every `AUDIT_DETAILS` (`System_id_valid`: "not
    /// `system_id.is_empty`", RM
    /// `docs/UML/classes/org.openehr.rm.common.audit_details.adoc` §Invariants)
    /// and occupies the `creating_system_id` position of every
    /// `OBJECT_VERSION_ID` this CDR mints (BASE
    /// `base_types/master05-identification_package.adoc` §Syntaxes).
    fn validate_system_id(&self, errors: &mut Vec<ConfigError>) {
        if let Err(source) =
            openehr_base::v1_3::base_types::identification::uid::Uid::new(&self.server.system_id)
        {
            errors.push(ConfigError::semantic(format!(
                "server.system_id {:?} is not a legal openEHR `uid` \
                 (iso_oid | uuid | internet_id — BASE master05 §Syntaxes): \
                 {source}. It is stamped into EHR.system_id, \
                 AUDIT_DETAILS.system_id, and the creating_system_id of every \
                 OBJECT_VERSION_ID",
                self.server.system_id
            )));
        }
    }

    /// `server.base_path` names this deployment's API root, and every rule it
    /// breaks is reported at once.
    ///
    /// The last segment is the ITS-REST API version `v1`: the overview locates
    /// the API at a deployment-chosen base followed by that segment
    /// (`ITS-REST/specifications/docs/overview/Resources.md` §Resource
    /// identification — "the request is made to an openEHR API (of version
    /// `v1`), located at `https://openEHRSys.example.com`"), and the REST
    /// policy is single-version. The first segment is `ferroehr`, which is our
    /// own rule and not configurable away. In between, any segment of RFC 3986
    /// unreserved characters is free, with no trailing slash and no empty
    /// segment.
    fn validate_base_path(&self, errors: &mut Vec<ConfigError>) {
        let path = self.server.base_path.as_str();
        let mut refuse = |rule: &str| {
            errors.push(ConfigError::semantic(format!(
                "server.base_path {path:?} {rule}"
            )));
        };

        if !path.starts_with('/') {
            refuse("must start with `/`");
        }
        let trailing_slash = path.len() > 1 && path.ends_with('/');
        if trailing_slash {
            refuse("must not end with `/`");
        }

        let body = path.strip_prefix('/').unwrap_or(path);
        let body = if trailing_slash {
            body.strip_suffix('/').unwrap_or(body)
        } else {
            body
        };
        let segments: Vec<&str> = body.split('/').collect();

        if segments.first() != Some(&"ferroehr") {
            refuse(
                "must begin with the `/ferroehr` segment (the product root is not configurable \
                 away)",
            );
        }
        if segments.last() != Some(&"v1") {
            refuse(
                "must end with the ITS-REST API version segment `v1` (overview Resources.md \
                 §Resource identification)",
            );
        }
        if segments.iter().any(|segment| segment.is_empty()) {
            refuse("must not contain an empty path segment (`//`)");
        }
        for segment in segments
            .iter()
            .filter(|segment| !segment.is_empty() && !segment.chars().all(is_unreserved))
        {
            refuse(&format!(
                "segment {segment:?} must use only RFC 3986 unreserved characters \
                 (A-Z a-z 0-9 - . _ ~)"
            ));
        }
    }

    /// The per-mechanism rules each security section owns.
    ///
    /// Authentication contributes the RFC-grounded shape of each configured
    /// mechanism (issuer, audience, leeway, key entropy, KDF cost floor);
    /// SMART adds the deprecated-grant rule plus everything the discovery
    /// document publishes (endpoint scheme, the RFC 8414 §2 required fields,
    /// PKCE).
    fn validate_mechanisms(&self, errors: &mut Vec<ConfigError>) {
        if let Err(e) = self.auth.validate() {
            errors.push(ConfigError::semantic(format!("auth: {e}")));
        }
        if let Err(e) = self.authz.validate() {
            errors.push(ConfigError::semantic(format!("authz: {e}")));
        }
        if let Err(e) = self.smart.validate() {
            errors.push(ConfigError::semantic(format!("smart: {e}")));
        }
        self.validate_smart_issuer(errors);
    }

    /// The signing section's own rules: a PGP mode needs a key, and a
    /// passphrase is given inline or by file, never both.
    fn validate_signing(&self, errors: &mut Vec<ConfigError>) {
        if matches!(
            self.signing.mode,
            crate::versioning::signature::config::Mode::Pgp
        ) && self.signing.key_path.is_none()
        {
            errors.push(ConfigError::semantic(
                "signing.mode = \"pgp\" requires signing.key_path".to_owned(),
            ));
        }
        if self.signing.key_passphrase.is_some() && self.signing.key_passphrase_file.is_some() {
            errors.push(ConfigError::semantic(
                "set only one of signing.key_passphrase / signing.key_passphrase_file".to_owned(),
            ));
        }
    }

    /// Every secret that can be given inline or by file is given exactly one
    /// way, and the OIDC key material names exactly one source.
    ///
    /// A symmetric secret and a static JWKS are competing explicit key
    /// sources; both configured is a contradiction, never resolved by silent
    /// precedence.
    fn validate_key_sources(&self, errors: &mut Vec<ConfigError>) {
        if let Some(oidc) = &self.auth.oidc {
            if oidc.hmac_secret.is_some() && oidc.hmac_secret_file.is_some() {
                errors.push(ConfigError::semantic(
                    "set only one of auth.oidc.hmac_secret / auth.oidc.hmac_secret_file".to_owned(),
                ));
            }
            if oidc.jwks_json.is_some() && oidc.jwks_json_file.is_some() {
                errors.push(ConfigError::semantic(
                    "set only one of auth.oidc.jwks_json / auth.oidc.jwks_json_file".to_owned(),
                ));
            }
            let hmac_configured = oidc.hmac_secret.is_some() || oidc.hmac_secret_file.is_some();
            let jwks_configured = oidc.jwks_json.is_some() || oidc.jwks_json_file.is_some();
            if hmac_configured && jwks_configured {
                errors.push(ConfigError::semantic(
                    "auth.oidc configures BOTH a symmetric secret (hmac_secret[_file]) and a \
                     static JWKS (jwks_json[_file]) — these are competing key sources; set \
                     exactly one, or neither to use issuer discovery"
                        .to_owned(),
                ));
            }
        }
        if self.multimedia.secret_access_key.is_some()
            && self.multimedia.secret_access_key_file.is_some()
        {
            errors.push(ConfigError::semantic(
                "set only one of multimedia.secret_access_key / \
                 multimedia.secret_access_key_file"
                    .to_owned(),
            ));
        }
    }

    /// The two issuers must agree. `smart.endpoints.issuer` tells third-party
    /// applications where to OBTAIN a token; `auth.oidc.issuer` is what this
    /// server will ACCEPT one from. Configured independently and left
    /// unchecked, a mismatch is silently broken in the most confusing way
    /// available: every app obtains a valid token and every request is
    /// refused as an invalid one. Both values are known here, so it is a boot
    /// error.
    fn validate_smart_issuer(&self, errors: &mut Vec<ConfigError>) {
        if !self.smart.enabled {
            return;
        }
        match (&self.smart.endpoints.issuer, &self.auth.oidc) {
            (_, None) => errors.push(ConfigError::semantic(
                "smart.enabled = true requires an [auth.oidc] issuer: the CDR directs \
                 applications to an authorization server, so it must be able to validate \
                 the tokens they come back with"
                    .to_owned(),
            )),
            (Some(advertised), Some(oidc))
                if advertised.trim().trim_end_matches('/')
                    != oidc.issuer.trim().trim_end_matches('/') =>
            {
                errors.push(ConfigError::semantic(format!(
                    "smart.endpoints.issuer ({advertised:?}) and auth.oidc.issuer ({:?}) \
                     name different authorization servers: applications would obtain tokens \
                     from the first and every request would be refused by the second",
                    oidc.issuer
                )));
            }
            _ => {}
        }
    }

    /// The redacted TOML rendering (secrets show `***`) for `/management/env`
    /// and `ferroehr config check`.
    ///
    /// # Errors
    /// [`ConfigError`] if the tree cannot be serialized to TOML.
    pub fn to_redacted_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self)
            .map_err(|e| ConfigError::semantic(format!("rendering config as TOML: {e}")))
    }

    /// The effective configuration as a redacted JSON tree, the source of the
    /// `GET /admin/config` endpoint and the `/management/env` snapshot the binary
    /// builds at boot. No openEHR spec governs configuration — our own
    /// design/extension.
    ///
    /// Redaction is a property of the leaf type rather than of a key-name scan:
    /// every secret-bearing field is typed [`secret::Secret`], whose
    /// [`Serialize`] emits the fixed [`secret::REDACTED`] placeholder, or
    /// [`secret::SecretUrl`], whose [`Serialize`] masks the URL `userinfo`
    /// component. Serializing `self` therefore yields a tree whose secret leaves
    /// are already masked, so a field cannot leak by being renamed and a secret
    /// nested anywhere is masked by its own type. A correctly typed new secret is
    /// redacted with no change here; one smuggled in as a bare `String` breaks
    /// that property, and the `redacted_json_masks_every_secret_field` test
    /// enumerates the current secret set as the standing backstop. Non-secret
    /// identifiers, such as a Basic user's `username` and `roles`, an OIDC
    /// `issuer`, or `auth.oidc.jwks_json` public verification material, stay
    /// visible.
    ///
    /// # Errors
    /// [`ConfigError`] if the tree cannot be serialized to JSON.
    pub fn to_redacted_json(&self) -> Result<serde_json::Value, ConfigError> {
        serde_json::to_value(self)
            .map_err(|e| ConfigError::semantic(format!("rendering config as JSON: {e}")))
    }
}

/// Whether `c` is an RFC 3986 §2.3 unreserved character, the set a path
/// segment may carry without percent-encoding.
const fn is_unreserved(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')
}

/// The port component of a `host:port` bind string, if parseable.
fn server_bind_port(bind: &str) -> Option<u16> {
    bind.rsplit_once(':').and_then(|(_, p)| p.parse().ok())
}

/// Semantic checks on `[terminology.external]`: enabling it needs a provider,
/// every provider needs its base URL, and every cross-reference must resolve.
/// A dangling reference would degrade silently — a route to a missing provider
/// quietly falls back to the default
/// server, an `oauth2_client` naming a missing client would send
/// unauthenticated requests, and half a mutual-TLS identity would connect
/// without a client certificate — so all three are boot errors (never make a
/// bad value a silent default). The TLS material itself (readable PEM,
/// a certificate where a certificate belongs, a key where a key belongs) is
/// validated when the provider is built, which is also boot time. No openEHR
/// spec governs configuration — our own design.
fn validate_terminology(
    terminology: &crate::service::terminology::config::ExternalTerminologyConfig,
    errors: &mut Vec<ConfigError>,
) {
    if terminology.enabled && terminology.providers.is_empty() {
        errors.push(ConfigError::semantic(
            "terminology.external.enabled = true requires at least one \
             [terminology.external.providers.<name>]"
                .to_owned(),
        ));
    }
    for (key, provider) in &terminology.routes {
        if !terminology.providers.contains_key(provider) {
            errors.push(ConfigError::semantic(format!(
                "terminology.external.routes.\"{key}\" names provider '{provider}', which has no \
                 [terminology.external.providers.{provider}]"
            )));
        }
    }
    for (name, provider) in &terminology.providers {
        validate_terminology_provider(name, provider, terminology, errors);
    }
    for (name, client) in &terminology.oauth2_clients {
        validate_terminology_client(name, client, errors);
    }
}

/// One `[terminology.external.providers.<name>]` block: its base URL, the
/// completeness of any mutual-TLS identity, and the resolvability of its
/// OAuth2 client reference.
///
/// A mutual-TLS identity is a certificate AND its key; half of one would
/// connect with no client certificate at all, which the server rejects at
/// handshake time — a boot error, not a runtime surprise.
fn validate_terminology_provider(
    name: &str,
    provider: &crate::service::terminology::config::FhirProviderConfig,
    terminology: &crate::service::terminology::config::ExternalTerminologyConfig,
    errors: &mut Vec<ConfigError>,
) {
    if provider.url.trim().is_empty() {
        errors.push(ConfigError::semantic(format!(
            "terminology.external.providers.{name}.url must not be empty (the FHIR R4B base \
             URL of the terminology server)"
        )));
    }
    match (&provider.client_cert_path, &provider.client_key_path) {
        (Some(_), None) => errors.push(ConfigError::semantic(format!(
            "terminology.external.providers.{name}.client_cert_path is set without \
             client_key_path (a client certificate needs its private key)"
        ))),
        (None, Some(_)) => errors.push(ConfigError::semantic(format!(
            "terminology.external.providers.{name}.client_key_path is set without \
             client_cert_path (a private key needs its certificate)"
        ))),
        _ => {}
    }
    let Some(client) = provider.oauth2_client.as_deref() else {
        return;
    };
    if !terminology.oauth2_clients.contains_key(client) {
        errors.push(ConfigError::semantic(format!(
            "terminology.external.providers.{name}.oauth2_client names '{client}', which has \
             no [terminology.external.oauth2_clients.{client}]"
        )));
    }
}

/// One `[terminology.external.oauth2_clients.<name>]` block: the
/// client-credentials grant needs a token endpoint, a client id, and a secret.
fn validate_terminology_client(
    name: &str,
    client: &crate::service::terminology::config::TerminologyOauth2Config,
    errors: &mut Vec<ConfigError>,
) {
    if client.token_url.trim().is_empty() {
        errors.push(ConfigError::semantic(format!(
            "terminology.external.oauth2_clients.{name}.token_url must not be empty"
        )));
    }
    if client.client_id.trim().is_empty() {
        errors.push(ConfigError::semantic(format!(
            "terminology.external.oauth2_clients.{name}.client_id must not be empty"
        )));
    }
    if client.client_secret.as_ref().is_none_or(Secret::is_empty)
        && client.client_secret_file.is_none()
    {
        errors.push(ConfigError::semantic(format!(
            "terminology.external.oauth2_clients.{name} requires client_secret or \
             client_secret_file (the client-credentials grant authenticates the client)"
        )));
    }
}

/// Assemble the configuration from explicit inputs — the pure seam every test
/// drives.
///
/// Runs the strict env + file passes, the conventional-alias layering, the
/// layered merge, and `*_file` secret resolution.
///
/// # Errors
/// [`ConfigErrors`] aggregating unknown-key, type, and file-resolution errors.
#[expect(
    clippy::implicit_hasher,
    reason = "a boot-once seam over the process environment: no call site \
              supplies a custom hasher, so the generic would be pure noise"
)]
pub fn assemble(
    file: Option<&Path>,
    env: &HashMap<String, String>,
    overrides: &[(String, String)],
) -> Result<FerroEhrConfig, ConfigErrors> {
    loader::assemble(file, env, overrides)
}

/// Boot loader: a thin process-environment shim over [`assemble`].
///
/// Discovers the config file, snapshots the environment, assembles, and emits
/// the dev-default-DB boot warning.
///
/// # Errors
/// [`ConfigErrors`] on discovery failure or any assembly error.
pub fn load(
    cli_config: Option<&Path>,
    overrides: &[(String, String)],
) -> Result<FerroEhrConfig, ConfigErrors> {
    let env: HashMap<String, String> = std::env::vars().collect();
    let file = loader::discover_file(cli_config, &env)?;
    let config = assemble(file.as_deref(), &env, overrides)?;

    // The dev default DSN is announced prominently at boot so it is never a
    // silent production trap.
    if config.db.is_dev_default() {
        tracing::warn!(
            url = crate::db::DEFAULT_URL,
            "[db].url is the built-in DEVELOPMENT DEFAULT ({}); no file/env/CLI value was \
             supplied. Set db.url (or FERROEHR__DB__URL / DATABASE_URL) for any non-dev \
             deployment — production MUST override it.",
            crate::db::DEFAULT_URL,
        );
    }

    // RFC 8725 §2.2: a symmetric signing key is shared with the authorization
    // server, so this resource server could mint the very tokens it accepts.
    if config
        .auth
        .oidc
        .as_ref()
        .is_some_and(auth::OidcConfig::uses_symmetric_key)
    {
        tracing::warn!(
            "auth.oidc uses a SYMMETRIC signing key (hmac_secret): a DEVELOPMENT posture. The \
             key is shared with the authorization server, so this server can mint the tokens it \
             accepts, and it cannot rotate without a restart. Use the issuer's OIDC discovery \
             document or auth.oidc.jwks_json for any non-dev deployment."
        );
    }
    Ok(config)
}

/// The file discovery order, exposed for `config check`.
///
/// # Errors
/// [`ConfigErrors`] if an explicitly-pointed-at file is missing/unreadable.
#[expect(
    clippy::implicit_hasher,
    reason = "a boot-once seam over the process environment, like `assemble`: \
              no call site supplies a custom hasher"
)]
pub fn discover_file(
    cli_config: Option<&Path>,
    env: &HashMap<String, String>,
) -> Result<Option<PathBuf>, ConfigErrors> {
    loader::discover_file(cli_config, env)
}

/// The `multimedia.endpoint` semantic checks.
///
/// An enabled integration with a blank or scheme-less endpoint would boot clean
/// and then fail on the first `DV_MULTIMEDIA` commit. A `${VAR:-}` compose
/// pass-through and an empty Helm value both produce exactly that, so it is
/// refused at boot where an operator can still act.
fn multimedia_endpoint_errors(
    config: &crate::extensions::multimedia::config::MultimediaConfig,
) -> Vec<ConfigError> {
    if !config.enabled {
        return Vec::new();
    }
    let Some(endpoint) = &config.endpoint else {
        return Vec::new();
    };
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return vec![ConfigError::semantic(
            "multimedia.endpoint is set but empty — give an absolute URL \
             (e.g. http://seaweedfs:8333) or remove the key to use default \
             AWS endpoint resolution"
                .to_owned(),
        )];
    }
    match url::Url::parse(trimmed) {
        // `seaweedfs:8333` parses as a URL (scheme `seaweedfs`, path `8333`),
        // so syntax alone does not catch the common "host:port with no scheme"
        // mistake — an S3 endpoint is http or https, and nothing else reaches
        // a bucket.
        Ok(url) if !matches!(url.scheme(), "http" | "https") => {
            vec![ConfigError::semantic(format!(
                "multimedia.endpoint {trimmed:?} has scheme {:?} — an S3 endpoint \
                 must be http or https (did you mean \"http://{trimmed}\"?)",
                url.scheme()
            ))]
        }
        Ok(_) => Vec::new(),
        Err(e) => vec![ConfigError::semantic(format!(
            "multimedia.endpoint {trimmed:?} is not an absolute URL: {e}"
        ))],
    }
}

#[cfg(test)]
mod tests {
    /// A retention below a jurisdiction's floor is a boot error naming both
    /// numbers (#3242); `0` keeps forever and passes; a jurisdiction with no
    /// registered floor imposes none.
    #[test]
    fn a_retention_below_the_jurisdiction_floor_is_refused() {
        let mut config = FerroEhrConfig::default();
        config.audit.store.retention_days = 30;
        config.privacy.identifier_scan.rules = vec!["nl-bsn".to_owned()];
        let errors = config
            .validate()
            .expect_err("30 days is below the NL floor");
        let text = errors.to_string();
        assert!(
            text.contains("retention_days = 30") && text.contains("1830"),
            "{text}"
        );

        config.audit.store.retention_days = 1830;
        config.validate().expect("the floor itself passes");
        config.audit.store.retention_days = 0;
        config.validate().expect("keep forever passes");

        config.audit.store.retention_days = 30;
        config.privacy.identifier_scan.rules = vec!["no-fodselsnummer".to_owned()];
        config
            .validate()
            .expect("a jurisdiction with no registered floor imposes none");
    }

    /// The German ceiling reaches a deployment only once it declares itself one
    /// of the SGB V § 307 controllers, and then caps the horizon — including
    /// "keep forever", which is the case the floor rule could never produce
    /// (#3346).
    #[test]
    fn the_sgb_v_ceiling_applies_only_to_a_declared_controller() {
        let mut config = FerroEhrConfig::default();
        config.privacy.identifier_scan.rules = vec!["de-kvnr".to_owned()];
        config.audit.store.retention_days = 0;
        config
            .validate()
            .expect("an undeclared deployment carries no ceiling");

        config.audit.store.sgb_v_309_controller = true;
        let errors = config
            .validate()
            .expect_err("keep forever outlasts the three-year period");
        let text = errors.to_string();
        assert!(
            text.contains("keep forever") && text.contains("1095"),
            "{text}"
        );

        config.audit.store.retention_days = 1096;
        let errors = config
            .validate()
            .expect_err("1096 days is above the ceiling");
        assert!(errors.to_string().contains("1095"), "{errors}");

        config.audit.store.retention_days = 1095;
        config.validate().expect("the ceiling itself passes");
    }

    /// A ceiling below a floor is a contradiction no horizon resolves, so it is
    /// refused at boot rather than silently resolved in favour of one of them
    /// (#3346).
    #[test]
    fn a_ceiling_below_a_floor_is_refused() {
        let mut config = FerroEhrConfig::default();
        config.audit.store.sgb_v_309_controller = true;
        config.privacy.identifier_scan.rules = vec!["de-kvnr".to_owned(), "nl-bsn".to_owned()];
        config.audit.store.retention_days = 1095;
        let errors = config
            .validate()
            .expect_err("the DE ceiling is below the NL floor");
        let text = errors.to_string();
        assert!(
            text.contains("ceiling of 1095") && text.contains("floor of 1830"),
            "{text}"
        );
    }

    use assert_fs::prelude::*;

    use super::*;
    use crate::config::authz::AbacParam;

    /// Build an injected env map from `(key, value)` pairs.
    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// A temp file carrying `content`, for the file-source cases.
    fn toml_file(content: &str) -> assert_fs::NamedTempFile {
        let f = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp");
        f.write_str(content).expect("write");
        f
    }

    fn assemble_ok(
        file: Option<&Path>,
        env: &HashMap<String, String>,
        overrides: &[(String, String)],
    ) -> FerroEhrConfig {
        match assemble(file, env, overrides) {
            Ok(c) => c,
            Err(e) => panic!("assemble failed: {e}"),
        }
    }

    fn json(c: &FerroEhrConfig) -> serde_json::Value {
        serde_json::to_value(c).expect("serialize")
    }

    // ── 1. Layering ──────────────────────────────────────────────────────────

    #[test]
    fn defaults_only_boot_equals_default() {
        let c = assemble_ok(None, &env(&[]), &[]);
        assert_eq!(json(&c), json(&FerroEhrConfig::default()));
    }

    #[test]
    fn layering_file_env_set() {
        let file = toml_file("[db]\nmax_connections = 5\n");
        // file overrides default (20).
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        assert_eq!(c.db.max_connections, 5);
        let c = assemble_ok(
            Some(file.path()),
            &env(&[("FERROEHR__DB__MAX_CONNECTIONS", "9")]),
            &[],
        );
        assert_eq!(c.db.max_connections, 9);
        let c = assemble_ok(
            Some(file.path()),
            &env(&[("FERROEHR__DB__MAX_CONNECTIONS", "9")]),
            &[("db.max_connections".to_owned(), "11".to_owned())],
        );
        assert_eq!(c.db.max_connections, 11);
    }

    #[test]
    fn conventional_aliases_lose_to_ferroehr_forms() {
        let c = assemble_ok(None, &env(&[("DATABASE_URL", "postgres://a@h/x")]), &[]);
        assert_eq!(c.db.url.expose(), "postgres://a@h/x");
        let c = assemble_ok(
            None,
            &env(&[
                ("DATABASE_URL", "postgres://a@h/x"),
                ("FERROEHR__DB__URL", "postgres://b@h/y"),
            ]),
            &[],
        );
        assert_eq!(c.db.url.expose(), "postgres://b@h/y");
    }

    #[test]
    fn libpq_convention_assembles_a_dsn_below_the_url_forms() {
        // PGHOST + friends alone assemble the DSN (the libpq environment
        // convention managed-Postgres integrations inject), password
        // percent-encoded.
        let c = assemble_ok(
            None,
            &env(&[
                ("PGHOST", "db.example.neon.tech"),
                ("PGUSER", "demo"),
                ("PGPASSWORD", "p@ss/w"),
                ("PGDATABASE", "ferroehr"),
                ("PGSSLMODE", "require"),
            ]),
            &[],
        );
        assert_eq!(
            c.db.url.expose(),
            "postgres://demo:p%40ss%2Fw@db.example.neon.tech/ferroehr?sslmode=require"
        );
        let c = assemble_ok(
            None,
            &env(&[
                ("PGHOST", "db.example.neon.tech"),
                ("PGUSER", "demo"),
                ("DATABASE_URL", "postgres://a@h/x"),
            ]),
            &[],
        );
        assert_eq!(c.db.url.expose(), "postgres://a@h/x");
        let c = assemble_ok(
            None,
            &env(&[
                ("PGHOST", "db.example.neon.tech"),
                ("DATABASE_URL", "postgres://a@h/x"),
                ("FERROEHR__DB__URL", "postgres://b@h/y"),
            ]),
            &[],
        );
        assert_eq!(c.db.url.expose(), "postgres://b@h/y");
    }

    #[test]
    fn unpooled_endpoints_win_over_pooled_ones() {
        // A transaction-pooled endpoint drops the session search_path, so the
        // direct form outranks the pooled one within the alias layer.
        let c = assemble_ok(
            None,
            &env(&[
                ("DATABASE_URL", "postgres://pooled@h/x"),
                ("DATABASE_URL_UNPOOLED", "postgres://direct@h/x"),
            ]),
            &[],
        );
        assert_eq!(c.db.url.expose(), "postgres://direct@h/x");
        let c = assemble_ok(
            None,
            &env(&[
                ("PGHOST", "pooled.neon.tech"),
                ("PGHOST_UNPOOLED", "direct.neon.tech"),
                ("PGUSER", "demo"),
                ("PGDATABASE", "ferroehr"),
            ]),
            &[],
        );
        assert_eq!(
            c.db.url.expose(),
            "postgres://demo@direct.neon.tech/ferroehr"
        );
    }

    #[test]
    fn port_convention_expands_to_an_all_interfaces_bind() {
        // PORT alone binds server.bind on all interfaces (the container-
        // platform convention: Vercel/Cloud Run/Heroku inject only the port).
        let c = assemble_ok(None, &env(&[("PORT", "3123")]), &[]);
        assert_eq!(c.server.bind, "0.0.0.0:3123");
        let c = assemble_ok(
            None,
            &env(&[
                ("PORT", "3123"),
                ("FERROEHR__SERVER__BIND", "127.0.0.1:9000"),
            ]),
            &[],
        );
        assert_eq!(c.server.bind, "127.0.0.1:9000");
    }

    // ── 2. Mapping ────────────────────────────────────────────────────────────

    #[test]
    fn env_mapping_scalars_maps_and_lists() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__SERVER__MAX_IN_FLIGHT", "64"),
                ("FERROEHR__AUTH__OIDC__ISSUER", "https://idp"),
                ("FERROEHR__AUTH__OIDC__AUDIENCES", "ferroehr,other"),
                ("FERROEHR__AUTH__OIDC__REQUEST_TIMEOUT_MS", "1500"),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__ONTO__URL",
                    "https://onto/fhir",
                ),
            ]),
            &[],
        );
        assert_eq!(c.server.max_in_flight, 64);
        let oidc = c.auth.oidc.expect("oidc table materialised from env");
        assert_eq!(oidc.issuer, "https://idp");
        assert_eq!(
            oidc.audiences,
            vec!["ferroehr".to_owned(), "other".to_owned()]
        );
        // The discovery-client hardening keys: one overridden from the env, the
        // other two at their documented defaults.
        assert_eq!(oidc.request_timeout_ms, 1_500);
        assert_eq!(oidc.connect_timeout_ms, 3_000);
        assert_eq!(oidc.negative_cache_ttl_seconds, 10);
        assert_eq!(
            c.terminology
                .external
                .providers
                .get("onto")
                .expect("onto")
                .url,
            "https://onto/fhir"
        );
    }

    /// EVERY `Vec`-typed key an operator can set through the environment parses
    /// as a list.
    ///
    /// A list-typed key missing from `alias::LIST_KEYS` is not mis-split — it is
    /// REFUSED, `invalid type: string …, expected a sequence`, so its documented
    /// env spelling is dead on arrival. Two shipped keys were in exactly that
    /// state (`signing.retired_key_paths`, `smart.endpoints.capabilities`) and
    /// neither was noticed until a live deployment probe hit the boot refusal:
    /// the value that carries a whole feature is only reachable from a TOML
    /// file, which no test asserted and no reader could tell.
    ///
    /// Single-element values matter as much as multi-element ones: the refusal
    /// fires on the TYPE, so even one path fails.
    #[test]
    fn every_list_typed_key_parses_from_a_single_env_value() {
        let c = assemble_ok(
            None,
            &env(&[
                // The key the signing rotation keyring depends on.
                (
                    "FERROEHR__SIGNING__RETIRED_KEY_PATHS",
                    "/etc/ferroehr/retired-2025.pub.asc",
                ),
                ("FERROEHR__SMART__ENDPOINTS__CAPABILITIES", "launch-ehr"),
            ]),
            &[],
        );
        assert_eq!(
            c.signing.retired_key_paths,
            vec![PathBuf::from("/etc/ferroehr/retired-2025.pub.asc")],
            "a single retired key path must parse as a one-element list"
        );
        assert_eq!(
            c.smart.endpoints.capabilities,
            vec!["launch-ehr".to_owned()]
        );
    }

    /// A list nested under a MAP key is addressable from the environment too,
    /// so it needs registration like any other — `authz.abac.policy.<kind>` is
    /// a map, not an array of tables, and
    /// `terminology.external.providers.<name>` above already proves the env
    /// grammar reaches into one.
    #[test]
    fn a_list_under_a_map_key_parses_from_env() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__AUTHZ__ABAC__POLICY__EHR__NAME", "ehr-policy"),
                (
                    "FERROEHR__AUTHZ__ABAC__POLICY__EHR__PARAMETERS",
                    "patient,template",
                ),
            ]),
            &[],
        );
        let rule = c.authz.abac.policy.get("ehr").expect("the ehr policy rule");
        assert_eq!(rule.name, "ehr-policy");
        assert_eq!(
            rule.parameters,
            vec![AbacParam::Patient, AbacParam::Template],
            "a policy's parameter list must be reachable from the environment"
        );
    }

    /// The same keys with SEVERAL values, since comma-splitting is the other
    /// half of what registration buys.
    #[test]
    fn every_list_typed_key_splits_several_env_values() {
        let c = assemble_ok(
            None,
            &env(&[
                (
                    "FERROEHR__SIGNING__RETIRED_KEY_PATHS",
                    "/keys/a.pub.asc,/keys/b.pub.asc",
                ),
                (
                    "FERROEHR__SMART__ENDPOINTS__CAPABILITIES",
                    "launch-ehr,sso-openid-connect",
                ),
            ]),
            &[],
        );
        assert_eq!(
            c.signing.retired_key_paths,
            vec![
                PathBuf::from("/keys/a.pub.asc"),
                PathBuf::from("/keys/b.pub.asc")
            ]
        );
        assert_eq!(
            c.smart.endpoints.capabilities,
            vec!["launch-ehr".to_owned(), "sso-openid-connect".to_owned()]
        );
    }

    /// The `[auth.oidc]` RFC-posture keys and the ABAC directory switch are
    /// reachable through the one env grammar — a documented env spelling that
    /// binds nothing would ship dead.
    #[test]
    fn env_mapping_auth_and_authz_posture_keys() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__AUTH__OIDC__ISSUER", "https://idp"),
                ("FERROEHR__AUTH__OIDC__AUDIENCES", "ferroehr"),
                ("FERROEHR__AUTH__OIDC__CLOCK_SKEW_LEEWAY_SECONDS", "120"),
                ("FERROEHR__AUTH__OIDC__ALLOW_INSECURE_ISSUER", "true"),
                ("FERROEHR__AUTHZ__ABAC__CHECK_DIRECTORY", "true"),
            ]),
            &[],
        );
        let oidc = c.auth.oidc.expect("oidc table materialised from env");
        assert_eq!(oidc.clock_skew_leeway_seconds, 120);
        assert!(oidc.allow_insecure_issuer);
        assert!(c.authz.abac.check_directory);

        // Unset, both carry their documented defaults.
        let d = assemble_ok(None, &env(&[]), &[]);
        assert!(!d.authz.abac.check_directory);
        assert_eq!(
            auth::OidcConfig::default().clock_skew_leeway_seconds,
            60,
            "the default leeway is the conventional 60 s"
        );
        assert!(!auth::OidcConfig::default().allow_insecure_issuer);
    }

    /// `[server] system_id` — the CDR's own openEHR system identifier (stamped
    /// into `EHR.system_id`, the `AUDIT_DETAILS.system_id` server default, and
    /// every `OBJECT_VERSION_ID.creating_system_id`). Pins all three layers:
    /// the compatibility default, the file value, and the `FERROEHR__` env form.
    #[test]
    fn server_system_id_default_file_and_env() {
        // Unset, the default is the service-layer constant.
        let c = assemble_ok(None, &env(&[]), &[]);
        assert_eq!(c.server.system_id, crate::service::DEFAULT_SYSTEM_ID);

        let file = toml_file("[server]\nsystem_id = \"cdr.hospital.example\"\n");
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        assert_eq!(c.server.system_id, "cdr.hospital.example");

        let c = assemble_ok(
            Some(file.path()),
            &env(&[("FERROEHR__SERVER__SYSTEM_ID", "cdr.env.example")]),
            &[],
        );
        assert_eq!(c.server.system_id, "cdr.env.example");

        // `system_id` and the `[server.identity]` display identity are
        // independent knobs — setting one must not disturb the other.
        assert_eq!(c.server.identity.solution, "FerroEHR");
    }

    // ── 3. Strictness ─────────────────────────────────────────────────────────

    #[test]
    fn unknown_env_var_is_a_boot_error_with_suggestion() {
        let err = assemble(None, &env(&[("FERROEHR__SIGNIN__ENABLED", "true")]), &[])
            .expect_err("unknown var");
        let msg = err.to_string();
        assert!(msg.contains("FERROEHR__SIGNIN__ENABLED"), "{msg}");
        assert!(msg.contains("signing"), "did-you-mean missing: {msg}");
    }

    #[test]
    fn near_miss_prefix_suggests_the_uniform_spelling() {
        // The mixed form (a single `_` after the prefix) does not bind; the
        // sweep names the exact uniform spelling.
        let err = assemble(None, &env(&[("FERROEHR_DB__URL", "postgres://x")]), &[])
            .expect_err("near-miss");
        let msg = err.to_string();
        assert!(msg.contains("FERROEHR_DB__URL"), "{msg}");
        assert!(
            msg.contains("FERROEHR__DB__URL"),
            "suggestion missing: {msg}"
        );
    }

    #[test]
    fn ferroehr_config_pointer_stays_accepted() {
        // `FERROEHR_CONFIG` is a file pointer, not a config key — it keeps its
        // single-`_` spelling and must never be flagged by the strict sweep.
        let c = assemble_ok(
            None,
            &env(&[("FERROEHR_CONFIG", "/etc/ferroehr/ferroehr.toml")]),
            &[],
        );
        assert_eq!(json(&c), json(&FerroEhrConfig::default()));
    }

    #[test]
    fn per_subsystem_config_pointer_is_a_boot_error() {
        // There is ONE configuration file. A per-subsystem `*_CONFIG` pointer
        // is not a recognized name, so it fails as an unknown
        // reserved-namespace variable rather than being read and ignored.
        let err = assemble(None, &env(&[("FERROEHR_SIGNING_CONFIG", "/x.toml")]), &[])
            .expect_err("retired pointer must fail");
        let msg = err.to_string();
        assert!(msg.contains("FERROEHR_SIGNING_CONFIG"), "{msg}");
    }

    #[test]
    fn unknown_file_key_is_rejected() {
        let file = toml_file("[db]\nmax_conections = 5\n");
        let err = assemble(Some(file.path()), &env(&[]), &[]).expect_err("unknown key");
        assert!(err.to_string().contains("max_conections"), "{err}");
    }

    // ── 4. One grammar, no synonyms ─────────────────────────────────────────

    #[test]
    fn single_separator_spelling_is_a_boot_error_naming_the_uniform_form() {
        // A single `_` where the grammar wants `__` is the easiest operator
        // mistake to make, so it fails at boot naming the exact spelling
        // meant — never silently honoured, and never set-but-unread.
        let err = assemble(None, &env(&[("FERROEHR_DB_MAX_CONNECTIONS", "7")]), &[])
            .expect_err("a single-separator spelling must fail");
        let msg = err.to_string();
        assert!(msg.contains("FERROEHR_DB_MAX_CONNECTIONS"), "{msg}");
        assert!(msg.contains("FERROEHR__DB__MAX_CONNECTIONS"), "{msg}");
    }

    // ── 5. Secrets ──────────────────────────────────────────────────────────

    #[test]
    fn secrets_never_render_and_file_resolves() {
        let file = toml_file("[auth.oidc]\nissuer = \"x\"\nhmac_secret = \"topsecret\"\n");
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        let rendered = c.to_redacted_toml().expect("toml");
        assert!(!rendered.contains("topsecret"), "secret leaked: {rendered}");
        assert!(!serde_json::to_string(&c).unwrap().contains("topsecret"));
        let secret = assert_fs::NamedTempFile::new("pass").expect("temp");
        secret.write_str("s3cret\n").expect("write");
        let file = toml_file(&format!(
            "[signing]\nkey_passphrase_file = \"{}\"\n",
            secret.path().display()
        ));
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        assert_eq!(
            c.signing
                .key_passphrase
                .as_ref()
                .expect("resolved")
                .expose(),
            "s3cret"
        );
    }

    #[test]
    fn secret_and_file_both_set_is_rejected() {
        let file =
            toml_file("[signing]\nkey_passphrase = \"a\"\nkey_passphrase_file = \"/dev/null\"\n");
        assert!(assemble(Some(file.path()), &env(&[]), &[]).is_err());
    }

    /// Every credential-bearing key reaches the same configuration through its
    /// `*_file` sibling as it does inline.
    ///
    /// These four had no file route at all, so the most valuable secret in the
    /// deployment — the database DSN — could only travel as an environment value,
    /// readable through `/proc/<pid>/environ` and inherited by every child
    /// process. The assertion is equivalence: the file route is an additional
    /// delivery mechanism, not a second configuration path that might differ.
    #[test]
    fn the_credential_keys_resolve_identically_from_a_file_and_inline() {
        const DSN: &str = "postgres://u:p@db.internal:5432/ferroehr";
        const BROKER: &str = "amqps://u:p@broker.internal:5671/%2f";
        const FHIR_BROKER: &str = "amqps://u:p@fhir-broker.internal:5671/%2f";
        const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2g";

        let secret_file = |name: &str, contents: &str| {
            let f = assert_fs::NamedTempFile::new(name).expect("temp");
            // A trailing newline is what `echo >` and a Kubernetes secret editor
            // both produce, so the reader must trim it.
            f.write_str(&format!("{contents}\n")).expect("write");
            f
        };

        let dsn = secret_file("dsn", DSN);
        let broker = secret_file("broker", BROKER);
        let fhir_broker = secret_file("fhir-broker", FHIR_BROKER);
        let hash = secret_file("hash", HASH);

        let from_files = toml_file(&format!(
            "[db]\nurl_file = \"{}\"\n\
             [events]\nurl_file = \"{}\"\n\
             [fhir.outbound]\nurl_file = \"{}\"\n\
             [[auth.basic.users]]\nusername = \"alice\"\npassword_hash_file = \"{}\"\n",
            dsn.path().display(),
            broker.path().display(),
            fhir_broker.path().display(),
            hash.path().display(),
        ));
        let via_file = assemble_ok(Some(from_files.path()), &env(&[]), &[]);

        let inline = toml_file(&format!(
            "[db]\nurl = \"{DSN}\"\n\
             [events]\nurl = \"{BROKER}\"\n\
             [fhir.outbound]\nurl = \"{FHIR_BROKER}\"\n\
             [[auth.basic.users]]\nusername = \"alice\"\npassword_hash = \"{HASH}\"\n"
        ));
        let via_inline = assemble_ok(Some(inline.path()), &env(&[]), &[]);

        assert_eq!(via_file.db.url.expose(), DSN);
        assert_eq!(via_file.db.url.expose(), via_inline.db.url.expose());
        assert_eq!(via_file.events.url.expose(), BROKER);
        assert_eq!(via_file.fhir.outbound.url.expose(), FHIR_BROKER);
        let user = &via_file.auth.basic.as_ref().expect("basic").users[0];
        assert_eq!(user.password_hash.expose(), HASH);

        // And a file-delivered DSN redacts exactly like an inline one — the
        // whole point of the `SecretUrl` type is that redaction is a property of
        // the type rather than of how the value arrived.
        let rendered = via_file.to_redacted_toml().expect("toml");
        assert!(
            !rendered.contains("u:p@"),
            "a file-loaded DSN must redact its credentials: {rendered}"
        );
        assert!(
            !serde_json::to_string(&via_file)
                .expect("json")
                .contains(HASH),
            "a file-loaded password hash must not render"
        );
    }

    /// Setting a credential both inline and as a file is refused for each of the
    /// seven, rather than one silently winning.
    ///
    /// Each of these fields has a non-empty dev default, so "the operator set it"
    /// means "it differs from that default" — the default itself must NOT count
    /// as a conflicting value, or the file route would be unusable.
    #[test]
    fn a_credential_set_both_inline_and_as_a_file_is_refused() {
        let cases = [
            "[db]\nurl = \"postgres://u:p@h:5432/d\"\nurl_file = \"/dev/null\"\n",
            "[db]\nmigrate_url = \"postgres://u:p@h:5432/d\"\n\
             migrate_url_file = \"/dev/null\"\n",
            "[storage.party]\nurl = \"postgres://u:p@h:5432/d\"\n\
             url_file = \"/dev/null\"\n",
            "[storage.linkage]\nurl = \"postgres://u:p@h:5432/d\"\n\
             url_file = \"/dev/null\"\n",
            "[events]\nurl = \"amqp://u:p@h:5672/%2f\"\nurl_file = \"/dev/null\"\n",
            "[fhir.outbound]\nurl = \"amqp://u:p@h:5672/%2f\"\nurl_file = \"/dev/null\"\n",
            "[[auth.basic.users]]\nusername = \"a\"\npassword_hash = \"x\"\n\
             password_hash_file = \"/dev/null\"\n",
        ];
        for case in cases {
            let file = toml_file(case);
            let outcome = assemble(Some(file.path()), &env(&[]), &[]);
            assert!(
                outcome.is_err(),
                "both-set must be refused, but this was accepted: {case}"
            );
        }
    }

    /// The URL file routes are reachable through the environment grammar too,
    /// which is how a container passes a mount path without a config file.
    ///
    /// Without this the documented env form could ship dead — the whole reason
    /// every section carries an env-mapping test.
    #[test]
    fn the_url_file_keys_are_reachable_through_the_env_grammar() {
        let dsn = assert_fs::NamedTempFile::new("dsn").expect("temp");
        dsn.write_str("postgres://u:p@h:5432/d\n").expect("write");
        let broker = assert_fs::NamedTempFile::new("broker").expect("temp");
        broker.write_str("amqps://u:p@b:5671/%2f\n").expect("write");
        // A DIFFERENT DSN, so the assertion below cannot pass by falling back
        // to `db.url` — which is what an unwired `migrate_url_file` would do.
        let migrate = assert_fs::NamedTempFile::new("migrate-dsn").expect("temp");
        migrate
            .write_str("postgres://m:p@h:5432/d\n")
            .expect("write");

        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__DB__URL_FILE", &dsn.path().display().to_string()),
                (
                    "FERROEHR__DB__MIGRATE_URL_FILE",
                    &migrate.path().display().to_string(),
                ),
                (
                    "FERROEHR__EVENTS__URL_FILE",
                    &broker.path().display().to_string(),
                ),
                (
                    "FERROEHR__FHIR__OUTBOUND__URL_FILE",
                    &broker.path().display().to_string(),
                ),
                (
                    "FERROEHR__STORAGE__PARTY__URL_FILE",
                    &migrate.path().display().to_string(),
                ),
            ]),
            &[],
        );
        assert_eq!(c.db.url.expose(), "postgres://u:p@h:5432/d");
        assert_eq!(c.db.migrate_dsn(), "postgres://m:p@h:5432/d");
        assert_eq!(
            c.storage.dsn(crate::db::domain::Domain::Party, &c.db),
            "postgres://m:p@h:5432/d",
            "the party domain's own DSN arrives through the env grammar, rather than \
             falling back to db.url as an unwired key would"
        );
        assert_eq!(c.events.url.expose(), "amqps://u:p@b:5671/%2f");
        assert_eq!(c.fhir.outbound.url.expose(), "amqps://u:p@b:5671/%2f");
    }

    /// The dev default is not a setting, so a `*_file` alone works without the
    /// operator having to blank the default first.
    #[test]
    fn a_file_route_works_against_the_untouched_default() {
        let dsn = assert_fs::NamedTempFile::new("dsn").expect("temp");
        dsn.write_str("postgres://u:p@h:5432/d").expect("write");
        let file = toml_file(&format!(
            "[db]\nurl_file = \"{}\"\nmax_connections = 7\n",
            dsn.path().display()
        ));
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        assert_eq!(c.db.url.expose(), "postgres://u:p@h:5432/d");
        assert_eq!(c.db.max_connections, 7);
    }

    /// `to_redacted_json` masks EVERY secret-bearing leaf in the whole config
    /// tree — the body `GET /admin/config` returns. Each secret is populated
    /// with a unique high-entropy sentinel; none may appear in the rendered
    /// JSON, while non-secret siblings (a Basic user's `username`/`roles`) stay
    /// visible. This is the standing enumeration of the current secret set: a
    /// new secret field added without redaction is caught here once wired
    /// into the fixture.
    #[test]
    fn redacted_json_masks_every_secret_field() {
        use crate::config::auth::{BasicConfig, BasicUser, OidcConfig};
        use crate::config::secret::{Secret, SecretUrl};

        // Each sentinel is unique so a leak is unambiguously attributable.
        const DB_PW: &str = "DB_PW_SENTINEL_9a1c";
        const BASIC_HASH: &str = "$argon2id$BASIC_HASH_SENTINEL_7b2d";
        const HMAC: &str = "HMAC_SENTINEL_4e3f";
        const PASSPHRASE: &str = "PASSPHRASE_SENTINEL_1d5a";
        const S3_KEY: &str = "S3_SECRET_SENTINEL_8c6b";
        const EVENTS_PW: &str = "EVENTS_PW_SENTINEL_2f7e";
        const FHIR_PW: &str = "FHIR_PW_SENTINEL_6a9d";

        let mut c = FerroEhrConfig::default();
        c.db.url = SecretUrl::new(format!(
            "postgres://dbuser:{DB_PW}@db.internal:5432/ferroehr"
        ));
        c.auth.basic = Some(BasicConfig {
            users: vec![BasicUser {
                username: "alice".to_owned(),
                password_hash: Secret::new(BASIC_HASH),
                password_hash_file: None,
                roles: vec!["ADMIN".to_owned()],
            }],
        });
        c.auth.oidc = Some(OidcConfig {
            issuer: "https://idp.example".to_owned(),
            hmac_secret: Some(Secret::new(HMAC)),
            ..OidcConfig::default()
        });
        c.signing.key_passphrase = Some(Secret::new(PASSPHRASE));
        c.multimedia.secret_access_key = Some(Secret::new(S3_KEY));
        c.multimedia.access_key_id = Some("AKIA_PUBLIC_ID".to_owned());
        c.events.url = SecretUrl::new(format!("amqp://mq:{EVENTS_PW}@broker:5672/vh"));
        c.fhir.outbound.url = SecretUrl::new(format!("amqps://fhir:{FHIR_PW}@bus:5671/vh"));

        let value = c.to_redacted_json().expect("render redacted json");
        let rendered = serde_json::to_string(&value).expect("stringify");

        for sentinel in [
            DB_PW, BASIC_HASH, HMAC, PASSPHRASE, S3_KEY, EVENTS_PW, FHIR_PW,
        ] {
            assert!(
                !rendered.contains(sentinel),
                "secret leaked into GET /admin/config body: {sentinel} in {rendered}"
            );
        }

        // Structural placeholders present where a secret was set.
        assert_eq!(value["auth"]["basic"]["users"][0]["password_hash"], "***");
        assert_eq!(value["auth"]["oidc"]["hmac_secret"], "***");
        assert_eq!(value["signing"]["key_passphrase"], "***");
        assert_eq!(value["multimedia"]["secret_access_key"], "***");
        assert_eq!(
            value["db"]["url"],
            "postgres://***@db.internal:5432/ferroehr"
        );
        assert_eq!(value["events"]["url"], "amqp://***@broker:5672/vh");
        assert_eq!(value["fhir"]["outbound"]["url"], "amqps://***@bus:5671/vh");

        // Non-secret identifiers stay visible (they are not credentials).
        assert_eq!(value["auth"]["basic"]["users"][0]["username"], "alice");
        assert_eq!(value["auth"]["basic"]["users"][0]["roles"][0], "ADMIN");
        assert_eq!(value["auth"]["oidc"]["issuer"], "https://idp.example");
        assert_eq!(value["multimedia"]["access_key_id"], "AKIA_PUBLIC_ID");
    }

    /// The `[privacy]` section is reachable from the environment, both scalars
    /// and both lists, and its semantic validation runs.
    #[test]
    fn the_privacy_section_maps_from_the_environment() {
        let c = assemble_ok(
            None,
            &env(&[
                (
                    "FERROEHR__PRIVACY__SUBJECT_NAMESPACES",
                    "urn:ferroehr:pseudonym,mpi.example",
                ),
                ("FERROEHR__PRIVACY__ALLOW_IDENTIFIED_PARTIES_IN_EHR", "true"),
                ("FERROEHR__PRIVACY__IDENTIFIER_SCAN__MODE", "warn"),
                (
                    "FERROEHR__PRIVACY__IDENTIFIER_SCAN__RULES",
                    "nl-bsn,gb-nhs-number",
                ),
                (
                    "FERROEHR__PRIVACY__IDENTIFIER_SCAN__PATTERNS",
                    r"MRN-[0-9]{6},PAT[0-9]{8}",
                ),
            ]),
            &[],
        );
        assert_eq!(
            c.privacy.subject_namespaces,
            vec![
                "urn:ferroehr:pseudonym".to_owned(),
                "mpi.example".to_owned()
            ]
        );
        assert!(c.privacy.allow_identified_parties_in_ehr);
        assert_eq!(
            c.privacy.identifier_scan.mode,
            crate::privacy::config::ScanMode::Warn
        );
        assert_eq!(
            c.privacy.identifier_scan.rules,
            vec!["nl-bsn".to_owned(), "gb-nhs-number".to_owned()]
        );
        assert_eq!(
            c.privacy.identifier_scan.patterns,
            vec!["MRN-[0-9]{6}".to_owned(), "PAT[0-9]{8}".to_owned()]
        );
        c.validate()
            .expect("a well-formed privacy section validates");
    }

    /// The national-identifier protection section maps from the environment,
    /// and the key never renders.
    #[test]
    fn the_demographic_section_maps_from_the_environment() {
        let c = assemble_ok(
            None,
            &env(&[
                (
                    "FERROEHR__DEMOGRAPHIC__IDENTIFIER_PROTECTION__ENABLED",
                    "true",
                ),
                (
                    "FERROEHR__DEMOGRAPHIC__IDENTIFIER_PROTECTION__SCHEMES",
                    "nl-bsn,se-personnummer",
                ),
                (
                    "FERROEHR__DEMOGRAPHIC__IDENTIFIER_PROTECTION__KEY",
                    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                ),
            ]),
            &[],
        );
        let protection = &c.demographic.identifier_protection;
        assert!(protection.enabled);
        assert_eq!(
            protection.schemes,
            vec!["nl-bsn".to_owned(), "se-personnummer".to_owned()]
        );
        assert!(protection.key.is_some(), "the root key binds from the env");
        // The key is a secret: rendering the effective configuration must not
        // print it, which is the property the admin config endpoint relies on.
        let rendered = serde_json::to_string(&c).expect("the config renders");
        assert!(
            !rendered.contains("000102030405"),
            "the root key must never render into the effective configuration"
        );
    }

    /// A pattern that does not compile is a BOOT error naming the pattern, not
    /// a scanner that silently runs one rule short.
    #[test]
    fn an_uncompilable_scan_pattern_is_a_boot_error() {
        let c = assemble_ok(
            None,
            &env(&[("FERROEHR__PRIVACY__IDENTIFIER_SCAN__PATTERNS", "[unclosed")]),
            &[],
        );
        let errors = c
            .validate()
            .expect_err("an invalid pattern must refuse boot");
        assert!(
            errors.to_string().contains("[unclosed"),
            "the refusal must name the pattern: {errors}"
        );
    }

    /// An identifier rule this build does not ship is a BOOT error listing the
    /// rules it does — never a jurisdiction the operator believes is covered
    /// and is not.
    #[test]
    fn an_unknown_identifier_rule_is_a_boot_error_listing_the_shipped_rules() {
        let c = assemble_ok(
            None,
            &env(&[(
                "FERROEHR__PRIVACY__IDENTIFIER_SCAN__RULES",
                "nl-bsn,zz-invented",
            )]),
            &[],
        );
        let errors = c
            .validate()
            .expect_err("an unknown rule key must refuse boot");
        let rendered = errors.to_string();
        assert!(rendered.contains("zz-invented"), "{rendered}");
        assert!(rendered.contains("gb-nhs-number"), "{rendered}");
    }

    /// The shipped default activates every rule the build carries, derived from
    /// the registry rather than restated — a rule added without reaching the
    /// default would protect nobody.
    #[test]
    fn the_default_rule_set_is_every_shipped_rule() {
        let default: Vec<String> = crate::privacy::detect::built_in_rules()
            .iter()
            .map(|rule| rule.key.to_owned())
            .collect();
        assert_eq!(
            FerroEhrConfig::default().privacy.identifier_scan.rules,
            default
        );
    }

    /// A blank pseudonym namespace would accept an empty
    /// `external_ref.namespace` as if a deployment had declared it.
    #[test]
    fn a_blank_pseudonym_namespace_is_a_boot_error() {
        let c = assemble_ok(
            None,
            &env(&[("FERROEHR__PRIVACY__SUBJECT_NAMESPACES", "  ")]),
            &[],
        );
        let errors = c
            .validate()
            .expect_err("a blank namespace must refuse boot");
        assert!(
            errors.to_string().contains("privacy.subject_namespaces"),
            "the refusal must name the key: {errors}"
        );
    }

    /// A cohort predicate the deployment cannot reach is a boot error, not an
    /// empty cohort: an unbindable key, a clinical archetype, a node that is
    /// not an at-code, and a kind that does not match its key would each match
    /// nothing while reporting a population of zero.
    #[test]
    fn an_unreachable_cohort_predicate_is_a_boot_error() {
        use crate::service::linkage::cohort::config::{PredicateBinding, PredicateKind};

        let with = |key: &str, binding: PredicateBinding| {
            let mut c = FerroEhrConfig::default();
            c.cohort.predicates.insert(key.to_owned(), binding);
            c
        };
        let good = || PredicateBinding {
            archetype: "openEHR-DEMOGRAPHIC-ADDRESS.address.v1".to_owned(),
            node: "at0012".to_owned(),
            kind: PredicateKind::Text,
        };

        // The five bindable keys, correctly bound, boot.
        assert!(with("city", good()).validate().is_ok());
        assert!(
            with(
                "age_band",
                PredicateBinding {
                    archetype: "openEHR-DEMOGRAPHIC-PERSON.person.v1".to_owned(),
                    node: "at0010".to_owned(),
                    kind: PredicateKind::BirthDate,
                }
            )
            .validate()
            .is_ok()
        );

        for (key, binding, expected) in [
            ("national_identifier", good(), "national_identifier"),
            (
                "city",
                PredicateBinding {
                    kind: PredicateKind::BirthDate,
                    ..good()
                },
                "birth_date",
            ),
            (
                "age_band",
                PredicateBinding {
                    node: "at0010".to_owned(),
                    archetype: "openEHR-DEMOGRAPHIC-PERSON.person.v1".to_owned(),
                    kind: PredicateKind::Text,
                },
                "birth_date",
            ),
            (
                "city",
                PredicateBinding {
                    archetype: "openEHR-EHR-OBSERVATION.blood_pressure.v2".to_owned(),
                    ..good()
                },
                "archetype",
            ),
            (
                "city",
                PredicateBinding {
                    node: "value/value".to_owned(),
                    ..good()
                },
                "node",
            ),
        ] {
            let errors = with(key, binding)
                .validate()
                .expect_err("an unreachable predicate must refuse boot");
            let rendered = errors.to_string();
            assert!(
                rendered.contains("cohort.predicates") && rendered.contains(expected),
                "the refusal must name the key and the defect: {rendered}"
            );
        }
    }

    // ── 6. Template sync ──────────────────────────────────────────────────────

    #[test]
    fn template_parses_to_default() {
        let file = toml_file(DEFAULT_TEMPLATE);
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        assert_eq!(json(&c), json(&FerroEhrConfig::default()));
    }

    #[test]
    fn template_mentions_every_section() {
        for section in alias::SECTIONS {
            let header = format!("[{section}]");
            let dotted = format!("[{section}."); // sub-tables count too
            let scalar = format!("\n{section} = "); // top-level scalar keys
            assert!(
                DEFAULT_TEMPLATE.contains(&header)
                    || DEFAULT_TEMPLATE.contains(&dotted)
                    || DEFAULT_TEMPLATE.contains(&scalar),
                "template missing section {section}"
            );
        }
    }

    /// Whether the template declares `key` as a key line — live (`key = …`) or
    /// as a `#?` reference line for an optional/secret/derived one. A mention
    /// inside another key's trailing comment does not count: a key mentioned
    /// only that way stays invisible to `ferroehr config default`.
    fn template_declares(key: &str) -> bool {
        DEFAULT_TEMPLATE.lines().any(|line| {
            let line = line.trim_start();
            let line = line.strip_prefix("#?").map_or(line, str::trim_start);
            line.strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        })
    }

    /// Every field the default config SERIALIZES must be a key in the template.
    ///
    /// Total for concrete fields and needs no hand-maintained list: it walks the
    /// serialized default, so a newly added field is covered the moment it
    /// exists. `Option::None` fields serialize away and are covered by
    /// [`template_declares_every_optional_oidc_field`] instead.
    #[test]
    fn template_declares_every_serialized_field() {
        fn walk(value: &serde_json::Value, missing: &mut Vec<String>) {
            let serde_json::Value::Object(map) = value else {
                return;
            };
            for (key, child) in map {
                match child {
                    serde_json::Value::Object(_) => walk(child, missing),
                    // An absent optional serializes to null, so this walk cannot
                    // see it at all — that class is the other test's job. An
                    // absent optional SUB-TABLE lands here too and is declared
                    // as a `[section]` header rather than a key line.
                    serde_json::Value::Null => {}
                    _ if template_declares(key) => {}
                    _ if DEFAULT_TEMPLATE.contains(&format!(".{key}]")) => {}
                    _ => missing.push(key.clone()),
                }
            }
        }
        let mut missing = Vec::new();
        walk(&json(&FerroEhrConfig::default()), &mut missing);
        assert!(
            missing.is_empty(),
            "the shipped ferroehr.toml template declares no key line for: {}. \
             Every field belongs in it — the file's own header calls itself the \
             complete configuration, and `ferroehr config default` is how an \
             operator discovers a key.",
            missing.join(", ")
        );
    }

    /// The field names a `deny_unknown_fields` struct accepts, taken from serde
    /// itself: an unknown key is refused with `expected one of …`, which is the
    /// authoritative set and needs no hand-maintained list.
    fn accepted_fields<T: serde::de::DeserializeOwned>(section: &str) -> Vec<String> {
        let Err(err) = toml::from_str::<T>("__probe_unknown_key__ = 1") else {
            panic!("[{section}] must carry deny_unknown_fields")
        };
        let err = err.to_string();
        let Some((_, list)) = err.split_once("expected one of ") else {
            panic!("serde no longer reports an accepted-field list for [{section}]: {err}")
        };
        let fields: Vec<String> = list
            .split([',', '`'])
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        // Without this the test passes vacuously if serde ever reformats that
        // message — a guard that silently checks nothing is worse than none.
        assert!(
            !fields.is_empty(),
            "extracted no fields for [{section}]; serde's message format changed \
             and this test is no longer checking anything: {err}"
        );
        fields
    }

    /// Optional fields serialize away when `None`, so the walk above cannot see
    /// them, which is the class an operator can least afford to have
    /// undeclared.
    ///
    /// Covered here are the sections carrying secret or file-indirection keys,
    /// where an undiscoverable key costs an operator most. Adding another
    /// section is one line, because the field list comes from serde.
    #[test]
    fn template_declares_every_optional_field_of_the_secret_bearing_sections() {
        let sections = [
            (
                "auth.oidc",
                accepted_fields::<auth::OidcConfig>("auth.oidc"),
            ),
            ("db", accepted_fields::<crate::db::DbConfig>("db")),
            (
                "signing",
                accepted_fields::<crate::versioning::signature::config::SigningConfig>("signing"),
            ),
            (
                "multimedia",
                accepted_fields::<crate::extensions::multimedia::config::MultimediaConfig>(
                    "multimedia",
                ),
            ),
        ];
        let missing: Vec<String> = sections
            .iter()
            .flat_map(|(section, fields)| {
                fields
                    .iter()
                    .filter(|f| !template_declares(f))
                    .map(move |f| format!("{section}.{f}"))
            })
            .collect();
        assert!(
            missing.is_empty(),
            "the template declares no key line for: {}",
            missing.join(", ")
        );
    }

    /// An enabled multimedia integration with a blank or relative endpoint is a
    /// BOOT error, not a panic on the first commit.
    #[test]
    fn an_enabled_multimedia_integration_refuses_a_blank_or_relative_endpoint() {
        for bad in ["", "   ", "seaweedfs:8333", "/bucket"] {
            let mut c = FerroEhrConfig::default();
            c.multimedia.enabled = true;
            c.multimedia.endpoint = Some(bad.to_owned());
            let errors = c.validate().expect_err("a bad endpoint must be refused");
            assert!(
                format!("{errors:?}").contains("multimedia.endpoint"),
                "the refusal must name the key so an operator can act on it, got: {errors:?}"
            );
        }
    }

    /// An absent endpoint is legitimate (default AWS resolution) and an
    /// absolute one is accepted, so the check above cannot be a blanket refusal.
    #[test]
    fn an_absent_or_absolute_multimedia_endpoint_is_accepted() {
        let mut c = FerroEhrConfig::default();
        c.multimedia.enabled = true;
        c.multimedia.endpoint = None;
        assert!(c.validate().is_ok(), "an absent endpoint is legitimate");
        c.multimedia.endpoint = Some("http://seaweedfs:8333".to_owned());
        assert!(c.validate().is_ok(), "an absolute endpoint is accepted");
    }

    // ── 7. Semantic validation ────────────────────────────────────────────────

    #[test]
    fn validate_pgp_requires_key_path() {
        let mut c = FerroEhrConfig::default();
        c.signing.mode = crate::versioning::signature::config::Mode::Pgp;
        assert!(c.validate().is_err());
        c.signing.key_path = Some(PathBuf::from("/k.asc"));
        assert!(c.validate().is_ok());
    }

    /// `smart.endpoints.issuer` tells applications where to OBTAIN a token;
    /// `auth.oidc.issuer` is what this server will ACCEPT one from. A mismatch is
    /// silently broken in the most confusing way available — every app obtains a
    /// valid token and every request is refused as invalid — so it is a boot
    /// error naming both keys.
    #[test]
    fn a_smart_issuer_that_disagrees_with_the_oidc_issuer_is_refused() {
        let publishable_smart = |issuer: &str| smart::SmartConfig {
            enabled: true,
            public_base_url: Some("https://cdr.example.com".to_owned()),
            endpoints: smart::SmartEndpoints {
                issuer: Some(issuer.to_owned()),
                authorization_endpoint: Some("https://as.example/authorize".to_owned()),
                token_endpoint: Some("https://as.example/token".to_owned()),
                response_types_supported: vec!["code".to_owned()],
                token_endpoint_auth_methods_supported: vec!["client_secret_basic".to_owned()],
                code_challenge_methods_supported: vec!["S256".to_owned()],
                ..smart::SmartEndpoints::default()
            },
            ..smart::SmartConfig::default()
        };
        let oidc = |issuer: &str| auth::OidcConfig {
            issuer: issuer.to_owned(),
            audiences: vec!["ferroehr".to_owned()],
            jwks_json: Some("{\"keys\":[]}".to_owned()),
            ..auth::OidcConfig::default()
        };

        let tree = |smart_issuer: &str, oidc_issuer: Option<&str>| FerroEhrConfig {
            smart: publishable_smart(smart_issuer),
            auth: auth::AuthConfig {
                oidc: oidc_issuer.map(oidc),
                ..auth::AuthConfig::default()
            },
            ..FerroEhrConfig::default()
        };

        let err = tree(
            "https://as.example/realms/ferroehr",
            Some("https://other-as.example/realms/ferroehr"),
        )
        .validate()
        .expect_err("two different authorization servers must be refused");
        let text = format!("{err:?}");
        assert!(text.contains("smart.endpoints.issuer"), "{text}");
        assert!(text.contains("auth.oidc.issuer"), "{text}");

        // Agreeing issuers — a trailing slash is the same identity.
        assert!(
            tree(
                "https://as.example/realms/ferroehr/",
                Some("https://as.example/realms/ferroehr"),
            )
            .validate()
            .is_ok(),
            "a trailing slash is not a mismatch",
        );

        // SMART enabled with no bearer validation at all: the CDR would send apps
        // to an authorization server whose tokens it cannot check.
        let err = tree("https://as.example/realms/ferroehr", None)
            .validate()
            .expect_err("SMART without [auth.oidc] must refuse");
        assert!(format!("{err:?}").contains("auth.oidc"), "{err:?}");
    }

    #[test]
    fn validate_hmac_and_jwks_together_is_refused() {
        let mut c = FerroEhrConfig::default();
        c.auth.oidc = Some(auth::OidcConfig {
            issuer: "https://idp.example.test".to_owned(),
            audiences: vec!["ferroehr".to_owned()],
            // `HS256`, because the fixture carries a symmetric secret and the
            // algorithm set is boot-bound to its key source.
            algorithms: vec!["HS256".to_owned()],
            hmac_secret: Some(Secret::new("0123456789abcdef0123456789abcdef")),
            jwks_json: Some("{\"keys\":[]}".to_owned()),
            ..auth::OidcConfig::default()
        });
        let err = c.validate().expect_err("competing key sources must refuse");
        assert!(err.to_string().contains("competing key sources"), "{err}");
        // Each source alone stays valid.
        if let Some(oidc) = c.auth.oidc.as_mut() {
            oidc.jwks_json = None;
        }
        assert!(c.validate().is_ok());
    }

    /// The `[auth.oidc]` boot rules reach the aggregated tree validation, so
    /// `ferroehr config check` and the boot path refuse the same shapes the
    /// section's own unit tests pin (RFC 7519 §4.1.3, RFC 8414 §2,
    /// RFC 8725 §3.5, RFC 9068 §4 step 6).
    #[test]
    fn validate_reports_auth_section_errors() {
        let mut c = FerroEhrConfig::default();
        c.auth.oidc = Some(auth::OidcConfig {
            issuer: "http://idp.example.test".to_owned(),
            ..auth::OidcConfig::default()
        });
        let err = c.validate().expect_err("an http issuer must refuse");
        assert!(err.to_string().contains("auth: auth.oidc.issuer"), "{err}");

        if let Some(oidc) = c.auth.oidc.as_mut() {
            oidc.issuer = "https://idp.example.test".to_owned();
        }
        let err = c.validate().expect_err("no audiences must refuse");
        assert!(err.to_string().contains("auth.oidc.audiences"), "{err}");

        if let Some(oidc) = c.auth.oidc.as_mut() {
            oidc.audiences = vec!["ferroehr".to_owned()];
        }
        assert!(c.validate().is_ok());
    }

    #[test]
    fn validate_management_port_must_differ_from_bind() {
        let mut c = FerroEhrConfig::default();
        c.server.bind = "0.0.0.0:8080".to_owned();
        c.management.port = Some(8080);
        assert!(c.validate().is_err());
        c.management.port = Some(9100);
        assert!(c.validate().is_ok());
    }

    /// The default base path and every shortened shape an operator may reach
    /// for satisfy the rules.
    #[test]
    fn validate_accepts_the_default_and_shortened_base_paths() {
        for base_path in [
            "/ferroehr/rest/openehr/v1",
            "/ferroehr/v1",
            "/ferroehr/openehr/v1",
            "/ferroehr/cdr/v1",
            "/ferroehr/rest-api/openehr-1.2.0/v1",
        ] {
            let mut c = FerroEhrConfig::default();
            c.server.base_path = base_path.to_owned();
            assert!(c.validate().is_ok(), "{base_path} must be accepted");
        }
    }

    /// Each rule refuses on its own, naming the key and the rule it broke.
    #[test]
    fn validate_refuses_a_malformed_base_path() {
        for (base_path, expected) in [
            ("ferroehr/rest/openehr/v1", "must start with `/`"),
            ("/ferroehrx/v1", "must begin with the `/ferroehr` segment"),
            ("/x/ferroehr/v1", "must begin with the `/ferroehr` segment"),
            (
                "/ferroehr",
                "must end with the ITS-REST API version segment",
            ),
            (
                "/ferroehr/v2",
                "must end with the ITS-REST API version segment",
            ),
            ("/ferroehr/v1/", "must not end with `/`"),
            ("/ferroehr//v1", "must not contain an empty path segment"),
            ("/ferroehr/re st/v1", "RFC 3986 unreserved characters"),
        ] {
            let mut c = FerroEhrConfig::default();
            c.server.base_path = base_path.to_owned();
            let err = c
                .validate()
                .expect_err("a malformed base path must be refused")
                .to_string();
            assert!(err.contains("server.base_path"), "{base_path}: {err}");
            assert!(err.contains(expected), "{base_path}: {err}");
        }
    }

    /// Validation is aggregated: a value breaking several rules reports them
    /// all in one boot error, so an operator fixes the key in one iteration.
    #[test]
    fn validate_reports_every_broken_base_path_rule_at_once() {
        let mut c = FerroEhrConfig::default();
        c.server.base_path = "cdr//api/".to_owned();
        let err = c
            .validate()
            .expect_err("a multiply-malformed base path must be refused")
            .to_string();
        for expected in [
            "must start with `/`",
            "must not end with `/`",
            "must begin with the `/ferroehr` segment",
            "must end with the ITS-REST API version segment",
            "must not contain an empty path segment",
        ] {
            assert!(err.contains(expected), "{expected} missing from: {err}");
        }
    }

    /// `server.system_id` is judged by the openEHR `uid` grammar itself.
    ///
    /// It occupies the `creating_system_id` position of every
    /// `OBJECT_VERSION_ID` this CDR mints — "`creating_system_id` = `uid`" with
    /// "`uid` = `iso_oid` | `uuid` | `internet_id`" (BASE
    /// `base_types/master05-identification_package.adoc` §Syntaxes) — and is
    /// stamped into `AUDIT_DETAILS.system_id`, whose RM invariant
    /// `System_id_valid` forbids an empty value. Both the accepted and the
    /// refused shapes are pinned, including the ones a non-empty/no-`::` check
    /// would have let through into unmintable version ids.
    #[test]
    fn validate_system_id_is_a_legal_uid() {
        let mut c = FerroEhrConfig::default();
        assert!(c.validate().is_ok());

        for refused in [
            "",                 // AUDIT_DETAILS.System_id_valid
            "   ",              // blank
            "cdr::hospital",    // the OBJECT_VERSION_ID field separator
            "cdr hospital",     // a space is in no `uid` production
            "cdr/hospital",     // nor is a path separator
            "-leading-hyphen",  // an internet_id label starts with a letter
            "trailing-hyphen-", // and ends with a letter or digit
            "1.2.840.",         // an iso_oid group may not be empty
        ] {
            c.server.system_id = refused.to_owned();
            assert!(
                c.validate().is_err(),
                "system_id {refused:?} is not a legal openEHR uid and must be refused at boot"
            );
        }

        for accepted in [
            "cdr.hospital.example",                 // internet_id
            "ferroehr.local",                       // internet_id (the default)
            "1.2.840.113554",                       // iso_oid
            "8849182c-82ad-4088-a07f-48ead4180515", // uuid
        ] {
            c.server.system_id = accepted.to_owned();
            assert!(
                c.validate().is_ok(),
                "system_id {accepted:?} is a legal openEHR uid and must boot"
            );
        }
    }

    #[test]
    fn validate_external_terminology_needs_a_provider() {
        let mut c = FerroEhrConfig::default();
        c.terminology.external.enabled = true;
        assert!(c.validate().is_err());
    }

    /// A provider table with no `url` fails to boot, naming the key. The
    /// section reads its defaults from one `Default` impl, so serde cannot
    /// report the missing key itself — the semantic pass owns it, which is
    /// also what lets every configuration error surface at once.
    #[test]
    fn validate_terminology_provider_needs_a_url() {
        let mut c = FerroEhrConfig::default();
        let external = &mut c.terminology.external;
        external.enabled = true;
        external.providers.insert(
            "default".to_owned(),
            crate::service::terminology::config::FhirProviderConfig::default(),
        );
        let err = c
            .validate()
            .expect_err("a provider with no url must be rejected");
        assert!(
            err.to_string()
                .contains("terminology.external.providers.default.url"),
            "got {err}"
        );

        c.terminology
            .external
            .providers
            .get_mut("default")
            .expect("provider")
            .url = "https://ts.example/fhir".to_owned();
        assert!(c.validate().is_ok());
    }

    /// A terminology route naming a provider that is not configured is a boot
    /// error — never a silent fall-back to the default server.
    #[test]
    fn validate_terminology_route_must_name_a_configured_provider() {
        let mut c = FerroEhrConfig::default();
        let external = &mut c.terminology.external;
        external.enabled = true;
        external.providers.insert(
            "default".to_owned(),
            crate::service::terminology::config::FhirProviderConfig {
                kind: crate::service::terminology::config::ProviderKind::Fhir,
                url: "https://ts.example/fhir".to_owned(),
                operation: crate::service::terminology::config::FhirOperation::ValidateCode,
                connect_timeout_ms: 2_000,
                request_timeout_ms: 10_000,
                oauth2_client: None,
                client_cert_path: None,
                client_key_path: None,
                ca_bundle_path: None,
                cache_ttl_secs: 300,
                cache_capacity: 10_000,
            },
        );
        assert!(c.validate().is_ok(), "a routeless provider set is valid");

        c.terminology
            .external
            .routes
            .insert("SNOMED-CT".to_owned(), "snomed".to_owned());
        let err = c.validate().expect_err("a dangling route must be rejected");
        assert!(err.to_string().contains("snomed"), "got {err}");

        c.terminology
            .external
            .routes
            .insert("SNOMED-CT".to_owned(), "default".to_owned());
        assert!(c.validate().is_ok());
    }

    /// A provider naming an `oauth2_client` with no matching client table is a
    /// boot error — never a silently unauthenticated terminology request. The
    /// client itself must carry a token endpoint, a client id, and a secret.
    #[test]
    fn validate_terminology_oauth2_client_reference_and_shape() {
        let mut c = FerroEhrConfig::default();
        let external = &mut c.terminology.external;
        external.enabled = true;
        external.providers.insert(
            "default".to_owned(),
            crate::service::terminology::config::FhirProviderConfig {
                kind: crate::service::terminology::config::ProviderKind::Fhir,
                url: "https://ts.example/fhir".to_owned(),
                operation: crate::service::terminology::config::FhirOperation::ValidateCode,
                connect_timeout_ms: 2_000,
                request_timeout_ms: 10_000,
                oauth2_client: Some("ts-client".to_owned()),
                client_cert_path: None,
                client_key_path: None,
                ca_bundle_path: None,
                cache_ttl_secs: 300,
                cache_capacity: 10_000,
            },
        );
        let err = c
            .validate()
            .expect_err("an unconfigured oauth2_client must be rejected");
        assert!(err.to_string().contains("ts-client"), "got {err}");

        // A client table with none of its mandatory keys: each is named. The
        // section reads its defaults from one `Default` impl, so serde reports
        // none of them and this pass owns all three.
        c.terminology.external.oauth2_clients.insert(
            "ts-client".to_owned(),
            crate::service::terminology::config::TerminologyOauth2Config::default(),
        );
        let err = c
            .validate()
            .expect_err("a client with no token_url/client_id/secret is rejected");
        for key in ["token_url", "client_id", "client_secret"] {
            assert!(err.to_string().contains(key), "{key} unnamed in {err}");
        }

        // A client with no secret cannot run the client-credentials grant.
        c.terminology.external.oauth2_clients.insert(
            "ts-client".to_owned(),
            crate::service::terminology::config::TerminologyOauth2Config {
                token_url: "https://idp.example/token".to_owned(),
                client_id: "cdr".to_owned(),
                ..crate::service::terminology::config::TerminologyOauth2Config::default()
            },
        );
        let err = c.validate().expect_err("a secretless client is rejected");
        assert!(err.to_string().contains("client_secret"), "got {err}");

        c.terminology
            .external
            .oauth2_clients
            .get_mut("ts-client")
            .expect("client")
            .client_secret = Some(Secret::new("s3cret"));
        assert!(c.validate().is_ok());
    }

    /// The new terminology keys are reachable through the one env grammar, map
    /// keys included.
    #[test]
    fn env_mapping_terminology_routes_and_oauth2_clients() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__TERMINOLOGY__EXTERNAL__ENABLED", "true"),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__SNOMED__URL",
                    "https://snowstorm/fhir",
                ),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__SNOMED__OAUTH2_CLIENT",
                    "ts-client",
                ),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__ROUTES__SNOMED-CT",
                    "snomed",
                ),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__OAUTH2_CLIENTS__TS-CLIENT__TOKEN_URL",
                    "https://idp/token",
                ),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__OAUTH2_CLIENTS__TS-CLIENT__CLIENT_ID",
                    "cdr",
                ),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__OAUTH2_CLIENTS__TS-CLIENT__CLIENT_SECRET",
                    "s3cret",
                ),
            ]),
            &[],
        );
        let external = &c.terminology.external;
        assert!(external.enabled);
        assert_eq!(
            external.providers.get("snomed").expect("snomed").url,
            "https://snowstorm/fhir"
        );
        assert_eq!(
            external.routes.get("snomed-ct").map(String::as_str),
            Some("snomed"),
            "map keys arrive lower-cased from the env grammar"
        );
        let client = external
            .oauth2_clients
            .get("ts-client")
            .expect("oauth2 client");
        assert_eq!(client.token_url, "https://idp/token");
        assert_eq!(
            client.client_secret.as_ref().map(Secret::expose),
            Some("s3cret")
        );
    }

    /// A mutual-TLS client identity is a certificate AND its key: half of one
    /// would connect presenting nothing, so it is a boot error. The trust
    /// anchor (`ca_bundle_path`) stands alone — verification against a private
    /// CA needs no client identity.
    #[test]
    fn validate_terminology_client_identity_needs_both_halves() {
        let mut c = FerroEhrConfig::default();
        let external = &mut c.terminology.external;
        external.enabled = true;
        external.providers.insert(
            "default".to_owned(),
            crate::service::terminology::config::FhirProviderConfig {
                kind: crate::service::terminology::config::ProviderKind::Fhir,
                url: "https://ts.example/fhir".to_owned(),
                operation: crate::service::terminology::config::FhirOperation::ValidateCode,
                connect_timeout_ms: 2_000,
                request_timeout_ms: 10_000,
                oauth2_client: None,
                client_cert_path: Some(PathBuf::from("/run/secrets/ts-client.crt.pem")),
                client_key_path: None,
                ca_bundle_path: None,
                cache_ttl_secs: 300,
                cache_capacity: 10_000,
            },
        );
        let err = c
            .validate()
            .expect_err("a certificate without its key must be rejected");
        assert!(err.to_string().contains("client_key_path"), "got {err}");

        let provider = c
            .terminology
            .external
            .providers
            .get_mut("default")
            .expect("provider");
        provider.client_cert_path = None;
        provider.client_key_path = Some(PathBuf::from("/run/secrets/ts-client.key.pem"));
        let err = c
            .validate()
            .expect_err("a key without its certificate must be rejected");
        assert!(err.to_string().contains("client_cert_path"), "got {err}");

        // Both halves together are valid; so is a bare trust anchor.
        let provider = c
            .terminology
            .external
            .providers
            .get_mut("default")
            .expect("provider");
        provider.client_cert_path = Some(PathBuf::from("/run/secrets/ts-client.crt.pem"));
        provider.ca_bundle_path = Some(PathBuf::from("/run/secrets/ts-ca.pem"));
        assert!(c.validate().is_ok());

        let provider = c
            .terminology
            .external
            .providers
            .get_mut("default")
            .expect("provider");
        provider.client_cert_path = None;
        provider.client_key_path = None;
        assert!(c.validate().is_ok(), "a CA bundle alone is valid");
    }

    /// The mutual-TLS keys are reachable through the one env grammar.
    #[test]
    fn env_mapping_terminology_provider_mtls_paths() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__TERMINOLOGY__EXTERNAL__ENABLED", "true"),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__SNOMED__URL",
                    "https://snowstorm/fhir",
                ),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__SNOMED__CLIENT_CERT_PATH",
                    "/run/secrets/ts-client.crt.pem",
                ),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__SNOMED__CLIENT_KEY_PATH",
                    "/run/secrets/ts-client.key.pem",
                ),
                (
                    "FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__SNOMED__CA_BUNDLE_PATH",
                    "/run/secrets/ts-ca.pem",
                ),
            ]),
            &[],
        );
        let provider = c
            .terminology
            .external
            .providers
            .get("snomed")
            .expect("snomed");
        assert_eq!(
            provider.client_cert_path.as_deref(),
            Some(Path::new("/run/secrets/ts-client.crt.pem"))
        );
        assert_eq!(
            provider.client_key_path.as_deref(),
            Some(Path::new("/run/secrets/ts-client.key.pem"))
        );
        assert_eq!(
            provider.ca_bundle_path.as_deref(),
            Some(Path::new("/run/secrets/ts-ca.pem"))
        );
    }
}
