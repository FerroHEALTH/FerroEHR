// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `GET /management/env` — the effective configuration, with secrets masked.
//!
//! The binary builds the snapshot once at boot with
//! `ferroehr::config::FerroEhrConfig::to_redacted_json`, which masks every
//! secret by its leaf type; this endpoint returns it after a recursive pass
//! that (a) masks any value under a secret-bearing key and (b) masks the
//! `userinfo` of every URL value with the same masker the configuration uses.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use axum::Json;
use ferroehr::config::secret::redact_userinfo;
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

/// Recursively redact a config value: secret-keyed values → [`MASK`]; the
/// `userinfo` of every URL string → masked by [`redact_userinfo`].
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
        Value::String(s) => Value::String(redact_userinfo(s)),
        other => other.clone(),
    }
}

/// Whether `key` (case-insensitive) names a secret-bearing field.
fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEY_MARKERS.iter().any(|m| lower.contains(m))
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
