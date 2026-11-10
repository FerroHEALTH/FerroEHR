// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The keys of the viewer's configuration that v5.6 read and v5.7.0 no longer
//! reads, each refused with its replacement.
//!
//! `[server]`, `[telemetry]`, `[oidc]` and `[session]` are family sections,
//! whose crates retired these keys in releases FerroEHR never shipped and
//! refuse an unknown key when they load, so [`refuse`] reads the merged tree
//! before the typed load and names each key it finds. Every refusal here was
//! introduced in v5.7.0 and lives for that one release. No openEHR spec
//! governs configuration: our own design.

use std::fmt;

use ferrohealth_config::removed::{Removals, Removed, RemovedKeys, Retired};
use serde::Deserialize;

use crate::locale::server::ServerMsg;

/// The FerroEHR release that retired every key here.
const SINCE: &str = "5.7.0";

/// How the value of a retired key's replacement is written, which the
/// refusal adds from the catalog once per form it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Form {
    /// A duration with its unit.
    Duration,
    /// A size with its unit.
    Size,
    /// One PEM holding a client certificate chain and its key.
    Identity,
    /// The value the retired key held.
    Same,
}

impl Form {
    /// The catalog line that says how a value of this form is written.
    fn note(self) -> Option<ServerMsg> {
        match self {
            Self::Duration => Some(ServerMsg::RetiredFormDuration),
            Self::Size => Some(ServerMsg::RetiredFormSize),
            Self::Identity => Some(ServerMsg::RetiredFormIdentity),
            Self::Same => None,
        }
    }
}

/// A retired key replaced by `<new>` in the same table, a duration.
macro_rules! duration {
    ($key:literal, $new:literal) => {
        (Retired::replaced($key, $new, SINCE), Form::Duration)
    };
}

/// A retired key replaced by `<new>` in the same table, a size.
macro_rules! size {
    ($key:literal, $new:literal) => {
        (Retired::replaced($key, $new, SINCE), Form::Size)
    };
}

/// A retired client certificate or key, replaced by the one identity file.
macro_rules! identity {
    ($key:literal) => {
        (
            Retired::replaced($key, "tls.client_identity_file", SINCE),
            Form::Identity,
        )
    };
}

/// A retired key replaced by `<new>` in the same table, with the same value.
macro_rules! renamed {
    ($key:literal, $new:literal) => {
        (Retired::replaced($key, $new, SINCE), Form::Same)
    };
}

const TABLES: [(&str, &[(Retired, Form)]); 6] = [
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
            renamed!("trust_roots_file", "tls.trust_roots_file"),
            identity!("client_identity_file"),
        ],
    ),
    ("cdr", &[duration!("request_timeout_secs", "timeout")]),
    (
        "oidc",
        &[
            renamed!("ca_file", "tls.trust_roots_file"),
            duration!("timeout_s", "timeout"),
        ],
    ),
    (
        "oidc.tls",
        &[
            identity!("client_certificate"),
            identity!("client_key"),
            identity!("client_key_file"),
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

/// Retired keys the configuration sets, each named with its replacement in
/// the same table, followed by how the replacements' values are written.
#[derive(Debug, thiserror::Error)]
#[error("{keys}{}", Notes(.notes))]
pub struct RetiredKeys {
    keys: RemovedKeys,
    notes: Vec<String>,
}

impl RetiredKeys {
    /// Every retired key that was set.
    #[must_use]
    pub fn keys(&self) -> &RemovedKeys {
        &self.keys
    }
}

/// Renders the catalog notes, each on its own line after the keys.
struct Notes<'a>(&'a [String]);

impl fmt::Display for Notes<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for note in self.0 {
            write!(formatter, "\n{note}")?;
        }
        Ok(())
    }
}

/// Refuses every retired key the merged `tree` sets.
///
/// # Errors
///
/// [`RetiredKeys`] when the tree sets any, naming each with its replacement.
pub fn refuse(tree: &toml::Table) -> Result<(), RetiredKeys> {
    let mut removals = Removals::new();
    let mut forms = Vec::new();
    for (table, retired) in TABLES {
        let Some(toml::Value::Table(keys)) = lookup(tree, table) else {
            continue;
        };
        for (key, form) in retired {
            if let Some(value) = lookup(keys, key.key()) {
                removals.check_under(set(value), key, table);
                forms.push(*form);
            }
        }
    }
    forms.sort_unstable();
    forms.dedup();
    removals.finish().map_err(|keys| RetiredKeys {
        keys,
        notes: std::iter::once(ServerMsg::RetiredSameTable)
            .chain(forms.into_iter().filter_map(Form::note))
            .map(crate::locale::tr_served)
            .collect(),
    })
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
            for (key, _) in retired {
                assert_eq!(key.since(), SINCE, "{}", key.key());
            }
        }
    }

    /// No refusal outlives the one release it lives for: each was introduced
    /// in the release this crate belongs to, a release candidate's included.
    #[test]
    fn no_refusal_outlives_its_release() {
        let release = env!("CARGO_PKG_VERSION")
            .split('-')
            .next()
            .expect("a version has a release part");
        let all: Vec<Retired> = TABLES
            .iter()
            .flat_map(|(_, retired)| retired.iter().map(|(key, _)| *key))
            .collect();
        let stale: Vec<&str> = ferrohealth_config::removed::stale(&all, release)
            .into_iter()
            .map(Retired::key)
            .collect();
        assert!(
            stale.is_empty(),
            "past their release, delete them: {stale:?}"
        );
    }

    /// A v5.6 key of each family section is named with its replacement, and
    /// the refusal says once how each form it names is written.
    #[test]
    fn a_v5_6_key_is_named_with_its_replacement() {
        let tree: toml::Table = toml::from_str(
            "[server]\nbody_limit_bytes = 1\n[oidc]\nca_file = \"x\"\n[session]\nidle_timeout_s = 60\n\
             [cdr]\nrequest_timeout_secs = 30\n",
        )
        .expect("a tree");
        let refused = refuse(&tree).expect_err("retired keys are refused");
        let named: Vec<String> = refused
            .keys()
            .removals()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(named.len(), 4, "{named:?}");
        for line in [
            "server.body_limit_bytes was replaced by body_limit",
            "oidc.ca_file was replaced by tls.trust_roots_file",
            "cdr.request_timeout_secs was replaced by timeout",
        ] {
            assert!(named.iter().any(|n| n == line), "{line} in {named:?}");
        }
        let rendered = refused.to_string();
        for msg in [
            ServerMsg::RetiredSameTable,
            ServerMsg::RetiredFormDuration,
            ServerMsg::RetiredFormSize,
        ] {
            let note = crate::locale::tr_served(msg);
            assert_eq!(rendered.matches(note.as_str()).count(), 1, "{rendered}");
        }
        assert!(
            !rendered.contains(&crate::locale::tr_served(ServerMsg::RetiredFormIdentity)),
            "{rendered}"
        );
    }

    /// A tree that sets no retired key is not refused.
    #[test]
    fn a_tree_without_retired_keys_passes() {
        let tree: toml::Table =
            toml::from_str("[server]\nbody_limit = \"1MiB\"\n").expect("a tree");
        assert!(refuse(&tree).is_ok());
    }
}
