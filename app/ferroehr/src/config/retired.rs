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

use ferrohealth_config::removed::Removals;

use crate::config::FerroEhrConfig;

/// The value form of a duration key, for a refusal.
const DURATION: &str = "a duration such as \"30s\", \"10min\" or \"7d\"";
/// The value form of a size key, for a refusal.
const SIZE: &str = "a size such as \"64MiB\"";
/// The value form of a retention key, for a refusal.
const PERIOD: &str = "a calendar period such as \"P5Y\" or \"P1830D\", unset to keep forever";

/// Every retired key `config` sets, each with its replacement.
#[must_use]
pub fn removals(config: &FerroEhrConfig) -> Removals {
    let mut removals = Removals::new();
    root(config, &mut removals);
    auth(config, &mut removals);
    authz(config, &mut removals);
    audit(config, &mut removals);
    record(config, &mut removals);
    if let Some(terminology) = &config.terminology {
        removals.replaced(
            terminology.api_enabled,
            "terminology.api_enabled",
            "api.terminology",
        );
        removals.replaced(
            terminology.server,
            "terminology.server",
            format!(
                "the [terminology] table itself, present to turn the server on: \
                 terminology.server.url is terminology.url, request_timeout_ms is timeout ({DURATION}), \
                 fail_on_error = true is on_error = \"refuse\" and oauth is terminology.oauth"
            ),
        );
    }
    let profiling = &config.management.profiling;
    removals.replaced(
        profiling.max_seconds,
        "management.profiling.max_seconds",
        format!("management.profiling.max_duration, {DURATION}"),
    );
    removals.replaced(
        config.management.port,
        "management.port",
        "management.listen, a socket address such as \"0.0.0.0:9090\"",
    );
    removals
}

/// The top-level keys and the tables that moved.
#[expect(
    clippy::too_many_lines,
    reason = "one row per retired table, so the map of moves reads as one table"
)]
fn root(config: &FerroEhrConfig, removals: &mut Removals) {
    let moved: [(_, &str, String); 20] = [
        (
            config.spec_profile,
            "spec_profile",
            "deployment.spec_profile".to_owned(),
        ),
        (
            config.deployment_profile,
            "deployment_profile",
            "deployment.profile".to_owned(),
        ),
        (
            config.deployment_accepts,
            "deployment_accepts",
            "deployment.accepts".to_owned(),
        ),
        (
            config.licence,
            "licence",
            "deployment.licence_file (licence.file)".to_owned(),
        ),
        (
            config.rest,
            "rest",
            format!(
                "[api]: rest.swagger_ui is api.openapi_ui, rest.body_limit_bytes is api.body_limit \
                 ({SIZE}), rest.cors_permissive = true is [api.cors] allow_origins = [\"*\"], \
                 rest.identity.solution and vendor are [api.options]; every other key keeps its name"
            ),
        ),
        (
            config.db,
            "db",
            format!(
                "[database]: db.acquire_timeout_secs is database.acquire_timeout and \
                 db.statement_timeout_ms is database.statement_timeout ({DURATION}), db.migrate is \
                 database.migrations; every other key keeps its name"
            ),
        ),
        (
            config.storage,
            "storage",
            "[database.<domain>]: storage.<domain>.url is database.<domain>.url and \
             storage.migrate.<domain>.url is database.<domain>.migrate_url, each with its \
             _file sibling"
                .to_owned(),
        ),
        (
            config.admin,
            "admin",
            "api.admin (admin.enabled)".to_owned(),
        ),
        (
            config.smart,
            "smart",
            "[auth.smart]: smart.enabled is auth.smart.discovery, smart.episode.enabled is \
             auth.smart.episode_launch, smart.launch_base64_json = true is \"launch-base64-json\" \
             in auth.capabilities; every other key keeps its name"
                .to_owned(),
        ),
        (
            config.scim,
            "scim",
            format!(
                "[auth.scim], present to serve SCIM: scim.max_results is \
                 auth.scim.filter_max_results, scim.deactivation_window_minutes is \
                 auth.scim.deactivation_window and scim.contact_erasure_minutes is \
                 auth.scim.contact_erasure_delay ({DURATION}), [[scim.client]] is \
                 [[auth.scim.client]]; every other key keeps its name"
            ),
        ),
        (
            config.signing,
            "signing",
            "[record.signing]: signing.key_path is record.signing.key_file and \
             signing.retired_key_paths is record.signing.retired_key_files; every other key \
             keeps its name"
                .to_owned(),
        ),
        (
            config.query,
            "query",
            format!("[record.query]: query.timeout_ms is record.query.timeout ({DURATION})"),
        ),
        (
            config.outbox,
            "outbox",
            format!(
                "[record.change_feed]: outbox.retention_days is record.change_feed.retention and \
                 outbox.prune_interval_secs is record.change_feed.prune_interval ({DURATION})"
            ),
        ),
        (
            config.multimedia,
            "multimedia",
            format!(
                "[record.multimedia]: multimedia.endpoint is record.multimedia.url, \
                 multimedia.threshold_bytes is record.multimedia.threshold ({SIZE}), and \
                 multimedia.allow_http is retired (an http:// url is plain HTTP)"
            ),
        ),
        (
            config.versioning,
            "versioning",
            "record.versioning.idempotent_resend (versioning.skip_identical_resend)".to_owned(),
        ),
        (
            config.ehr_index,
            "ehr_index",
            format!(
                "record.subject_index (\"pixm\" or \"local\"; ehr_index.provider = \"ihe\" is \
                 \"pixm\") and the [pixm] table: ehr_index.pixm_url is pixm.url, \
                 ehr_index.invocation is pixm.method, ehr_index.request_timeout_ms is pixm.timeout \
                 ({DURATION}), ehr_index.tls and ehr_index.oauth are pixm.tls and pixm.oauth"
            ),
        ),
        (
            config.origins,
            "origins",
            "[[authz.origin]] ([[origins.registered]], the same keys)".to_owned(),
        ),
        (
            config.demographic,
            "demographic",
            "[privacy.identifier_protection], present to turn protection on \
             (demographic.identifier_protection.enabled is retired; schemes, key and key_file \
             keep their names)"
                .to_owned(),
        ),
        (
            config.cohort,
            "cohort",
            "[privacy.cohort] (the same keys)".to_owned(),
        ),
        (
            config.access_delay,
            "access_delay",
            format!(
                "[privacy.access_delay], present to turn the delay on: access_delay.hours is \
                 privacy.access_delay.max_delay ({DURATION}, in whole hours), access_delay.enabled \
                 is retired, categories keeps its name"
            ),
        ),
    ];
    for (removed, key, replacement) in moved {
        removals.replaced(removed, key, replacement);
    }
}

/// The retired keys inside `[auth]`.
fn auth(config: &FerroEhrConfig, removals: &mut Removals) {
    let smart = &config.auth.smart;
    removals.replaced(smart.enabled, "auth.smart.enabled", "auth.smart.discovery");
    removals.replaced(
        smart.episode,
        "auth.smart.episode",
        "auth.smart.episode_launch",
    );
    removals.replaced(
        smart.launch_base64_json,
        "auth.smart.launch_base64_json",
        "\"launch-base64-json\" in auth.capabilities",
    );
    if let Some(scim) = &config.auth.scim {
        removals.replaced(
            scim.enabled,
            "auth.scim.enabled",
            "the [auth.scim] table's presence",
        );
        removals.replaced(
            scim.max_results,
            "auth.scim.max_results",
            "auth.scim.filter_max_results",
        );
        removals.replaced(
            scim.deactivation_window_minutes,
            "auth.scim.deactivation_window_minutes",
            format!("auth.scim.deactivation_window, {DURATION}"),
        );
        removals.replaced(
            scim.contact_erasure_minutes,
            "auth.scim.contact_erasure_minutes",
            format!("auth.scim.contact_erasure_delay, {DURATION}"),
        );
    }
}

/// The retired keys inside `[authz]`.
fn authz(config: &FerroEhrConfig, removals: &mut Removals) {
    let authz = &config.authz;
    removals.replaced(
        authz.abac.cedar.reload_secs,
        "authz.abac.cedar.reload_secs",
        format!("authz.abac.cedar.reload_interval, {DURATION}"),
    );
    let remote = &authz.abac.remote;
    removals.replaced(
        remote.server,
        "authz.abac.remote.server",
        "authz.abac.remote.url",
    );
    removals.replaced(
        remote.connect_timeout_ms,
        "authz.abac.remote.connect_timeout_ms",
        format!("authz.abac.remote.connect_timeout, {DURATION}"),
    );
    removals.replaced(
        remote.request_timeout_ms,
        "authz.abac.remote.request_timeout_ms",
        format!("authz.abac.remote.timeout, {DURATION}"),
    );
    removals.replaced(
        authz.party_binding.provisional_report_days,
        "authz.party_binding.provisional_report_days",
        format!("authz.party_binding.provisional_report_after, {DURATION}"),
    );
    for (index, agreement) in authz.federation.agreement.iter().enumerate() {
        let key = |name: &str| format!("authz.federation.agreement[{index}].{name}");
        removals.replaced(
            agreement.discard_window_hours,
            key("discard_window_hours"),
            format!("{}, {DURATION}", key("discard_window")),
        );
        removals.replaced(
            agreement.chained_log_retention_years,
            key("chained_log_retention_years"),
            format!("{}, {PERIOD}", key("chained_log_retention")),
        );
    }
}

/// The retired keys inside `[audit]`.
fn audit(config: &FerroEhrConfig, removals: &mut Removals) {
    let store = &config.audit.store;
    for (removed, key) in [
        (store.retention_days, "audit.store.retention_days"),
        (store.retention_years, "audit.store.retention_years"),
    ] {
        removals.replaced(removed, key, format!("audit.store.retention, {PERIOD}"));
    }
    removals.replaced(
        store.verify_interval_seconds,
        "audit.store.verify_interval_seconds",
        format!("audit.store.verify_interval, {DURATION}"),
    );
    removals.replaced(
        store.full_verify_interval_seconds,
        "audit.store.full_verify_interval_seconds",
        format!("audit.store.full_verify_interval, {DURATION}"),
    );
    if let Some(syslog) = &config.audit.syslog {
        let identity = "[audit.syslog.tls] client_identity_file, one PEM holding the client \
                        certificate chain and its private key";
        removals.replaced(
            syslog.tls_ca_file,
            "audit.syslog.tls_ca_file",
            "[audit.syslog.tls] trust_roots_file",
        );
        removals.replaced(
            syslog.tls_identity_cert_file,
            "audit.syslog.tls_identity_cert_file",
            identity,
        );
        removals.replaced(
            syslog.tls_identity_key_file,
            "audit.syslog.tls_identity_key_file",
            identity,
        );
        removals.replaced(
            syslog.enabled,
            "audit.syslog.enabled",
            "the [audit.syslog] table's presence",
        );
        for (removed, key) in [
            (syslog.host, "audit.syslog.host"),
            (syslog.port, "audit.syslog.port"),
        ] {
            removals.replaced(
                removed,
                key,
                "audit.syslog.url, such as \"tls://arr.example:6514\"",
            );
        }
        removals.replaced(
            syslog.spool_max_messages,
            "audit.syslog.spool_max_messages",
            "audit.syslog.spool_max_events",
        );
        removals.replaced(
            syslog.spool_max_bytes,
            "audit.syslog.spool_max_bytes",
            format!("audit.syslog.spool_max_size, {SIZE}"),
        );
        removals.replaced(
            syslog.retry_max_ms,
            "audit.syslog.retry_max_ms",
            format!("audit.syslog.retry.max_backoff, {DURATION}"),
        );
    }
    if let Some(feed) = &config.audit.fhir_feed {
        removals.replaced(
            feed.enabled,
            "audit.fhir_feed.enabled",
            "the [audit.fhir_feed] table's presence",
        );
        removals.replaced(
            feed.poll_interval_ms,
            "audit.fhir_feed.poll_interval_ms",
            format!("audit.fhir_feed.poll_interval, {DURATION}"),
        );
        removals.replaced(
            feed.retry_max_ms,
            "audit.fhir_feed.retry_max_ms",
            format!("audit.fhir_feed.retry.max_backoff, {DURATION}"),
        );
    }
}

/// The retired keys inside `[record]`.
fn record(config: &FerroEhrConfig, removals: &mut Removals) {
    removals.retired(
        config.record.multimedia.allow_http,
        "record.multimedia.allow_http",
        "an http:// record.multimedia.url is plain HTTP, which a production deployment names \
         as a gap",
    );
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
