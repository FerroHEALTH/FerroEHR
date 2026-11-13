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
pub mod agreement;
pub mod auth;
pub mod auth_section;
pub mod authz;
pub mod binding;
pub mod clients;
pub mod deployment;
pub mod management;
pub mod profile;
pub mod record;
pub mod redact;
pub mod rest;
pub mod retention;
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
/// table a boot error. One table per concern: what the deployment is, the
/// family's shared sections (`[server]`, `[telemetry]`, `[usage_report]`,
/// `[auth]`), FerroEHR's own surfaces and policies, and one table per outbound
/// integration (`[terminology]`, `[pixm]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-must-set" = ["database.url"],
    "x-ferroehr-class" = { "server": "infrastructure", "telemetry": "infrastructure", "usage_report": "infrastructure", "database": "infrastructure",
    "database.max_connections": "optional", "database.min_connections": "optional", "database.acquire_timeout": "optional", "database.statement_timeout": "optional" })))]
pub struct FerroEhrConfig {
    /// `[deployment]`: the declared posture, the openEHR specification set
    /// and the licence token.
    pub deployment: profile::DeploymentConfig,
    /// `[server]` — the HTTP listener: the family's section (the listen
    /// address, the timeouts, the body limit, the overload bound, the drain,
    /// TLS, the trusted proxies, the caller rate and compression).
    pub server: ferrohealth_server::section::ServerSection,
    /// `[telemetry]`: the console, the request log and the OpenTelemetry
    /// export, the family's shared section.
    pub telemetry: ferrohealth_telemetry::section::TelemetrySection,
    /// `[usage_report]` — the installation report to `FerroPULSE` (on by default).
    pub usage_report: ferrohealth_usage_report::section::UsageReportSection,
    /// `[auth]` — authentication: the `ferrohealth-smart` section, whose
    /// serialization writes every secret as `***`, `[auth.idp]`,
    /// `[auth.smart]` and `[auth.scim]`.
    pub auth: auth_section::AuthConfig,
    /// `[authz]` — RBAC, ABAC, assurance, the party binding, the system
    /// clients, the federation agreements and the grant ceiling.
    pub authz: authz::AuthzConfig,
    /// `[api]` — the openEHR REST API: the base path, the system identifier,
    /// the `OPTIONS` identity, the `OpenAPI` UI, CORS, the clinical body tier
    /// and the ADMIN and terminology groups.
    pub api: rest::ApiConfig,
    /// `[database]` — the shared `PostgreSQL` DSN, pool tuning, migrations
    /// and one table per storage domain.
    pub database: crate::db::DbConfig,
    /// `[management]` — the management/observability surface.
    pub management: management::ManagementConfig,
    /// `[audit]` — the IHE ATNA audit trail / System Log (local Audit Record
    /// Repository + the syslog and FHIR-feed forwarding sinks).
    pub audit: crate::system_log::config::AuditConfig,
    /// `[privacy]` — the data-minimisation policy: the subject namespaces,
    /// the identifier scan and protection, the cohort query and the access
    /// delay.
    pub privacy: crate::privacy::config::PrivacyConfig,
    /// `[record]` — the record itself: the subject index, versioning,
    /// signing, the query engine, the change feed and multimedia.
    pub record: record::RecordConfig,
    /// `[terminology]` — the outbound FHIR terminology server; absent, none
    /// is asked.
    pub terminology: Option<crate::service::terminology::config::TerminologyServerConfig>,
    /// `[pixm]` — the IHE PIXm patient identity source the subject index asks
    /// (ITI-83) under `record.subject_index = "pixm"`.
    pub pixm: Option<crate::service::ehr_index::config::PixmConfig>,
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
            deployment: profile::DeploymentConfig::default(),
            server: ferrohealth_server::section::ServerSection {
                listen: std::net::SocketAddr::from(([0, 0, 0, 0], 8080)),
                body_limit: ferrohealth_config::units::ByteSize::from_mib(64),
                ..ferrohealth_server::section::ServerSection::default()
            },
            telemetry: ferrohealth_telemetry::section::TelemetrySection::default(),
            usage_report: ferrohealth_usage_report::section::UsageReportSection::default(),
            auth: auth_section::AuthConfig::default(),
            authz: authz::AuthzConfig::default(),
            api: rest::ApiConfig::default(),
            database: crate::db::DbConfig::default(),
            management: management::ManagementConfig::default(),
            audit: crate::system_log::config::AuditConfig::default(),
            privacy: crate::privacy::config::PrivacyConfig::default(),
            record: record::RecordConfig::default(),
            terminology: None,
            pixm: None,
        }
    }
}

/// The shipped default `ferroehr.toml`, which `tools/config-reference` generates (#5316, #5455).
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
    /// Redaction is a property of the leaf type alone: every secret-bearing
    /// field is typed `ferrohealth-secret`'s `Secret`, whose [`Serialize`]
    /// writes `ferrohealth_secret::REDACTED`, or `SecretUrl`, whose
    /// [`Serialize`] masks the URL `userinfo` and query. Every URL key that may
    /// carry `userinfo` is a `SecretUrl`, and no pass over the tree or key-name
    /// scan follows the types. An issuer is an identifier, refused at boot when
    /// it carries `userinfo`. The `redacted_json_masks_every_secret_field` and
    /// `url_userinfo_reaches_no_rendering` tests enumerate the secret and URL
    /// keys. Non-secret identifiers, such as a Basic user's `user` and
    /// `scopes`, an `issuer`, or `auth.jwks_file` (a path), stay visible.
    ///
    /// # Errors
    /// [`ConfigError`] if the tree cannot be serialized to JSON.
    pub fn to_redacted_json(&self) -> Result<serde_json::Value, ConfigError> {
        serde_json::to_value(self)
            .map_err(|e| ConfigError::semantic(format!("rendering config as JSON: {e}")))
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
    if config.database.is_dev_default() {
        tracing::warn!(
            url = crate::db::DEFAULT_URL,
            "database.url is the built-in DEVELOPMENT DEFAULT ({}); no file/env/CLI value \
             was supplied. Set database.url (or FERROEHR__DATABASE__URL / DATABASE_URL) for \
             any non-dev deployment — production MUST override it.",
            crate::db::DEFAULT_URL,
        );
    }

    Ok((loaded, config))
}

#[cfg(test)]
mod audit_tests;
#[cfg(test)]
mod secret_tests;
#[cfg(test)]
mod template_tests;
#[cfg(test)]
mod tests {
    use assert_fs::prelude::*;

    use std::path::PathBuf;

    use super::*;
    use crate::config::authz::AbacParam;
    use crate::service::ehr_index::config::SubjectIndex;
    use crate::storage::cohort_predicate::PredicateKind;
    use ferrohealth_secret::secret_url::SecretUrl;

    /// The shipped defaults of a FerroEHR run on its own: the explicit
    /// `record.subject_index = "local"` opt-in, without which the unset index
    /// refuses the start.
    pub(super) fn standalone() -> FerroEhrConfig {
        let mut config = FerroEhrConfig::default();
        config.record.subject_index = Some(SubjectIndex::Local);
        config
    }

    /// The shipped defaults, with no identity source and no opt-in, are
    /// refused, and the refusal names both choices; the opt-in alone passes.
    #[test]
    fn the_defaults_refuse_until_an_identity_source_or_local_is_chosen() {
        let refused = FerroEhrConfig::default()
            .validate()
            .expect_err("no identity source and no opt-in");
        let text = refused.to_string();
        assert!(text.contains("subject_index = \"pixm\""), "{text}");
        assert!(text.contains("subject_index = \"local\""), "{text}");
        assert!(standalone().validate().is_ok());
    }

    /// `c` serialized as the TOML table `ferroehr config check` renders
    /// (`ferrohealth_config::report::check_report` writes the same table).
    pub(super) fn rendered(c: &FerroEhrConfig) -> String {
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

    /// The refusal `assemble` gives a file carrying `content`, as text.
    pub(super) fn file_refusal(content: &str) -> String {
        let file = toml_file(content);
        assemble(Some(file.path()), &env(&[]), &[])
            .expect_err(content)
            .to_string()
    }

    pub(super) fn assemble_ok(
        file: Option<&Path>,
        env: &HashMap<String, String>,
        overrides: &[(String, String)],
    ) -> FerroEhrConfig {
        match assemble(file, env, overrides) {
            Ok(c) => c,
            Err(e) => panic!("assemble failed: {e}"),
        }
    }

    pub(super) fn json(c: &FerroEhrConfig) -> serde_json::Value {
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
        let file = toml_file("[database]\nmax_connections = 5\n");
        // file overrides default (20).
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        assert_eq!(c.database.max_connections, 5);
        let c = assemble_ok(
            Some(file.path()),
            &env(&[("FERROEHR__DATABASE__MAX_CONNECTIONS", "9")]),
            &[],
        );
        assert_eq!(c.database.max_connections, 9);
        let c = assemble_ok(
            Some(file.path()),
            &env(&[("FERROEHR__DATABASE__MAX_CONNECTIONS", "9")]),
            &[("database.max_connections".to_owned(), "11".to_owned())],
        );
        assert_eq!(c.database.max_connections, 11);
    }

    #[test]
    fn conventional_aliases_lose_to_ferroehr_forms() {
        let c = assemble_ok(None, &env(&[("DATABASE_URL", "postgres://a@h/x")]), &[]);
        assert_eq!(c.database.url.expose(), "postgres://a@h/x");
        let c = assemble_ok(
            None,
            &env(&[
                ("DATABASE_URL", "postgres://a@h/x"),
                ("FERROEHR__DATABASE__URL", "postgres://b@h/y"),
            ]),
            &[],
        );
        assert_eq!(c.database.url.expose(), "postgres://b@h/y");
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
            c.database.url.expose(),
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
        assert_eq!(c.database.url.expose(), "postgres://a@h/x");
        let c = assemble_ok(
            None,
            &env(&[
                ("PGHOST", "db.example.neon.tech"),
                ("DATABASE_URL", "postgres://a@h/x"),
                ("FERROEHR__DATABASE__URL", "postgres://b@h/y"),
            ]),
            &[],
        );
        assert_eq!(c.database.url.expose(), "postgres://b@h/y");
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
        assert_eq!(c.database.url.expose(), "postgres://direct@h/x");
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
            c.database.url.expose(),
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
                ("FERROEHR__AUTH__ISSUER__0__TIMEOUT", "1500ms"),
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
        assert_eq!(issuer.issuer.expose(), "https://idp");
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
        assert_eq!(
            issuer.timeout,
            ferrohealth_config::units::Duration::from_millis(1_500)
        );
        assert_eq!(
            issuer.jwks_max_age,
            ferrohealth_config::units::Duration::from_mins(10)
        );
        assert_eq!(
            issuer.jwks_refresh_interval,
            ferrohealth_config::units::Duration::from_mins(1)
        );
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
    /// state (`signing.retired_key_files`, the former `smart.endpoints.capabilities`) and
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
                    "FERROEHR__RECORD__SIGNING__RETIRED_KEY_FILES",
                    "/etc/ferroehr/retired-2025.pub.asc",
                ),
                ("FERROEHR__AUTH__CAPABILITIES", "launch-ehr"),
                ("FERROEHR__AUTH__ISSUER__0__ISSUER", "https://idp"),
                ("FERROEHR__AUTH__ISSUER__0__AUDIENCE", "ferroehr"),
                ("FERROEHR__AUTH__ISSUER__0__ALGORITHMS", "ES256"),
                ("FERROEHR__AUTH__SCOPES_SUPPORTED", "openid"),
                ("FERROEHR__AUTH__SMART__BACKEND_CLIENTS", "etl"),
                (
                    "FERROEHR__AUTH__SMART__PATIENT_ISSUERS",
                    "https://as.example",
                ),
                ("FERROEHR__AUTH__SMART__DEMOGRAPHIC_CLIENTS", "mpi"),
            ]),
            &[],
        );
        assert_eq!(
            c.record.signing.retired_key_files,
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
        assert_eq!(c.auth.smart.backend_clients, vec!["etl".to_owned()]);
        assert_eq!(
            c.auth.smart.patient_issuers,
            vec![SecretUrl::new("https://as.example")]
        );
        assert_eq!(c.auth.smart.demographic_clients, vec!["mpi".to_owned()]);
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
                    "FERROEHR__RECORD__SIGNING__RETIRED_KEY_FILES",
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
            c.record.signing.retired_key_files,
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
                ("FERROEHR__AUTH__ISSUER__0__CLOCK_SKEW", "2min"),
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
        assert_eq!(
            issuer.clock_skew,
            ferrohealth_config::units::Duration::from_mins(2)
        );
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
        assert_eq!(d.auth.shared.issuers(), []);
        let entry = ferrohealth_smart::config::IssuerEntry::default();
        assert_eq!(
            entry.clock_skew,
            ferrohealth_config::units::Duration::from_secs(60),
            "the default leeway is the conventional 60 s"
        );
        assert_eq!(entry.profile, ferrohealth_smart::profile::Preset::Rfc9068);
    }

    /// `[api] system_id` — the CDR's own openEHR system identifier (stamped
    /// into `EHR.system_id`, the `AUDIT_DETAILS.system_id` server default, and
    /// every `OBJECT_VERSION_ID.creating_system_id`). Pins all three layers:
    /// the compatibility default, the file value, and the `FERROEHR__` env form.
    #[test]
    fn rest_system_id_default_file_and_env() {
        // Unset, the default is the service-layer constant.
        let c = assemble_ok(None, &env(&[]), &[]);
        assert_eq!(c.api.system_id, crate::service::DEFAULT_SYSTEM_ID);

        let file = toml_file("[api]\nsystem_id = \"cdr.hospital.example\"\n");
        let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
        assert_eq!(c.api.system_id, "cdr.hospital.example");

        let c = assemble_ok(
            Some(file.path()),
            &env(&[("FERROEHR__API__SYSTEM_ID", "cdr.env.example")]),
            &[],
        );
        assert_eq!(c.api.system_id, "cdr.env.example");

        // `system_id` and the `[api.options]` display identity are
        // independent knobs — setting one must not disturb the other.
        assert_eq!(c.api.options.solution, "FerroEHR");
    }

    // ── 3. Strictness ─────────────────────────────────────────────────────────

    #[test]
    fn unknown_env_var_is_a_boot_error_with_suggestion() {
        let err = assemble(None, &env(&[("FERROEHR__TELEMETRI__FILTER", "info")]), &[])
            .expect_err("unknown var");
        let msg = err.to_string();
        assert!(msg.contains("FERROEHR__TELEMETRI__FILTER"), "{msg}");
        assert!(
            msg.contains("did you mean `telemetry"),
            "did-you-mean missing: {msg}"
        );
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
        let file = toml_file("[database]\nmax_conections = 5\n");
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
        let zero = toml_file("[server]\nheader_read_timeout = \"0s\"\n");
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
            c.database.max_connections,
            FerroEhrConfig::default().database.max_connections
        );
        let c = assemble_ok(
            None,
            &env(&[("FERROEHR__DATABASE__MAX_CONNECTIONS", "7")]),
            &[],
        );
        assert_eq!(c.database.max_connections, 7);
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
                ("FERROEHR__RECORD__SUBJECT_INDEX", "local"),
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
                ("FERROEHR__RECORD__SUBJECT_INDEX", "local"),
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
                ("FERROEHR__RECORD__SUBJECT_INDEX", "local"),
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
                    "FERROEHR__PRIVACY__IDENTIFIER_PROTECTION__SCHEMES",
                    "nl-bsn,se-personnummer",
                ),
                (
                    "FERROEHR__PRIVACY__IDENTIFIER_PROTECTION__KEY",
                    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                ),
            ]),
            &[],
        );
        let protection = c
            .privacy
            .identifier_protection
            .as_ref()
            .expect("the table is present");
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
            c.privacy.cohort.predicates.insert(key.to_owned(), binding);
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
                rendered.contains("privacy.cohort.predicates") && rendered.contains(expected),
                "the refusal must name the key and the defect: {rendered}"
            );
        }
    }

    /// An enabled multimedia integration with a blank or relative endpoint is a
    /// BOOT error, not a panic on the first commit.
    #[test]
    fn an_enabled_multimedia_integration_refuses_a_blank_or_relative_endpoint() {
        for bad in ["", "   ", "seaweedfs:8333", "/bucket"] {
            let mut c = standalone();
            c.record.multimedia.enabled = true;
            c.record.multimedia.url = Some(SecretUrl::new(bad));
            let errors = c.validate().expect_err("a bad endpoint must be refused");
            assert!(
                format!("{errors:?}").contains("record.multimedia.url"),
                "the refusal must name the key so an operator can act on it, got: {errors:?}"
            );
        }
    }

    /// An absent endpoint is legitimate (default AWS resolution) and an
    /// absolute one is accepted, so the check above cannot be a blanket refusal.
    #[test]
    fn an_absent_or_absolute_multimedia_endpoint_is_accepted() {
        let mut c = standalone();
        c.record.multimedia.enabled = true;
        c.record.multimedia.url = None;
        assert!(c.validate().is_ok(), "an absent endpoint is legitimate");
        c.record.multimedia.url = Some(SecretUrl::new("http://seaweedfs:8333"));
        assert!(c.validate().is_ok(), "an absolute endpoint is accepted");
    }

    // ── 7. Semantic validation ────────────────────────────────────────────────

    #[test]
    fn validate_pgp_requires_key_path() {
        let mut c = standalone();
        c.record.signing.mode = crate::versioning::signature::config::Mode::Pgp;
        assert!(c.validate().is_err());
        c.record.signing.key_file = Some(PathBuf::from("/k.asc"));
        assert!(c.validate().is_ok());
    }

    /// The discovery document is built from the `[auth]` issuer's metadata and
    /// sends applications to that issuer, so serving it without an issuer whose
    /// tokens this server accepts is refused.
    #[test]
    fn a_served_smart_document_needs_an_issuer() {
        let mut c = standalone();
        c.auth.smart.discovery = true;
        c.auth.smart.public_base_url = Some(SecretUrl::new("https://cdr.example.com"));
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
        issuer[0].jwks_file = jwks.path().display().to_string();
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
        issuer[0].audience = Vec::new();
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
        c.management.listen = Some("0.0.0.0:8080".parse().expect("an address"));
        assert!(c.validate().is_err());
        c.management.listen = Some("127.0.0.1:8080".parse().expect("an address"));
        assert!(
            c.validate().is_err(),
            "every interface includes the loopback one"
        );
        c.management.listen = Some("0.0.0.0:9100".parse().expect("an address"));
        assert!(c.validate().is_ok());
    }

    /// The clinical body tier is refused at or above the listener's limit, the
    /// ceiling the bulk routes take, and accepted below it.
    #[test]
    fn validate_keeps_the_clinical_body_tier_below_the_listener_limit() {
        let mut c = standalone();
        c.server.body_limit = ferrohealth_config::units::ByteSize::from_bytes(64);
        c.api.body_limit = Some(ferrohealth_config::units::ByteSize::from_bytes(64));
        let err = c.validate().expect_err("a tier at the ceiling").to_string();
        assert!(err.contains("api.body_limit"), "{err}");
        c.api.body_limit = Some(ferrohealth_config::units::ByteSize::from_bytes(16));
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
            c.api.base_path = base_path.to_owned();
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
            c.api.base_path = base_path.to_owned();
            let err = c
                .validate()
                .expect_err("a malformed base path must be refused")
                .to_string();
            assert!(err.contains("api.base_path"), "{base_path}: {err}");
            assert!(err.contains(expected), "{base_path}: {err}");
        }
    }

    /// Validation is aggregated: a value breaking several rules reports them
    /// all in one boot error, so an operator fixes the key in one iteration.
    #[test]
    fn validate_reports_every_broken_base_path_rule_at_once() {
        let mut c = standalone();
        c.api.base_path = "cdr//api/".to_owned();
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

    /// `api.system_id` is judged by the openEHR `uid` grammar itself.
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
            c.api.system_id = refused.to_owned();
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
            c.api.system_id = accepted.to_owned();
            assert!(
                c.validate().is_ok(),
                "system_id {accepted:?} is a legal openEHR uid and must boot"
            );
        }
    }

    /// A `[terminology]` table with no URL fails to boot, naming the key. The
    /// section reads its defaults from one `Default` impl, so serde cannot
    /// report the missing key itself — the semantic pass owns it, which is
    /// also what lets every configuration error surface at once.
    #[test]
    fn validate_terminology_server_needs_a_url() {
        use crate::service::terminology::config::TerminologyServerConfig;
        let mut c = standalone();
        c.terminology = Some(TerminologyServerConfig::default());
        let err = c
            .validate()
            .expect_err("a server with no url must be rejected");
        assert!(
            err.to_string().contains("terminology.endpoint"),
            "got {err}"
        );

        c.terminology = Some(TerminologyServerConfig {
            endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
                url: SecretUrl::new("https://ts.example/fhir"),
                ..ferrohealth_smart::config::endpoint::EndpointSection::default()
            },
            ..TerminologyServerConfig::default()
        });
        assert!(c.validate().is_ok());
    }

    /// The terminology server's client-credentials grant is checked like every
    /// other outbound grant: an incomplete one is a boot error naming its
    /// keys, never a silently unauthenticated terminology request.
    #[test]
    fn validate_terminology_server_oauth_shape() {
        use crate::service::terminology::config::TerminologyServerConfig;
        use ferrohealth_smart::config::credentials::OauthSection;

        let with = |url: &str, oauth: OauthSection| {
            let mut c = standalone();
            let mut endpoint = ferrohealth_smart::config::endpoint::EndpointSection {
                url: SecretUrl::new(url),
                ..ferrohealth_smart::config::endpoint::EndpointSection::default()
            };
            endpoint.credentials.oauth = Some(oauth);
            c.terminology = Some(TerminologyServerConfig {
                endpoint,
                ..TerminologyServerConfig::default()
            });
            c
        };
        let err = with("https://ts.example/fhir", OauthSection::default())
            .validate()
            .expect_err("a grant with no token endpoint, client id or secret is rejected");
        for key in [
            "terminology.endpoint.credentials.oauth",
            "token_endpoint",
            "client_id",
            "client_secret",
        ] {
            assert!(err.to_string().contains(key), "{key} unnamed in {err}");
        }

        let complete = || OauthSection {
            token_endpoint: SecretUrl::new("https://idp.example/token"),
            client_id: "cdr".to_owned(),
            client_secret: Some(secrecy::SecretString::from("s3cret")),
            ..OauthSection::default()
        };
        assert!(
            with("https://ts.example/fhir", complete())
                .validate()
                .is_ok()
        );

        // A grant beside userinfo in the server URL would send two
        // authorizations.
        let err = with("https://u:p@ts.example/fhir", complete())
            .validate()
            .expect_err("userinfo beside a grant is rejected");
        assert!(
            err.to_string()
                .contains("terminology.endpoint.url carries credentials"),
            "got {err}"
        );
    }

    /// Every terminology key is a scalar, so the whole section, its grant
    /// included, is reachable through the one env grammar.
    #[test]
    fn env_mapping_terminology_server_and_oauth() {
        let c = assemble_ok(
            None,
            &env(&[
                (
                    "FERROEHR__TERMINOLOGY__ENDPOINT__URL",
                    "http://ferroterm:8080/r4b",
                ),
                ("FERROEHR__TERMINOLOGY__ON_ERROR", "refuse"),
                ("FERROEHR__TERMINOLOGY__ENDPOINT__TIMEOUT", "2500ms"),
                ("FERROEHR__TERMINOLOGY__ENDPOINT__RETRY__MAX_ATTEMPTS", "2"),
                (
                    "FERROEHR__TERMINOLOGY__ENDPOINT__CREDENTIALS__OAUTH__TOKEN_ENDPOINT",
                    "https://idp/token",
                ),
                (
                    "FERROEHR__TERMINOLOGY__ENDPOINT__CREDENTIALS__OAUTH__CLIENT_ID",
                    "cdr",
                ),
                (
                    "FERROEHR__TERMINOLOGY__ENDPOINT__CREDENTIALS__OAUTH__CLIENT_SECRET",
                    "s3cret",
                ),
                (
                    "FERROEHR__TERMINOLOGY__ENDPOINT__CREDENTIALS__OAUTH__SCOPE",
                    "system/*.read",
                ),
            ]),
            &[],
        );
        let server = c.terminology.as_ref().expect("the table from env");
        assert!(server.refuses_on_error());
        assert_eq!(server.endpoint.url.expose(), "http://ferroterm:8080/r4b");
        assert_eq!(
            server.endpoint.timeout,
            ferrohealth_config::units::Duration::from_millis(2_500)
        );
        assert_eq!(server.endpoint.retry.max_attempts, 2);
        let oauth = server
            .endpoint
            .credentials
            .oauth
            .as_ref()
            .expect("the grant from env");
        assert_eq!(oauth.token_endpoint.expose(), "https://idp/token");
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
            "terminology.endpoint.credentials.oauth",
            "audit.fhir_feed.endpoint.credentials.oauth",
            "authz.abac.remote.endpoint.credentials.oauth",
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
