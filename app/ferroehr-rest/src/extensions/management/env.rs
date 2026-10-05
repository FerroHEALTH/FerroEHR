// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `GET /management/env` — the effective configuration, with secrets masked.
//!
//! The binary composes a snapshot of the effective configuration (REST, auth,
//! management, telemetry, DB) as a JSON value; this endpoint returns it after a
//! recursive redaction pass that (a) masks any value under a secret-bearing key
//! and (b) strips credentials from DSN/URL values, keeping only the host and
//! database. No secret substring ever leaves the process.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use axum::Json;
use serde_json::Value;

/// Substrings (case-insensitive) that mark a JSON key as secret-bearing. Any
/// value under such a key is replaced with `MASK`.
const SECRET_KEY_MARKERS: &[&str] = &[
    "password",
    "secret",
    "hmac",
    "token",
    "jwks",
    "credential",
    "apikey",
    "api_key",
    "private_key",
];

/// The masked-value sentinel.
const MASK: &str = "***";

/// `GET /management/env`.
pub(super) fn env(snapshot: &Value) -> Json<Value> {
    Json(redact(snapshot))
}

/// Recursively redact a config value: secret-keyed values → [`MASK`]; strings
/// that look like a credentialed DSN/URL → credentials stripped.
#[must_use]
pub(crate) fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    if is_secret_key(k) {
                        (k.clone(), Value::String(MASK.to_owned()))
                    } else {
                        (k.clone(), redact(v))
                    }
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        Value::String(s) => Value::String(mask_dsn(s).unwrap_or_else(|| s.clone())),
        other => other.clone(),
    }
}

/// Whether `key` (case-insensitive) names a secret-bearing field.
fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEY_MARKERS.iter().any(|m| lower.contains(m))
}

/// If `s` looks like a URL with `user:password@host` userinfo, return a copy
/// with the userinfo masked (`scheme://***@host/...`). Otherwise `None`.
fn mask_dsn(s: &str) -> Option<String> {
    let (scheme, rest) = s.split_once("://")?;
    // The authority ends at the first '/', '?' or '#'.
    let authority = rest
        .find(['/', '?', '#'])
        .and_then(|end| rest.get(..end))
        .unwrap_or(rest);
    let at = authority.find('@')?;
    // Only mask when there is userinfo containing a credential separator or any
    // non-empty userinfo (a bare `host@` is not a DSN we care about).
    if authority.get(..at)?.is_empty() {
        return None;
    }
    let host_and_after = rest.get(at + 1..)?;
    Some(format!("{scheme}://{MASK}@{host_and_after}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masks_secret_keys() {
        let cfg = json!({
            "auth": {
                "basic": { "users": [{ "username": "alice", "password_hash": "$argon2id$secret" }] },
                "oidc": { "issuer": "https://kc", "hmac_secret": "topsecret", "jwks_json": "{...}" }
            }
        });
        let out = redact(&cfg);
        let text = out.to_string();
        assert!(!text.contains("$argon2id$secret"), "hash leaked: {text}");
        assert!(!text.contains("topsecret"), "hmac leaked: {text}");
        assert!(!text.contains("{...}"), "jwks leaked: {text}");
        assert_eq!(out["auth"]["basic"]["users"][0]["username"], "alice");
        assert_eq!(out["auth"]["basic"]["users"][0]["password_hash"], MASK);
        assert_eq!(out["auth"]["oidc"]["hmac_secret"], MASK);
    }

    #[test]
    fn masks_dsn_credentials_keeps_host_and_db() {
        let cfg = json!({ "db": { "url": "postgres://ferroehr:hunter2@db.internal:5432/ferroehr?sslmode=require" } });
        let out = redact(&cfg);
        let url = out["db"]["url"].as_str().expect("string");
        assert!(!url.contains("hunter2"), "password leaked: {url}");
        assert!(url.contains("db.internal:5432"), "host lost: {url}");
        assert!(url.contains("/ferroehr"), "db name lost: {url}");
        assert_eq!(
            url,
            "postgres://***@db.internal:5432/ferroehr?sslmode=require"
        );
    }

    #[test]
    fn leaves_credential_free_urls_untouched() {
        let cfg = json!({ "issuer": "https://keycloak.example/auth/realms/ferroehr" });
        let out = redact(&cfg);
        assert_eq!(
            out["issuer"],
            "https://keycloak.example/auth/realms/ferroehr"
        );
    }
}
