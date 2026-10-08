// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared configuration value types for the whole server configuration tree.
//!
//! No openEHR spec governs configuration — our own design. These two newtypes
//! centralise secret handling so redaction is a property of the
//! type, not of a per-endpoint redactor list:
//!
//! - [`Secret`] wraps a [`secrecy::SecretString`]; it deserializes from a plain
//!   TOML/env string, but its `Serialize`, `Display`, and `Debug` all render the
//!   fixed placeholder [`REDACTED`], so a secret can never reach the
//!   `/management/env` snapshot, `ferroehr config check` output, or a debug log.
//! - [`SecretUrl`] carries a connection URL verbatim for connection use
//!   (exposed only via [`SecretUrl::expose`]), while every rendering replaces
//!   the `userinfo` component with [`REDACTED`]
//!   (`postgres://***@host:5432/db`).
//!
//! Both types are configuration primitives the whole platform shares —
//! both `ferroehr` and `ferroehr-rest` already depend on — so every section
//! struct across the two upper crates can use one shared secret representation.
//! Each `Secret`-typed key has a `*_file` sibling resolved by the loader
//! (`ferroehr::config`) into the `Secret` immediately after extraction, so
//! consumers only ever see the resolved value.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The fixed placeholder every secret rendering emits. Never a real value.
pub const REDACTED: &str = "***";

/// A secret string that never renders itself. Deserializes from a plain string;
/// `Serialize`/`Display`/`Debug` all emit [`REDACTED`].
#[derive(Clone)]
pub struct Secret(SecretString);

impl Secret {
    /// Wrap a plain string as a secret.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(SecretString::from(value.into()))
    }

    /// The underlying secret value, for the one consumer that must use it
    /// (a KDF verify, a passphrase, an HMAC key). Never log the result.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    /// Whether the secret is the empty string (an unset/blank value).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.expose_secret().is_empty()
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Quote so it reads as a redacted string field in a struct Debug.
        write!(f, "{REDACTED:?}")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.0.expose_secret() == other.0.expose_secret()
    }
}

impl Eq for Secret {}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

impl Serialize for Secret {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(REDACTED)
    }
}

/// A connection URL that carries embedded credentials.
///
/// Stored verbatim for connection use ([`SecretUrl::expose`]); every
/// rendering (`Serialize`/`Display`/`Debug`) replaces the `userinfo` with
/// [`REDACTED`].
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretUrl(String);

impl SecretUrl {
    /// Wrap a URL string verbatim.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// The full URL, credentials included — for connection use only.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the URL is empty (unset).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The URL with any `userinfo` component redacted (`scheme://***@host…`).
    #[must_use]
    pub fn redacted(&self) -> String {
        redact_userinfo(&self.0)
    }
}

/// Returns `url` with the `userinfo` (`user[:password]@`) of its authority
/// replaced by [`REDACTED`].
///
/// The one userinfo masker of the configuration tree: [`SecretUrl`] renders
/// through it, and so does every surface that quotes a URL it did not type.
/// The authority is the span after `://` up to the first `/`, `?` or `#`
/// (RFC 3986 §3.2), and its `userinfo` ends at the last `@` inside it; an `@`
/// in the path, query or fragment is not one. A URL with no `userinfo` is
/// returned unchanged. Pure string surgery, with no percent-decoding.
#[must_use]
pub fn redact_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let Some(authority) = rest.get(..authority_end) else {
        return url.to_owned();
    };
    let Some(at) = authority.rfind('@') else {
        return url.to_owned();
    };
    let Some(after_at) = rest.get(at + 1..) else {
        return url.to_owned();
    };
    format!("{scheme}://{REDACTED}@{after_at}")
}

impl fmt::Debug for SecretUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.redacted())
    }
}

impl fmt::Display for SecretUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.redacted())
    }
}

impl<'de> Deserialize<'de> for SecretUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

impl Serialize for SecretUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.redacted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_never_renders() {
        let s = Secret::new("top-secret-value");
        assert_eq!(s.expose(), "top-secret-value");
        assert!(!format!("{s:?}").contains("top-secret-value"));
        assert!(!format!("{s}").contains("top-secret-value"));
        assert_eq!(format!("{s}"), "***");
        assert_eq!(serde_json::to_string(&s).expect("serialize"), "\"***\"");
    }

    #[test]
    fn secret_round_trips_from_string() {
        let s: Secret = serde_json::from_str("\"hunter2\"").expect("deserialize");
        assert_eq!(s.expose(), "hunter2");
    }

    #[test]
    fn secret_default_is_empty() {
        assert!(Secret::default().is_empty());
    }

    #[test]
    fn secret_url_redacts_userinfo_but_keeps_connection_form() {
        let u = SecretUrl::new("postgres://ferroehr:ferroehr@localhost:5432/ferroehr");
        assert_eq!(
            u.expose(),
            "postgres://ferroehr:ferroehr@localhost:5432/ferroehr"
        );
        assert_eq!(u.redacted(), "postgres://***@localhost:5432/ferroehr");
        assert_eq!(format!("{u}"), "postgres://***@localhost:5432/ferroehr");
        assert!(!format!("{u:?}").contains("ferroehr:ferroehr"));
        assert_eq!(
            serde_json::to_string(&u).expect("serialize"),
            "\"postgres://***@localhost:5432/ferroehr\""
        );
    }

    #[test]
    fn secret_url_handles_amqp_vhost_and_no_userinfo() {
        let u = SecretUrl::new("amqp://guest:guest@localhost:5672/%2f");
        assert_eq!(u.redacted(), "amqp://***@localhost:5672/%2f");

        // No userinfo → unchanged (the '@' rule is confined to the authority).
        let plain = SecretUrl::new("postgres://localhost:5432/ferroehr");
        assert_eq!(plain.redacted(), "postgres://localhost:5432/ferroehr");

        // An '@' in the path must not be mistaken for userinfo.
        let at_in_path = SecretUrl::new("https://host/path/@handle");
        assert_eq!(at_in_path.redacted(), "https://host/path/@handle");
    }

    #[test]
    fn userinfo_ends_at_the_authority_per_rfc_3986() {
        // The authority ends at `?` and `#` as well as `/` (RFC 3986 §3.2).
        assert_eq!(
            redact_userinfo("https://host?next=a@b"),
            "https://host?next=a@b"
        );
        assert_eq!(redact_userinfo("https://host#a@b"), "https://host#a@b");
        assert_eq!(
            redact_userinfo("https://u:p@host?next=a@b"),
            "https://***@host?next=a@b"
        );
        // A password holding a raw `@` is masked whole: the userinfo ends at
        // the LAST `@` of the authority.
        assert_eq!(
            redact_userinfo("https://u:p@ss@host:443/x"),
            "https://***@host:443/x"
        );
        assert_eq!(redact_userinfo("not a url"), "not a url");
    }

    #[test]
    fn secret_url_round_trips_from_string() {
        let u: SecretUrl = serde_json::from_str("\"postgres://u:p@h/db\"").expect("deserialize");
        assert_eq!(u.expose(), "postgres://u:p@h/db");
    }
}
