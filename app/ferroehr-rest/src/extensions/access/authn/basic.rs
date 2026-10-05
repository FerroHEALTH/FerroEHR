// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP Basic authentication against an Argon2 PHC password store.
//!
//! Basic is one of the two authentication mechanisms; the CNF security suites
//! exercise the Basic flow's 401/403 behaviour
//! (`docs/specs/openehr/CNF/tests/platform/robot/SECURITY_TESTS/`), which the
//! authn middleware ([`super`]) enforces per ITS-REST §Authentication and
//! authorization. Password verification uses `argon2` (never a hand-rolled
//! comparison); only the PHC hash is ever stored.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 7): RFC 7519 leaves the claim set open; \
              decided-on claims lift into typed fields"
)]

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use base64::Engine as _;
use base64::alphabet;
use base64::engine::general_purpose::GeneralPurposeConfig;
use base64::engine::{DecodePaddingMode, GeneralPurpose};
use http::HeaderValue;

use super::{AuthError, AuthMethod, Principal};
use ferroehr::config::auth::BasicConfig;

/// RFC 4648 standard-alphabet decoder for the RFC 7617 `Basic` credential,
/// **padded only**: RFC 7617 §2 defers to RFC 4648 §4, and RFC 4648 §3.2
/// requires the pad characters "unless the specification referring to this
/// document explicitly states otherwise" — RFC 7617 does not. Non-alphabet
/// bytes, interior padding, and non-canonical trailing bits are rejected too.
static BASIC_B64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

/// The PHC verified when no configured user matches the presented name.
///
/// Without it the unknown-user path returns before the KDF runs, so its
/// response time distinguishes "no such user" from "wrong password" — an
/// account-enumeration oracle. It is DERIVED rather than a hardcoded digest: a
/// literal that failed to parse would make the defence silently free, and the
/// parameters must track the floor the configured hashes are validated against
/// (<https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html>
/// §Argon2id) so the work performed matches a real verification.
static ENUMERATION_DEFENCE_PHC: std::sync::LazyLock<Option<String>> =
    std::sync::LazyLock::new(|| {
        let params = argon2::Params::new(19_456, 2, 1, None).ok()?;
        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        // A fixed salt, because this PHC is a timing decoy rather than a
        // credential: nothing verifies against it, so a per-boot random salt
        // would only make the constant unreproducible.
        argon2::password_hash::PasswordHasher::hash_password_with_salt(
            &argon2,
            b"no-such-user-placeholder",
            b"ferroehr-enum-defence",
        )
        .ok()
        .map(|hash| hash.to_string())
    });

/// Verify a `Basic <base64>` credential against the configured user store.
///
/// # Errors
/// [`AuthError::InvalidCredentials`] on a malformed header, unknown user, or a
/// password that does not match the stored Argon2 hash.
#[expect(
    clippy::map_err_ignore,
    reason = "every failure on the credential path collapses to one opaque outcome \
              deliberately: a caller must not learn from the 401 whether the header \
              was malformed, the user unknown, or the password wrong"
)]
pub(super) fn verify(header: &HeaderValue, cfg: &BasicConfig) -> Result<Principal, AuthError> {
    let raw = header.to_str().map_err(|_| AuthError::InvalidCredentials)?;
    let b64 = raw
        .strip_prefix("Basic ")
        .or_else(|| raw.strip_prefix("basic "))
        .ok_or(AuthError::InvalidCredentials)?;
    let decoded = BASIC_B64
        .decode(b64.trim())
        .map_err(|_| AuthError::InvalidCredentials)?;
    let decoded = String::from_utf8(decoded).map_err(|_| AuthError::InvalidCredentials)?;
    let (username, password) = decoded
        .split_once(':')
        .ok_or(AuthError::InvalidCredentials)?;

    // An unknown user still pays the KDF, against a fixed hash, so the response
    // time carries no account-existence signal.
    let Some(user) = cfg.users.iter().find(|u| u.username == username) else {
        if let Some(phc) = ENUMERATION_DEFENCE_PHC.as_deref()
            && let Ok(decoy) = PasswordHash::new(phc)
        {
            // The outcome is discarded deliberately — only the work matters.
            let _outcome = Argon2::default().verify_password(password.as_bytes(), &decoy);
        }
        return Err(AuthError::InvalidCredentials);
    };

    let parsed = PasswordHash::new(user.password_hash.expose())
        .map_err(|_| AuthError::InvalidCredentials)?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| AuthError::InvalidCredentials)?;

    // Basic-user roles are configured (default `["USER"]`); normalize to
    // upper-case so they match the RBAC role model regardless of config casing.
    let roles = user
        .roles
        .iter()
        .map(|r| r.trim().to_ascii_uppercase())
        .filter(|r| !r.is_empty())
        .collect();

    Ok(Principal {
        subject: username.to_owned(),
        scopes: Vec::new(),
        roles,
        // Basic auth carries no JWT claims (ABAC under Basic is rejected).
        claims: serde_json::Map::new(),
        method: AuthMethod::Basic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::PasswordHasher;
    use ferroehr::config::auth::BasicUser;

    fn hash(pw: &str) -> String {
        // Fixed salt keeps the test hermetic without the argon2 `rand` feature.
        Argon2::default()
            .hash_password_with_salt(pw.as_bytes(), b"1234567890123456")
            .expect("hash")
            .to_string()
    }

    fn store() -> BasicConfig {
        BasicConfig {
            users: vec![BasicUser {
                username: "alice".to_owned(),
                password_hash: ferroehr::config::secret::Secret::new(hash("s3cret")),
                password_hash_file: None,
                roles: vec!["user".to_owned()],
            }],
        }
    }

    fn header(user: &str, pw: &str) -> HeaderValue {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pw}"));
        HeaderValue::from_str(&format!("Basic {token}")).unwrap()
    }

    fn raw_header(token: &str) -> HeaderValue {
        HeaderValue::from_str(&format!("Basic {token}")).unwrap()
    }

    #[test]
    fn valid_credentials_authenticate() {
        let p = verify(&header("alice", "s3cret"), &store()).expect("ok");
        assert_eq!(p.subject, "alice");
        assert_eq!(p.method, AuthMethod::Basic);
    }

    #[test]
    fn configured_roles_are_upper_cased() {
        // The configured lower-case `user` role surfaces normalized on the
        // Principal (Basic role config, upper-casing).
        let p = verify(&header("alice", "s3cret"), &store()).expect("ok");
        assert_eq!(p.roles, vec!["USER".to_owned()]);
        assert!(p.claims.is_empty(), "Basic carries no JWT claims");
    }

    #[test]
    fn admin_user_role_config() {
        let cfg = BasicConfig {
            users: vec![BasicUser {
                username: "root".to_owned(),
                password_hash: ferroehr::config::secret::Secret::new(hash("s3cret")),
                password_hash_file: None,
                roles: vec!["ADMIN".to_owned()],
            }],
        };
        let p = verify(&header("root", "s3cret"), &cfg).expect("ok");
        assert_eq!(p.roles, vec!["ADMIN".to_owned()]);
    }

    #[test]
    fn wrong_password_rejected() {
        let err = verify(&header("alice", "nope"), &store()).expect_err("reject");
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[test]
    fn unknown_user_rejected() {
        let err = verify(&header("bob", "s3cret"), &store()).expect_err("reject");
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    /// An unknown user must pay the KDF, or the response time says whether the
    /// account exists. Asserted on WORK, not on a wall-clock threshold: a
    /// timing assertion is flaky under load, while the derived decoy hash is
    /// either parseable-and-verified or the defence is silently free.
    #[test]
    fn unknown_user_pays_the_kdf() {
        let phc = ENUMERATION_DEFENCE_PHC
            .as_deref()
            .expect("the decoy PHC should be derivable at the OWASP floor");
        let parsed = PasswordHash::new(phc).expect("the decoy PHC should parse");
        assert_eq!(
            parsed.algorithm.as_str(),
            "argon2id",
            "the decoy must use the same algorithm as a real verification",
        );
        // The floor the configured hashes are boot-validated against
        // (OWASP Password Storage §Argon2id), so the work matches.
        let params = argon2::Params::try_from(&parsed).expect("params");
        assert_eq!(params.m_cost(), 19_456);
        assert_eq!(params.t_cost(), 2);
        assert_eq!(params.p_cost(), 1);
        // And the placeholder password must not verify against it, so the decoy
        // can never authenticate anyone.
        assert!(
            verify(
                &header("no-such-user", "no-such-user-placeholder"),
                &store()
            )
            .is_err(),
            "the decoy hash must never authenticate",
        );
    }

    /// The canonical padded form IS the credential: RFC 7617 §2 defers to
    /// RFC 4648 §4, and RFC 4648 §3.2 requires the pad characters "unless the
    /// specification referring to this document explicitly states otherwise" —
    /// RFC 7617 states no such thing.
    ///
    /// The fixture must be a credential whose encoding actually pads:
    /// `alice:s3cret` is 12 bytes, so it encodes to 16 characters with no
    /// padding at all, and stripping `=` from it changes nothing.
    #[test]
    fn unpadded_credential_refused() {
        let padded = base64::engine::general_purpose::STANDARD.encode("alice:s3cre");
        assert!(
            padded.ends_with('='),
            "fixture must exercise padding, else the assertion is vacuous",
        );
        let err = verify(&raw_header(padded.trim_end_matches('=')), &store())
            .expect_err("the unpadded form is not an RFC 7617 credential");
        assert!(matches!(err, AuthError::InvalidCredentials));
        // The padded form of a WRONG password still reaches the KDF and fails
        // there, so the refusal above is about the encoding, not the password.
        let wrong = verify(&raw_header(&padded), &store()).expect_err("wrong password");
        assert!(matches!(wrong, AuthError::InvalidCredentials));
    }

    #[test]
    fn non_alphabet_byte_rejected() {
        let err = verify(&raw_header("YWxpY2U6*zNjcmV0"), &store()).expect_err("reject");
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[test]
    fn excess_padding_rejected() {
        // Deliberate strictness increase over the retired hand-rolled decoder,
        // which trimmed ANY number of trailing '='. Canonical encoders never
        // emit this, and garbage credentials were a 401 anyway — the wire
        // outcome is unchanged.
        let err = verify(&raw_header("QQ==="), &store()).expect_err("reject");
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[test]
    fn interior_padding_rejected() {
        let err = verify(&raw_header("YWx=pY2U6czNjcmV0"), &store()).expect_err("reject");
        assert!(matches!(err, AuthError::InvalidCredentials));
    }
}
