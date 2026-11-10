// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The keys the shipped `ferroehr.toml` holds: what an operator sets to run a
//! deployment, each `(table path, key)` in the order the file lists them.
//! Every other key keeps its default and is listed on the reference page.
//! Which keys an operator sets is a judgement no type carries, so it is the
//! one hand-written list of the generator; a key it names that the types do
//! not hold fails the generation. No specification governs it: our own design.

/// The operator keys.
pub(crate) const KEYS: &[(&str, &str)] = &[
    ("deployment", "profile"),
    ("deployment", "accepts"),
    ("deployment", "licence_file"),
    ("server", "listen"),
    ("server", "base_path"),
    ("server", "trusted_proxies"),
    ("server.tls", "certificate_file"),
    ("server.tls", "key_file"),
    ("database", "url"),
    ("database", "migrate_url"),
    ("database", "migrations"),
    ("database", "max_connections"),
    ("telemetry", "format"),
    ("telemetry", "filter"),
    ("telemetry.otlp", "endpoint"),
    ("usage_report", "enabled"),
    ("usage_report", "deployment"),
    ("api", "system_id"),
    ("api.cors", "allow_origins"),
    ("auth", "enabled"),
    ("auth.issuer", "issuer"),
    ("auth.issuer", "kind"),
    ("auth.issuer", "profile"),
    ("auth.issuer", "audience"),
    ("auth.issuer", "jwks_file"),
    ("authz.rbac", "enabled"),
    ("authz.rbac", "admin_role"),
    ("authz.rbac", "role_issuers"),
    ("audit", "enabled"),
    ("audit", "source_id"),
    ("audit", "fail_mode"),
    ("audit.store", "enabled"),
    ("audit.store", "retention"),
    ("management", "enabled"),
    ("management", "listen"),
    ("privacy", "subject_namespaces"),
    ("record", "subject_index"),
    ("record.signing", "enabled"),
    ("record.signing", "key_file"),
    ("record.multimedia", "enabled"),
    ("record.multimedia", "url"),
    ("record.multimedia", "bucket"),
    ("terminology.endpoint", "url"),
];
