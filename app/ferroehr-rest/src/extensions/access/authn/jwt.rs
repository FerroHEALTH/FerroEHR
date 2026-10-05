// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OAuth2/OIDC bearer-token validation (resource-server role).
//!
//! The CDR validates access tokens presented as `Authorization: Bearer <jwt>`:
//! it verifies the signature against a key source and checks the `iss`/`aud`/
//! `exp` claims via `jsonwebtoken`. Three key sources are supported, in
//! precedence order: a symmetric HMAC secret, a static JWKS document, or a JWKS
//! discovered from the issuer's OIDC metadata (`openidconnect`) and cached.
//!
//! This is the SMART resource-server duty: validate a presented access token,
//! never issue one — obtaining tokens is an Authorization-Server concern
//! (`ITS-REST/docs/smart_app_launch/master06-authentication.adoc` §Supported
//! Authentication Flows).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 7): RFC 7519 leaves the claim set open; \
              decided-on claims lift into typed fields"
)]

use std::sync::Arc;
use std::time::Duration;

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};

use super::{AuthError, AuthMethod, Principal, TokenRejection};
use ferroehr::config::auth::OidcConfig;

/// A configured bearer-token validator.
pub(super) struct JwtValidator {
    issuer: String,
    audiences: Vec<String>,
    algorithms: Vec<Algorithm>,
    keys: KeySource,
    /// Dotted JWT claim paths mined for RBAC roles; configurable via
    /// `authz.rbac.role_claims`.
    role_claims: Vec<String>,
    /// Accepted clock skew on the time-based claims, in seconds
    /// (`auth.oidc.clock_skew_leeway_seconds`).
    leeway_seconds: u64,
    /// Require every token to claim the RFC 9068 access-token profile
    /// (`typ: at+jwt`); `auth.oidc.require_at_jwt`.
    require_at_jwt: bool,
}

/// The RFC 9068 §2.1 media type for a JWT access token, as it appears in the
/// `typ` header.
///
/// The RFC admits the `application/` prefix being omitted, and matching is
/// case-insensitive.
const AT_JWT_TYP: &str = "at+jwt";

// The role model lives in `access::authz` so the REST layer and the RBAC gate
// share one implementation.
use crate::extensions::access::authz::roles::extract_roles;

enum KeySource {
    /// Symmetric HS256 secret.
    Hmac(DecodingKey),
    /// A fixed JWKS document.
    Static(JwkSet),
    /// A JWKS discovered from the issuer and cached with a short TTL.
    Remote(RemoteJwks),
}

impl JwtValidator {
    /// Builds a validator with explicit RBAC role-claim paths, from
    /// `authz.rbac.role_claims`.
    ///
    /// # Errors
    /// Returns a message when the algorithm list or key material is invalid.
    pub(super) fn with_role_claims(
        cfg: &OidcConfig,
        role_claims: Vec<String>,
    ) -> Result<Self, String> {
        #[expect(
            clippy::map_err_ignore,
            reason = "`jsonwebtoken`'s algorithm parse error carries no detail beyond \
                      \"unrecognised name\", and the message already echoes the name"
        )]
        let algorithms = cfg
            .algorithms
            .iter()
            .map(|a| {
                a.parse::<Algorithm>()
                    .map_err(|_| format!("unknown JWT algorithm: {a}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if algorithms.is_empty() {
            return Err("oidc.algorithms must not be empty".to_owned());
        }

        let keys = if let Some(secret) = &cfg.hmac_secret {
            KeySource::Hmac(DecodingKey::from_secret(secret.expose().as_bytes()))
        } else if let Some(jwks_json) = &cfg.jwks_json {
            let set: JwkSet = serde_json::from_str(jwks_json)
                .map_err(|e| format!("invalid oidc.jwks_json: {e}"))?;
            KeySource::Static(set)
        } else {
            KeySource::Remote(RemoteJwks::new(cfg)?)
        };

        Ok(Self {
            issuer: cfg.issuer.clone(),
            audiences: cfg.audiences.clone(),
            algorithms,
            keys,
            role_claims,
            leeway_seconds: cfg.clock_skew_leeway_seconds,
            require_at_jwt: cfg.require_at_jwt,
        })
    }

    /// Validates a raw bearer token, the value after `Bearer `.
    ///
    /// # Errors
    /// [`AuthError::InvalidToken`] for any signature/claim/format failure,
    /// carrying the [`TokenRejection`] that names which check refused it.
    pub(super) async fn validate(&self, token: &str) -> Result<Principal, AuthError> {
        let header =
            decode_header(token).map_err(|e| AuthError::InvalidToken(TokenRejection::from(e)))?;
        if !self.algorithms.contains(&header.alg) {
            return Err(AuthError::InvalidToken(
                TokenRejection::AlgorithmNotAccepted(header.alg),
            ));
        }

        // RFC 9068 §4 step 1 / RFC 8725 §3.11: verify the media type BEFORE any
        // claim is read, so a token minted for another purpose (an ID token, a
        // refresh token) can never be spent here as an access token.
        let typ = header.typ.as_deref().map(str::trim);
        let claims_at_jwt_profile = match typ {
            Some(t) if t.eq_ignore_ascii_case(AT_JWT_TYP) => true,
            Some(t) if t.eq_ignore_ascii_case("application/at+jwt") => true,
            // RFC 7519 §5.1 makes the generic `JWT` type legal; RFC 8725 §3.11
            // only requires that a type, if present, is one this server accepts.
            Some(t) if t.eq_ignore_ascii_case("jwt") => false,
            Some(other) => {
                return Err(AuthError::InvalidToken(
                    TokenRejection::TokenTypeNotAccessToken(other.to_owned()),
                ));
            }
            None => false,
        };
        if self.require_at_jwt && !claims_at_jwt_profile {
            return Err(AuthError::InvalidToken(
                TokenRejection::AtJwtProfileRequired,
            ));
        }

        let mut validation = Validation::new(header.alg);
        validation.algorithms = self.algorithms.clone();
        validation.set_issuer(&[&self.issuer]);
        // `audiences` is boot-guaranteed non-empty whenever `[auth.oidc]` is
        // present, so there is no audience-less branch: a token minted for a
        // different resource server must never authenticate here
        // (RFC 7519 §4.1.3; RFC 9068 §4 step 4).
        validation.set_audience(&self.audiences);
        // RFC 7519 §4.1.5: `nbf` is a "MUST NOT be accepted before" claim, and
        // the crate does not check it unless asked.
        validation.validate_nbf = true;
        // RFC 9068 §2.2 requires `iss`, `exp`, `aud` and `sub` on an access
        // token; the crate's default requires only `exp`.
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = self.leeway_seconds;

        let key = self.decoding_key(header.kid.as_deref(), header.alg).await?;
        // The full claim map is retained for RBAC role extraction and ABAC
        // attribute resolution; `jsonwebtoken` validates `exp`/`iss`/`aud` from
        // the raw payload regardless of the deserialize target.
        let data = decode::<serde_json::Map<String, serde_json::Value>>(token, &key, &validation)
            .map_err(|e| AuthError::InvalidToken(TokenRejection::from(e)))?;
        let claims = data.claims;

        // RFC 7519 §4.1.2 makes `sub` optional, but this principal is stamped
        // into the ATNA audit trail — an unattributable caller is refused, never
        // recorded under a fabricated identity.
        let subject = claims
            .get("sub")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(AuthError::InvalidToken(TokenRejection::SubjectMissing))?
            .to_owned();

        // RFC 9068 §4 step 1: a token that claims the profile is held to the
        // whole of §2.2. One that does not is validated under the general JWT
        // rules only, because the RFC prescribes nothing for it.
        if claims_at_jwt_profile {
            for required in ["iat", "jti", "client_id"] {
                if !claims.contains_key(required) {
                    return Err(AuthError::InvalidToken(
                        TokenRejection::AtJwtProfileClaimMissing(required.to_owned()),
                    ));
                }
            }
        }

        // Scopes: the space-delimited `scope` string plus the `scp` array,
        // preserved verbatim on the principal for ABAC/diagnostic consumers.
        let mut scopes: Vec<String> = claims
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .map(|s| s.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default();
        if let Some(scp) = claims.get("scp").and_then(serde_json::Value::as_array) {
            scopes.extend(scp.iter().filter_map(|v| v.as_str().map(str::to_owned)));
        }

        // Roles from the configured claim paths, normalized to upper-case. The
        // defaults are the RFC 9068 §2.2.3.1 carriers; `scope` is not among them
        // (an OAuth2 scope is a client grant, RFC 6749 §3.3, not a role).
        let roles = extract_roles(&claims, &self.role_claims);

        Ok(Principal {
            subject,
            scopes,
            roles,
            claims,
            method: AuthMethod::Bearer,
        })
    }

    async fn decoding_key(
        &self,
        kid: Option<&str>,
        alg: Algorithm,
    ) -> Result<DecodingKey, AuthError> {
        match &self.keys {
            KeySource::Hmac(key) => Ok(key.clone()),
            KeySource::Static(set) => jwk_to_key(set, kid, alg),
            KeySource::Remote(remote) => {
                let set = remote.jwks().await?;
                jwk_to_key(&set, kid, alg)
            }
        }
    }
}

/// Selects the signing key for a token, honouring the JWK usage facets.
///
/// A `kid` names the key outright (RFC 7515 §4.1.4). Without one the candidates
/// are narrowed by `use` (RFC 7517 §4.2), `key_ops` (§4.3) and `alg` (§4.4), and
/// an ambiguous remainder is refused rather than resolved by position: taking
/// the first key would let a rotation, or an encryption key sharing the
/// document, decide which key verifies a clinical request (RFC 8725 §3.1).
///
/// # Errors
/// [`AuthError::InvalidToken`] when no key matches, when more than one remains
/// after narrowing, or when the selected JWK cannot be turned into a key.
fn jwk_to_key(set: &JwkSet, kid: Option<&str>, alg: Algorithm) -> Result<DecodingKey, AuthError> {
    let jwk = if let Some(kid) = kid {
        set.find(kid)
            .ok_or_else(|| AuthError::InvalidToken(TokenRejection::UnknownKeyId(kid.to_owned())))?
    } else {
        let candidates: Vec<_> = set
            .keys
            .iter()
            .filter(|jwk| {
                let common = &jwk.common;
                let usable_for_signing = common
                    .public_key_use
                    .as_ref()
                    .is_none_or(|u| matches!(u, jsonwebtoken::jwk::PublicKeyUse::Signature));
                let permits_verify = common.key_operations.as_ref().is_none_or(|ops| {
                    ops.iter()
                        .any(|op| matches!(op, jsonwebtoken::jwk::KeyOperations::Verify))
                });
                let matches_alg = common.key_algorithm.as_ref().is_none_or(|declared| {
                    declared
                        .to_string()
                        .eq_ignore_ascii_case(&format!("{alg:?}"))
                });
                usable_for_signing && permits_verify && matches_alg
            })
            .collect();
        match candidates.as_slice() {
            [single] => *single,
            [] => {
                return Err(AuthError::InvalidToken(TokenRejection::NoUsableKey));
            }
            many => {
                return Err(AuthError::InvalidToken(TokenRejection::AmbiguousKey(
                    many.len(),
                )));
            }
        }
    };
    DecodingKey::from_jwk(jwk).map_err(|e| AuthError::InvalidToken(TokenRejection::UnusableJwk(e)))
}

/// How long successfully fetched key material stays usable without a refetch.
const JWKS_TTL: Duration = Duration::from_mins(5);

/// The single cache slot: one issuer per validator, so the key is a constant.
const CACHE_KEY: &str = "jwks";

/// A JWKS fetched from an issuer's OIDC discovery document, cached briefly.
///
/// Both outcomes are cached with different lifetimes ([`JwksExpiry`]): success
/// for [`JWKS_TTL`], failure for `oidc.negative_cache_ttl_seconds`, which keeps
/// an issuer outage from turning every bearer request into a fresh discovery
/// attempt. No openEHR spec governs this — our own design.
struct RemoteJwks {
    issuer: String,
    /// Built once, so the timeouts are a configuration-time property and the
    /// connection pool survives across fetches.
    http: openidconnect::reqwest::Client,
    cache: moka::future::Cache<&'static str, Result<Arc<JwkSet>, AuthError>>,
}

/// Per-entry lifetimes for the [`RemoteJwks`] cache.
struct JwksExpiry {
    negative_ttl: Duration,
}

impl moka::Expiry<&'static str, Result<Arc<JwkSet>, AuthError>> for JwksExpiry {
    fn expire_after_create(
        &self,
        _key: &&'static str,
        value: &Result<Arc<JwkSet>, AuthError>,
        _created_at: std::time::Instant,
    ) -> Option<Duration> {
        match value {
            Ok(_) => Some(JWKS_TTL),
            // A zero negative TTL expires the entry on the next read, which is
            // the documented "negative caching off" setting.
            Err(_) => Some(self.negative_ttl),
        }
    }
}

impl RemoteJwks {
    /// Builds the discovery client and its cache from the `[auth.oidc]` section.
    ///
    /// # Errors
    /// Returns a message when `reqwest` cannot build a client with the
    /// configured timeouts.
    fn new(cfg: &OidcConfig) -> Result<Self, String> {
        let http = openidconnect::reqwest::ClientBuilder::new()
            // SSRF hardening: an issuer must not redirect us anywhere.
            .redirect(openidconnect::reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(cfg.connect_timeout_ms))
            .timeout(Duration::from_millis(cfg.request_timeout_ms))
            .build()
            .map_err(|e| format!("oidc discovery HTTP client: {e}"))?;
        let expiry = JwksExpiry {
            negative_ttl: Duration::from_secs(cfg.negative_cache_ttl_seconds),
        };
        Ok(Self {
            issuer: cfg.issuer.clone(),
            http,
            cache: moka::future::Cache::builder()
                .max_capacity(1)
                .expire_after(expiry)
                .build(),
        })
    }

    async fn jwks(&self) -> Result<Arc<JwkSet>, AuthError> {
        // `get_with` coalesces concurrent misses onto one fetch AND stores the
        // outcome whichever way it went; `try_get_with` would discard the error.
        self.cache.get_with(CACHE_KEY, self.fetch()).await
    }

    async fn fetch(&self) -> Result<Arc<JwkSet>, AuthError> {
        let issuer_url = openidconnect::IssuerUrl::new(self.issuer.clone())
            .map_err(|e| AuthError::KeyResolution(format!("issuer URL: {e}")))?;
        let metadata =
            openidconnect::core::CoreProviderMetadata::discover_async(issuer_url, &self.http)
                .await
                .map_err(|e| AuthError::KeyResolution(format!("OIDC discovery: {e}")))?;
        let jwks_uri = metadata.jwks_uri().url().clone();
        let body = self
            .http
            .get(jwks_uri)
            .send()
            .await
            .map_err(|e| AuthError::KeyResolution(format!("JWKS fetch: {e}")))?
            .text()
            .await
            .map_err(|e| AuthError::KeyResolution(format!("JWKS body: {e}")))?;
        let set: JwkSet = serde_json::from_str(&body)
            .map_err(|e| AuthError::KeyResolution(format!("JWKS parse: {e}")))?;
        Ok(Arc::new(set))
    }
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{EncodingKey, Header, encode};

    use super::*;
    use serde_json::{Value, json};

    const SECRET: &str = "test-signing-secret";
    const ISSUER: &str = "https://issuer.example";
    /// The audience every fixture token is minted for. `audiences` is mandatory
    /// whenever `[auth.oidc]` is present (a token for another resource server
    /// must never authenticate here), so there is no audience-less fixture.
    const AUDIENCE: &str = "ferroehr";

    fn now() -> u64 {
        // Test-only wall clock; jiff is a workspace dep.
        u64::try_from(jiff::Timestamp::now().as_second()).unwrap()
    }

    fn validator(audiences: &[&str]) -> JwtValidator {
        let audiences: Vec<String> = if audiences.is_empty() {
            vec![AUDIENCE.to_owned()]
        } else {
            audiences.iter().map(|s| (*s).to_owned()).collect()
        };
        JwtValidator::with_role_claims(
            &OidcConfig {
                issuer: ISSUER.to_owned(),
                audiences,
                algorithms: vec!["HS256".to_owned()],
                hmac_secret: Some(ferroehr::config::secret::Secret::new(SECRET.to_owned())),
                jwks_json: None,
                ..OidcConfig::default()
            },
            ferroehr::config::authz::RbacConfig::default().role_claims,
        )
        .expect("validator")
    }

    fn token(claims: &Value) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .expect("encode")
    }

    fn base_claims() -> Value {
        json!({
            "sub": "alice",
            "iss": ISSUER,
            "aud": AUDIENCE,
            "exp": now() + 3600,
            "scope": "openid profile",
        })
    }

    #[tokio::test]
    async fn valid_token_authenticates() {
        let p = validator(&[])
            .validate(&token(&base_claims()))
            .await
            .expect("ok");
        assert_eq!(p.subject, "alice");
        assert_eq!(p.method, AuthMethod::Bearer);
        assert!(p.scopes.contains(&"openid".to_owned()));
        // The validated claim set is retained on the Principal.
        assert_eq!(p.claims.get("sub").and_then(Value::as_str), Some("alice"));
    }

    #[tokio::test]
    async fn token_without_sub_rejected() {
        let mut c = base_claims();
        c.as_object_mut().expect("object").remove("sub");
        let err = validator(&[])
            .validate(&token(&c))
            .await
            .expect_err("reject");
        assert!(
            matches!(&err, AuthError::InvalidToken(TokenRejection::ClaimSet(_))),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn token_with_blank_sub_rejected() {
        let mut c = base_claims();
        c["sub"] = json!("   ");
        let err = validator(&[])
            .validate(&token(&c))
            .await
            .expect_err("reject");
        assert!(
            matches!(err, AuthError::InvalidToken(TokenRejection::SubjectMissing)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn expired_token_rejected() {
        let mut c = base_claims();
        c["exp"] = json!(now() - 3600); // beyond jsonwebtoken's default 60s leeway
        let err = validator(&[])
            .validate(&token(&c))
            .await
            .expect_err("reject");
        assert!(
            matches!(err, AuthError::InvalidToken(TokenRejection::Expired(_))),
            "got {err:?}"
        );
    }

    /// RFC 0201 makes the cause chain part of the `Error` contract. It is
    /// asserted by DOWNCASTING to the concrete types — `source().is_some()`
    /// stays true even when a hop carries the wrong one.
    #[tokio::test]
    async fn a_token_rejection_downcasts_to_its_jsonwebtoken_cause() {
        let mut c = base_claims();
        c["exp"] = json!(now() - 3600);
        let err = validator(&[])
            .validate(&token(&c))
            .await
            .expect_err("reject");

        let rejection = std::error::Error::source(&err)
            .expect("AuthError carries the typed rejection")
            .downcast_ref::<TokenRejection>()
            .expect("the first hop is the TokenRejection");
        assert_eq!(rejection.label(), "expired");

        let cause = std::error::Error::source(rejection)
            .expect("the rejection carries its jsonwebtoken error")
            .downcast_ref::<jsonwebtoken::errors::Error>()
            .expect("the second hop is the concrete jsonwebtoken error");
        assert!(
            matches!(
                cause.kind(),
                jsonwebtoken::errors::ErrorKind::ExpiredSignature
            ),
            "got {cause:?}"
        );
    }

    /// The defect this classification exists to remove: expiry and a bad
    /// signature used to be one opaque string, so an operator could not tell a
    /// clock problem from someone presenting tokens this server never minted.
    #[tokio::test]
    async fn expired_and_tampered_tokens_report_different_reasons() {
        let mut c = base_claims();
        c["exp"] = json!(now() - 3600);
        let expired = validator(&[])
            .validate(&token(&c))
            .await
            .expect_err("reject");

        let mut t = token(&base_claims());
        let flip = t.len() - 10;
        let tampered_char = if t.as_bytes()[flip] == b'A' { 'B' } else { 'A' };
        t.replace_range(flip..=flip, &tampered_char.to_string());
        let tampered = validator(&[]).validate(&t).await.expect_err("reject");

        assert_eq!(expired.label(), "expired");
        assert_eq!(tampered.label(), "signature");
    }

    #[tokio::test]
    async fn wrong_issuer_rejected() {
        let mut c = base_claims();
        c["iss"] = json!("https://evil.example");
        let err = validator(&[])
            .validate(&token(&c))
            .await
            .expect_err("reject");
        assert!(
            matches!(err, AuthError::InvalidToken(TokenRejection::Issuer(_))),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn wrong_audience_rejected() {
        let mut c = base_claims();
        c["aud"] = json!("other-api");
        let err = validator(&["my-api"])
            .validate(&token(&c))
            .await
            .expect_err("reject");
        assert!(
            matches!(err, AuthError::InvalidToken(TokenRejection::Audience(_))),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn correct_audience_accepted() {
        let mut c = base_claims();
        c["aud"] = json!("my-api");
        let p = validator(&["my-api"])
            .validate(&token(&c))
            .await
            .expect("ok");
        assert_eq!(p.subject, "alice");
    }

    #[tokio::test]
    async fn tampered_signature_rejected() {
        // Tamper with a char in the *middle* of the base64url signature, not
        // the last one: the final char of a 43-char HS256 signature encodes
        // only 4 meaningful bits, so an 'A'↔'B' flip there lands in the
        // ignored trailing bits ~1/16 of the time (decodes to the identical
        // signature → flaky false-accept).
        let mut t = token(&base_claims());
        let flip = t.len() - 10;
        let tampered = if t.as_bytes()[flip] == b'A' { 'B' } else { 'A' };
        t.replace_range(flip..=flip, &tampered.to_string());
        let err = validator(&[]).validate(&t).await.expect_err("reject");
        assert!(
            matches!(err, AuthError::InvalidToken(TokenRejection::Signature(_))),
            "got {err:?}"
        );
    }

    /// RFC 7519 §4.1.5: `nbf` is a "MUST NOT be accepted before" claim, and the
    /// crate does not check it unless asked.
    #[tokio::test]
    async fn not_yet_valid_token_rejected() {
        let mut c = base_claims();
        c["nbf"] = json!(now() + 3_600);
        let err = validator(&[])
            .validate(&token(&c))
            .await
            .expect_err("a token not yet valid must be refused");
        assert!(
            matches!(err, AuthError::InvalidToken(TokenRejection::NotYetValid(_))),
            "got {err:?}"
        );
    }

    /// RFC 9068 §2.2 requires `iss` on an access token; the crate's default
    /// required-claim set is `exp` alone, so an issuer-less token would have
    /// authenticated without this.
    #[tokio::test]
    async fn token_without_iss_rejected() {
        let mut c = base_claims();
        c.as_object_mut().expect("object").remove("iss");
        let err = validator(&[])
            .validate(&token(&c))
            .await
            .expect_err("a token without `iss` must be refused");
        assert!(
            matches!(err, AuthError::InvalidToken(TokenRejection::ClaimSet(_))),
            "got {err:?}"
        );
    }

    /// RFC 7519 §4.1.3 / RFC 9068 §4 step 4: a token minted for a different
    /// resource server must never authenticate here.
    #[tokio::test]
    async fn token_for_another_audience_rejected() {
        let mut c = base_claims();
        c["aud"] = json!("some-other-resource-server");
        let err = validator(&[])
            .validate(&token(&c))
            .await
            .expect_err("a token for another audience must be refused");
        assert!(
            matches!(err, AuthError::InvalidToken(TokenRejection::Audience(_))),
            "got {err:?}"
        );
    }

    /// RFC 9068 §2.1 + §4 step 1: a token CLAIMING the `at+jwt` profile is held
    /// to the whole of §2.2, so `iat`/`jti`/`client_id` become mandatory for it.
    #[tokio::test]
    async fn typ_at_jwt_enforces_the_profile_claims() {
        let mut header = Header::new(Algorithm::HS256);
        header.typ = Some("at+jwt".to_owned());
        let mint = |claims: &Value| {
            encode(
                &header,
                claims,
                &EncodingKey::from_secret(SECRET.as_bytes()),
            )
            .expect("encode")
        };

        // Profile claimed but incomplete → refused.
        let err = validator(&[])
            .validate(&mint(&base_claims()))
            .await
            .expect_err("an at+jwt token without iat/jti/client_id must be refused");
        assert!(
            matches!(
                err,
                AuthError::InvalidToken(TokenRejection::AtJwtProfileClaimMissing(_))
            ),
            "got {err:?}"
        );

        // Profile claimed and complete → accepted.
        let mut complete = base_claims();
        complete["iat"] = json!(now());
        complete["jti"] = json!("token-id-1");
        complete["client_id"] = json!("some-client");
        let p = validator(&[])
            .validate(&mint(&complete))
            .await
            .expect("a complete at+jwt access token authenticates");
        assert_eq!(p.subject, "alice");

        // A token that does NOT claim the profile is validated under the general
        // JWT rules — the RFC prescribes nothing more for it.
        assert!(
            validator(&[])
                .validate(&token(&base_claims()))
                .await
                .is_ok(),
            "an untyped token must still authenticate by default",
        );
    }

    /// RFC 8725 §3.12: an ID token is not an access token. Refusing an
    /// unexpected `typ` outright stops one being spent here.
    #[tokio::test]
    async fn id_token_typ_rejected() {
        let mut header = Header::new(Algorithm::HS256);
        header.typ = Some("JWT+ID".to_owned());
        let raw = encode(
            &header,
            &base_claims(),
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .expect("encode");
        let err = validator(&[])
            .validate(&raw)
            .await
            .expect_err("a non-access-token type must be refused");
        assert!(
            matches!(
                err,
                AuthError::InvalidToken(TokenRejection::TokenTypeNotAccessToken(_))
            ),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn keycloak_realm_access_roles_extracted() {
        // Roles live under `realm_access.roles` and surface upper-cased. The
        // `scope` claim is NOT a role source: an OAuth2 scope grants a client
        // delegated authority (RFC 6749 §3.3) and asserts nothing about the
        // subject's roles — mining it made the at-least-one-role gate vacuous
        // for every OIDC token, since `openid` alone satisfied it.
        let mut c = base_claims();
        c["realm_access"] = json!({ "roles": ["user", "ferroehr-admin"] });
        c["scope"] = json!("openid EHR_READ");
        let p = validator(&[]).validate(&token(&c)).await.expect("ok");
        assert!(p.roles.contains(&"USER".to_owned()));
        assert!(p.roles.contains(&"FERROEHR-ADMIN".to_owned()));
        assert!(
            !p.roles.contains(&"OPENID".to_owned()),
            "`openid` is a scope, not a role",
        );
        assert!(
            !p.roles.contains(&"EHR_READ".to_owned()),
            "a scope must not become a role",
        );
        // Scopes stay on the principal verbatim for SMART enforcement.
        assert!(p.scopes.contains(&"openid".to_owned()));
    }

    #[test]
    fn extract_roles_upper_cases_and_dedups() {
        let claims = json!({
            "realm_access": { "roles": ["admin", "Admin", "user"] },
            "scope": "user OFFLINE_ACCESS",
        })
        .as_object()
        .cloned()
        .unwrap();
        let roles = extract_roles(
            &claims,
            &ferroehr::config::authz::RbacConfig::default().role_claims,
        );
        // `admin`/`Admin` collapse; `user` stays once. `OFFLINE_ACCESS` is a
        // SCOPE and never becomes a role (RFC 6749 §3.3), so it is absent even
        // though the token carries it.
        assert_eq!(roles, vec!["ADMIN".to_owned(), "USER".to_owned()]);
    }

    #[test]
    fn extract_roles_missing_paths_yields_empty() {
        let claims = json!({ "sub": "x" }).as_object().cloned().unwrap();
        assert!(
            extract_roles(
                &claims,
                &ferroehr::config::authz::RbacConfig::default().role_claims
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_roles_custom_nested_path() {
        let claims = json!({ "resource_access": { "client": { "roles": ["writer"] } } })
            .as_object()
            .cloned()
            .unwrap();
        let roles = extract_roles(&claims, &["resource_access.client.roles".to_owned()]);
        assert_eq!(roles, vec!["WRITER".to_owned()]);
    }

    /// The remote (OIDC-discovery) key source: timeout budget + negative
    /// caching. No openEHR spec governs auth transport hardening — our own
    /// design.
    mod remote_discovery {
        use std::time::Instant;

        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;

        const DISCOVERY_PATH: &str = "/.well-known/openid-configuration";
        const JWKS_PATH: &str = "/jwks";

        /// A discovery-key-source config: no HMAC secret, no static JWKS.
        fn oidc(
            issuer: &str,
            negative_cache_ttl_seconds: u64,
            request_timeout_ms: u64,
        ) -> OidcConfig {
            OidcConfig {
                issuer: issuer.to_owned(),
                // Mandatory whenever `[auth.oidc]` is present, so the discovery
                // fixtures declare it like any real deployment.
                audiences: vec![AUDIENCE.to_owned()],
                algorithms: vec!["HS256".to_owned()],
                request_timeout_ms,
                negative_cache_ttl_seconds,
                ..OidcConfig::default()
            }
        }

        /// The minimum OIDC provider metadata `CoreProviderMetadata` accepts;
        /// `issuer` must echo the requested issuer exactly.
        fn metadata(issuer: &str) -> Value {
            json!({
                "issuer": issuer,
                "authorization_endpoint": format!("{issuer}/authorize"),
                "token_endpoint": format!("{issuer}/token"),
                "jwks_uri": format!("{issuer}{JWKS_PATH}"),
                "response_types_supported": ["code"],
                "subject_types_supported": ["public"],
                "id_token_signing_alg_values_supported": ["HS256"],
            })
        }

        /// A JWKS carrying the same HS256 secret [`token`] signs with, so a
        /// recovered issuer can actually validate a token.
        fn jwks_body() -> Value {
            let jwk = jsonwebtoken::jwk::Jwk::from_encoding_key(
                &EncodingKey::from_secret(SECRET.as_bytes()),
                Algorithm::HS256,
            )
            .expect("jwk from secret");
            json!({ "keys": [jwk] })
        }

        async fn mount_healthy(server: &MockServer) {
            Mock::given(method("GET"))
                .and(path(DISCOVERY_PATH))
                .respond_with(ResponseTemplate::new(200).set_body_json(metadata(&server.uri())))
                .mount(server)
                .await;
            Mock::given(method("GET"))
                .and(path(JWKS_PATH))
                .respond_with(ResponseTemplate::new(200).set_body_json(jwks_body()))
                .mount(server)
                .await;
        }

        #[tokio::test]
        async fn stalled_issuer_fails_within_the_configured_request_timeout() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(DISCOVERY_PATH))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(metadata(&server.uri()))
                        .set_delay(Duration::from_secs(30)),
                )
                .mount(&server)
                .await;

            let remote = RemoteJwks::new(&oidc(&server.uri(), 10, 300)).expect("client");
            let started = Instant::now();
            let err = remote.jwks().await.expect_err("the timeout must fire");
            let elapsed = started.elapsed();

            assert!(matches!(err, AuthError::KeyResolution(_)), "got {err:?}");
            // The 300 ms budget, generously slacked; without it the request
            // would park for the mock's 30 s (or, on a blackhole, the OS TCP
            // timeout — 75 s+).
            assert!(
                elapsed < Duration::from_secs(5),
                "discovery took {elapsed:?}: the request timeout did not fire"
            );
        }

        #[tokio::test]
        async fn failed_fetch_is_cached_so_no_second_request_reaches_the_issuer() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(DISCOVERY_PATH))
                .respond_with(ResponseTemplate::new(500))
                .expect(1)
                .mount(&server)
                .await;

            let remote = RemoteJwks::new(&oidc(&server.uri(), 60, 5_000)).expect("client");
            assert!(remote.jwks().await.is_err(), "issuer is down");
            assert!(
                remote.jwks().await.is_err(),
                "still down, answered from cache"
            );

            // The mock's `.expect(1)` is verified when the server drops; the
            // explicit count names the failure if a second attempt goes out.
            let attempts = server.received_requests().await.expect("recorded requests");
            assert_eq!(
                attempts.len(),
                1,
                "the negative cache did not suppress the second discovery attempt"
            );
        }

        #[tokio::test]
        async fn negative_entry_expires_and_a_recovered_issuer_validates() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(DISCOVERY_PATH))
                .respond_with(ResponseTemplate::new(503))
                .mount(&server)
                .await;

            let cfg = oidc(&server.uri(), 1, 5_000);
            let validator = JwtValidator::with_role_claims(
                &cfg,
                ferroehr::config::authz::RbacConfig::default().role_claims,
            )
            .expect("validator");
            let mut claims = base_claims();
            claims["iss"] = json!(server.uri());
            let token = token(&claims);

            let err = validator
                .validate(&token)
                .await
                .expect_err("issuer is down");
            assert!(matches!(err, AuthError::KeyResolution(_)), "got {err:?}");

            server.reset().await;
            mount_healthy(&server).await;

            // Still inside the 1 s negative TTL: refused from cache, no wire I/O.
            assert!(validator.validate(&token).await.is_err());
            let during_ttl = server.received_requests().await.expect("recorded requests");
            assert!(
                during_ttl.is_empty(),
                "a request escaped during the negative TTL: {during_ttl:?}"
            );

            tokio::time::sleep(Duration::from_millis(1_200)).await;
            let principal = validator
                .validate(&token)
                .await
                .expect("the recovered issuer must validate");
            assert_eq!(principal.subject, "alice");
        }
    }
}
