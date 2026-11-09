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
//! Precedence, lowest first: built-in `Default` impls, the config file (named
//! by `--config` or `FERROEHR_CONFIG`), the `FERROEHR__*` environment (`__` for
//! nesting), then `--set key=value` overrides; the loader is the family's
//! `ferrohealth-config`, and [`loader::loader`] names its fallbacks.
//! [`assemble`] is a function of `(file, env_map, overrides)` with no
//! process-global env, so the whole test plan runs on injected inputs.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

pub mod loader;

pub mod abac_rules;
pub mod access_delay;
pub mod agreement;
pub mod auth;
pub mod auth_section;
pub mod authz;
pub mod binding;
pub mod clients;
pub mod deployment;
pub mod management;
pub mod profile;
pub mod redact;
pub mod rest;
pub mod scim;
pub mod smart;
mod validate;
pub mod versioning;

use std::collections::HashMap;
use std::path::Path;

use ferrohealth_config::loader::Loaded;
use serde::{Deserialize, Serialize};

use crate::config::loader::{ConfigError, ConfigErrors};

/// The complete server configuration.
///
/// Every section has a `Default`, so the file may be empty or absent
/// (zero-config boot). `deny_unknown_fields` makes a misspelled top-level
/// table a boot error.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// `/management/status`. Ignored under `sandbox`, which accepts everything.
    pub deployment_accepts: Vec<deployment::DeploymentGap>,
    /// `[server]` — the HTTP listener: the family's section (the listen
    /// address, the timeouts, the body limit, the overload bound, the drain,
    /// TLS, the trusted proxies, the caller rate and compression).
    pub server: ferrohealth_server::section::ServerSection,
    /// `[rest]` — the openEHR REST surface: the API base path, the system
    /// identifier, the System-Options identity, the Swagger UI, CORS and the
    /// clinical body tier.
    pub rest: rest::RestConfig,
    /// `[db]` — the shared `PostgreSQL` DSN, pool tuning and migration mode.
    pub db: crate::db::DbConfig,
    /// `[storage]` — one DSN per storage domain (`clinical`, `party`,
    /// `audit`), each defaulting to `[db].url`.
    pub storage: crate::db::domain::StorageConfig,
    /// `[telemetry]`: the console, the request log and the OpenTelemetry
    /// export, the family's shared section.
    pub telemetry: ferrohealth_telemetry::section::TelemetrySection,
    /// `[auth]` — authentication: the `ferrohealth-smart` section, whose
    /// serialization writes every secret as `***`, and `[auth.idp]`.
    pub auth: auth_section::AuthConfig,
    /// `[authz]` — RBAC + ABAC.
    pub authz: authz::AuthzConfig,
    /// `[admin]` — the ADMIN API group.
    pub admin: rest::AdminConfig,
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
    /// `[outbox]` — the commit-event outbox readers and its retention.
    pub outbox: crate::extensions::outbox::config::OutboxConfig,
    /// `[terminology]` — terminology API + the terminology server.
    pub terminology: crate::service::terminology::config::TerminologyConfig,
    /// `[multimedia]` — `DV_MULTIMEDIA` externalization.
    pub multimedia: crate::extensions::multimedia::config::MultimediaConfig,
    /// `[audit]` — the IHE ATNA audit trail / System Log (local Audit Record
    /// Repository + the syslog and FHIR-feed forwarding sinks).
    pub audit: crate::system_log::config::AuditConfig,
    /// `[origins]` — the origins of data this deployment recognises, which a
    /// retention period and an access rule may name (EHDS Annex II 3.4).
    pub origins: crate::origins::OriginsConfig,
    /// `[privacy]` — the clinical-side data-minimisation policy.
    pub privacy: crate::privacy::config::PrivacyConfig,
    /// `[demographic]` — the demographic domain, including national-identifier
    /// protection.
    pub demographic: crate::identifier_protection::config::DemographicConfig,
    /// `[cohort]` — the cross-domain cohort-query allow-list and limits.
    pub cohort: crate::service::linkage::cohort::config::CohortConfig,
    /// `[ehr_index]` — who holds the subject-to-EHR cross-reference: an IHE
    /// patient identity source (the default) or the in-instance map.
    pub ehr_index: crate::service::ehr_index::config::EhrIndexConfig,
    /// `[usage_report]` — the installation report to `FerroPULSE` (on by default).
    pub usage_report: ferrohealth_usage_report::section::UsageReportSection,
    /// `[access_delay]` — the EHDS Art. 3(3) delay of the person's own access.
    pub access_delay: access_delay::AccessDelayConfig,
    /// `[versioning]` — the opt-in answer to an identical re-send (#4415).
    pub versioning: versioning::VersioningConfig,
    /// `[scim]` — the SCIM 2.0 service provider and its provisioning clients
    /// (off by default).
    pub scim: scim::ScimConfig,
}

impl Default for FerroEhrConfig {
    /// Every section's own default, except FerroEHR's `[server]`: the
    /// listener on every interface at port 8080, the port the images expose,
    /// and a 64 MiB body ceiling, since the largest operational template in
    /// the vendored CKM corpus is 5.4 MB. The loader layers this section
    /// under the file ([`loader::loader`]), so a partial `[server]` table
    /// keeps both. No openEHR spec governs configuration: our own design.
    fn default() -> Self {
        Self {
            spec_profile: profile::SpecProfile::default(),
            deployment_profile: deployment::DeploymentProfile::default(),
            deployment_accepts: Vec::new(),
            server: ferrohealth_server::section::ServerSection {
                listen: std::net::SocketAddr::from(([0, 0, 0, 0], 8080)),
                body_limit_bytes: 64 * 1024 * 1024,
                ..ferrohealth_server::section::ServerSection::default()
            },
            rest: rest::RestConfig::default(),
            db: crate::db::DbConfig::default(),
            storage: crate::db::domain::StorageConfig::default(),
            telemetry: ferrohealth_telemetry::section::TelemetrySection::default(),
            auth: auth_section::AuthConfig::default(),
            authz: authz::AuthzConfig::default(),
            admin: rest::AdminConfig::default(),
            smart: smart::SmartConfig::default(),
            management: management::ManagementConfig::default(),
            signing: crate::versioning::signature::config::SigningConfig::default(),
            licence: crate::licence::config::LicenceConfig::default(),
            query: crate::service::query::config::QueryConfig::default(),
            outbox: crate::extensions::outbox::config::OutboxConfig::default(),
            terminology: crate::service::terminology::config::TerminologyConfig::default(),
            multimedia: crate::extensions::multimedia::config::MultimediaConfig::default(),
            audit: crate::system_log::config::AuditConfig::default(),
            origins: crate::origins::OriginsConfig::default(),
            privacy: crate::privacy::config::PrivacyConfig::default(),
            demographic: crate::identifier_protection::config::DemographicConfig::default(),
            cohort: crate::service::linkage::cohort::config::CohortConfig::default(),
            ehr_index: crate::service::ehr_index::config::EhrIndexConfig::default(),
            usage_report: ferrohealth_usage_report::section::UsageReportSection::default(),
            access_delay: access_delay::AccessDelayConfig::default(),
            versioning: versioning::VersioningConfig::default(),
            scim: scim::ScimConfig::default(),
        }
    }
}

/// The annotated default template, shipped as
/// `app/ferroehr/assets/ferroehr.default.toml` — a hand-maintained asset kept in
/// sync with the schema by the template tests.
pub const DEFAULT_TEMPLATE: &str = include_str!("../../assets/ferroehr.default.toml");

impl FerroEhrConfig {
    /// Returns the redacted configuration as a TOML table, the tree
    /// `ferroehr report` records.
    ///
    /// The table is [`Self::to_redacted_json`]'s tree, so every rendering
    /// masks the same leaves the same way.
    ///
    /// # Errors
    /// [`ConfigError`] if the tree cannot be serialized to TOML.
    pub fn to_redacted_table(&self) -> Result<toml::Table, ConfigError> {
        toml::Table::try_from(redact::without_nulls(self.to_redacted_json()?))
            .map_err(|e| ConfigError::semantic(format!("rendering config as TOML: {e}")))
    }

    /// Returns the effective configuration as a redacted JSON tree, the one
    /// redacted tree every rendering derives from.
    ///
    /// The binary builds the `/management/env` and `GET /admin/config`
    /// snapshot with it; [`Self::to_redacted_table`] renders it. No openEHR spec governs
    /// configuration — our own design/extension.
    ///
    /// Redaction is a property of the leaf type rather than of a key-name scan:
    /// every secret-bearing field is typed `ferrohealth-secret`'s `Secret`, whose
    /// [`Serialize`] emits the fixed `***` placeholder, or
    /// `SecretUrl`, whose [`Serialize`] masks the URL `userinfo`
    /// component. Every URL key that may carry `userinfo` is a `SecretUrl`;
    /// a URL key whose value is an identifier or is published
    /// (`auth.issuer`, the `smart` URLs) is refused at boot when it
    /// carries `userinfo`. A field therefore cannot leak by being renamed, and
    /// the `redacted_json_masks_every_secret_field` test enumerates the current
    /// secret set as the standing backstop. On top of the types, every string
    /// leaf of the tree that holds a URL passes `ferrohealth-smart`'s
    /// [`mask_userinfo`](ferrohealth_smart::config::mask_userinfo), so a URL in a key
    /// that is not typed `SecretUrl`, or in a configuration no validation has
    /// run over, is masked too. `[auth]` is the `ferrohealth-smart` section,
    /// whose serialization writes each Basic password hash as `***`.
    /// Non-secret identifiers, such as a Basic user's `user` and `scopes`, an
    /// `issuer` without `userinfo`, or `auth.jwks_file` (a path), stay visible.
    ///
    /// # Errors
    /// [`ConfigError`] if the tree cannot be serialized to JSON.
    pub fn to_redacted_json(&self) -> Result<serde_json::Value, ConfigError> {
        let mut tree = serde_json::to_value(self)
            .map_err(|e| ConfigError::semantic(format!("rendering config as JSON: {e}")))?;
        redact::mask_url_userinfo(&mut tree);
        Ok(tree)
    }
}

/// Assemble the configuration from explicit inputs — the seam every test
/// drives.
///
/// Reads the file, the environment map and the overrides through
/// [`loader::loader`], then resolves every `*_file` secret sibling.
///
/// # Errors
/// [`ConfigErrors`] for an unreadable file, any refusal of the loader, and
/// any `*_file` secret that cannot be resolved.
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
    loader::assemble_from(file, env, overrides).map(|(_loaded, config)| config)
}

/// Boot loader: reads the configuration of the running process.
///
/// The file is the one `cli_config` or `FERROEHR_CONFIG` names, and without
/// either only the defaults, the environment and `overrides` stand. Emits
/// the dev-default-DB boot warning.
///
/// # Errors
/// [`ConfigErrors`] on an unreadable file or any refusal of the loader.
pub fn load(
    cli_config: Option<&Path>,
    overrides: &[(String, String)],
) -> Result<FerroEhrConfig, ConfigErrors> {
    let (_loaded, config) = load_with_provenance(cli_config, overrides)?;
    Ok(config)
}

/// Reads the configuration of the running process as [`load`] does, with the
/// loaded tree whose provenance `ferroehr config check` renders.
///
/// # Errors
/// [`ConfigErrors`] on an unreadable file or any refusal of the loader.
pub fn load_with_provenance(
    cli_config: Option<&Path>,
    overrides: &[(String, String)],
) -> Result<(Loaded<FerroEhrConfig>, FerroEhrConfig), ConfigErrors> {
    let (loaded, config) = loader::load_process(cli_config, overrides)?;

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

    Ok((loaded, config))
}

#[cfg(test)]
mod audit_tests;
#[cfg(test)]
mod tests {
    use assert_fs::prelude::*;

    use std::path::PathBuf;

    use super::*;
    use crate::config::authz::AbacParam;
    use crate::service::ehr_index::config::EhrIndexConfig;
    use crate::storage::cohort_predicate::PredicateKind;
    use ferrohealth_secret::secret_url::SecretUrl;

    /// The shipped defaults of a FerroEHR run on its own: the explicit
    /// `[ehr_index] provider = "local"` opt-in, without which the default IHE
    /// provider refuses for want of an endpoint.
    pub(super) fn standalone() -> FerroEhrConfig {
        FerroEhrConfig {
            ehr_index: EhrIndexConfig::local(),
            ..FerroEhrConfig::default()
        }
    }

    /// The shipped defaults, with no identity source and no opt-in, are
    /// refused, and the refusal names both choices; the opt-in alone passes.
    #[test]
    fn the_defaults_refuse_until_an_identity_source_or_local_is_chosen() {
        let refused = FerroEhrConfig::default()
            .validate()
            .expect_err("no identity source and no opt-in");
        let text = refused.to_string();
        assert!(text.contains("ehr_index.pixm_url"), "{text}");
        assert!(text.contains("provider = \"local\""), "{text}");
        assert!(standalone().validate().is_ok());
    }

    /// `c` serialized as the TOML table `ferroehr config check` renders
    /// (`ferrohealth_config::report::check_report` writes the same table).
    fn rendered(c: &FerroEhrConfig) -> String {
        toml::Table::try_from(c)
            .expect("the tree renders as TOML")
            .to_string()
    }

    /// Build an injected env map from `(key, value)` pairs.
    pub(super) fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// A temp file carrying `content`, for the file-source cases.
    pub(super) fn toml_file(content: &str) -> assert_fs::NamedTempFile {
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
    fn port_convention_expands_to_an_all_interfaces_listen_address() {
        // PORT alone sets server.listen on all interfaces (the container-
        // platform convention: Vercel/Cloud Run/Heroku inject only the port).
        let c = assemble_ok(None, &env(&[("PORT", "3123")]), &[]);
        assert_eq!(c.server.listen.to_string(), "0.0.0.0:3123");
        let c = assemble_ok(
            None,
            &env(&[
                ("PORT", "3123"),
                ("FERROEHR__SERVER__LISTEN", "127.0.0.1:9000"),
            ]),
            &[],
        );
        assert_eq!(c.server.listen.to_string(), "127.0.0.1:9000");
    }

    // ── 2. Mapping ────────────────────────────────────────────────────────────

    #[test]
    fn env_mapping_scalars_maps_and_lists() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__SERVER__MAX_CONCURRENT_REQUESTS", "64"),
                ("FERROEHR__AUTH__ISSUER__0__ISSUER", "https://idp"),
                ("FERROEHR__AUTH__ISSUER__0__KIND", "persons"),
                ("FERROEHR__AUTH__ISSUER__0__AUDIENCE", "ferroehr,other"),
                ("FERROEHR__AUTH__ISSUER__0__TIMEOUT_MS", "1500"),
                ("FERROEHR__AUTHZ__ABAC__POLICY__EHR__NAME", "ehr-policy"),
            ]),
            &[],
        );
        assert_eq!(
            c.server.max_concurrent_requests,
            std::num::NonZeroUsize::new(64)
        );
        let [issuer] = c.auth.shared.issuers() else {
            panic!("one issuer table from the env");
        };
        assert_eq!(issuer.issuer, "https://idp");
        assert_eq!(
            issuer.kind,
            Some(ferrohealth_smart::kind::IssuerKind::Persons)
        );
        assert_eq!(
            issuer.audience,
            vec!["ferroehr".to_owned(), "other".to_owned()]
        );
        // The key-set client keys: one overridden from the env, the other two at
        // their documented defaults.
        assert_eq!(issuer.timeout_ms, 1_500);
        assert_eq!(issuer.jwks_max_age_seconds, 600);
        assert_eq!(issuer.jwks_refresh_interval_seconds, 60);
        assert_eq!(
            c.authz
                .abac
                .policy
                .get("ehr")
                .map(|rule| rule.name.as_str()),
            Some("ehr-policy")
        );
    }

    /// EVERY `Vec`-typed key an operator can set through the environment parses
    /// as a list.
    ///
    /// A list-typed key missing from `alias::LIST_KEYS` is not mis-split — it is
    /// REFUSED, `invalid type: string …, expected a sequence`, so its documented
    /// env spelling is dead on arrival. Two shipped keys were in exactly that
    /// state (`signing.retired_key_paths`, the former `smart.endpoints.capabilities`) and
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
                ("FERROEHR__AUTH__CAPABILITIES", "launch-ehr"),
                ("FERROEHR__AUTH__ISSUER__0__ISSUER", "https://idp"),
                ("FERROEHR__AUTH__ISSUER__0__AUDIENCE", "ferroehr"),
                ("FERROEHR__AUTH__ISSUER__0__ALGORITHMS", "ES256"),
                ("FERROEHR__AUTH__SCOPES_SUPPORTED", "openid"),
                ("FERROEHR__SMART__BACKEND_CLIENTS", "etl"),
                ("FERROEHR__SMART__PATIENT_ISSUERS", "https://as.example"),
                ("FERROEHR__SMART__DEMOGRAPHIC_CLIENTS", "mpi"),
                ("FERROEHR__OUTBOX__READERS", "ferrobridge"),
            ]),
            &[],
        );
        assert_eq!(
            c.signing.retired_key_paths,
            vec![PathBuf::from("/etc/ferroehr/retired-2025.pub.asc")],
            "a single retired key path must parse as a one-element list"
        );
        assert_eq!(c.auth.shared.capabilities, vec!["launch-ehr".to_owned()]);
        let [issuer] = c.auth.shared.issuers() else {
            panic!("one issuer table from the env");
        };
        assert_eq!(issuer.audience, vec!["ferroehr".to_owned()]);
        assert_eq!(issuer.algorithms.len(), 1);
        assert_eq!(c.auth.shared.scopes_supported, vec!["openid".to_owned()]);
        assert_eq!(c.smart.backend_clients, vec!["etl".to_owned()]);
        assert_eq!(
            c.smart.patient_issuers,
            vec!["https://as.example".to_owned()]
        );
        assert_eq!(c.smart.demographic_clients, vec!["mpi".to_owned()]);
        assert_eq!(c.outbox.readers, vec!["ferrobridge".to_owned()]);
    }

    /// A list nested under a MAP key is addressable from the environment too,
    /// so it needs registration like any other — `authz.abac.policy.<kind>` is
    /// a map, not an array of tables, and the scalar case above already proves
    /// the env grammar reaches into one.
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
                    "FERROEHR__AUTH__CAPABILITIES",
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
            c.auth.shared.capabilities,
            vec!["launch-ehr".to_owned(), "sso-openid-connect".to_owned()]
        );
    }

    /// The `[auth]` posture keys, the ABAC directory switch and the ABAC patient
    /// namespace are reachable through the one env grammar — a documented env
    /// spelling that binds nothing would ship dead.
    #[test]
    fn env_mapping_auth_and_authz_posture_keys() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__AUTH__ISSUER__0__ISSUER", "https://idp"),
                ("FERROEHR__AUTH__ISSUER__0__AUDIENCE", "ferroehr"),
                ("FERROEHR__AUTH__ISSUER__0__CLOCK_SKEW_SECONDS", "120"),
                ("FERROEHR__AUTH__ISSUER__0__PROFILE", "keycloak"),
                ("FERROEHR__AUTHZ__ABAC__CHECK_DIRECTORY", "true"),
                (
                    "FERROEHR__AUTHZ__ABAC__PATIENT_NAMESPACE",
                    "nl.example.patients",
                ),
            ]),
            &[],
        );
        let [issuer] = c.auth.shared.issuers() else {
            panic!("one issuer table from the env");
        };
        assert_eq!(issuer.clock_skew_seconds, 120);
        assert_eq!(issuer.profile, ferrohealth_smart::profile::Preset::Keycloak);
        assert!(c.authz.abac.check_directory);
        assert_eq!(
            c.authz.abac.patient_namespace(),
            Some("nl.example.patients")
        );

        // Unset, both carry their documented defaults.
        let d = assemble_ok(None, &env(&[]), &[]);
        assert!(!d.authz.abac.check_directory);
        assert_eq!(d.authz.abac.patient_namespace(), None);
        assert!(d.auth.shared.issuers().is_empty());
        let entry = ferrohealth_smart::config::IssuerEntry::default();
        assert_eq!(
            entry.clock_skew_seconds, 60,
            "the default leeway is the conventional 60 s"
        );
        assert_eq!(entry.profile, ferrohealth_smart::profile::Preset::Rfc9068);
    }

    /// `[rest] system_id` — the CDR's own openEHR system identifier (stamped
    /// into `EHR.system_id`, the `AUDIT_DETAILS.system_id` server default, and
    /// every `OBJECT_VERSION_ID.creating_system_id`). Pins all three layers:
    /// the compatibility default, the file value, and the `FERROEHR__` env form.
    #[test]
    fn rest_system_id_default_file_and_env() {
        // Unset, the default is the service-layer constant.
        let c = assemble_ok(None, &env(&[]), &[]);
        assert_eq!(c.rest.system_id, crate::service::DEFAULT_SYSTEM_ID);

        let file = toml_file("[rest]\nsystem_id = \"cdr.hospital.example\"\n");
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        assert_eq!(c.rest.system_id, "cdr.hospital.example");

        let c = assemble_ok(
            Some(file.path()),
            &env(&[("FERROEHR__REST__SYSTEM_ID", "cdr.env.example")]),
            &[],
        );
        assert_eq!(c.rest.system_id, "cdr.env.example");

        // `system_id` and the `[rest.identity]` display identity are
        // independent knobs — setting one must not disturb the other.
        assert_eq!(c.rest.identity.solution, "FerroEHR");
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
    fn near_miss_prefix_binds_nothing() {
        // The mixed form (a single `_` after the prefix) is outside the
        // override grammar, so it sets no key and leaves the default in place.
        let c = assemble_ok(None, &env(&[("FERROEHR_DB__URL", "postgres://x")]), &[]);
        assert_eq!(json(&c), json(&FerroEhrConfig::default()));
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
    fn per_subsystem_config_pointer_is_never_read() {
        // There is ONE configuration file. A per-subsystem `*_CONFIG` pointer
        // names no file the loader reads, so the configuration stays the
        // default.
        let c = assemble_ok(None, &env(&[("FERROEHR_SIGNING_CONFIG", "/x.toml")]), &[]);
        assert_eq!(json(&c), json(&FerroEhrConfig::default()));
    }

    #[test]
    fn unknown_file_key_is_rejected() {
        let file = toml_file("[db]\nmax_conections = 5\n");
        let err = assemble(Some(file.path()), &env(&[]), &[]).expect_err("unknown key");
        assert!(err.to_string().contains("max_conections"), "{err}");
    }

    /// The `[server]` keys FerroEHR read before the family section took the
    /// table are refused by name, so a configuration written for them fails
    /// to boot instead of running on defaults.
    #[test]
    fn the_retired_server_keys_are_refused() {
        for (text, key) in [
            ("[server]\nbind = \"0.0.0.0:8080\"\n", "bind"),
            ("[server]\nmax_in_flight = 64\n", "max_in_flight"),
            ("[server]\nsystem_id = \"cdr.example\"\n", "system_id"),
            ("[server.limits]\nbody_bytes = 1\n", "limits"),
            ("[server.rate_limit]\nenabled = false\n", "rate_limit"),
            (
                "[server.connection]\nmax_concurrent_streams = 1\n",
                "connection",
            ),
            (
                "[server.tls]\nenabled = true\ncertificate_file = \"a\"\nkey_file = \"b\"\n",
                "enabled",
            ),
        ] {
            let file = toml_file(text);
            let err = assemble(Some(file.path()), &env(&[]), &[]).expect_err(text);
            assert!(err.to_string().contains(key), "{text}: {err}");
        }
    }

    /// The listener's bounds refuse what cannot be served: a zero header-read
    /// bound (it would disconnect every client) and a TLS minimum below 1.2
    /// (RFC 8996).
    #[test]
    fn the_listener_refuses_a_zero_head_bound_and_a_deprecated_tls_minimum() {
        let zero = toml_file("[server]\nheader_read_timeout_ms = 0\n");
        assert!(assemble(Some(zero.path()), &env(&[]), &[]).is_err());
        let old_tls = toml_file(
            "[server.tls]\ncertificate_file = \"a\"\nkey_file = \"b\"\nmin_version = \"1.1\"\n",
        );
        let err = assemble(Some(old_tls.path()), &env(&[]), &[]).expect_err("TLS 1.1");
        assert!(err.to_string().contains("RFC 8996"), "{err}");
    }

    /// A client CA under `client_auth = "none"`, written or by default, is
    /// refused at load naming the setting: before 5.5.0 a CA alone meant a
    /// required client certificate, and under `none` it would ask for none
    /// (GHSA-52mf-qg39-4q4j).
    #[test]
    fn a_client_ca_without_a_client_certificate_policy_is_refused() {
        for policy in ["", "client_auth = \"none\"\n"] {
            let file = toml_file(&format!(
                "[server.tls]\ncertificate_file = \"a\"\nkey_file = \"b\"\n{policy}\
                 client_ca_file = \"ca.pem\"\n"
            ));
            let err = assemble(Some(file.path()), &env(&[]), &[])
                .expect_err("a client CA under none is refused");
            let text = err.to_string();
            for word in ["client_ca_file", "client_auth", "required", "optional"] {
                assert!(text.contains(word), "{policy:?}: {word} in {err}");
            }
        }
        let required = toml_file(
            "[server.tls]\ncertificate_file = \"a\"\nkey_file = \"b\"\n\
             client_auth = \"required\"\nclient_ca_file = \"ca.pem\"\n",
        );
        assert!(assemble(Some(required.path()), &env(&[]), &[]).is_ok());
    }

    /// A URL loses its userinfo and query; text without `://` keeps its text.
    #[test]
    fn mask_userinfo_masks_only_a_url() {
        assert_eq!(
            ferrohealth_smart::config::mask_userinfo("postgres://u:Qz7@db:5432/x?password=Qz7"),
            "postgres://***@db:5432/x?***"
        );
        assert_eq!(
            ferrohealth_smart::config::mask_userinfo("s3.eu-west-1.amazonaws.com"),
            "s3.eu-west-1.amazonaws.com"
        );
    }

    // ── 4. One grammar, no synonyms ─────────────────────────────────────────

    #[test]
    fn only_the_uniform_spelling_binds() {
        // NOTE: ferrohealth-config 0.2.5 checks only the `FERROEHR__` override form,
        // since `FERROEHR_VIEWER` nests under `FERROEHR`; a single `_` sets nothing.
        let c = assemble_ok(None, &env(&[("FERROEHR_DB_MAX_CONNECTIONS", "7")]), &[]);
        assert_eq!(
            c.db.max_connections,
            FerroEhrConfig::default().db.max_connections
        );
        let c = assemble_ok(None, &env(&[("FERROEHR__DB__MAX_CONNECTIONS", "7")]), &[]);
        assert_eq!(c.db.max_connections, 7);
    }

    // ── 5. Secrets ──────────────────────────────────────────────────────────

    #[test]
    fn secrets_never_render_and_file_resolves() {
        let file =
            toml_file("[[auth.basic.users]]\nuser = \"alice\"\npassword_hash = \"topsecret\"\n");
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        let rendered = rendered(&c);
        assert!(!rendered.contains("topsecret"), "secret leaked: {rendered}");
        assert!(
            rendered.contains("password_hash = \"***\""),
            "the hash renders masked: {rendered}"
        );
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
    /// These had no file route at all, so the most valuable secret in the
    /// deployment — the database DSN — could only travel as an environment value,
    /// readable through `/proc/<pid>/environ` and inherited by every child
    /// process. The assertion is equivalence: the file route is an additional
    /// delivery mechanism, not a second configuration path that might differ.
    #[test]
    fn the_credential_keys_resolve_identically_from_a_file_and_inline() {
        const DSN: &str = "postgres://u:p@db.internal:5432/ferroehr";
        const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2g";

        let secret_file = |name: &str, contents: &str| {
            let f = assert_fs::NamedTempFile::new(name).expect("temp");
            // A trailing newline is what `echo >` and a Kubernetes secret editor
            // both produce, so the reader must trim it.
            f.write_str(&format!("{contents}\n")).expect("write");
            f
        };

        let dsn = secret_file("dsn", DSN);
        let hash = secret_file("hash", HASH);

        let from_files = toml_file(&format!(
            "[db]\nurl_file = \"{}\"\n\
             [[auth.basic.users]]\nuser = \"alice\"\npassword_hash_file = \"{}\"\n",
            dsn.path().display(),
            hash.path().display(),
        ));
        let via_file = assemble_ok(Some(from_files.path()), &env(&[]), &[]);

        let inline = toml_file(&format!(
            "[db]\nurl = \"{DSN}\"\n\
             [[auth.basic.users]]\nuser = \"alice\"\npassword_hash = \"{HASH}\"\n"
        ));
        let via_inline = assemble_ok(Some(inline.path()), &env(&[]), &[]);

        assert_eq!(via_file.db.url.expose(), DSN);
        assert_eq!(via_file.db.url.expose(), via_inline.db.url.expose());
        let user = &via_file.auth.shared.basic.as_ref().expect("basic").users[0];
        assert_eq!(
            user.password_hash
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
            Some(HASH)
        );

        // And a file-delivered DSN redacts exactly like an inline one — the
        // whole point of the `SecretUrl` type is that redaction is a property of
        // the type rather than of how the value arrived.
        let rendered = rendered(&via_file);
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
    /// six, rather than one silently winning.
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
            "[storage.audit]\nurl = \"postgres://u:p@h:5432/d\"\n\
             url_file = \"/dev/null\"\n",
            "[storage.migrate.party]\nurl = \"postgres://u:p@h:5432/d\"\n\
             url_file = \"/dev/null\"\n",
            "[[auth.basic.users]]\nuser = \"a\"\npassword_hash = \"x\"\n\
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
                    "FERROEHR__STORAGE__PARTY__URL_FILE",
                    &migrate.path().display().to_string(),
                ),
                (
                    "FERROEHR__STORAGE__MIGRATE__AUDIT__URL_FILE",
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
        assert_eq!(
            c.storage.migrate_dsn(crate::db::domain::Domain::Audit),
            Some("postgres://m:p@h:5432/d"),
            "a relocated domain's migration credential arrives through the env grammar"
        );
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

    /// Userinfo written into ANY URL key reaches neither the TOML rendering
    /// (`ferroehr config check`), the TOML table (`ferroehr report`), the JSON
    /// tree (`/management/env`, `GET /admin/config`) nor `Debug`; each URL
    /// keeps its host and path with the userinfo masked.
    #[test]
    fn url_userinfo_reaches_no_rendering() {
        use ferrohealth_smart::config::credentials::OauthSection;

        let leaky =
            |n: u32| SecretUrl::new(format!("https://u{n}:URL_PW_SENTINEL_{n}@h{n}:1/p{n}"));
        let mut c = standalone();
        c.db.url = leaky(1);
        c.db.migrate_url = Some(leaky(2));
        c.storage.clinical.url = Some(leaky(3));
        c.storage.party.url = Some(leaky(4));
        c.storage.migrate.clinical.url = Some(leaky(5));
        c.storage.audit.url = Some(leaky(6));
        c.storage.migrate.party.url = Some(leaky(15));
        c.audit.fhir_feed.url = leaky(8);
        c.terminology.server.url = leaky(9);
        c.terminology.server.oauth = Some(OauthSection {
            token_endpoint: "https://u10:URL_PW_SENTINEL_10@h10:1/p10".to_owned(),
            ..OauthSection::default()
        });
        c.authz.abac.remote.server = Some(leaky(12));
        c.telemetry.otlp = Some(ferrohealth_telemetry::section::OtlpSection {
            endpoint: leaky(13),
            ..ferrohealth_telemetry::section::OtlpSection::default()
        });
        c.multimedia.endpoint = Some(leaky(14));
        // URL 11 was the usage report endpoint, whose type refuses userinfo, so it
        // can hold no sentinel; URL 7 was the removed event broker.
        assert!(
            ferrohealth_usage_report::section::Endpoint::new(
                "https://u11:URL_PW_SENTINEL_11@h11:1/p11"
            )
            .is_err(),
            "the usage report endpoint refuses userinfo"
        );

        let toml = rendered(&c);
        let table = c.to_redacted_table().expect("table").to_string();
        let json = serde_json::to_string(&c.to_redacted_json().expect("json")).expect("json");
        let debug = format!("{c:?}");
        for (surface, rendered) in [
            ("toml", &toml),
            ("table", &table),
            ("json", &json),
            ("debug", &debug),
        ] {
            assert!(
                !rendered.contains("URL_PW_SENTINEL"),
                "URL userinfo leaked into the {surface} rendering: {rendered}"
            );
            for n in (1..=15).filter(|n| *n != 7 && *n != 11) {
                assert!(
                    rendered.contains(&format!("https://***@h{n}:1/p{n}")),
                    "URL {n} lost its host and path in the {surface} rendering: {rendered}"
                );
            }
        }
    }

    /// A refusal of an issuer or a published SMART URL never prints the
    /// userinfo it carries.
    #[test]
    fn an_issuer_refusal_masks_userinfo() {
        let mut c = standalone();
        c.smart.enabled = true;
        c.smart.public_base_url = Some("https://a:SMART_PW_SENTINEL@cdr.example".to_owned());
        c.auth.shared.issuer = auth_section::machines("https://b:OIDC_PW_SENTINEL@idp.example/r");
        let text = c.validate().expect_err("refused").to_string();
        assert!(!text.contains("SMART_PW_SENTINEL"), "{text}");
        assert!(!text.contains("OIDC_PW_SENTINEL"), "{text}");
    }

    /// `to_redacted_json` masks EVERY secret-bearing leaf in the whole config
    /// tree — the body `GET /admin/config` returns. Each secret is populated
    /// with a unique high-entropy sentinel; none may appear in the rendered
    /// JSON, while non-secret siblings (a Basic user's `user`/`scopes`) stay
    /// visible. This is the standing enumeration of the current secret set: a
    /// new secret field added without redaction is caught here once wired
    /// into the fixture.
    #[test]
    fn redacted_json_masks_every_secret_field() {
        use ferrohealth_secret::secret::Secret;
        use ferrohealth_secret::secret_url::SecretUrl;
        use ferrohealth_smart::config::{BasicSection, BasicUser};

        // Each sentinel is unique so a leak is unambiguously attributable.
        const DB_PW: &str = "DB_PW_SENTINEL_9a1c";
        const BASIC_HASH: &str = "$argon2id$BASIC_HASH_SENTINEL_7b2d";
        const PASSPHRASE: &str = "PASSPHRASE_SENTINEL_1d5a";
        const S3_KEY: &str = "S3_SECRET_SENTINEL_8c6b";

        let mut c = standalone();
        c.db.url = SecretUrl::new(format!(
            "postgres://dbuser:{DB_PW}@db.internal:5432/ferroehr"
        ));
        c.auth.shared.basic = Some(BasicSection {
            users: vec![BasicUser {
                user: "alice".to_owned(),
                password_hash: Some(secrecy::SecretString::from(BASIC_HASH)),
                password_hash_file: None,
                scopes: vec!["user/composition-*.r".to_owned()],
                roles: Vec::new(),
            }],
            cache_ttl_seconds: 0,
        });
        c.auth.shared.issuer = auth_section::machines("https://idp.example");
        c.signing.key_passphrase = Some(Secret::new(PASSPHRASE));
        c.multimedia.secret_access_key = Some(Secret::new(S3_KEY));
        c.multimedia.access_key_id = Some("AKIA_PUBLIC_ID".to_owned());

        let value = c.to_redacted_json().expect("render redacted json");
        let rendered = serde_json::to_string(&value).expect("stringify");

        for sentinel in [DB_PW, BASIC_HASH, PASSPHRASE, S3_KEY] {
            assert!(
                !rendered.contains(sentinel),
                "secret leaked into GET /admin/config body: {sentinel} in {rendered}"
            );
        }

        // Structural placeholders present where a secret was set.
        assert_eq!(value["auth"]["basic"]["users"][0]["password_hash"], "***");
        assert_eq!(value["signing"]["key_passphrase"], "***");
        assert_eq!(value["multimedia"]["secret_access_key"], "***");
        assert_eq!(
            value["db"]["url"],
            "postgres://***@db.internal:5432/ferroehr"
        );

        // Non-secret identifiers stay visible (they are not credentials).
        assert_eq!(value["auth"]["basic"]["users"][0]["user"], "alice");
        assert_eq!(
            value["auth"]["basic"]["users"][0]["scopes"][0],
            "user/composition-*.r"
        );
        assert_eq!(value["auth"]["issuer"][0]["issuer"], "https://idp.example");
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
                // The explicit standalone opt-in, without which the default
                // IHE provider refuses for want of an endpoint.
                ("FERROEHR__EHR_INDEX__PROVIDER", "local"),
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

    /// Both purpose lists of `[audit]` are reachable from the environment, and
    /// an emergency code that is an agreed purpose code validates.
    #[test]
    fn the_audit_purpose_lists_map_from_the_environment() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__AUDIT__PURPOSE_CODES", "TREAT,ETREAT"),
                ("FERROEHR__AUDIT__EMERGENCY_PURPOSE_CODES", "ETREAT"),
                ("FERROEHR__EHR_INDEX__PROVIDER", "local"),
            ]),
            &[],
        );
        assert_eq!(
            c.audit.purpose_codes,
            vec!["TREAT".to_owned(), "ETREAT".to_owned()]
        );
        assert_eq!(c.audit.emergency_purpose_codes, vec!["ETREAT".to_owned()]);
        c.validate()
            .expect("an emergency code on the purpose list validates");
    }

    /// An emergency purpose code that is not an agreed purpose code is a boot
    /// error naming the code, including when no purpose list is configured.
    #[test]
    fn an_emergency_code_outside_the_purpose_codes_is_refused() {
        let mut config = standalone();
        config.audit.emergency_purpose_codes = vec!["ETREAT".to_owned()];
        let text = config
            .validate()
            .expect_err("no purpose list agrees ETREAT")
            .to_string();
        assert!(
            text.contains("\"ETREAT\"") && text.contains("audit.purpose_codes"),
            "{text}"
        );

        config.audit.purpose_codes = vec!["TREAT".to_owned()];
        config
            .validate()
            .expect_err("TREAT alone does not agree ETREAT");

        config.audit.purpose_codes.push("ETREAT".to_owned());
        config.validate().expect("ETREAT is agreed");
    }

    /// A `restricted` EHR default with authentication off is a boot error, since
    /// nothing could enforce it; with authentication on it validates, RBAC on or
    /// off, and an `open` default never trips the check.
    #[test]
    fn a_restricted_ehr_default_without_authentication_is_refused() {
        let mut config = standalone();
        config.authz.rbac.ehr_access_default = authz::EhrAccessDefault::Restricted;
        config
            .validate()
            .expect("restricted with authentication validates");
        config.authz.rbac.enabled = false;
        config
            .validate()
            .expect("restricted with RBAC off validates: the per-EHR gate applies it");

        config.auth.shared.enabled = false;
        let text = config
            .validate()
            .expect_err("restricted cannot be enforced without authentication")
            .to_string();
        assert!(
            text.contains("ehr_access_default") && text.contains("auth.enabled = false"),
            "{text}"
        );

        config.authz.rbac.ehr_access_default = authz::EhrAccessDefault::Open;
        config
            .validate()
            .expect("an open default needs no authentication");
    }

    /// The `[usage_report]` section maps from the environment, the switch
    /// turns it off, and a plain-HTTP collector off loopback is refused.
    #[test]
    fn the_usage_report_section_maps_from_the_environment() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__USAGE_REPORT__ENABLED", "false"),
                (
                    "FERROEHR__USAGE_REPORT__ENDPOINT",
                    "https://collector.example/v1/report",
                ),
                ("FERROEHR__USAGE_REPORT__DEPLOYMENT", "compose"),
                ("FERROEHR__EHR_INDEX__PROVIDER", "local"),
            ]),
            &[],
        );
        assert!(!c.usage_report.enabled);
        assert_eq!(
            c.usage_report.endpoint.url().as_str(),
            "https://collector.example/v1/report"
        );
        assert_eq!(
            c.usage_report.deployment,
            ferrohealth_usage_report::section::Deployment::Compose
        );
        c.validate()
            .expect("a well-formed usage report section validates");

        let refused = assemble(
            None,
            &env(&[(
                "FERROEHR__USAGE_REPORT__ENDPOINT",
                "http://collector.example/v1/report",
            )]),
            &[],
        )
        .expect_err("plain http off loopback is refused");
        assert!(
            refused.to_string().contains("usage_report.endpoint"),
            "{refused}"
        );
    }

    /// `slow_aql_ms` left the section with the AQL series, and a state
    /// directory is refused: the state lives in the clinical database.
    #[test]
    fn the_usage_report_section_refuses_the_keys_ferroehr_does_not_use() {
        let gone = assemble(
            None,
            &env(&[("FERROEHR__USAGE_REPORT__SLOW_AQL_MS", "250")]),
            &[],
        )
        .expect_err("slow_aql_ms is no longer a key");
        assert!(gone.to_string().contains("slow_aql_ms"), "{gone}");

        let c = assemble_ok(
            None,
            &env(&[("FERROEHR__USAGE_REPORT__STATE_DIR", "/var/lib/ferroehr")]),
            &[],
        );
        let refused = c.validate().expect_err("a state directory is refused");
        assert!(
            refused.to_string().contains("usage_report.state_dir"),
            "{refused}"
        );
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
        // NOTE: no specification governs this: our own design; the loader drops a blank
        // part of a comma-separated variable, so the blank namespace comes from the file.
        let file = toml_file("[privacy]\nsubject_namespaces = [\"  \"]\n");
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
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
        use crate::service::linkage::cohort::config::PredicateBinding;

        let with = |key: &str, binding: PredicateBinding| {
            let mut c = standalone();
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
        let sections = match serde_json::to_value(FerroEhrConfig::default()) {
            Ok(serde_json::Value::Object(members)) => members.keys().cloned().collect::<Vec<_>>(),
            other => panic!("the tree serializes as an object: {other:?}"),
        };
        for section in sections {
            let header = format!("[{section}]");
            let dotted = format!("[{section}."); // sub-tables count too
            let scalar = format!("\n{section} = "); // top-level scalar keys
            assert!(
                DEFAULT_TEMPLATE.contains(&header)
                    || DEFAULT_TEMPLATE.contains(&dotted)
                    || DEFAULT_TEMPLATE.contains(&scalar)
                    || DEFAULT_TEMPLATE.contains(&format!("\n#? {section} = ")),
                "template missing section {section}"
            );
        }
    }

    /// Whether the template declares `key` as a key line — live (`key = …`) or
    /// as a `#?` reference line for an optional/secret/derived one. A mention
    /// inside another key's trailing comment does not count: a key mentioned
    /// only that way stays invisible to an operator reading the template.
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
             complete configuration, and the shipped template is how an \
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
                "auth",
                auth_section::live_fields(
                    accepted_fields::<ferrohealth_smart::config::AuthSection>("auth"),
                ),
            ),
            (
                "authz.assurance",
                accepted_fields::<ferrohealth_smart::assurance::AssuranceSection>(
                    "authz.assurance",
                ),
            ),
            (
                "authz.party_binding",
                accepted_fields::<binding::PartyBindingConfig>("authz.party_binding"),
            ),
            ("db", accepted_fields::<crate::db::DbConfig>("db")),
            ("ehr_index", accepted_fields::<EhrIndexConfig>("ehr_index")),
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
                // A sub-table field is declared by its own `[section.field]`
                // header, exactly as the serialized-field walk accepts one, or
                // by the `[[section.field.…]]` header of the array it holds.
                fields
                    .iter()
                    .filter(move |f| {
                        !template_declares(f)
                            && !DEFAULT_TEMPLATE.contains(&format!("[{section}.{f}]"))
                            && !DEFAULT_TEMPLATE.contains(&format!("[[{section}.{f}."))
                    })
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
            let mut c = standalone();
            c.multimedia.enabled = true;
            c.multimedia.endpoint = Some(SecretUrl::new(bad));
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
        let mut c = standalone();
        c.multimedia.enabled = true;
        c.multimedia.endpoint = None;
        assert!(c.validate().is_ok(), "an absent endpoint is legitimate");
        c.multimedia.endpoint = Some(SecretUrl::new("http://seaweedfs:8333"));
        assert!(c.validate().is_ok(), "an absolute endpoint is accepted");
    }

    // ── 7. Semantic validation ────────────────────────────────────────────────

    #[test]
    fn validate_pgp_requires_key_path() {
        let mut c = standalone();
        c.signing.mode = crate::versioning::signature::config::Mode::Pgp;
        assert!(c.validate().is_err());
        c.signing.key_path = Some(PathBuf::from("/k.asc"));
        assert!(c.validate().is_ok());
    }

    /// The discovery document is built from the `[auth]` issuer's metadata and
    /// sends applications to that issuer, so serving it without an issuer whose
    /// tokens this server accepts is refused.
    #[test]
    fn a_served_smart_document_needs_an_issuer() {
        let mut c = standalone();
        c.smart.enabled = true;
        c.smart.public_base_url = Some("https://cdr.example.com".to_owned());
        let err = c
            .validate()
            .expect_err("SMART without an [auth] issuer must refuse");
        assert!(format!("{err:?}").contains("[auth] issuer"), "{err:?}");
        c.auth.shared.issuer = auth_section::machines("https://as.example/realms/ferroehr");
        c.authz.party_binding.provider = Some(binding::one_provider());
        assert!(c.validate().is_ok(), "{:?}", c.validate());
    }

    /// The HMAC key source and the inline key set are gone: both are unknown
    /// keys, refused by the loader like any typo, so no tree carries competing
    /// key sources.
    #[test]
    fn the_removed_key_sources_are_unknown_keys() {
        for body in [
            "[auth]\nhmac_secret = \"0123456789abcdef0123456789abcdef\"\n",
            "[auth]\njwks_json = \"{}\"\n",
            "[auth.oidc]\nissuer = \"https://idp.example.test\"\n",
        ] {
            let file = toml_file(body);
            assert!(
                assemble(Some(file.path()), &env(&[]), &[]).is_err(),
                "{body} must be refused"
            );
        }
    }

    /// RFC 7518 §6.4: a symmetric key in `auth.jwks_file` refuses the whole
    /// tree, so neither the boot path nor `ferroehr config check` nor
    /// `ferroehr report` (each validates before rendering) can print it.
    #[test]
    fn a_symmetric_jwks_key_refuses_the_tree_before_any_rendering() {
        let secret_k = "b2N0LWtleS1tYXRlcmlhbC10aGF0LW11c3QtbmV2ZXItcHJpbnQ";
        let jwks = assert_fs::NamedTempFile::new("jwks.json").expect("temp");
        jwks.write_str(&format!(r#"{{"keys":[{{"kty":"oct","k":"{secret_k}"}}]}}"#))
            .expect("write");
        let mut c = standalone();
        let mut issuer = auth_section::machines("https://idp.example.test");
        if let ferrohealth_smart::config::IssuerKey::List(entries) = &mut issuer {
            entries[0].jwks_file = jwks.path().display().to_string();
        }
        c.auth.shared.issuer = issuer;
        let err = c.validate().expect_err("an oct key must refuse the tree");
        let text = err.to_string();
        assert!(
            text.contains("jwks_file") && text.contains("symmetric key"),
            "{text}"
        );
        assert!(!text.contains(secret_k), "{text}");
    }

    /// The `[auth]` boot rules reach the aggregated tree validation, so
    /// `ferroehr config check` and the boot path refuse the same shapes the
    /// section's own unit tests pin (RFC 7519 §4.1.3, RFC 8414 §2, RFC 9068 §4).
    #[test]
    fn validate_reports_auth_section_errors() {
        let mut c = standalone();
        c.auth.shared.issuer = auth_section::machines("http://idp.example.test");
        let err = c.validate().expect_err("an http issuer must refuse");
        assert!(err.to_string().contains("auth:"), "{err}");

        let mut issuer = auth_section::machines("https://idp.example.test");
        if let ferrohealth_smart::config::IssuerKey::List(entries) = &mut issuer {
            entries[0].audience = Vec::new();
        }
        c.auth.shared.issuer = issuer;
        let err = c.validate().expect_err("no audience must refuse");
        assert!(err.to_string().contains("audience"), "{err}");

        c.auth.shared.issuer = auth_section::machines("https://idp.example.test");
        c.authz.party_binding.provider = Some(binding::one_provider());
        assert!(c.validate().is_ok(), "{:?}", c.validate());
    }

    #[test]
    fn validate_management_port_must_differ_from_the_listen_port() {
        let mut c = standalone();
        c.server.listen = "0.0.0.0:8080".parse().expect("an address");
        c.management.port = Some(8080);
        assert!(c.validate().is_err());
        c.management.port = Some(9100);
        assert!(c.validate().is_ok());
    }

    /// The clinical body tier is refused at or above the listener's limit, the
    /// ceiling the bulk routes take, and accepted below it.
    #[test]
    fn validate_keeps_the_clinical_body_tier_below_the_listener_limit() {
        let mut c = standalone();
        c.server.body_limit_bytes = 64;
        c.rest.body_limit_bytes = Some(64);
        let err = c.validate().expect_err("a tier at the ceiling").to_string();
        assert!(err.contains("rest.body_limit_bytes"), "{err}");
        c.rest.body_limit_bytes = Some(16);
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
            let mut c = standalone();
            c.rest.base_path = base_path.to_owned();
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
            let mut c = standalone();
            c.rest.base_path = base_path.to_owned();
            let err = c
                .validate()
                .expect_err("a malformed base path must be refused")
                .to_string();
            assert!(err.contains("rest.base_path"), "{base_path}: {err}");
            assert!(err.contains(expected), "{base_path}: {err}");
        }
    }

    /// Validation is aggregated: a value breaking several rules reports them
    /// all in one boot error, so an operator fixes the key in one iteration.
    #[test]
    fn validate_reports_every_broken_base_path_rule_at_once() {
        let mut c = standalone();
        c.rest.base_path = "cdr//api/".to_owned();
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

    /// `rest.system_id` is judged by the openEHR `uid` grammar itself.
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
        let mut c = standalone();
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
            c.rest.system_id = refused.to_owned();
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
            c.rest.system_id = accepted.to_owned();
            assert!(
                c.validate().is_ok(),
                "system_id {accepted:?} is a legal openEHR uid and must boot"
            );
        }
    }

    /// Enabling the terminology server with no URL fails to boot, naming the
    /// key. The section reads its defaults from one `Default` impl, so serde
    /// cannot report the missing key itself — the semantic pass owns it, which
    /// is also what lets every configuration error surface at once.
    #[test]
    fn validate_terminology_server_needs_a_url() {
        let mut c = standalone();
        c.terminology.server.enabled = true;
        let err = c
            .validate()
            .expect_err("an enabled server with no url must be rejected");
        assert!(
            err.to_string().contains("terminology.server.url"),
            "got {err}"
        );

        c.terminology.server.url = SecretUrl::new("https://ts.example/fhir");
        assert!(c.validate().is_ok());
    }

    /// The terminology server's client-credentials grant is checked like every
    /// other outbound grant: an incomplete one is a boot error naming its
    /// keys, never a silently unauthenticated terminology request.
    #[test]
    fn validate_terminology_server_oauth_shape() {
        use ferrohealth_smart::config::credentials::OauthSection;

        let mut c = standalone();
        c.terminology.server.enabled = true;
        c.terminology.server.url = SecretUrl::new("https://ts.example/fhir");
        c.terminology.server.oauth = Some(OauthSection::default());
        let err = c
            .validate()
            .expect_err("a grant with no token endpoint, client id or secret is rejected");
        for key in [
            "terminology.server.oauth",
            "token_endpoint",
            "client_id",
            "client_secret",
        ] {
            assert!(err.to_string().contains(key), "{key} unnamed in {err}");
        }

        c.terminology.server.oauth = Some(OauthSection {
            token_endpoint: "https://idp.example/token".to_owned(),
            client_id: "cdr".to_owned(),
            client_secret: Some(secrecy::SecretString::from("s3cret")),
            ..OauthSection::default()
        });
        assert!(c.validate().is_ok());

        // A grant beside userinfo in the server URL would send two
        // authorizations.
        c.terminology.server.url = SecretUrl::new("https://u:p@ts.example/fhir");
        let err = c
            .validate()
            .expect_err("userinfo beside a grant is rejected");
        assert!(
            err.to_string()
                .contains("terminology.server carries credentials"),
            "got {err}"
        );

        // A disabled server's grant is not in use, so it is not checked.
        c.terminology.server.enabled = false;
        c.terminology.server.oauth = Some(OauthSection::default());
        assert!(c.validate().is_ok());
    }

    /// Every terminology key is a scalar, so the whole section, its grant
    /// included, is reachable through the one env grammar.
    #[test]
    fn env_mapping_terminology_server_and_oauth() {
        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__TERMINOLOGY__SERVER__ENABLED", "true"),
                (
                    "FERROEHR__TERMINOLOGY__SERVER__URL",
                    "http://ferroterm:8080/r4b",
                ),
                ("FERROEHR__TERMINOLOGY__SERVER__FAIL_ON_ERROR", "true"),
                ("FERROEHR__TERMINOLOGY__SERVER__REQUEST_TIMEOUT_MS", "2500"),
                (
                    "FERROEHR__TERMINOLOGY__SERVER__OAUTH__TOKEN_ENDPOINT",
                    "https://idp/token",
                ),
                ("FERROEHR__TERMINOLOGY__SERVER__OAUTH__CLIENT_ID", "cdr"),
                (
                    "FERROEHR__TERMINOLOGY__SERVER__OAUTH__CLIENT_SECRET",
                    "s3cret",
                ),
                (
                    "FERROEHR__TERMINOLOGY__SERVER__OAUTH__SCOPE",
                    "system/*.read",
                ),
            ]),
            &[],
        );
        let server = &c.terminology.server;
        assert!(server.enabled);
        assert!(server.fail_on_error);
        assert_eq!(server.url.expose(), "http://ferroterm:8080/r4b");
        assert_eq!(server.request_timeout_ms, 2_500);
        let oauth = server.oauth.as_ref().expect("the grant from env");
        assert_eq!(oauth.token_endpoint, "https://idp/token");
        assert_eq!(oauth.client_id, "cdr");
        assert_eq!(
            oauth
                .client_secret
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
            Some("s3cret")
        );
        assert_eq!(oauth.scope, "system/*.read");
    }

    /// The role keys moved to the shared `[auth]` section: their old place
    /// under `[authz.rbac]` stops startup naming the key, from the file and
    /// from the environment, and `[auth] role_claims` keeps the shared
    /// section's default when the file leaves it out.
    #[test]
    fn the_moved_role_keys_are_refused_at_their_old_place() {
        for (body, key) in [
            ("[authz.rbac]\nrole_claims = [\"roles\"]\n", "role_claims"),
            (
                "[authz.rbac.basic_roles]\nalice = [\"ADMIN\"]\n",
                "basic_roles",
            ),
        ] {
            let file = toml_file(body);
            let err = assemble(Some(file.path()), &env(&[]), &[]).expect_err("moved key");
            assert!(err.to_string().contains(key), "{key}: {err}");
        }
        let err = assemble(
            None,
            &env(&[("FERROEHR__AUTHZ__RBAC__ROLE_CLAIMS", "roles")]),
            &[],
        )
        .expect_err("moved env key");
        assert!(err.to_string().contains("ROLE_CLAIMS"), "{err}");

        let c = assemble_ok(
            None,
            &env(&[
                ("FERROEHR__AUTH__ISSUER__0__ISSUER", "https://idp.example/r"),
                ("FERROEHR__AUTH__ISSUER__0__ROLE_CLAIMS", "app_roles,roles"),
            ]),
            &[],
        );
        let roles = c.auth.shared.issuers()[0].role_claims.as_deref();
        assert_eq!(
            roles,
            Some(&["app_roles".to_owned(), "roles".to_owned()][..])
        );
    }

    /// Every outbound grant is the `ferrohealth-smart` client table, so the
    /// key `token_url` that preceded `token_endpoint` stops startup by name
    /// in each of the three tables that take a grant.
    #[test]
    fn the_old_token_url_key_is_refused_in_every_grant() {
        for table in [
            "terminology.server.oauth",
            "audit.fhir_feed.oauth",
            "authz.abac.remote.oauth",
        ] {
            let file = toml_file(&format!(
                "[{table}]\ntoken_url = \"https://idp.example/token\"\nclient_id = \"c\"\n"
            ));
            let err =
                assemble(Some(file.path()), &env(&[]), &[]).expect_err("the old key is unknown");
            assert!(err.to_string().contains("token_url"), "{table}: {err}");
        }
    }
}
