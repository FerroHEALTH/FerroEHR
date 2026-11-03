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
//! only: CDR credentials never reach a drawn page or a readable cookie. Both
//! expiries are timestamps INSIDE the sealed payload, so a stolen cookie
//! cannot be revived by editing its attributes. The idle anchor SLIDES on
//! every authenticated request ([`crate::server::login_guard`] re-seals the
//! cookie) except a read of the session route, which the page's session
//! watch makes on its own; the sign-in time never moves, so the absolute
//! lifetime (`session.absolute_minutes`) ends the session whatever the
//! activity (OWASP Session Management Cheat Sheet, idle and absolute
//! timeouts).

use std::collections::BTreeMap;

use base64::Engine;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use ferrohealth_ui::view::session::Left;

use crate::config::SessionConfig;
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
    /// [`ViewerError::Failed`] when the configured secret is not valid
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
            .map_err(|e| ViewerError::failed("session.secret is not base64", e))?;
        let key = cookie::Key::try_from(bytes.as_slice()).map_err(|e| {
            ViewerError::failed("session.secret must decode to at least 64 bytes", e)
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

/// The sealed payload: keyed entries, the sliding idle anchor and the fixed
/// sign-in time the absolute lifetime counts from.
///
/// Both times are required, so a cookie sealed before the sign-in time
/// existed does not open and its user signs in again.
#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    touched: i64,
    signed_in: i64,
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
    /// When the payload was last sealed, `None` for a session no cookie
    /// carried.
    touched: Option<i64>,
    /// When the session's user signed in, `None` until a sealing stamps it.
    signed_in: Option<i64>,
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
    /// [`ViewerError::Failed`] when the value does not serialize (a bug).
    pub fn insert<T: Serialize>(&mut self, key: &str, value: &T) -> Result<(), ViewerError> {
        let value = serde_json::to_string(value)
            .map_err(|e| ViewerError::failed("session serialization", e))?;
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

    /// Signs `viewer` in: stores it under [`SESSION_KEY`] and starts the
    /// absolute lifetime again at the next sealing.
    ///
    /// # Errors
    /// [`ViewerError::Failed`] when the value does not serialize (a bug).
    pub fn sign_in(&mut self, viewer: &ViewerSession) -> Result<(), ViewerError> {
        self.insert(SESSION_KEY, viewer)?;
        self.signed_in = None;
        Ok(())
    }

    /// Moves the idle anchor to now, as the sealing of this request's answer
    /// will.
    pub fn touch(&mut self) {
        self.touched = Some(now_seconds());
    }

    /// What the signed-in session has left under `config`, as it was last
    /// sealed or [`touched`](Self::touch); `None` with nobody signed in or
    /// once either timeout has passed.
    #[must_use]
    pub fn left(&self, config: &SessionConfig) -> Option<Left> {
        self.left_at(config, now_seconds())
    }

    /// [`Self::left`] at `now`.
    fn left_at(&self, config: &SessionConfig, now: i64) -> Option<Left> {
        if !self.entries.contains_key(SESSION_KEY) {
            return None;
        }
        let absolute_end = self
            .signed_in?
            .saturating_add(seconds(config.absolute_minutes));
        let idle_end = self
            .touched?
            .saturating_add(seconds(config.idle_minutes))
            .min(absolute_end);
        let absolute = absolute_end.saturating_sub(now);
        let idle = idle_end.saturating_sub(now);
        (idle > 0).then(|| Left {
            idle: u32::try_from(idle).unwrap_or(u32::MAX),
            absolute: u32::try_from(absolute).unwrap_or(u32::MAX),
        })
    }
}

/// `minutes` in seconds, saturating.
fn seconds(minutes: u64) -> i64 {
    i64::try_from(minutes.saturating_mul(60)).unwrap_or(i64::MAX)
}

/// Current wall-clock seconds (jiff is the one time library).
fn now_seconds() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// Decode the session from a request's headers.
///
/// Absent, unparsable, tampered, wrong-key, idle-expired and
/// lifetime-expired cookies all decode to the empty session, and so does a
/// cookie sealed without a sign-in time: on this seam every defect means the
/// same thing — no authenticated caller — and the guard turns that into a
/// redirect or a typed `Unauthenticated`, never a 500.
#[must_use]
pub fn unseal(
    keys: &SessionKeys,
    headers: &http::HeaderMap,
    config: &SessionConfig,
) -> CookieSession {
    unseal_at(keys, headers, config, now_seconds())
}

/// [`unseal`] at `now`.
fn unseal_at(
    keys: &SessionKeys,
    headers: &http::HeaderMap,
    config: &SessionConfig,
    now: i64,
) -> CookieSession {
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
    if now.saturating_sub(envelope.touched) >= seconds(config.idle_minutes)
        || now.saturating_sub(envelope.signed_in) >= seconds(config.absolute_minutes)
    {
        return CookieSession::default();
    }
    CookieSession {
        entries: envelope.entries,
        touched: Some(envelope.touched),
        signed_in: Some(envelope.signed_in),
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

/// Seal a session into a `Set-Cookie` value (fresh idle anchor, the sign-in
/// time kept, or stamped now for a session that has none yet).
///
/// The cookie lives until the first of the two timeouts. An empty session
/// seals to the REMOVAL cookie (`Max-Age=0`), so "save an emptied session"
/// and "sign out" are the same operation.
///
/// # Errors
/// [`ViewerError::Failed`] when the payload does not serialize or the
/// sealed value is not a valid header (both bugs, surfaced loudly).
pub fn set_cookie(
    keys: &SessionKeys,
    session: &CookieSession,
    config: &SessionConfig,
) -> Result<http::HeaderValue, ViewerError> {
    set_cookie_at(keys, session, config, now_seconds())
}

/// [`set_cookie`] at `now`.
fn set_cookie_at(
    keys: &SessionKeys,
    session: &CookieSession,
    config: &SessionConfig,
    now: i64,
) -> Result<http::HeaderValue, ViewerError> {
    let cookie = if session.is_empty() {
        removal_cookie(config.cookie_secure)
    } else {
        let signed_in = session.signed_in.unwrap_or(now);
        let envelope = Envelope {
            touched: now,
            signed_in,
            entries: session.entries.clone(),
        };
        let payload = serde_json::to_string(&envelope)
            .map_err(|e| ViewerError::failed("session serialization", e))?;
        let lifetime_left = signed_in
            .saturating_add(seconds(config.absolute_minutes))
            .saturating_sub(now);
        let max_age = seconds(config.idle_minutes).min(lifetime_left).max(0);
        let mut jar = cookie::CookieJar::new();
        jar.private_mut(&keys.key)
            .add(base_cookie(payload, config.cookie_secure, Some(max_age)));
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
        .map_err(|e| ViewerError::failed("session cookie header", e))
    // NOTE: no openEHR spec governs the viewer session — our own design;
    // flags per the OWASP Session Management Cheat Sheet (HttpOnly, Lax for
    // the OIDC top-level callback redirect, Secure behind TLS).
}

/// The attribute set every variant of our cookie carries.
fn base_cookie(
    value: String,
    cookie_secure: bool,
    max_age_seconds: Option<i64>,
) -> cookie::Cookie<'static> {
    let mut builder = cookie::Cookie::build((SESSION_COOKIE, value))
        .path("/")
        .http_only(true)
        .secure(cookie_secure)
        .same_site(cookie::SameSite::Lax);
    if let Some(max_age) = max_age_seconds {
        builder = builder.max_age(cookie::time::Duration::seconds(max_age));
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
            &scope.state.config.session,
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
/// [`ViewerError::Failed`] outside a request or on a sealing failure.
pub fn commit(session: &CookieSession) -> Result<(), ViewerError> {
    let state = crate::scope::state()?;
    let header = set_cookie(&state.session_keys, session, &state.config.session)?;
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
        set_cookie, set_cookie_at, unseal, unseal_at,
    };
    use crate::config::SessionConfig;
    use ferrohealth_ui::view::session::Left;

    /// The session settings with `idle_minutes`, the default lifetime and
    /// the `Secure` flag as `secure` says.
    fn config(secure: bool, idle_minutes: u64) -> SessionConfig {
        SessionConfig {
            idle_minutes,
            cookie_secure: secure,
            ..SessionConfig::default()
        }
    }

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
        let header = set_cookie(&sealing, &session_with_viewer(), &config(true, 60)).unwrap();
        // A DIFFERENT SessionKeys value from the same secret — the
        // multi-instance property under test.
        let other_instance = SessionKeys::from_secret(&base64_of(&[0u8; 96])).unwrap();
        let restored = unseal(&other_instance, &headers_with(&header), &config(true, 60));
        let restored_session: ViewerSession = restored.get(SESSION_KEY).unwrap();
        assert_eq!(restored_session.identity, "demo");
    }

    #[test]
    fn the_cookie_is_ciphertext_with_the_hardening_attributes() {
        let header = set_cookie(&keys(), &session_with_viewer(), &config(true, 60)).unwrap();
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
        let header = set_cookie(&keys(), &session_with_viewer(), &config(false, 60)).unwrap();
        let stranger = SessionKeys::from_secret(&base64_of(&[7u8; 96])).unwrap();
        assert!(unseal(&stranger, &headers_with(&header), &config(false, 60)).is_empty());

        let mut headers = headers_with(&header);
        let tampered = headers[http::header::COOKIE]
            .to_str()
            .unwrap()
            .replace('=', "=X");
        headers.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&tampered).unwrap(),
        );
        assert!(unseal(&keys(), &headers, &config(false, 60)).is_empty());
    }

    #[test]
    fn an_idle_expired_payload_reads_as_no_session() {
        let sealing = keys();
        let header = set_cookie(&sealing, &session_with_viewer(), &config(false, 60)).unwrap();
        // idle_minutes = 0 makes any positive age expired.
        assert!(unseal(&sealing, &headers_with(&header), &config(false, 0)).is_empty());
    }

    #[test]
    fn an_emptied_session_seals_to_the_removal_cookie() {
        let header = set_cookie(&keys(), &CookieSession::default(), &config(true, 60)).unwrap();
        let text = header.to_str().unwrap();
        assert!(text.contains("Max-Age=0"), "not a removal cookie: {text}");
        assert!(text.starts_with(&format!("{SESSION_COOKIE}=;")));
    }

    /// One sign-in at `T0`, then a request every half hour: the absolute
    /// lifetime still ends the session twelve hours after the sign-in.
    #[test]
    fn activity_never_carries_a_session_past_its_absolute_lifetime() {
        const T0: i64 = 1_800_000_000;
        let config = config(false, 60);
        let lifetime = 720 * 60;
        let mut header = set_cookie_at(&keys(), &session_with_viewer(), &config, T0).unwrap();
        let mut at = T0;
        while at + 1800 < T0 + lifetime {
            at += 1800;
            let open = unseal_at(&keys(), &headers_with(&header), &config, at);
            assert!(!open.is_empty(), "the session ended early, at {}", at - T0);
            header = set_cookie_at(&keys(), &open, &config, at).unwrap();
        }
        assert!(
            unseal_at(&keys(), &headers_with(&header), &config, T0 + lifetime).is_empty(),
            "the session outlived its absolute lifetime"
        );
    }

    /// Ten minutes before the lifetime ends, an extend leaves ten minutes,
    /// never the hour of the idle window, and the cookie dies with it.
    #[test]
    fn an_extend_cannot_pass_the_absolute_end() {
        const T0: i64 = 1_800_000_000;
        let config = config(false, 60);
        let late = T0 + 710 * 60;
        let mut header = set_cookie_at(&keys(), &session_with_viewer(), &config, T0).unwrap();
        for at in (T0..late).step_by(1800).skip(1).chain([late]) {
            let open = unseal_at(&keys(), &headers_with(&header), &config, at);
            header = set_cookie_at(&keys(), &open, &config, at).unwrap();
        }
        let open = unseal_at(&keys(), &headers_with(&header), &config, late);
        assert_eq!(
            Some(Left {
                idle: 600,
                absolute: 600
            }),
            open.left_at(&config, late)
        );
        assert!(
            header.to_str().unwrap().contains("Max-Age=600"),
            "the cookie outlives the session: {header:?}"
        );
    }

    /// A read of what is left changes nothing: only a sealing slides the
    /// idle anchor.
    #[test]
    fn what_is_left_counts_down_from_the_last_sealing() {
        const T0: i64 = 1_800_000_000;
        let config = config(false, 60);
        let header = set_cookie_at(&keys(), &session_with_viewer(), &config, T0).unwrap();
        let open = unseal_at(&keys(), &headers_with(&header), &config, T0 + 600);
        assert_eq!(
            Some(Left {
                idle: 3000,
                absolute: 720 * 60 - 600
            }),
            open.left_at(&config, T0 + 600)
        );
        assert_eq!(None, CookieSession::default().left_at(&config, T0));
    }

    /// A cookie sealed before the sign-in time was carried does not open,
    /// so its user signs in again.
    #[test]
    fn a_cookie_sealed_without_a_sign_in_time_reads_as_no_session() {
        let viewer = serde_json::to_string(&session_with_viewer().entries[SESSION_KEY]).unwrap();
        let payload = format!(
            r#"{{"touched":{},"entries":{{"{SESSION_KEY}":{viewer}}}}}"#,
            jiff::Timestamp::now().as_second()
        );
        let mut jar = cookie::CookieJar::new();
        jar.private_mut(&keys().key)
            .add(cookie::Cookie::new(SESSION_COOKIE, payload));
        let sealed = jar.get(SESSION_COOKIE).unwrap().to_string();
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&sealed).unwrap(),
        );
        assert!(unseal(&keys(), &headers, &config(false, 60)).is_empty());
    }

    #[test]
    fn a_short_or_malformed_secret_is_refused_and_empty_is_ephemeral() {
        assert!(SessionKeys::from_secret(&base64_of(&[1u8; 32])).is_err());
        assert!(SessionKeys::from_secret("not base64!!").is_err());
        assert!(SessionKeys::from_secret("  ").is_ok());
    }
}
