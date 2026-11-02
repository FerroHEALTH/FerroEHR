// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Sealed-cookie session state and the auth guard every read of the CDR calls.
//!
//! Every route is a public HTTP endpoint: "only my UI calls this" is never
//! assumed.
//!
//! The whole session lives in ONE encrypted, authenticated cookie
//! (AES-256-GCM via the `cookie` crate's private jar), so any viewer
//! instance holding the same key can read it — the viewer runs
//! multi-instance on serverless platforms, keeps no database by mandate,
//! and gets no sticky sessions there. The browser stores ciphertext
//! only: CDR credentials never reach a drawn page or a readable cookie. Idle expiry is a timestamp INSIDE the sealed payload,
//! so a stolen clock-limited cookie cannot be revived by editing its
//! attributes. The anchor SLIDES on any authenticated request:
//! [`crate::server::login_guard`] re-seals the cookie on every authenticated
//! response, and that includes requests the browser makes by itself — the
//! shell's own health poll among them — so the window measures traffic from
//! this session, not deliberate user activity.

use std::collections::BTreeMap;

use base64::Engine;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;

/// The [`ViewerSession`] record's entry key inside the sealed cookie payload.
///
/// The viewer keeps no session store of any kind: the payload is a small
/// keyed map carried whole in the encrypted [`SESSION_COOKIE`], so this names
/// an entry in that map rather than a record held anywhere on a server.
pub const SESSION_KEY: &str = "viewer_session";

/// The sealed session cookie's name.
pub const SESSION_COOKIE: &str = "ferroehr_viewer_session";

/// The sealing key pair (signing + encryption halves of one 64-byte secret).
///
/// `Debug` is manual and redacting: key material must never appear in logs.
#[derive(Clone)]
pub struct SessionKeys {
    key: cookie::Key,
}

impl std::fmt::Debug for SessionKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionKeys").finish_non_exhaustive()
    }
}

impl SessionKeys {
    /// Build the sealing key from the configured secret.
    ///
    /// An empty secret yields an ephemeral random key with a WARN: sessions
    /// then bind to one instance and one process lifetime — fine for the
    /// single-replica deployments (compose, the chart's default), wrong for
    /// anything multi-instance. A configured secret must be base64 decoding
    /// to at least 64 bytes.
    ///
    /// # Errors
    /// [`ViewerError::Internal`] when the configured secret is not valid
    /// base64 or decodes to fewer than 64 bytes (a boot-time refusal).
    pub fn from_secret(secret: &str) -> Result<Self, ViewerError> {
        if secret.trim().is_empty() {
            tracing::warn!(
                "session.secret is empty: using an ephemeral per-instance key: sessions will not \
                 survive a restart and will NOT work across multiple instances; configure \
                 session.secret (base64, at least 64 bytes) for any scaled deployment"
            );
            return Ok(Self {
                key: cookie::Key::generate(),
            });
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(secret.trim())
            .map_err(|e| ViewerError::Internal(format!("session.secret is not base64: {e}")))?;
        let key = cookie::Key::try_from(bytes.as_slice()).map_err(|e| {
            ViewerError::Internal(format!(
                "session.secret must decode to at least 64 bytes: {e}"
            ))
        })?;
        Ok(Self { key })
    }
}

/// The credential the BFF attaches to outbound CDR calls.
///
/// `Debug` is manual and redacting: log identifiers and shapes, never bodies.
#[derive(Clone, Serialize, Deserialize)]
pub enum Credential {
    /// Basic auth, validated against the CDR at login.
    Basic {
        /// The CDR username.
        username: String,
        /// The CDR password (sealed cookie only, never plaintext client-side).
        password: String,
    },
    /// A bearer token from the viewer's OIDC login.
    Bearer {
        /// The access token (sealed cookie only, never plaintext client-side).
        access_token: String,
    },
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Basic { username, .. } => f
                .debug_struct("Basic")
                .field("username", username)
                .field("password", &"<redacted>")
                .finish(),
            Self::Bearer { .. } => f
                .debug_struct("Bearer")
                .field("access_token", &"<redacted>")
                .finish(),
        }
    }
}

/// One authenticated viewer session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerSession {
    /// Display identity (username or OIDC `preferred_username`/`sub`).
    pub identity: String,
    /// How the session authenticates to the CDR.
    pub credential: Credential,
    /// Scopes granted to the OIDC token (empty for Basic) — the "what can
    /// I do right now" panel reads these.
    pub scopes: Vec<String>,
}

/// The sealed payload: keyed entries plus the sliding idle anchor.
#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    touched: i64,
    // Each entry is its own JSON text — values decode straight to their
    // types, so no untyped-carrier seam exists here.
    entries: BTreeMap<String, String>,
}

/// One request's decoded view of the sealed cookie's keyed payload.
///
/// Mutate it, then [`commit`] (or attach [`set_cookie`] on a plain axum
/// response) to persist the change.
#[derive(Debug, Default)]
pub struct CookieSession {
    entries: BTreeMap<String, String>,
}

impl CookieSession {
    /// Read one typed entry (`None` when absent or of a different shape).
    #[must_use]
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.entries
            .get(key)
            .and_then(|value| serde_json::from_str(value).ok())
    }

    /// Insert one typed entry.
    ///
    /// # Errors
    /// [`ViewerError::Internal`] when the value does not serialize (a bug).
    pub fn insert<T: Serialize>(&mut self, key: &str, value: &T) -> Result<(), ViewerError> {
        let value = serde_json::to_string(value)
            .map_err(|e| ViewerError::Internal(format!("session serialization: {e}")))?;
        self.entries.insert(key.to_owned(), value);
        Ok(())
    }

    /// Remove one entry, returning it typed when it was present.
    pub fn remove<T: DeserializeOwned>(&mut self, key: &str) -> Option<T> {
        self.entries
            .remove(key)
            .and_then(|value| serde_json::from_str(&value).ok())
    }

    /// Whether the session carries no entries at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Current wall-clock seconds (jiff is the one time library).
fn now_seconds() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// Decode the session from a request's headers.
///
/// Absent, unparsable, tampered, wrong-key and idle-expired cookies all
/// decode to the empty session: on this seam every defect means the same
/// thing — no authenticated caller — and the guard turns that into a
/// redirect or a typed `Unauthenticated`, never a 500.
#[must_use]
pub fn unseal(keys: &SessionKeys, headers: &http::HeaderMap, idle_minutes: u64) -> CookieSession {
    let Some(sealed) = cookie_value(headers) else {
        return CookieSession::default();
    };
    let mut jar = cookie::CookieJar::new();
    jar.add_original(cookie::Cookie::new(SESSION_COOKIE, sealed));
    let Some(open) = jar.private(&keys.key).get(SESSION_COOKIE) else {
        return CookieSession::default();
    };
    let Ok(envelope) = serde_json::from_str::<Envelope>(open.value()) else {
        return CookieSession::default();
    };
    // Inclusive, so a zero-minute idle window admits no session at all.
    let idle = i64::try_from(idle_minutes.saturating_mul(60)).unwrap_or(i64::MAX);
    if now_seconds().saturating_sub(envelope.touched) >= idle {
        return CookieSession::default();
    }
    CookieSession {
        entries: envelope.entries,
    }
}

/// The raw sealed value of our cookie from a `Cookie` request header.
fn cookie_value(headers: &http::HeaderMap) -> Option<String> {
    headers.get_all(http::header::COOKIE).iter().find_map(|h| {
        let raw = h.to_str().ok()?;
        cookie::Cookie::split_parse(raw.to_owned())
            .filter_map(Result::ok)
            .find(|c| c.name() == SESSION_COOKIE)
            .map(|c| c.value().to_owned())
    })
}

/// Seal a session into a `Set-Cookie` value (fresh idle anchor).
///
/// An empty session seals to the REMOVAL cookie (`Max-Age=0`), so "save an
/// emptied session" and "sign out" are the same operation.
///
/// # Errors
/// [`ViewerError::Internal`] when the payload does not serialize or the
/// sealed value is not a valid header (both bugs, surfaced loudly).
pub fn set_cookie(
    keys: &SessionKeys,
    session: &CookieSession,
    cookie_secure: bool,
    idle_minutes: u64,
) -> Result<http::HeaderValue, ViewerError> {
    let cookie = if session.is_empty() {
        removal_cookie(cookie_secure)
    } else {
        let envelope = Envelope {
            touched: now_seconds(),
            entries: session.entries.clone(),
        };
        let payload = serde_json::to_string(&envelope)
            .map_err(|e| ViewerError::Internal(format!("session serialization: {e}")))?;
        let mut jar = cookie::CookieJar::new();
        jar.private_mut(&keys.key)
            .add(base_cookie(payload, cookie_secure, Some(idle_minutes)));
        let Some(sealed) = jar.get(SESSION_COOKIE) else {
            return Err(ViewerError::Internal(
                "session sealing produced no cookie".to_owned(),
            ));
        };
        sealed.clone()
    };
    // Rendered unencoded: the sealed value is base64, whose alphabet is
    // entirely within RFC 6265's cookie-octet set.
    http::HeaderValue::from_str(&cookie.to_string())
        .map_err(|e| ViewerError::Internal(format!("session cookie header: {e}")))
    // NOTE: no openEHR spec governs the viewer session — our own design;
    // flags per the OWASP Session Management Cheat Sheet (HttpOnly, Lax for
    // the OIDC top-level callback redirect, Secure behind TLS).
}

/// The attribute set every variant of our cookie carries.
fn base_cookie(
    value: String,
    cookie_secure: bool,
    max_age_minutes: Option<u64>,
) -> cookie::Cookie<'static> {
    let mut builder = cookie::Cookie::build((SESSION_COOKIE, value))
        .path("/")
        .http_only(true)
        .secure(cookie_secure)
        .same_site(cookie::SameSite::Lax);
    if let Some(minutes) = max_age_minutes {
        builder = builder.max_age(cookie::time::Duration::seconds(
            i64::try_from(minutes.saturating_mul(60)).unwrap_or(i64::MAX),
        ));
    }
    builder.build()
}

/// The sign-out cookie: same attributes, empty value, `Max-Age=0`.
fn removal_cookie(cookie_secure: bool) -> cookie::Cookie<'static> {
    let mut cookie = base_cookie(String::new(), cookie_secure, None);
    cookie.set_max_age(cookie::time::Duration::ZERO);
    cookie
}

/// The session the request being drawn carries.
///
/// # Errors
/// [`ViewerError::Internal`] when called outside a request (a bug).
pub fn http_session() -> Result<CookieSession, ViewerError> {
    crate::scope::with(|scope| {
        unseal(
            &scope.state.session_keys,
            &scope.parts.headers,
            scope.state.config.session.idle_minutes,
        )
    })
    .ok_or_else(outside_request)
}

/// The error of a read that needs the request but runs outside one, a defect
/// of its caller.
#[must_use]
pub fn outside_request() -> ViewerError {
    ViewerError::Internal("no request in scope".to_owned())
}

/// Persists a mutated session: appends the sealed `Set-Cookie` to the
/// answer of the request being drawn.
///
/// # Errors
/// [`ViewerError::Internal`] outside a request or on a sealing failure.
pub fn commit(session: &CookieSession) -> Result<(), ViewerError> {
    let state = crate::scope::state()?;
    let header = set_cookie(
        &state.session_keys,
        session,
        state.config.session.cookie_secure,
        state.config.session.idle_minutes,
    )?;
    crate::scope::with(|scope| scope.answer(|answer| answer.cookies.push(header)))
        .ok_or_else(outside_request)
}

/// The guard: return the authenticated session or `Unauthenticated`.
///
/// Every read of the CDR or of the session state calls this first. A missing, tampered or idle-expired cookie is the same answer:
/// [`ViewerError::Unauthenticated`].
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Internal`] outside a request (a bug).
pub fn require_session() -> Result<ViewerSession, ViewerError> {
    let session = http_session()?
        .get::<ViewerSession>(SESSION_KEY)
        .ok_or(ViewerError::Unauthenticated);
    if let Err(error) = &session {
        crate::session_client::note_if_over(error);
    }
    session
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "test assertions/fixtures"
    )]

    use super::{
        CookieSession, Credential, SESSION_COOKIE, SESSION_KEY, SessionKeys, ViewerSession,
        set_cookie, unseal,
    };

    fn keys() -> SessionKeys {
        // 96 zero bytes, base64 — a fixed key so tests are deterministic.
        SessionKeys::from_secret(&base64_of(&[0u8; 96])).unwrap()
    }

    fn base64_of(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn headers_with(set_cookie_value: &http::HeaderValue) -> http::HeaderMap {
        // Reuse the Set-Cookie's `name=value` prefix as the request Cookie.
        let full = set_cookie_value.to_str().unwrap();
        let pair = full.split(';').next().unwrap();
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(pair).unwrap(),
        );
        headers
    }

    fn session_with_viewer() -> CookieSession {
        let mut session = CookieSession::default();
        session
            .insert(
                SESSION_KEY,
                &ViewerSession {
                    identity: "demo".to_owned(),
                    credential: Credential::Basic {
                        username: "demo".to_owned(),
                        password: "hunter2".to_owned(),
                    },
                    scopes: vec![],
                },
            )
            .unwrap();
        session
    }

    #[test]
    fn seals_and_unseals_across_instances_sharing_the_key() {
        let sealing = keys();
        let header = set_cookie(&sealing, &session_with_viewer(), true, 60).unwrap();
        // A DIFFERENT SessionKeys value from the same secret — the
        // multi-instance property under test.
        let other_instance = SessionKeys::from_secret(&base64_of(&[0u8; 96])).unwrap();
        let restored = unseal(&other_instance, &headers_with(&header), 60);
        let restored_session: ViewerSession = restored.get(SESSION_KEY).unwrap();
        assert_eq!(restored_session.identity, "demo");
    }

    #[test]
    fn the_cookie_is_ciphertext_with_the_hardening_attributes() {
        let header = set_cookie(&keys(), &session_with_viewer(), true, 60).unwrap();
        let text = header.to_str().unwrap();
        assert!(!text.contains("hunter2"), "credential visible: {text}");
        assert!(!text.contains("demo"), "identity visible: {text}");
        assert!(text.contains("HttpOnly"));
        assert!(text.contains("SameSite=Lax"));
        assert!(text.contains("Secure"));
        assert!(text.contains("Max-Age=3600"));
    }

    #[test]
    fn a_wrong_key_and_a_tampered_value_both_read_as_no_session() {
        let header = set_cookie(&keys(), &session_with_viewer(), false, 60).unwrap();
        let stranger = SessionKeys::from_secret(&base64_of(&[7u8; 96])).unwrap();
        assert!(unseal(&stranger, &headers_with(&header), 60).is_empty());

        let mut headers = headers_with(&header);
        let tampered = headers[http::header::COOKIE]
            .to_str()
            .unwrap()
            .replace('=', "=X");
        headers.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&tampered).unwrap(),
        );
        assert!(unseal(&keys(), &headers, 60).is_empty());
    }

    #[test]
    fn an_idle_expired_payload_reads_as_no_session() {
        let sealing = keys();
        let header = set_cookie(&sealing, &session_with_viewer(), false, 60).unwrap();
        // idle_minutes = 0 makes any positive age expired.
        assert!(unseal(&sealing, &headers_with(&header), 0).is_empty());
    }

    #[test]
    fn an_emptied_session_seals_to_the_removal_cookie() {
        let header = set_cookie(&keys(), &CookieSession::default(), true, 60).unwrap();
        let text = header.to_str().unwrap();
        assert!(text.contains("Max-Age=0"), "not a removal cookie: {text}");
        assert!(text.starts_with(&format!("{SESSION_COOKIE}=;")));
    }

    #[test]
    fn a_short_or_malformed_secret_is_refused_and_empty_is_ephemeral() {
        assert!(SessionKeys::from_secret(&base64_of(&[1u8; 32])).is_err());
        assert!(SessionKeys::from_secret("not base64!!").is_err());
        assert!(SessionKeys::from_secret("  ").is_ok());
    }
}
