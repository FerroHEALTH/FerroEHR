// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The keys of the viewer's configuration that v5.6 read and v5.7.0 no longer
//! reads, each refused with its replacement.
//!
//! `[server]`, `[telemetry]`, `[oidc]` and `[session]` are family sections,
//! whose crates retired these keys in releases FerroEHR never shipped and
//! refuse an unknown key when they load, so [`removals`] reads the merged tree
//! before the typed load and names each key it finds. Every refusal here was
//! introduced in v5.7.0 and lives for that one release. No openEHR spec
//! governs configuration: our own design.

use ferrohealth_config::removed::{Removals, Removed, Retired};
use serde::Deserialize;

/// The FerroEHR release that retired every key here.
const SINCE: &str = "5.7.0";

/// A retired key whose replacement is `<new>` in the same table, a duration.
macro_rules! duration {
    ($key:literal, $new:literal) => {
        Retired::replaced(
            $key,
            concat!(
                $new,
                " in the same table, a duration such as \"30s\", \"10min\" or \"1h\""
            ),
            SINCE,
        )
    };
}

/// A retired key whose replacement is `<new>` in the same table, a size.
macro_rules! size {
    ($key:literal, $new:literal) => {
        Retired::replaced(
            $key,
            concat!($new, " in the same table, a size such as \"16MiB\""),
            SINCE,
        )
    };
}

/// What replaced the client certificate and key of a TLS table.
macro_rules! identity {
    () => {
        "tls.client_identity_file in the same table, one PEM holding the chain and the key"
    };
}

const TABLES: [(&str, &[Retired]); 6] = [
    (
        "server",
        &[
            duration!("request_timeout_ms", "request_timeout"),
            duration!("header_read_timeout_ms", "header_read_timeout"),
            duration!("idle_timeout_ms", "idle_timeout"),
            size!("body_limit_bytes", "body_limit"),
            duration!("drain_delay_ms", "drain_delay"),
            duration!("shutdown_timeout_ms", "shutdown_timeout"),
            duration!("retry_after_s", "retry_after"),
            size!("compression_min_bytes", "compression_min_size"),
            duration!("tls.reload_interval_s", "tls.reload_interval"),
        ],
    ),
    (
        "telemetry.otlp",
        &[
            Retired::replaced("trust_roots_file", "tls.trust_roots_file", SINCE),
            Retired::replaced("client_identity_file", identity!(), SINCE),
        ],
    ),
    ("cdr", &[duration!("request_timeout_secs", "timeout")]),
    (
        "oidc",
        &[
            Retired::replaced("ca_file", "tls.trust_roots_file", SINCE),
            duration!("timeout_s", "timeout"),
        ],
    ),
    (
        "oidc.tls",
        &[
            Retired::replaced("client_certificate", identity!(), SINCE),
            Retired::replaced("client_key", identity!(), SINCE),
            Retired::replaced("client_key_file", identity!(), SINCE),
        ],
    ),
    (
        "session",
        &[
            duration!("sign_in_timeout_s", "sign_in_timeout"),
            duration!("idle_timeout_s", "idle_timeout"),
            duration!("absolute_timeout_s", "absolute_timeout"),
        ],
    ),
];

/// Every retired key the merged `tree` sets, each with its replacement.
#[must_use]
pub fn removals(tree: &toml::Table) -> Removals {
    let mut removals = Removals::new();
    for (table, retired) in TABLES {
        let Some(toml::Value::Table(keys)) = lookup(tree, table) else {
            continue;
        };
        for key in retired {
            if let Some(value) = lookup(keys, key.key()) {
                removals.check_under(set(value), key, table);
            }
        }
    }
    removals
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every refusal records the release that introduced it, v5.7.0, the one
    /// release it lives for.
    #[test]
    fn every_refusal_records_its_release() {
        for (_, retired) in TABLES {
            for key in retired {
                assert_eq!(key.since(), SINCE, "{}", key.key());
            }
        }
    }

    /// A v5.6 key of each family section is named with its replacement.
    #[test]
    fn a_v5_6_key_is_named_with_its_replacement() {
        let tree: toml::Table = toml::from_str(
            "[server]\nbody_limit_bytes = 1\n[oidc]\nca_file = \"x\"\n[session]\nidle_timeout_s = 60\n\
             [cdr]\nrequest_timeout_secs = 30\n",
        )
        .expect("a tree");
        let named: Vec<String> = removals(&tree)
            .removals()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(named.len(), 4, "{named:?}");
        assert!(
            named
                .iter()
                .any(|n| n.starts_with("server.body_limit_bytes was replaced by body_limit"))
        );
        assert!(
            named
                .iter()
                .any(|n| n.starts_with("oidc.ca_file was replaced by tls.trust_roots_file"))
        );
        assert!(
            named
                .iter()
                .any(|n| n.starts_with("cdr.request_timeout_secs was replaced by timeout"))
        );
    }
}
