// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The keys of the family sections (`[server]`, `[telemetry]`, `[auth]`) that
//! v5.6 read and v5.7.0 no longer reads, each refused with its replacement.
//!
//! The family crates retired these keys in releases FerroEHR never shipped,
//! and dropped their own refusals after one release, so for a FerroEHR
//! operator v5.7.0 is the release that retires them. Those sections refuse an
//! unknown key when they load, so [`removals`] reads the merged tree before
//! the typed load and names each key it finds. Every refusal here was
//! introduced in v5.7.0 and lives for that one release. No openEHR spec
//! governs configuration: our own design.

use ferrohealth_config::removed::{Removals, Removed, Retired};
use serde::Deserialize;

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

/// The FerroEHR release that retired every key here.
const SINCE: &str = "5.7.0";

/// A retired key whose replacement is `<new>, <form>` in the same table.
macro_rules! renamed {
    ($key:literal, $new:literal, $form:expr) => {
        Retired::replaced($key, concat!($new, " in the same table, ", $form), SINCE)
    };
}

const SERVER: [Retired; 9] = [
    renamed!("request_timeout_ms", "request_timeout", duration!()),
    renamed!("header_read_timeout_ms", "header_read_timeout", duration!()),
    renamed!("idle_timeout_ms", "idle_timeout", duration!()),
    renamed!("body_limit_bytes", "body_limit", size!()),
    renamed!("drain_delay_ms", "drain_delay", duration!()),
    renamed!("shutdown_timeout_ms", "shutdown_timeout", duration!()),
    renamed!(
        "retry_after_s",
        "retry_after",
        "whole seconds such as \"1s\""
    ),
    renamed!("compression_min_bytes", "compression_min_size", size!()),
    renamed!("tls.reload_interval_s", "tls.reload_interval", duration!()),
];

const OTLP: [Retired; 2] = [
    Retired::replaced("trust_roots_file", "tls.trust_roots_file", SINCE),
    Retired::replaced(
        "client_identity_file",
        "tls.client_identity_file, one PEM holding the chain and the key",
        SINCE,
    ),
];

/// What replaced a key `[auth]` read for every issuer at once.
macro_rules! per_issuer {
    () => {
        "the same key in each [[auth.issuer]] table"
    };
}

const AUTH: [Retired; 14] = [
    Retired::replaced("audience", per_issuer!(), SINCE),
    Retired::replaced("algorithms", per_issuer!(), SINCE),
    Retired::replaced(
        "token_profile",
        "profile in each [[auth.issuer]] table",
        SINCE,
    ),
    Retired::replaced(
        "clock_skew_seconds",
        concat!("clock_skew in each [[auth.issuer]] table, ", duration!()),
        SINCE,
    ),
    Retired::replaced(
        "jwks_max_age_seconds",
        concat!("jwks_max_age in each [[auth.issuer]] table, ", duration!()),
        SINCE,
    ),
    Retired::replaced(
        "jwks_refresh_interval_seconds",
        concat!(
            "jwks_refresh_interval in each [[auth.issuer]] table, ",
            duration!()
        ),
        SINCE,
    ),
    Retired::replaced("jwks_file", per_issuer!(), SINCE),
    Retired::replaced(
        "timeout_ms",
        concat!("timeout in each [[auth.issuer]] table, ", duration!()),
        SINCE,
    ),
    Retired::replaced("role_claims", per_issuer!(), SINCE),
    Retired::replaced("mode", per_issuer!(), SINCE),
    Retired::replaced("introspection", per_issuer!(), SINCE),
    Retired::replaced(
        "acting_for_claim",
        "delegates in the [[auth.issuer]] table, which lists the clients that act for a person",
        SINCE,
    ),
    Retired::replaced("client_tokens", "kind in each [[auth.issuer]] table", SINCE),
    Retired::replaced("ca_file", "auth.tls.trust_roots_file", SINCE),
];

const ISSUER: [Retired; 8] = [
    Retired::replaced("token_profile", "profile", SINCE),
    Retired::replaced(
        "acting_for_claim",
        "delegates, which lists the clients that act for a person",
        SINCE,
    ),
    Retired::replaced("client_tokens", "kind", SINCE),
    renamed!("clock_skew_seconds", "clock_skew", duration!()),
    renamed!("jwks_max_age_seconds", "jwks_max_age", duration!()),
    renamed!(
        "jwks_refresh_interval_seconds",
        "jwks_refresh_interval",
        duration!()
    ),
    renamed!("timeout_ms", "timeout", duration!()),
    renamed!(
        "introspection.cache_max_age_seconds",
        "introspection.cache_max_age",
        duration!()
    ),
];

const BASIC: [Retired; 1] = [renamed!("cache_ttl_seconds", "cache_ttl", duration!())];

const DPOP: [Retired; 2] = [
    renamed!("max_age_seconds", "max_age", duration!()),
    renamed!("clock_skew_seconds", "clock_skew", duration!()),
];

const ENTRA: [Retired; 2] = [
    renamed!("timeout_ms", "timeout", duration!()),
    renamed!("max_response_bytes", "max_response_size", size!()),
];

const PARTY: [Retired; 1] = [Retired::removed(
    "claim",
    "the issuer's subject is the binding key, its claim no longer configurable",
    SINCE,
)];

const OAUTH: [Retired; 6] = [
    Retired::replaced("ca_file", "tls.trust_roots_file", SINCE),
    Retired::replaced(
        "client_certificate_file",
        "tls.client_identity_file, one PEM holding the chain and the key",
        SINCE,
    ),
    Retired::replaced(
        "client_key_file",
        "tls.client_identity_file, one PEM holding the chain and the key",
        SINCE,
    ),
    renamed!("refresh_margin_ms", "refresh_margin", duration!()),
    renamed!("timeout_ms", "timeout", duration!()),
    renamed!(
        "previous_key_overlap_ms",
        "previous_key_overlap",
        duration!()
    ),
];

const FLAT_ISSUER: Retired = Retired::replaced(
    "auth.issuer",
    "one [[auth.issuer]] table per trusted issuer, with issuer, kind and audience",
    SINCE,
);

/// Every key this module refuses, for the test that holds each to the one
/// release it lives for.
#[cfg(test)]
pub(crate) fn retired() -> Vec<Retired> {
    let mut all = vec![FLAT_ISSUER];
    for table in [
        &SERVER[..],
        &OTLP,
        &AUTH,
        &ISSUER,
        &BASIC,
        &DPOP,
        &ENTRA,
        &PARTY,
        &OAUTH,
    ] {
        all.extend_from_slice(table);
    }
    all
}

/// The `oauth` table of v5.6 that keeps its place, `[authz.abac.remote.oauth]`.
const OAUTH_TABLES: [&str; 1] = ["authz.abac.remote.oauth"];

/// Every retired key of a family section the merged `tree` sets, each with
/// its replacement.
#[must_use]
pub fn removals(tree: &toml::Table) -> Removals {
    let mut removals = Removals::new();
    for (table, retired) in [
        ("server", &SERVER[..]),
        ("telemetry.otlp", &OTLP),
        ("auth", &AUTH),
        ("auth.basic", &BASIC),
        ("auth.dpop", &DPOP),
        ("auth.idp.entra", &ENTRA),
    ]
    .into_iter()
    .chain(OAUTH_TABLES.map(|table| (table, &OAUTH[..])))
    {
        if let Some(toml::Value::Table(keys)) = lookup(tree, table) {
            check(keys, retired, table, &mut removals);
        }
    }
    match lookup(tree, "auth.issuer") {
        Some(value @ toml::Value::String(_)) => removals.check(set(value), &FLAT_ISSUER),
        Some(toml::Value::Array(issuers)) => each(issuers, &ISSUER, "auth.issuer", &mut removals),
        _ => {}
    }
    if let Some(toml::Value::Array(parties)) = lookup(tree, "auth.party") {
        each(parties, &PARTY, "auth.party", &mut removals);
    }
    removals
}

/// Checks every table of the array of tables at the dotted `table`.
fn each(items: &[toml::Value], retired: &[Retired], table: &str, removals: &mut Removals) {
    for (index, item) in items.iter().enumerate() {
        if let toml::Value::Table(keys) = item {
            check(keys, retired, &format!("{table}[{index}]"), removals);
        }
    }
}

/// Checks every key of `retired` that `keys`, the table at the dotted
/// `table`, sets.
fn check(keys: &toml::Table, retired: &[Retired], table: &str, removals: &mut Removals) {
    for key in retired {
        if let Some(value) = lookup(keys, key.key()) {
            removals.check_under(set(value), key, table);
        }
    }
}

/// The value at the dotted `key` of `table`.
fn lookup<'a>(table: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let (head, rest) = key
        .split_once('.')
        .map_or((key, None), |(h, r)| (h, Some(r)));
    let value = table.get(head)?;
    match (rest, value) {
        (None, value) => Some(value),
        (Some(rest), toml::Value::Table(inner)) => lookup(inner, rest),
        (Some(_), _) => None,
    }
}

/// A [`Removed`] marked set: the sources wrote `value` at its key.
#[expect(
    clippy::expect_used,
    reason = "Removed reads any value through serde's IgnoredAny, so reading one cannot fail"
)]
fn set(value: &toml::Value) -> Removed {
    Removed::deserialize(value.clone()).expect("a Removed should read any value")
}
