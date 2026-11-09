// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The keys of the configuration tree before v5.7.0, each refused at load with
//! the key that replaced it.
//!
//! Every retired key stays in the tree as a `ferrohealth-config` `Removed`
//! field, so the strict load still accepts it, and [`removals`] names each one
//! the sources set, all of them at once: an old file never starts with a
//! default in the place of what its operator wrote, and no key is read under
//! two names. A whole table that moved is refused by its old name, and the
//! refusal names the table it moved to and every key inside it whose name or
//! value form changed. Every refusal here was introduced in v5.7.0 and lives
//! for that one release: v5.8.0 deletes this module and the `Removed` fields,
//! and an old key is then an unknown key. No openEHR spec governs
//! configuration: our own design.

use ferrohealth_config::removed::{Removals, Retired};

use crate::config::FerroEhrConfig;

/// The value form of a duration key, for a refusal.
macro_rules! duration {
    () => {
        "a duration such as \"30s\", \"10min\" or \"7d\""
    };
}

/// The value form of a size key, for a refusal.
macro_rules! size {
    () => {
        "a size such as \"64MiB\""
    };
}

/// The value form of a retention key, for a refusal.
macro_rules! period {
    () => {
        "a calendar period such as \"P5Y\" or \"P1830D\", unset to keep forever"
    };
}

/// The FerroEHR release that retired every key here.
const SINCE: &str = "5.7.0";

const TERMINOLOGY_API_ENABLED: Retired =
    Retired::replaced("terminology.api_enabled", "api.terminology", SINCE);
const TERMINOLOGY_SERVER: Retired = Retired::replaced(
    "terminology.server",
    concat!(
        "the [terminology] table itself, present to turn the server on: ",
        "terminology.server.url is terminology.endpoint.url, request_timeout_ms is ",
        "terminology.endpoint.timeout (",
        duration!(),
        "), fail_on_error = true is on_error = \"refuse\" and oauth is ",
        "terminology.endpoint.credentials.oauth"
    ),
    SINCE,
);
const PROFILING_MAX_SECONDS: Retired = Retired::replaced(
    "management.profiling.max_seconds",
    concat!("management.profiling.max_duration, ", duration!()),
    SINCE,
);
const MANAGEMENT_PORT: Retired = Retired::replaced(
    "management.port",
    "management.listen, a socket address such as \"0.0.0.0:9090\"",
    SINCE,
);
const SPEC_PROFILE: Retired = Retired::replaced("spec_profile", "deployment.spec_profile", SINCE);
const DEPLOYMENT_PROFILE: Retired =
    Retired::replaced("deployment_profile", "deployment.profile", SINCE);
const DEPLOYMENT_ACCEPTS: Retired =
    Retired::replaced("deployment_accepts", "deployment.accepts", SINCE);
const LICENCE: Retired =
    Retired::replaced("licence", "deployment.licence_file (licence.file)", SINCE);
const REST: Retired = Retired::replaced(
    "rest",
    concat!(
        "[api]: rest.swagger_ui is api.openapi_ui, rest.body_limit_bytes is api.body_limit (",
        size!(),
        "), rest.cors_permissive = true is [api.cors] allow_origins = [\"*\"], ",
        "rest.identity.solution and vendor are [api.options]; every other key keeps its name"
    ),
    SINCE,
);
const DB: Retired = Retired::replaced(
    "db",
    concat!(
        "[database]: db.acquire_timeout_secs is database.acquire_timeout and ",
        "db.statement_timeout_ms is database.statement_timeout (",
        duration!(),
        "), db.migrate is database.migrations; every other key keeps its name"
    ),
    SINCE,
);
const STORAGE: Retired = Retired::replaced(
    "storage",
    "[database.<domain>]: storage.<domain>.url is database.<domain>.url and \
     storage.migrate.<domain>.url is database.<domain>.migrate_url, each with its _file sibling",
    SINCE,
);
const ADMIN: Retired = Retired::replaced("admin", "api.admin (admin.enabled)", SINCE);
const SMART: Retired = Retired::replaced(
    "smart",
    "[auth.smart]: smart.enabled is auth.smart.discovery, smart.episode.enabled is \
     auth.smart.episode_launch, smart.launch_base64_json = true is \"launch-base64-json\" in \
     auth.capabilities; every other key keeps its name",
    SINCE,
);
const SCIM: Retired = Retired::replaced(
    "scim",
    concat!(
        "[auth.scim], present to serve SCIM: scim.max_results is auth.scim.filter_max_results, ",
        "scim.deactivation_window_minutes is auth.scim.deactivation_window and ",
        "scim.contact_erasure_minutes is auth.scim.contact_erasure_delay (",
        duration!(),
        "), [[scim.client]] is [[auth.scim.client]]; every other key keeps its name"
    ),
    SINCE,
);
const SIGNING: Retired = Retired::replaced(
    "signing",
    "[record.signing]: signing.key_path is record.signing.key_file and \
     signing.retired_key_paths is record.signing.retired_key_files; every other key keeps its \
     name",
    SINCE,
);
const QUERY: Retired = Retired::replaced(
    "query",
    concat!(
        "[record.query]: query.timeout_ms is record.query.timeout (",
        duration!(),
        ")"
    ),
    SINCE,
);
const OUTBOX: Retired = Retired::replaced(
    "outbox",
    concat!(
        "[record.change_feed]: outbox.retention_days is record.change_feed.retention and ",
        "outbox.prune_interval_secs is record.change_feed.prune_interval (",
        duration!(),
        ")"
    ),
    SINCE,
);
const MULTIMEDIA: Retired = Retired::replaced(
    "multimedia",
    concat!(
        "[record.multimedia]: multimedia.endpoint is record.multimedia.url, ",
        "multimedia.threshold_bytes is record.multimedia.threshold (",
        size!(),
        "), and multimedia.allow_http is retired (an http:// url is plain HTTP)"
    ),
    SINCE,
);
const VERSIONING: Retired = Retired::replaced(
    "versioning",
    "record.versioning.idempotent_resend (versioning.skip_identical_resend)",
    SINCE,
);
const EHR_INDEX: Retired = Retired::replaced(
    "ehr_index",
    concat!(
        "record.subject_index (\"pixm\" or \"local\"; ehr_index.provider = \"ihe\" is \"pixm\") ",
        "and the [pixm] table: ehr_index.pixm_url is pixm.endpoint.url, ehr_index.invocation is ",
        "pixm.method, ehr_index.request_timeout_ms is pixm.endpoint.timeout (",
        duration!(),
        "), ehr_index.tls and ehr_index.oauth are pixm.endpoint.tls and ",
        "pixm.endpoint.credentials.oauth"
    ),
    SINCE,
);
const ORIGINS: Retired = Retired::replaced(
    "origins",
    "[[authz.origin]] ([[origins.registered]], the same keys)",
    SINCE,
);
const DEMOGRAPHIC: Retired = Retired::replaced(
    "demographic",
    "[privacy.identifier_protection], present to turn protection on \
     (demographic.identifier_protection.enabled is retired; schemes, key and key_file keep \
     their names)",
    SINCE,
);
const COHORT: Retired = Retired::replaced("cohort", "[privacy.cohort] (the same keys)", SINCE);
const ACCESS_DELAY: Retired = Retired::replaced(
    "access_delay",
    concat!(
        "[privacy.access_delay], present to turn the delay on: access_delay.hours is ",
        "privacy.access_delay.max_delay (",
        duration!(),
        ", in whole hours), access_delay.enabled is retired, categories keeps its name"
    ),
    SINCE,
);
const SMART_ENABLED: Retired =
    Retired::replaced("auth.smart.enabled", "auth.smart.discovery", SINCE);
const SMART_EPISODE: Retired =
    Retired::replaced("auth.smart.episode", "auth.smart.episode_launch", SINCE);
const SMART_LAUNCH_BASE64_JSON: Retired = Retired::replaced(
    "auth.smart.launch_base64_json",
    "\"launch-base64-json\" in auth.capabilities",
    SINCE,
);
const SCIM_ENABLED: Retired = Retired::replaced(
    "auth.scim.enabled",
    "the [auth.scim] table's presence",
    SINCE,
);
const SCIM_MAX_RESULTS: Retired = Retired::replaced(
    "auth.scim.max_results",
    "auth.scim.filter_max_results",
    SINCE,
);
const SCIM_DEACTIVATION_WINDOW_MINUTES: Retired = Retired::replaced(
    "auth.scim.deactivation_window_minutes",
    concat!("auth.scim.deactivation_window, ", duration!()),
    SINCE,
);
const SCIM_CONTACT_ERASURE_MINUTES: Retired = Retired::replaced(
    "auth.scim.contact_erasure_minutes",
    concat!("auth.scim.contact_erasure_delay, ", duration!()),
    SINCE,
);
const CEDAR_RELOAD_SECS: Retired = Retired::replaced(
    "authz.abac.cedar.reload_secs",
    concat!("authz.abac.cedar.reload_interval, ", duration!()),
    SINCE,
);
const REMOTE_SERVER: Retired = Retired::replaced(
    "authz.abac.remote.server",
    "authz.abac.remote.endpoint.url",
    SINCE,
);
const REMOTE_CONNECT_TIMEOUT_MS: Retired = Retired::replaced(
    "authz.abac.remote.connect_timeout_ms",
    concat!("authz.abac.remote.endpoint.connect_timeout, ", duration!()),
    SINCE,
);
const REMOTE_REQUEST_TIMEOUT_MS: Retired = Retired::replaced(
    "authz.abac.remote.request_timeout_ms",
    concat!("authz.abac.remote.endpoint.timeout, ", duration!()),
    SINCE,
);
const PARTY_BINDING_ENABLED: Retired = Retired::removed(
    "authz.party_binding.enabled",
    "binding has no switch, and every persons issuer binds its logins through its \
     [[auth.party]] table",
    SINCE,
);
const PROFESSIONAL: Retired = Retired::removed(
    "authz.professional",
    "the [[auth.issuer]] kind and profile classify the caller, and a client acting for a person \
     is one the issuer lists in delegates",
    SINCE,
);
const PROVISIONAL_REPORT_DAYS: Retired = Retired::replaced(
    "authz.party_binding.provisional_report_days",
    concat!(
        "authz.party_binding.provisional_report_after, ",
        duration!()
    ),
    SINCE,
);
const AGREEMENT_DISCARD_WINDOW_HOURS: Retired = Retired::replaced(
    "discard_window_hours",
    concat!("discard_window in the same table, ", duration!()),
    SINCE,
);
const AGREEMENT_CHAINED_LOG_RETENTION_YEARS: Retired = Retired::replaced(
    "chained_log_retention_years",
    concat!("chained_log_retention in the same table, ", period!()),
    SINCE,
);
const STORE_RETENTION_DAYS: Retired = Retired::replaced(
    "audit.store.retention_days",
    concat!("audit.store.retention, ", period!()),
    SINCE,
);
const STORE_RETENTION_YEARS: Retired = Retired::replaced(
    "audit.store.retention_years",
    concat!("audit.store.retention, ", period!()),
    SINCE,
);
const STORE_VERIFY_INTERVAL_SECONDS: Retired = Retired::replaced(
    "audit.store.verify_interval_seconds",
    concat!("audit.store.verify_interval, ", duration!()),
    SINCE,
);
const STORE_FULL_VERIFY_INTERVAL_SECONDS: Retired = Retired::replaced(
    "audit.store.full_verify_interval_seconds",
    concat!("audit.store.full_verify_interval, ", duration!()),
    SINCE,
);
/// What replaced the two client identity files of `[audit.syslog]`.
macro_rules! syslog_identity {
    () => {
        "[audit.syslog.tls] client_identity_file, one PEM holding the client certificate chain \
         and its private key"
    };
}
const SYSLOG_TLS_CA_FILE: Retired = Retired::replaced(
    "audit.syslog.tls_ca_file",
    "[audit.syslog.tls] trust_roots_file",
    SINCE,
);
const SYSLOG_TLS_IDENTITY_CERT_FILE: Retired = Retired::replaced(
    "audit.syslog.tls_identity_cert_file",
    syslog_identity!(),
    SINCE,
);
const SYSLOG_TLS_IDENTITY_KEY_FILE: Retired = Retired::replaced(
    "audit.syslog.tls_identity_key_file",
    syslog_identity!(),
    SINCE,
);
const SYSLOG_ENABLED: Retired = Retired::replaced(
    "audit.syslog.enabled",
    "the [audit.syslog] table's presence",
    SINCE,
);
const SYSLOG_HOST: Retired = Retired::replaced(
    "audit.syslog.host",
    "audit.syslog.url, such as \"tls://arr.example:6514\"",
    SINCE,
);
const SYSLOG_PORT: Retired = Retired::replaced(
    "audit.syslog.port",
    "audit.syslog.url, such as \"tls://arr.example:6514\"",
    SINCE,
);
const SYSLOG_SPOOL_MAX_MESSAGES: Retired = Retired::replaced(
    "audit.syslog.spool_max_messages",
    "audit.syslog.spool_max_events",
    SINCE,
);
const SYSLOG_SPOOL_MAX_BYTES: Retired = Retired::replaced(
    "audit.syslog.spool_max_bytes",
    concat!("audit.syslog.spool_max_size, ", size!()),
    SINCE,
);
const SYSLOG_RETRY_MAX_MS: Retired = Retired::replaced(
    "audit.syslog.retry_max_ms",
    concat!("audit.syslog.retry.max_backoff, ", duration!()),
    SINCE,
);
const FEED_ENABLED: Retired = Retired::replaced(
    "audit.fhir_feed.enabled",
    "the [audit.fhir_feed] table's presence",
    SINCE,
);
const FEED_POLL_INTERVAL_MS: Retired = Retired::replaced(
    "audit.fhir_feed.poll_interval_ms",
    concat!("audit.fhir_feed.poll_interval, ", duration!()),
    SINCE,
);
const FEED_RETRY_MAX_MS: Retired = Retired::replaced(
    "audit.fhir_feed.retry_max_ms",
    concat!("audit.fhir_feed.endpoint.retry.max_backoff, ", duration!()),
    SINCE,
);
const REMOTE_OAUTH: Retired = Retired::replaced(
    "authz.abac.remote.oauth",
    "[authz.abac.remote.endpoint.credentials.oauth], whose tls table holds trust_roots_file and \
     client_identity_file and whose refresh_margin, timeout and previous_key_overlap are \
     durations",
    SINCE,
);
const FEED_URL: Retired =
    Retired::replaced("audit.fhir_feed.url", "audit.fhir_feed.endpoint.url", SINCE);
const FEED_OAUTH: Retired = Retired::replaced(
    "audit.fhir_feed.oauth",
    "[audit.fhir_feed.endpoint.credentials.oauth], whose tls table holds trust_roots_file and \
     client_identity_file and whose refresh_margin, timeout and previous_key_overlap are \
     durations",
    SINCE,
);
const MULTIMEDIA_ALLOW_HTTP: Retired = Retired::removed(
    "record.multimedia.allow_http",
    "an http:// record.multimedia.url is plain HTTP, which a production deployment names as a \
     gap",
    SINCE,
);

/// Every key this module refuses, for the test that holds each to the one
/// release it lives for.
#[cfg(test)]
pub(crate) const RETIRED: [Retired; 60] = [
    TERMINOLOGY_API_ENABLED,
    TERMINOLOGY_SERVER,
    PROFILING_MAX_SECONDS,
    MANAGEMENT_PORT,
    SPEC_PROFILE,
    DEPLOYMENT_PROFILE,
    DEPLOYMENT_ACCEPTS,
    LICENCE,
    REST,
    DB,
    STORAGE,
    ADMIN,
    SMART,
    SCIM,
    SIGNING,
    QUERY,
    OUTBOX,
    MULTIMEDIA,
    VERSIONING,
    EHR_INDEX,
    ORIGINS,
    DEMOGRAPHIC,
    COHORT,
    ACCESS_DELAY,
    SMART_ENABLED,
    SMART_EPISODE,
    SMART_LAUNCH_BASE64_JSON,
    SCIM_ENABLED,
    SCIM_MAX_RESULTS,
    SCIM_DEACTIVATION_WINDOW_MINUTES,
    SCIM_CONTACT_ERASURE_MINUTES,
    CEDAR_RELOAD_SECS,
    REMOTE_SERVER,
    REMOTE_CONNECT_TIMEOUT_MS,
    REMOTE_REQUEST_TIMEOUT_MS,
    PARTY_BINDING_ENABLED,
    PROFESSIONAL,
    PROVISIONAL_REPORT_DAYS,
    AGREEMENT_DISCARD_WINDOW_HOURS,
    AGREEMENT_CHAINED_LOG_RETENTION_YEARS,
    STORE_RETENTION_DAYS,
    STORE_RETENTION_YEARS,
    STORE_VERIFY_INTERVAL_SECONDS,
    STORE_FULL_VERIFY_INTERVAL_SECONDS,
    SYSLOG_TLS_CA_FILE,
    SYSLOG_TLS_IDENTITY_CERT_FILE,
    SYSLOG_TLS_IDENTITY_KEY_FILE,
    SYSLOG_ENABLED,
    SYSLOG_HOST,
    SYSLOG_PORT,
    SYSLOG_SPOOL_MAX_MESSAGES,
    SYSLOG_SPOOL_MAX_BYTES,
    SYSLOG_RETRY_MAX_MS,
    FEED_ENABLED,
    FEED_POLL_INTERVAL_MS,
    FEED_RETRY_MAX_MS,
    FEED_URL,
    FEED_OAUTH,
    REMOTE_OAUTH,
    MULTIMEDIA_ALLOW_HTTP,
];

/// Every retired key `config` sets, each with its replacement.
#[must_use]
pub fn removals(config: &FerroEhrConfig) -> Removals {
    let mut removals = Removals::new();
    root(config, &mut removals);
    auth(config, &mut removals);
    authz(config, &mut removals);
    audit(config, &mut removals);
    if let Some(terminology) = &config.terminology {
        removals.check(terminology.api_enabled, &TERMINOLOGY_API_ENABLED);
        removals.check(terminology.server, &TERMINOLOGY_SERVER);
    }
    removals.check(
        config.management.profiling.max_seconds,
        &PROFILING_MAX_SECONDS,
    );
    removals.check(config.management.port, &MANAGEMENT_PORT);
    removals.check(config.record.multimedia.allow_http, &MULTIMEDIA_ALLOW_HTTP);
    removals
}

/// The top-level keys and the tables that moved.
fn root(config: &FerroEhrConfig, removals: &mut Removals) {
    for (removed, retired) in [
        (config.spec_profile, &SPEC_PROFILE),
        (config.deployment_profile, &DEPLOYMENT_PROFILE),
        (config.deployment_accepts, &DEPLOYMENT_ACCEPTS),
        (config.licence, &LICENCE),
        (config.rest, &REST),
        (config.db, &DB),
        (config.storage, &STORAGE),
        (config.admin, &ADMIN),
        (config.smart, &SMART),
        (config.scim, &SCIM),
        (config.signing, &SIGNING),
        (config.query, &QUERY),
        (config.outbox, &OUTBOX),
        (config.multimedia, &MULTIMEDIA),
        (config.versioning, &VERSIONING),
        (config.ehr_index, &EHR_INDEX),
        (config.origins, &ORIGINS),
        (config.demographic, &DEMOGRAPHIC),
        (config.cohort, &COHORT),
        (config.access_delay, &ACCESS_DELAY),
    ] {
        removals.check(removed, retired);
    }
}

/// The retired keys inside `[auth]`.
fn auth(config: &FerroEhrConfig, removals: &mut Removals) {
    let smart = &config.auth.smart;
    removals.check(smart.enabled, &SMART_ENABLED);
    removals.check(smart.episode, &SMART_EPISODE);
    removals.check(smart.launch_base64_json, &SMART_LAUNCH_BASE64_JSON);
    if let Some(scim) = &config.auth.scim {
        removals.check(scim.enabled, &SCIM_ENABLED);
        removals.check(scim.max_results, &SCIM_MAX_RESULTS);
        removals.check(
            scim.deactivation_window_minutes,
            &SCIM_DEACTIVATION_WINDOW_MINUTES,
        );
        removals.check(scim.contact_erasure_minutes, &SCIM_CONTACT_ERASURE_MINUTES);
    }
}

/// The retired keys inside `[authz]`.
fn authz(config: &FerroEhrConfig, removals: &mut Removals) {
    let authz = &config.authz;
    removals.check(authz.abac.cedar.reload_secs, &CEDAR_RELOAD_SECS);
    let remote = &authz.abac.remote;
    removals.check(remote.server, &REMOTE_SERVER);
    removals.check(remote.connect_timeout_ms, &REMOTE_CONNECT_TIMEOUT_MS);
    removals.check(remote.request_timeout_ms, &REMOTE_REQUEST_TIMEOUT_MS);
    removals.check(remote.oauth, &REMOTE_OAUTH);
    removals.check(authz.party_binding.enabled, &PARTY_BINDING_ENABLED);
    removals.check(authz.professional, &PROFESSIONAL);
    removals.check(
        authz.party_binding.provisional_report_days,
        &PROVISIONAL_REPORT_DAYS,
    );
    for (index, agreement) in authz.federation.agreement.iter().enumerate() {
        let table = format!("authz.federation.agreement[{index}]");
        removals.check_under(
            agreement.discard_window_hours,
            &AGREEMENT_DISCARD_WINDOW_HOURS,
            &table,
        );
        removals.check_under(
            agreement.chained_log_retention_years,
            &AGREEMENT_CHAINED_LOG_RETENTION_YEARS,
            &table,
        );
    }
}

/// The retired keys inside `[audit]`.
fn audit(config: &FerroEhrConfig, removals: &mut Removals) {
    let store = &config.audit.store;
    removals.check(store.retention_days, &STORE_RETENTION_DAYS);
    removals.check(store.retention_years, &STORE_RETENTION_YEARS);
    removals.check(
        store.verify_interval_seconds,
        &STORE_VERIFY_INTERVAL_SECONDS,
    );
    removals.check(
        store.full_verify_interval_seconds,
        &STORE_FULL_VERIFY_INTERVAL_SECONDS,
    );
    if let Some(syslog) = &config.audit.syslog {
        for (removed, retired) in [
            (syslog.tls_ca_file, &SYSLOG_TLS_CA_FILE),
            (
                syslog.tls_identity_cert_file,
                &SYSLOG_TLS_IDENTITY_CERT_FILE,
            ),
            (syslog.tls_identity_key_file, &SYSLOG_TLS_IDENTITY_KEY_FILE),
            (syslog.enabled, &SYSLOG_ENABLED),
            (syslog.host, &SYSLOG_HOST),
            (syslog.port, &SYSLOG_PORT),
            (syslog.spool_max_messages, &SYSLOG_SPOOL_MAX_MESSAGES),
            (syslog.spool_max_bytes, &SYSLOG_SPOOL_MAX_BYTES),
            (syslog.retry_max_ms, &SYSLOG_RETRY_MAX_MS),
        ] {
            removals.check(removed, retired);
        }
    }
    if let Some(feed) = &config.audit.fhir_feed {
        removals.check(feed.enabled, &FEED_ENABLED);
        removals.check(feed.poll_interval_ms, &FEED_POLL_INTERVAL_MS);
        removals.check(feed.retry_max_ms, &FEED_RETRY_MAX_MS);
        removals.check(feed.url, &FEED_URL);
        removals.check(feed.oauth, &FEED_OAUTH);
    }
}

/// `fields` without `retired`, the keys a section keeps only to refuse them,
/// for the tests that hold the shipped file to the live keys.
#[cfg(test)]
pub(crate) fn live(fields: Vec<String>, retired: &[&str]) -> Vec<String> {
    fields
        .into_iter()
        .filter(|field| !retired.contains(&field.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// Every refusal records the release that introduced it, v5.7.0, the one
    /// release it lives for, and each names a key no other refusal names.
    #[test]
    fn every_refusal_records_its_release_and_names_its_own_key() {
        let shared = crate::config::retired_shared::retired();
        let all: Vec<&Retired> = RETIRED.iter().chain(&shared).collect();
        for retired in &all {
            assert_eq!(retired.since(), "5.7.0", "{}", retired.key());
        }
        let own: BTreeSet<&str> = RETIRED.iter().map(Retired::key).collect();
        assert_eq!(own.len(), RETIRED.len(), "a key is declared twice");
    }
}
