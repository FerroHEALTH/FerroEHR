// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authentication configuration (Basic + OAuth2/OIDC bearer).
//!
//! No openEHR spec governs authentication mechanics, ITS-REST leaving the scheme
//! open — our own design, with the IETF OAuth2/JOSE RFCs as the authority for
//! every rule enforced here. This is the `[auth]` section of the one server
//! configuration tree and carries no loader of its own.
//!
//! Secrets use the shared [`crate::config::secret::Secret`] newtype: a password
//! hash or HMAC secret deserializes from a plain string but never renders itself
//! (`Debug`, `/management/env` and `ferroehr config check` all show `***`). Each
//! secret key has a `*_file` sibling for file-based indirection, resolved by the
//! loader.
//!
//! Boot validation lives in [`AuthConfig::validate`] and
//! [`AuthConfig::require_mechanism`]: a configuration a resource server cannot
//! honour is refused at startup rather than degraded at the first request.

use std::path::PathBuf;

use argon2::{Algorithm, Params, PasswordHash};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::secret::Secret;

/// Top-level authentication settings (`[auth]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// Master switch. When `false`, all requests pass unauthenticated (dev only).
    pub enabled: bool,
    /// Basic-auth user store (username → Argon2 PHC hash). Absent → Basic disabled.
    pub basic: Option<BasicConfig>,
    /// Verified Basic-credential cache TTL in seconds (`0` disables the cache).
    /// Argon2 verification costs tens of milliseconds of CPU **per call by
    /// design**; re-running it on every request of a busy client turns the
    /// KDF's work factor into a self-inflicted throughput ceiling. A credential
    /// that has verified successfully is therefore remembered — as a SHA-256
    /// digest of the presented header, never plaintext — and re-verified only
    /// after the TTL (which bounds credential-revocation lag exactly like a
    /// session lifetime).
    pub verified_cache_ttl_seconds: u64,
    /// OAuth2/OIDC bearer validation. Absent → bearer disabled.
    pub oidc: Option<OidcConfig>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            basic: None,
            oidc: None,
            // 60 s: long enough that a busy client pays the KDF once a minute
            // instead of per request, short enough that a revoked credential
            // dies within a minute.
            verified_cache_ttl_seconds: 60,
        }
    }
}

impl AuthConfig {
    /// Whether at least one authentication mechanism is configured.
    #[must_use]
    pub fn has_mechanism(&self) -> bool {
        self.basic.is_some() || self.oidc.is_some()
    }

    /// The authentication schemes this deployment can name in a `401`
    /// challenge, in the order it offers them.
    ///
    /// RFC 9110 §11.6.1 requires a challenge to name a scheme applicable to the
    /// target resource, so this is exactly the set a `WWW-Authenticate` may
    /// carry. Returned as display text for the boot log: an operator reading
    /// `Basic, Bearer` can see at a glance which tables actually took effect.
    /// `none` when no mechanism is configured (only reachable with
    /// [`Self::enabled`] off — [`Self::require_mechanism`] refuses that
    /// combination at boot).
    #[must_use]
    pub fn advertised_mechanisms(&self) -> String {
        let mut schemes: Vec<&str> = Vec::new();
        if self.basic.is_some() {
            schemes.push("Basic");
        }
        if self.oidc.is_some() {
            schemes.push("Bearer");
        }
        if schemes.is_empty() {
            "none".to_owned()
        } else {
            schemes.join(", ")
        }
    }

    /// Validates the configured mechanisms at boot.
    ///
    /// Each present mechanism table is judged on its own terms, whatever
    /// [`Self::enabled`] says: a table an operator wrote must be honourable, so
    /// flipping the master switch on can never newly break the deployment.
    ///
    /// # Errors
    /// [`AuthConfigError`] for the first failing rule: an unusable `[auth.oidc]`
    /// issuer, audience, leeway or symmetric secret, or a Basic password hash
    /// below the OWASP Argon2id floor.
    pub fn validate(&self) -> Result<(), AuthConfigError> {
        if let Some(basic) = &self.basic {
            basic.validate()?;
        }
        if let Some(oidc) = &self.oidc {
            oidc.validate()?;
        }
        Ok(())
    }

    /// Requires an authentication mechanism whenever authentication is enabled.
    ///
    /// RFC 9110 §11.6.1 requires a `401` challenge to name a scheme *applicable
    /// to the target resource*, and a server with no mechanism has none: it
    /// could only refuse every request while advertising a scheme it does not
    /// implement.
    ///
    /// # Errors
    /// [`AuthConfigError::NoMechanism`] when [`Self::enabled`] is set and
    /// neither `[auth.basic]` nor `[auth.oidc]` is configured.
    pub fn require_mechanism(&self) -> Result<(), AuthConfigError> {
        if self.enabled && !self.has_mechanism() {
            return Err(AuthConfigError::NoMechanism);
        }
        Ok(())
    }
}

/// A boot-time authentication-configuration error — a hard error that aborts
/// startup rather than letting the server run a posture it cannot honour.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthConfigError {
    /// `auth.enabled` with no mechanism configured (RFC 9110 §11.6.1).
    #[error(
        "auth.enabled = true but no authentication mechanism is configured: add \
         [[auth.basic.users]] entries or an [auth.oidc] issuer, or set auth.enabled = false"
    )]
    NoMechanism,
    /// `auth.oidc.issuer` is blank.
    #[error("auth.oidc.issuer is required when the [auth.oidc] table is present")]
    IssuerMissing,
    /// `auth.oidc.issuer` does not parse as an absolute URL (RFC 8414 §2).
    #[error("auth.oidc.issuer {issuer:?} is not an absolute URL (RFC 8414 §2): {reason}")]
    IssuerNotAUrl {
        /// The rejected issuer value.
        issuer: String,
        /// The URL parse failure, as reported by `url`.
        reason: String,
    },
    /// `auth.oidc.issuer` carries a query component (RFC 8414 §2).
    #[error("auth.oidc.issuer {0:?} must have no query component (RFC 8414 §2)")]
    IssuerHasQuery(String),
    /// `auth.oidc.issuer` carries a fragment component (RFC 8414 §2).
    #[error("auth.oidc.issuer {0:?} must have no fragment component (RFC 8414 §2)")]
    IssuerHasFragment(String),
    /// `auth.oidc.issuer` does not use the `https` scheme (RFC 8414 §2, §6.2).
    #[error(
        "auth.oidc.issuer {0:?} must use the https scheme (RFC 8414 §2, §6.2); set \
         auth.oidc.allow_insecure_issuer = true for a development issuer"
    )]
    IssuerNotHttps(String),
    /// `auth.oidc.audiences` is empty or all-blank (RFC 7519 §4.1.3).
    #[error(
        "auth.oidc.audiences must list at least one accepted audience: a token whose `aud` \
         does not name this resource server MUST be rejected (RFC 7519 §4.1.3, RFC 9068 §4)"
    )]
    AudiencesRequired,
    /// `auth.oidc.clock_skew_leeway_seconds` exceeds the cap.
    #[error(
        "auth.oidc.clock_skew_leeway_seconds = {0} exceeds the {1} s cap: expiry leeway may be \
         no more than a few minutes (RFC 9068 §4 step 6)"
    )]
    LeewayTooLarge(u64, u64),
    /// A symmetric (`HS*`) algorithm is declared without a symmetric key, or an
    /// asymmetric one with only a symmetric key.
    #[error(
        "auth.oidc.algorithms lists {algorithm:?} but the configured key source is \
         {key_source}: a \
         key is bound to ONE algorithm family, and accepting an algorithm the key material \
         cannot verify invites algorithm-confusion (RFC 8725 §3.1, RFC 7515 §10.4)"
    )]
    AlgorithmKeySourceMismatch {
        /// The offending entry of `auth.oidc.algorithms`.
        algorithm: String,
        /// Which key source is configured (`hmac_secret`, `jwks_json`, or
        /// issuer discovery). Not named `source`: `thiserror` reads a field of
        /// that name as the error's `Error::source`.
        key_source: &'static str,
    },
    /// `auth.oidc.algorithms` names `none` — an unsigned token.
    #[error(
        "auth.oidc.algorithms may not contain {0:?}: an unsigned token proves nothing, and \
         `alg: none` is the canonical JWT forgery (RFC 8725 §3.2, RFC 9068 §4 step 5)"
    )]
    AlgorithmNoneRejected(String),
    /// `auth.oidc.hmac_secret` is shorter than the entropy floor (RFC 8725 §3.5).
    #[error(
        "auth.oidc.hmac_secret is {0} bytes; a keyed-MAC key must be at least {1} bytes of \
         high-entropy material and never a human-memorizable password (RFC 8725 §3.5)"
    )]
    HmacSecretTooShort(usize, usize),
    /// A `[[auth.basic.users]]` entry carries no `username`.
    #[error("every [[auth.basic.users]] entry requires a non-blank username")]
    BasicUserWithoutUsername,
    /// A Basic user's `password_hash` is not a parsable PHC string.
    #[error("auth.basic.users[{username:?}].password_hash is not a PHC hash string: {reason}")]
    PasswordHashUnparsable {
        /// The offending user's login name (never the hash itself).
        username: String,
        /// The PHC parse failure, as reported by `argon2`.
        reason: String,
    },
    /// A Basic user's `password_hash` uses an algorithm other than Argon2id.
    #[error(
        "auth.basic.users[{username:?}].password_hash uses {algorithm:?}; only argon2id is \
         accepted (OWASP Password Storage Cheat Sheet §Argon2id)"
    )]
    PasswordHashNotArgon2id {
        /// The offending user's login name.
        username: String,
        /// The PHC algorithm identifier found in the hash.
        algorithm: String,
    },
    /// A Basic user's Argon2 cost parameters are below the OWASP floor.
    #[error(
        "auth.basic.users[{username:?}].password_hash has m={m},t={t},p={p}, below the minimum \
         m={min_m},t={min_t},p={min_p} (OWASP Password Storage Cheat Sheet §Argon2id)"
    )]
    PasswordHashTooWeak {
        /// The offending user's login name.
        username: String,
        /// The configured memory cost, in KiB.
        m: u32,
        /// The configured time cost (iterations).
        t: u32,
        /// The configured degree of parallelism.
        p: u32,
        /// The required minimum memory cost, in KiB.
        min_m: u32,
        /// The required minimum time cost.
        min_t: u32,
        /// The required minimum parallelism.
        min_p: u32,
    },
}

/// The minimum Argon2id memory cost, in KiB, the OWASP Password Storage Cheat
/// Sheet §Argon2id prescribes (19 MiB) — also `argon2` 0.6.0's
/// `Params::DEFAULT_M_COST`.
pub const MIN_ARGON2_M_COST: u32 = 19 * 1024;

/// The minimum Argon2id time cost (iterations) per the OWASP Password Storage
/// Cheat Sheet §Argon2id.
pub const MIN_ARGON2_T_COST: u32 = 2;

/// The minimum Argon2id degree of parallelism per the OWASP Password Storage
/// Cheat Sheet §Argon2id.
pub const MIN_ARGON2_P_COST: u32 = 1;

/// Basic-auth user store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BasicConfig {
    /// The configured users. Passwords are stored only as Argon2 PHC hashes.
    pub users: Vec<BasicUser>,
}

impl BasicConfig {
    /// Validate every stored password hash against the OWASP Argon2id floor.
    ///
    /// The verifier takes its cost parameters from the stored PHC string
    /// (`argon2` 0.6.0 `impl TryFrom<&PasswordHash> for Params`), so a
    /// deliberately cheap hash would verify happily; the floor is therefore
    /// judged here, through the same parse the verifier uses.
    fn validate(&self) -> Result<(), AuthConfigError> {
        for user in &self.users {
            if user.username.trim().is_empty() {
                return Err(AuthConfigError::BasicUserWithoutUsername);
            }
            let hash = PasswordHash::new(user.password_hash.expose()).map_err(|e| {
                AuthConfigError::PasswordHashUnparsable {
                    username: user.username.clone(),
                    reason: e.to_string(),
                }
            })?;
            if !matches!(
                Algorithm::try_from(hash.algorithm.as_str()),
                Ok(Algorithm::Argon2id)
            ) {
                return Err(AuthConfigError::PasswordHashNotArgon2id {
                    username: user.username.clone(),
                    algorithm: hash.algorithm.as_str().to_owned(),
                });
            }
            let params =
                Params::try_from(&hash).map_err(|e| AuthConfigError::PasswordHashUnparsable {
                    username: user.username.clone(),
                    reason: e.to_string(),
                })?;
            if params.m_cost() < MIN_ARGON2_M_COST
                || params.t_cost() < MIN_ARGON2_T_COST
                || params.p_cost() < MIN_ARGON2_P_COST
            {
                return Err(AuthConfigError::PasswordHashTooWeak {
                    username: user.username.clone(),
                    m: params.m_cost(),
                    t: params.t_cost(),
                    p: params.p_cost(),
                    min_m: MIN_ARGON2_M_COST,
                    min_t: MIN_ARGON2_T_COST,
                    min_p: MIN_ARGON2_P_COST,
                });
            }
        }
        Ok(())
    }
}

/// One Basic-auth principal (`[[auth.basic.users]]`).
///
/// `username` and `password_hash` are mandatory, enforced by
/// [`AuthConfig::validate`] rather than by serde: an omitted one lands here as
/// the empty [`Default`] value and is then refused at boot by name, which reads
/// better than a bare missing-field error and cannot be mistaken for a usable
/// credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BasicUser {
    /// The login name presented in the `Authorization: Basic` header.
    pub username: String,
    /// Argon2 PHC hash string (`$argon2id$v=19$...`). Never a plaintext password.
    pub password_hash: Secret,
    /// Path to a file holding the PHC hash, read at boot in place of
    /// [`Self::password_hash`].
    ///
    /// A hash is an offline cracking target, so it belongs in a mounted secret
    /// rather than inline in a configuration file that deployment tooling may
    /// treat as non-sensitive. The same Argon2id parameter floor is enforced
    /// either way, because validation runs after the file is resolved. Setting
    /// both is a boot error.
    pub password_hash_file: Option<PathBuf>,
    /// Roles granted to this user (normalized to upper-case when authenticated),
    /// feeding the RBAC gate. Defaults to `["USER"]` — the baseline clinical
    /// role — when unspecified; configure `["ADMIN"]` for an administrative
    /// account.
    pub roles: Vec<String>,
}

impl Default for BasicUser {
    fn default() -> Self {
        Self {
            username: String::new(),
            password_hash: Secret::new(String::new()),
            password_hash_file: None,
            roles: vec!["USER".to_owned()],
        }
    }
}

/// OAuth2/OIDC bearer configuration.
///
/// Validation happens as a resource server: the token's signature is checked
/// against a key source and the `iss`/`aud` claims validated. The
/// authorization-code client flow (the `oauth2` crate) is a client concern,
/// not a CDR's, so it is out of scope here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OidcConfig {
    /// Expected token issuer (`iss`). Also the OIDC discovery base when no
    /// static key material is supplied. Required when the table is present.
    ///
    /// Boot-validated as an RFC 8414 §2 issuer identifier: an absolute `https`
    /// URL with no query and no fragment component. The scheme requirement is
    /// relaxed only by [`Self::allow_insecure_issuer`].
    pub issuer: String,
    /// Accepted audiences (`aud`) — at least one is required.
    ///
    /// RFC 7519 §4.1.3 obliges a recipient that does not identify itself with a
    /// value in a present `aud` claim to reject the JWT, and RFC 9068 §4 step 4
    /// makes the check unconditional for an access token; a resource server with
    /// no declared audience would therefore accept tokens minted for a different
    /// resource server, and OpenID Connect ID tokens (whose `aud` is a client
    /// id) alongside them (RFC 8725 §3.9, §3.12).
    pub audiences: Vec<String>,
    /// Accepted signature algorithms (e.g. `["RS256"]`). Defaults to `RS256`.
    pub algorithms: Vec<String>,
    /// Accepted clock-skew leeway, in seconds, on the time-based claims
    /// (default 60; boot error above [`MAX_CLOCK_SKEW_LEEWAY_SECONDS`]).
    ///
    /// RFC 7519 §4.1.4 permits "some small leeway, usually no more than a few
    /// minutes, to account for clock skew", and RFC 9068 §4 step 6 repeats the
    /// bound for access tokens — so the key is capped rather than free, since a
    /// large leeway silently extends every token's life.
    pub clock_skew_leeway_seconds: u64,
    /// Require every bearer token to claim the RFC 9068 access-token profile
    /// (`typ: at+jwt`); default `false`.
    ///
    /// RFC 9068 §2.1 makes `at+jwt` a SHOULD for the AUTHORIZATION SERVER, and
    /// §4 step 1's MUST attaches to tokens claiming the profile — the RFC
    /// prescribes nothing for a token that does not claim it. So a token
    /// carrying the type gets the full §4 rule set (`iat`, `jti` and `client_id`
    /// required) whatever this key says, while turning it on additionally
    /// REFUSES a token that omits the type. Off by default because requiring it
    /// rejects issuers that do not set it, including the identity provider the
    /// quickstart overlay ships.
    pub require_at_jwt: bool,
    /// Accepts a non-`https` [`Self::issuer`] (default `false`).
    ///
    /// RFC 8414 §6.2 requires TLS for issuer metadata, so this is a development
    /// and test affordance only — a plain-HTTP issuer exposes token
    /// verification material to a network attacker. The no-query/no-fragment
    /// structural rules of RFC 8414 §2 still apply.
    pub allow_insecure_issuer: bool,
    /// A symmetric HS256 secret — the simplest key source (tests/dev).
    pub hmac_secret: Option<Secret>,
    /// File-based indirection for [`Self::hmac_secret`] (K8s/Docker secrets).
    /// Exactly one of the pair may be set; the loader reads and trims the file.
    pub hmac_secret_file: Option<PathBuf>,
    /// A static JWKS document (JSON). Preferred over discovery when present.
    pub jwks_json: Option<String>,
    /// File-based indirection for [`Self::jwks_json`] — a JWKS is a file-shaped
    /// blob that never belonged in an env var. The loader reads the file.
    pub jwks_json_file: Option<PathBuf>,
    /// TCP connect timeout in milliseconds for the OIDC discovery + JWKS
    /// fetches (default 3000).
    ///
    /// Applies to the discovery key source only (no [`Self::hmac_secret`], no
    /// [`Self::jwks_json`]): without it an issuer that blackholes packets parks
    /// the bearer request until the OS TCP timeout, holding a request slot.
    pub connect_timeout_ms: u64,
    /// Whole-request timeout in milliseconds for the OIDC discovery + JWKS
    /// fetches (default 5000), covering connect, TLS, and body read.
    pub request_timeout_ms: u64,
    /// How long a FAILED discovery/JWKS fetch is remembered, in seconds
    /// (default 10; `0` disables negative caching).
    ///
    /// Without it every bearer request during an issuer outage re-attempts
    /// discovery, so an unreachable issuer produces one outbound connection
    /// attempt per incoming request. Remembering the failure degrades the outage to
    /// fast `401`s instead. Keep it short — it is also the recovery lag once
    /// the issuer returns. Successfully fetched key material keeps its own,
    /// longer lifetime and is unaffected.
    pub negative_cache_ttl_seconds: u64,
}

/// The largest accepted [`OidcConfig::clock_skew_leeway_seconds`] (5 minutes) —
/// the outer edge of RFC 9068 §4 step 6's "no more than a few minutes".
pub const MAX_CLOCK_SKEW_LEEWAY_SECONDS: u64 = 300;

/// The smallest accepted [`OidcConfig::hmac_secret`], in bytes.
///
/// RFC 8725 §3.5 forbids a human-memorizable password as a keyed-MAC key; 32
/// bytes is the output size of the SHA-256 the smallest supported HS\* algorithm
/// uses, so a shorter key adds no strength over one at the floor.
pub const MIN_HMAC_SECRET_BYTES: usize = 32;

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            audiences: Vec::new(),
            // RS256 is the one algorithm OpenID Connect Discovery §4.2 makes
            // mandatory to implement, so it is the only safe starting set.
            algorithms: vec!["RS256".to_owned()],
            // 60 s: the conventional clock-skew allowance, well inside
            // RFC 9068 §4 step 6's "no more than a few minutes".
            clock_skew_leeway_seconds: 60,
            // RFC 9068 §2.1 makes `at+jwt` a SHOULD for the authorization
            // server, so requiring it would refuse conforming issuers.
            require_at_jwt: false,
            allow_insecure_issuer: false,
            hmac_secret: None,
            hmac_secret_file: None,
            jwks_json: None,
            jwks_json_file: None,
            // 3 s: ample for a TCP+TLS handshake to a healthy issuer on any
            // realistic network, short enough that a blackholed one fails fast.
            connect_timeout_ms: 3_000,
            // 5 s: the whole-request budget the remote-PDP and terminology
            // clients use, so one outbound-HTTP posture spans the server.
            request_timeout_ms: 5_000,
            // 10 s: long enough that an outage costs one discovery attempt per
            // issuer rather than one per request, short enough that recovery is
            // barely noticed.
            negative_cache_ttl_seconds: 10,
        }
    }
}

impl OidcConfig {
    /// Whether a symmetric key is the configured signing-key source.
    ///
    /// The boot path warns on this: an HS\* key shared with the authorization
    /// server lets this resource server MINT tokens it would then accept, which
    /// no asymmetric key source allows (RFC 8725 §2.2, §3.5).
    #[must_use]
    pub fn uses_symmetric_key(&self) -> bool {
        self.hmac_secret.is_some() || self.hmac_secret_file.is_some()
    }

    /// Validate the `[auth.oidc]` table at boot.
    fn validate(&self) -> Result<(), AuthConfigError> {
        self.validate_issuer()?;
        if self.audiences.iter().all(|a| a.trim().is_empty()) {
            return Err(AuthConfigError::AudiencesRequired);
        }
        if self.clock_skew_leeway_seconds > MAX_CLOCK_SKEW_LEEWAY_SECONDS {
            return Err(AuthConfigError::LeewayTooLarge(
                self.clock_skew_leeway_seconds,
                MAX_CLOCK_SKEW_LEEWAY_SECONDS,
            ));
        }
        if let Some(secret) = &self.hmac_secret {
            let len = secret.expose().len();
            if len < MIN_HMAC_SECRET_BYTES {
                return Err(AuthConfigError::HmacSecretTooShort(
                    len,
                    MIN_HMAC_SECRET_BYTES,
                ));
            }
        }
        self.validate_algorithms()?;
        Ok(())
    }

    /// Bind the accepted algorithm set to the configured key source.
    ///
    /// A key belongs to ONE algorithm family: an HMAC secret can verify `HS*`
    /// and nothing else, and a JWKS carries public keys that verify `RS*`/`ES*`/
    /// `PS*` and nothing else. Accepting an algorithm the key material cannot
    /// verify is the algorithm-confusion setup RFC 8725 §3.1 and RFC 7515 §10.4
    /// warn about — most famously an `RS256` deployment that also accepts
    /// `HS256`, letting an attacker sign with the PUBLIC key as if it were a
    /// shared secret.
    ///
    /// `none` is refused outright: an unsigned token proves nothing.
    fn validate_algorithms(&self) -> Result<(), AuthConfigError> {
        let symmetric_key = self.hmac_secret.is_some() || self.hmac_secret_file.is_some();
        let source = if symmetric_key {
            "auth.oidc.hmac_secret (a symmetric key, which verifies HS* only)"
        } else if self.jwks_json.is_some() || self.jwks_json_file.is_some() {
            "auth.oidc.jwks_json (public keys, which verify RS*/ES*/PS* only)"
        } else {
            "the issuer's discovered JWKS (public keys, which verify RS*/ES*/PS* only)"
        };
        for declared in &self.algorithms {
            let name = declared.trim();
            if name.eq_ignore_ascii_case("none") {
                return Err(AuthConfigError::AlgorithmNoneRejected(name.to_owned()));
            }
            // The JOSE `HS*` family, matched without slicing: a byte range into a
            // `str` panics on a UTF-8 boundary, and an operator's typo is
            // arbitrary text.
            let mut chars = name.chars();
            let is_symmetric = matches!(
                (chars.next(), chars.next()),
                (Some('H' | 'h'), Some('S' | 's'))
            );
            if is_symmetric != symmetric_key {
                return Err(AuthConfigError::AlgorithmKeySourceMismatch {
                    algorithm: name.to_owned(),
                    key_source: source,
                });
            }
        }
        Ok(())
    }

    /// Judge [`Self::issuer`] against the RFC 8414 §2 issuer-identifier rules.
    fn validate_issuer(&self) -> Result<(), AuthConfigError> {
        let issuer = self.issuer.trim();
        if issuer.is_empty() {
            return Err(AuthConfigError::IssuerMissing);
        }
        let url = Url::parse(issuer).map_err(|e| AuthConfigError::IssuerNotAUrl {
            issuer: issuer.to_owned(),
            reason: e.to_string(),
        })?;
        if url.query().is_some() {
            return Err(AuthConfigError::IssuerHasQuery(issuer.to_owned()));
        }
        if url.fragment().is_some() {
            return Err(AuthConfigError::IssuerHasFragment(issuer.to_owned()));
        }
        if url.scheme() != "https" && !self.allow_insecure_issuer {
            return Err(AuthConfigError::IssuerNotHttps(issuer.to_owned()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 32-byte high-entropy secret — exactly at the RFC 8725 §3.5 floor.
    const GOOD_SECRET: &str = "0123456789abcdef0123456789abcdef";

    /// An `[auth.oidc]` table that satisfies every boot rule.
    fn valid_oidc() -> OidcConfig {
        OidcConfig {
            issuer: "https://idp.example/realms/ferroehr".to_owned(),
            audiences: vec!["ferroehr".to_owned()],
            ..OidcConfig::default()
        }
    }

    /// A key belongs to ONE algorithm family, so accepting an algorithm the key
    /// material cannot verify is the algorithm-confusion setup RFC 8725 §3.1 and
    /// RFC 7515 §10.4 warn about: most famously an `RS256` deployment that also
    /// accepts `HS256`, letting an attacker sign with the PUBLIC key as if it
    /// were a shared secret.
    #[test]
    fn hs256_with_a_public_key_source_is_a_boot_error() {
        // Discovery (no local key material) verifies public keys only.
        let discovery = OidcConfig {
            algorithms: vec!["HS256".to_owned()],
            ..valid_oidc()
        };
        assert!(matches!(
            discovery.validate(),
            Err(AuthConfigError::AlgorithmKeySourceMismatch { .. })
        ));

        // And the mirror: a symmetric secret cannot verify RS256.
        let symmetric = OidcConfig {
            algorithms: vec!["RS256".to_owned()],
            hmac_secret: Some(Secret::new(GOOD_SECRET.to_owned())),
            ..valid_oidc()
        };
        assert!(matches!(
            symmetric.validate(),
            Err(AuthConfigError::AlgorithmKeySourceMismatch { .. })
        ));

        // The matching pairs both boot.
        let hmac_ok = OidcConfig {
            algorithms: vec!["HS256".to_owned()],
            hmac_secret: Some(Secret::new(GOOD_SECRET.to_owned())),
            ..valid_oidc()
        };
        assert!(hmac_ok.validate().is_ok());
        assert!(valid_oidc().validate().is_ok(), "RS256 + discovery");
    }

    /// RFC 8725 §3.2 / RFC 9068 §4 step 5: an unsigned token proves nothing, and
    /// `alg: none` is the canonical JWT forgery. The refusal was implicit in the
    /// crate's algorithm parser; it is now a boot error naming the key.
    #[test]
    fn algorithm_none_is_unconfigurable() {
        for spelling in ["none", "None", "NONE"] {
            let cfg = OidcConfig {
                algorithms: vec![spelling.to_owned()],
                ..valid_oidc()
            };
            assert!(
                matches!(
                    cfg.validate(),
                    Err(AuthConfigError::AlgorithmNoneRejected(_))
                ),
                "`{spelling}` must be refused at boot",
            );
        }
    }

    /// An Argon2id PHC string at the OWASP floor, with the given costs.
    fn phc(m: u32, t: u32, p: u32) -> String {
        format!("$argon2id$v=19$m={m},t={t},p={p}$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2hoYQ")
    }

    fn basic(hash: String) -> BasicConfig {
        BasicConfig {
            users: vec![BasicUser {
                username: "clinician".to_owned(),
                password_hash: Secret::new(hash),
                ..BasicUser::default()
            }],
        }
    }

    /// The one `impl Default` is the single source of every default: serde's
    /// container-level `default` fills each omitted field from it, so a
    /// partially-specified entry keeps the baseline `["USER"]` role, and the two
    /// mandatory fields are enforced by boot validation instead.
    #[test]
    fn a_partial_basic_user_falls_back_to_the_default_impl() {
        let user: BasicUser = toml::from_str(
            "username = \"alice\"\npassword_hash = \"$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aA\"\n",
        )
        .expect("partial user table");
        assert_eq!(user.roles, vec!["USER".to_owned()]);

        let cfg = AuthConfig {
            basic: Some(BasicConfig {
                users: vec![BasicUser::default()],
            }),
            ..AuthConfig::default()
        };
        assert_eq!(
            cfg.validate(),
            Err(AuthConfigError::BasicUserWithoutUsername),
            "an entry with no username must not pass as a usable credential"
        );

        let cfg = AuthConfig {
            basic: Some(BasicConfig {
                users: vec![BasicUser {
                    username: "alice".to_owned(),
                    ..BasicUser::default()
                }],
            }),
            ..AuthConfig::default()
        };
        assert!(
            matches!(
                cfg.validate(),
                Err(AuthConfigError::PasswordHashUnparsable { .. })
            ),
            "an entry with no password hash must not pass either"
        );
    }

    #[test]
    fn a_valid_oidc_table_boots() {
        let cfg = AuthConfig {
            oidc: Some(valid_oidc()),
            ..AuthConfig::default()
        };
        assert_eq!(cfg.validate(), Ok(()));
        assert_eq!(cfg.require_mechanism(), Ok(()));
    }

    /// RFC 7519 §4.1.3 + RFC 9068 §4 step 4: a resource server that declares no
    /// audience cannot reject a token minted for a different one, so an
    /// `[auth.oidc]` table without `audiences` is refused at boot.
    #[test]
    fn oidc_without_audiences_is_a_boot_error() {
        for audiences in [vec![], vec![String::new()], vec!["  ".to_owned()]] {
            let cfg = AuthConfig {
                oidc: Some(OidcConfig {
                    audiences,
                    ..valid_oidc()
                }),
                ..AuthConfig::default()
            };
            assert_eq!(cfg.validate(), Err(AuthConfigError::AudiencesRequired));
        }
    }

    /// RFC 9068 §4 step 6: expiry leeway may be "no more than a few minutes".
    #[test]
    fn leeway_above_the_cap_is_a_boot_error() {
        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                clock_skew_leeway_seconds: MAX_CLOCK_SKEW_LEEWAY_SECONDS + 1,
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert_eq!(
            cfg.validate(),
            Err(AuthConfigError::LeewayTooLarge(
                MAX_CLOCK_SKEW_LEEWAY_SECONDS + 1,
                MAX_CLOCK_SKEW_LEEWAY_SECONDS
            ))
        );

        // The cap itself is accepted, as is the default.
        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                clock_skew_leeway_seconds: MAX_CLOCK_SKEW_LEEWAY_SECONDS,
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert_eq!(cfg.validate(), Ok(()));
        assert_eq!(OidcConfig::default().clock_skew_leeway_seconds, 60);
    }

    /// RFC 8725 §3.5: a human-memorizable password must not be a keyed-MAC key.
    #[test]
    fn short_hmac_secret_is_a_boot_error() {
        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                hmac_secret: Some(Secret::new("correct horse battery")),
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert_eq!(
            cfg.validate(),
            Err(AuthConfigError::HmacSecretTooShort(
                21,
                MIN_HMAC_SECRET_BYTES
            ))
        );

        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                hmac_secret: Some(Secret::new(GOOD_SECRET)),
                // A symmetric key verifies `HS*` only, and the algorithm set is
                // boot-bound to its key source.
                algorithms: vec!["HS256".to_owned()],
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert_eq!(cfg.validate(), Ok(()));
        assert!(
            cfg.oidc
                .as_ref()
                .is_some_and(OidcConfig::uses_symmetric_key)
        );
    }

    /// RFC 8414 §2 + §6.2: the issuer identifier is an `https` URL, so a
    /// plain-HTTP issuer is refused unless explicitly opted into for dev/test.
    #[test]
    fn http_issuer_is_a_boot_error() {
        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                issuer: "http://keycloak:8081/auth/realms/ferroehr".to_owned(),
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert_eq!(
            cfg.validate(),
            Err(AuthConfigError::IssuerNotHttps(
                "http://keycloak:8081/auth/realms/ferroehr".to_owned()
            ))
        );

        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                issuer: "http://keycloak:8081/auth/realms/ferroehr".to_owned(),
                allow_insecure_issuer: true,
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert_eq!(cfg.validate(), Ok(()));
    }

    /// RFC 8414 §2: the issuer identifier "has no query or fragment
    /// components" — a structural rule `allow_insecure_issuer` does not relax.
    #[test]
    fn issuer_with_query_is_a_boot_error() {
        let with_query = "https://idp.example/realms/ferroehr?tenant=a";
        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                issuer: with_query.to_owned(),
                allow_insecure_issuer: true,
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert_eq!(
            cfg.validate(),
            Err(AuthConfigError::IssuerHasQuery(with_query.to_owned()))
        );

        let with_fragment = "https://idp.example/realms/ferroehr#frag";
        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                issuer: with_fragment.to_owned(),
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert_eq!(
            cfg.validate(),
            Err(AuthConfigError::IssuerHasFragment(with_fragment.to_owned()))
        );
    }

    #[test]
    fn blank_and_unparsable_issuers_are_boot_errors() {
        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                issuer: "   ".to_owned(),
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert_eq!(cfg.validate(), Err(AuthConfigError::IssuerMissing));

        let cfg = AuthConfig {
            oidc: Some(OidcConfig {
                issuer: "idp.example".to_owned(),
                ..valid_oidc()
            }),
            ..AuthConfig::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(AuthConfigError::IssuerNotAUrl { ref issuer, .. }) if issuer == "idp.example"
        ));
    }

    /// OWASP Password Storage Cheat Sheet §Argon2id: the minimum configuration
    /// is Argon2id with m=19456 KiB, t=2, p=1. The verifier reads its cost
    /// parameters from the stored PHC string, so a cheaper hash would verify.
    #[test]
    fn weak_argon2_params_are_a_boot_error() {
        for (m, t, p) in [(4_096, 2, 1), (16 * 1024, 2, 1), (MIN_ARGON2_M_COST, 1, 1)] {
            let cfg = AuthConfig {
                basic: Some(basic(phc(m, t, p))),
                ..AuthConfig::default()
            };
            assert!(
                matches!(
                    cfg.validate(),
                    Err(AuthConfigError::PasswordHashTooWeak { .. })
                ),
                "m={m},t={t},p={p} is below the OWASP floor and must be refused"
            );
        }

        // `p=0` is not even a representable Argon2 parameter set, so it is
        // refused one step earlier, by the PHC parse itself.
        let cfg = AuthConfig {
            basic: Some(basic(phc(MIN_ARGON2_M_COST, MIN_ARGON2_T_COST, 0))),
            ..AuthConfig::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(AuthConfigError::PasswordHashUnparsable { .. })
        ));

        // At the floor, and above it, the store boots.
        for (m, t, p) in [
            (MIN_ARGON2_M_COST, MIN_ARGON2_T_COST, MIN_ARGON2_P_COST),
            (65_536, 3, 4),
        ] {
            let cfg = AuthConfig {
                basic: Some(basic(phc(m, t, p))),
                ..AuthConfig::default()
            };
            assert_eq!(cfg.validate(), Ok(()), "m={m},t={t},p={p} meets the floor");
        }
    }

    #[test]
    fn a_non_argon2id_password_hash_is_a_boot_error() {
        let cfg = AuthConfig {
            basic: Some(basic(
                "$argon2i$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2hoYQ"
                    .to_owned(),
            )),
            ..AuthConfig::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(AuthConfigError::PasswordHashNotArgon2id { .. })
        ));

        let cfg = AuthConfig {
            basic: Some(basic("not-a-phc-string".to_owned())),
            ..AuthConfig::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(AuthConfigError::PasswordHashUnparsable { .. })
        ));
    }

    /// RFC 9110 §11.6.1: a `401` challenge must name a scheme applicable to the
    /// target resource, so authentication with no mechanism cannot be honoured.
    #[test]
    fn auth_enabled_without_a_mechanism_is_a_boot_error() {
        let cfg = AuthConfig::default();
        assert!(cfg.enabled && !cfg.has_mechanism());
        assert_eq!(cfg.require_mechanism(), Err(AuthConfigError::NoMechanism));

        // Either mechanism satisfies it, and a disabled switch is exempt.
        let with_basic = AuthConfig {
            basic: Some(basic(phc(
                MIN_ARGON2_M_COST,
                MIN_ARGON2_T_COST,
                MIN_ARGON2_P_COST,
            ))),
            ..AuthConfig::default()
        };
        assert_eq!(with_basic.require_mechanism(), Ok(()));
        let disabled = AuthConfig {
            enabled: false,
            ..AuthConfig::default()
        };
        assert_eq!(disabled.require_mechanism(), Ok(()));
    }
}
