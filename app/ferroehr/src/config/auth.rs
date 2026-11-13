// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authentication configuration: the `[auth]` section and the rules FerroEHR
//! adds to it.
//!
//! `[auth]` is the `ferrohealth-smart` crate's [`AuthSection`], the section
//! every FerroHEALTH product embeds: the trusted issuers and their key sets,
//! the accepted algorithms (asymmetric only, RFC 8725 §3.1 and §3.2), the
//! audiences (RFC 9068 §4), the Basic users of `[[auth.basic.users]]` with
//! the scopes each holds, and the product scope. The crate checks tokens and
//! never issues them, and it refuses at resolution a Basic hash below the
//! Argon2id floor, an issuer with `userinfo` and private or symmetric key
//! material in a `jwks_file`. This module holds what FerroEHR adds on top:
//! the patient-data assurance type of `[authz.assurance]`. No openEHR spec
//! governs authentication mechanics, ITS-REST leaving the scheme open — our
//! own design, with the IETF OAuth and JOSE RFCs as the authority for every
//! rule enforced here.

use std::sync::Arc;

use ferrohealth_smart::config::{AuthSection, ConfigError, ProductStandingSection, Resolved};
use ferrohealth_smart::kind::Actor;
use ferrohealth_smart::kind::standing::{Answer, ClientStanding, Issued, Standing, StandingError};

/// The protection space every challenge names when `[auth] realm` is empty
/// (RFC 6750 §3, RFC 7617 §2).
pub const DEFAULT_REALM: &str = "ferroehr";

/// The product scope when `[auth] product_scope` is empty: the scope that
/// admits the surfaces no SMART on openEHR resource family covers.
///
/// SMART App Launch 2.2.0, Scopes and Launch Context, §Extensibility gives an
/// extension scope the form "any string starting with `__`".
pub const DEFAULT_PRODUCT_SCOPE: &str = "__ferroehr.platform";

/// Returns whether `[auth]` names at least one token issuer.
#[must_use]
pub fn has_bearer(section: &AuthSection) -> bool {
    !section.issuer.is_empty()
}

/// Returns whether `[auth]` admits at least one Basic user.
#[must_use]
pub fn has_basic(section: &AuthSection) -> bool {
    section
        .basic
        .as_ref()
        .is_some_and(|basic| !basic.users.is_empty())
}

/// Returns the schemes the server advertises, `Bearer, Basic` or `none`.
#[must_use]
pub fn advertised_mechanisms(section: &AuthSection) -> String {
    let mut schemes: Vec<&str> = Vec::new();
    if has_bearer(section) {
        schemes.push("Bearer");
    }
    if has_basic(section) {
        schemes.push("Basic");
    }
    if schemes.is_empty() {
        "none".to_owned()
    } else {
        schemes.join(", ")
    }
}

/// Returns the realm every challenge names: `[auth] realm`, or
/// [`DEFAULT_REALM`].
#[must_use]
pub fn realm(section: &AuthSection) -> &str {
    if section.realm.is_empty() {
        DEFAULT_REALM
    } else {
        &section.realm
    }
}

/// Returns the product scope: `[auth] product_scope`, or
/// [`DEFAULT_PRODUCT_SCOPE`].
#[must_use]
pub fn product_scope(section: &AuthSection) -> &str {
    if section.product_scope.is_empty() {
        DEFAULT_PRODUCT_SCOPE
    } else {
        &section.product_scope
    }
}

/// Resolves `[auth]` for a reader that classifies no token.
///
/// The boot checks, the party mapping and the federation trust read the
/// issuers and the Basic store; a standing question asked of the result fails
/// ([`Unasked`]).
///
/// # Errors
/// [`AuthConfigError::Section`] when the crate refuses the section.
pub fn resolve(section: &AuthSection) -> Result<Resolved, AuthConfigError> {
    resolve_with(section, Arc::new(Unasked))
}

/// Resolves `[auth]` with every issuer's standing questions answered by
/// `standing`.
///
/// The questions are which clients a `persons` issuer's people sign in
/// through or are acted for by, which RFC 8693 actors are delegates, and
/// which system subjects are admitted without a scope.
///
/// An enabled section with Basic users and no issuer resolves to a Basic-only
/// surface; one that admits no credential at all is refused.
///
/// # Errors
/// [`AuthConfigError::Section`] when the crate refuses the section.
pub fn resolve_with(
    section: &AuthSection,
    standing: Arc<dyn Standing>,
) -> Result<Resolved, AuthConfigError> {
    ProductStandingSection(section.clone())
        .resolve_with_tls(&ferrohealth_smart::config::ServerTls, standing)
        .map_err(|source| AuthConfigError::Section { source })
}

/// The standing of a resolution that classifies no token: every question
/// fails, so a token classified through it is refused with `503`, never
/// admitted or refused on a list no one read.
#[derive(Debug, Clone, Copy)]
pub struct Unasked;

/// The failure of a standing question asked of [`Unasked`].
#[derive(Debug, thiserror::Error)]
#[error(
    "this [auth] resolution answers no standing question; the server's gate reads the policy grants"
)]
pub struct NoStanding;

impl Standing for Unasked {
    fn client<'a>(&'a self, _client: Issued<'a>) -> Answer<'a, ClientStanding> {
        Box::pin(async { Err(StandingError::new(NoStanding)) })
    }

    fn delegate<'a>(&'a self, _issuer: &'a str, _actor: &'a Actor) -> Answer<'a, bool> {
        Box::pin(async { Err(StandingError::new(NoStanding)) })
    }

    fn scopeless<'a>(&'a self, _subject: Issued<'a>) -> Answer<'a, bool> {
        Box::pin(async { Err(StandingError::new(NoStanding)) })
    }
}

/// Validates `[auth]` at boot through the crate's resolution.
///
/// # Errors
/// [`AuthConfigError::Section`] naming the rule the crate reports broken.
pub fn validate(section: &AuthSection) -> Result<(), AuthConfigError> {
    // NOTE: RFC 9110 §11.6.1, a section with no mechanism is refused by
    // `require_mechanism`, whose message names the keys that fix it.
    if require_mechanism(section).is_err() {
        return Ok(());
    }
    resolve(section).map(|_resolved| ())
}

/// Refuses an enabled `[auth]` that names neither an issuer nor a Basic user.
///
/// RFC 9110 §11.6.1: a `401` challenge must name a scheme applicable to the
/// target resource, so authentication with no mechanism cannot be honoured.
///
/// # Errors
/// [`AuthConfigError::NoMechanism`].
pub fn require_mechanism(section: &AuthSection) -> Result<(), AuthConfigError> {
    if section.enabled && !has_bearer(section) && !has_basic(section) {
        return Err(AuthConfigError::NoMechanism);
    }
    Ok(())
}

/// An `[auth]` or `[authz.assurance]` setting the
/// server refuses to start with.
#[derive(Debug, thiserror::Error)]
pub enum AuthConfigError {
    /// Authentication is on and nothing can authenticate a caller.
    #[error(
        "auth.enabled = true but no authentication mechanism is configured: add \
         [[auth.basic.users]] entries or an [auth] issuer, or set auth.enabled = false"
    )]
    NoMechanism,
    /// The `ferrohealth-smart` crate refuses the section.
    #[error("the shared [auth] section is refused")]
    Section {
        /// The rule the crate reports broken.
        #[source]
        source: ConfigError,
    },
}

#[cfg(test)]
mod tests {
    use ferrohealth_smart::assurance::{AssuranceError, AssuranceLevel, AssuranceSection};
    use ferrohealth_smart::config::{BasicSection, BasicUser, IssuerEntry, ScopelessSubject};
    use ferrohealth_smart::kind::IssuerKind;
    use secrecy::SecretString;

    use super::*;

    /// An `[auth]` section trusting the `machines` issuer `issuer` for the
    /// `ferroehr` audience.
    fn section(issuer: &str) -> AuthSection {
        AuthSection {
            issuer: vec![IssuerEntry {
                issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer.to_owned()),
                kind: Some(IssuerKind::Machines),
                audience: vec!["ferroehr".to_owned()],
                ..IssuerEntry::default()
            }],
            ..AuthSection::default()
        }
    }

    /// The one issuer table of a section [`section`] built.
    fn entry(cfg: &mut AuthSection) -> &mut IssuerEntry {
        cfg.issuer.first_mut().expect("one issuer")
    }

    /// The `[[auth.issuer]]` table head of the parsed cases.
    const ISSUER: &str = "[[issuer]]\nissuer = \"https://idp.example\"\nkind = \"machines\"\naudience = [\"ferroehr\"]\n";

    /// A section that satisfies every boot rule.
    fn valid() -> AuthSection {
        section("https://idp.example/realms/ferroehr")
    }

    /// Parses an `[auth]` table body.
    fn parse(body: &str) -> Result<AuthSection, toml::de::Error> {
        toml::from_str(body)
    }

    /// The OWASP Password Storage Cheat Sheet §Argon2id floor the crate holds
    /// a Basic hash to: m=19456 KiB, t=2, p=1.
    const FLOOR: (u32, u32, u32) = (19 * 1024, 2, 1);

    /// `err` with every source in its chain, as an operator reads it.
    fn chain(err: &AuthConfigError) -> String {
        let mut text = err.to_string();
        let mut source = std::error::Error::source(err);
        while let Some(cause) = source {
            text.push_str(": ");
            text.push_str(&cause.to_string());
            source = cause.source();
        }
        text
    }

    /// A section [`valid`] built whose issuer's `jwks_file` holds `text`.
    fn with_jwks(name: &str, text: &str) -> AuthSection {
        let dir = std::env::temp_dir().join(format!("ferroehr-jwks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write");
        let mut cfg = valid();
        entry(&mut cfg).jwks_file = path.display().to_string();
        cfg
    }

    /// RFC 7662: `mode = "introspection"` is served, and an
    /// `[auth.introspection]` table beside `mode = "jwt"` is still refused,
    /// so the table is never ignored while tokens are checked as JWTs.
    #[test]
    fn token_introspection_is_served_and_its_table_needs_the_mode() {
        let served = parse(&format!(
            "{ISSUER}mode = \"introspection\"\n\
             [issuer.introspection.client]\nclient_id = \"cdr\"\nclient_secret = \"s3cret\"\n"
        ))
        .expect("parses");
        assert!(validate(&served).is_ok(), "{:?}", validate(&served));
        let ignored = parse(&format!(
            "{ISSUER}[issuer.introspection]\nendpoint = \"https://idp.example/introspect\"\n"
        ))
        .expect("parses");
        assert!(
            matches!(validate(&ignored), Err(AuthConfigError::Section { .. })),
            "a table beside mode = \"jwt\" is refused"
        );
    }

    /// RFC 8725 §3.1, §3.2 and RFC 9068 §4: the resource server accepts
    /// asymmetric algorithms only, so an `HS*` algorithm, which an HMAC secret
    /// would have verified, is refused at boot whatever the key source.
    #[test]
    fn a_symmetric_algorithm_is_a_boot_error() {
        let cfg = parse(&format!("{ISSUER}algorithms = [\"HS256\"]\n"))
            .expect("HS256 is a JOSE algorithm name");
        assert!(matches!(
            validate(&cfg),
            Err(AuthConfigError::Section { .. })
        ));
        assert!(validate(&valid()).is_ok(), "the asymmetric default boots");
    }

    /// The HMAC key source is gone with the move to the shared crate: an
    /// `hmac_secret` is an unknown key, refused like any typo.
    #[test]
    fn an_hmac_secret_is_refused_as_an_unknown_key() {
        for body in [
            "issuer = \"https://idp.example\"\naudience = [\"ferroehr\"]\nhmac_secret = \"0123456789abcdef0123456789abcdef\"\n",
            "issuer = \"https://idp.example\"\naudience = [\"ferroehr\"]\nhmac_secret_file = \"/run/secrets/hmac\"\n",
            "issuer = \"https://idp.example\"\naudience = [\"ferroehr\"]\njwks_json = \"{}\"\n",
        ] {
            let err = parse(body).expect_err("an unknown key must be refused");
            assert!(err.to_string().contains("unknown field"), "{err}");
        }
    }

    /// RFC 7517 §4.1 / RFC 7518 §6.4: a symmetric key in the handed-over key
    /// set is a secret the resource server can never use, so boot refuses it
    /// before any rendering could print it.
    #[test]
    fn a_symmetric_key_in_the_jwks_file_is_refused_at_boot() {
        let secret_k = "c3VwZXItc2VjcmV0LXN5bW1ldHJpYy1rZXktbWF0ZXJpYWw";
        let cfg = with_jwks(
            "oct.json",
            &format!(r#"{{"keys":[{{"kty":"oct","kid":"shared","k":"{secret_k}"}}]}}"#),
        );
        let err = validate(&cfg).expect_err("an oct key must be refused");
        assert!(matches!(err, AuthConfigError::Section { .. }), "{err}");
        assert!(
            !chain(&err).contains(secret_k),
            "the refusal must not quote the key"
        );
    }

    /// RFC 7518 §6.3.2: a private RSA member makes the key a private key, which
    /// a verifier never needs; a public key set passes.
    #[test]
    fn private_key_material_in_the_jwks_file_is_refused_and_public_keys_pass() {
        let cfg = with_jwks(
            "private.json",
            r#"{"keys":[{"kty":"RSA","kid":"k1","n":"AQAB","e":"AQAB","d":"cHJpdmF0ZQ"}]}"#,
        );
        let err = validate(&cfg).expect_err("a private member must be refused");
        assert!(matches!(err, AuthConfigError::Section { .. }), "{err}");
        assert!(
            !chain(&err).contains("cHJpdmF0ZQ"),
            "the refusal must not quote the private member"
        );
        let public = with_jwks(
            "public.json",
            r#"{"keys":[{"kty":"RSA","kid":"k1","n":"AQAB","e":"AQAB"}]}"#,
        );
        assert!(validate(&public).is_ok(), "{:?}", validate(&public));
    }

    /// RFC 7517 §5: a `jwks_file` that is no JWK Set refuses the start, where
    /// the crate reads the key set once, and the refusal never quotes the
    /// value it choked on.
    #[tokio::test]
    async fn a_jwks_file_that_is_no_jwk_set_refuses_the_start() {
        for (name, text) in [
            ("garbage.json", "not json"),
            ("shape.json", r#"{"keys":"c2VjcmV0LWtleS1tYXRlcmlhbA"}"#),
        ] {
            let resolved = resolve(&with_jwks(name, text)).expect("the section resolves");
            let settings = resolved.issuers.into_iter().next().expect("one issuer");
            let err = ferrohealth_smart::resource::ResourceServer::discover(settings)
                .await
                .expect_err("not a JWK Set");
            let mut shown = err.to_string();
            let mut source = std::error::Error::source(&err);
            while let Some(cause) = source {
                shown.push_str(&cause.to_string());
                source = cause.source();
            }
            assert!(!shown.contains("c2VjcmV0"), "{name}: {shown}");
        }
    }

    /// RFC 8725 §3.2 / RFC 9068 §4 step 5: an unsigned token proves nothing, and
    /// `alg: none` is the canonical JWT forgery, so no spelling of it parses as
    /// an accepted algorithm.
    #[test]
    fn algorithm_none_is_unconfigurable() {
        for spelling in ["none", "None", "NONE"] {
            let body = format!("{ISSUER}algorithms = [\"{spelling}\"]\n");
            assert!(
                parse(&body).is_err(),
                "`{spelling}` must be refused at boot"
            );
        }
    }

    /// An Argon2id PHC string with the given costs.
    fn phc(m: u32, t: u32, p: u32) -> String {
        format!("$argon2id$v=19$m={m},t={t},p={p}$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2hoYQ")
    }

    /// A Basic-only section with one user holding `hash`.
    fn basic(hash: &str) -> AuthSection {
        AuthSection {
            basic: Some(BasicSection {
                users: vec![BasicUser {
                    user: "clinician".to_owned(),
                    password_hash: Some(SecretString::from(hash.to_owned())),
                    password_hash_file: None,
                }],
                cache_ttl: ferrohealth_config::units::Duration::ZERO,
            }),
            ..AuthSection::default()
        }
    }

    /// A Basic entry with no user-id, or with no hash, is refused at boot
    /// rather than passing as a usable credential.
    #[test]
    fn a_partial_basic_user_is_a_boot_error() {
        let floor = phc(FLOOR.0, FLOOR.1, FLOOR.2);
        let mut no_user = basic(&floor);
        if let Some(section) = no_user.basic.as_mut() {
            section.users.iter_mut().for_each(|user| user.user.clear());
        }
        assert!(matches!(
            validate(&no_user),
            Err(AuthConfigError::Section { .. })
        ));
        let mut no_hash = basic(&floor);
        if let Some(section) = no_hash.basic.as_mut() {
            section
                .users
                .iter_mut()
                .for_each(|user| user.password_hash = None);
        }
        assert!(matches!(
            validate(&no_hash),
            Err(AuthConfigError::Section { .. })
        ));
        assert!(
            validate(&basic(&floor)).is_ok(),
            "a Basic-only section boots"
        );
    }

    #[test]
    fn a_valid_issuer_boots() {
        assert!(validate(&valid()).is_ok());
        assert!(require_mechanism(&valid()).is_ok());
        assert_eq!(advertised_mechanisms(&valid()), "Bearer");
    }

    /// RFC 7519 §4.1.3 + RFC 9068 §4 step 4: a resource server that declares no
    /// audience cannot reject a token minted for a different one.
    #[test]
    fn an_issuer_without_an_audience_is_a_boot_error() {
        for audience in [vec![], vec![String::new()]] {
            let mut cfg = valid();
            entry(&mut cfg).audience = audience;
            assert!(matches!(
                validate(&cfg),
                Err(AuthConfigError::Section { .. })
            ));
        }
    }

    /// RFC 9068 §4 step 6: expiry leeway may be "no more than a few minutes".
    #[test]
    fn a_clock_skew_above_the_cap_is_a_boot_error() {
        let mut cfg = valid();
        entry(&mut cfg).clock_skew = ferrohealth_config::units::Duration::from_secs(301);
        assert!(matches!(
            validate(&cfg),
            Err(AuthConfigError::Section { .. })
        ));
        entry(&mut cfg).clock_skew = ferrohealth_config::units::Duration::from_secs(300);
        assert!(validate(&cfg).is_ok(), "the cap itself is accepted");
        assert_eq!(
            IssuerEntry::default().clock_skew,
            ferrohealth_config::units::Duration::from_secs(60)
        );
    }

    /// RFC 8414 §2 + §6.2: the issuer identifier is an `https` URL. Plain HTTP
    /// is accepted on a loopback host alone, the crate's rule, which replaces
    /// the former `allow_insecure_issuer` switch.
    #[test]
    fn a_non_loopback_http_issuer_is_a_boot_error() {
        let cfg = section("http://keycloak:8081/auth/realms/ferroehr");
        assert!(matches!(
            validate(&cfg),
            Err(AuthConfigError::Section { .. })
        ));
        assert!(validate(&section("http://localhost:8081/auth/realms/ferroehr")).is_ok());
        assert!(validate(&section("http://127.0.0.1:8081/realms/ferroehr")).is_ok());
    }

    /// RFC 8414 §2: the issuer identifier "has no query or fragment
    /// components".
    #[test]
    fn an_issuer_with_a_query_or_a_fragment_is_a_boot_error() {
        for issuer in [
            "https://idp.example/realms/ferroehr?tenant=a",
            "https://idp.example/realms/ferroehr#frag",
        ] {
            assert!(
                matches!(
                    validate(&section(issuer)),
                    Err(AuthConfigError::Section { .. })
                ),
                "{issuer}"
            );
        }
    }

    /// OpenID Connect Core 1.0 §1.2: an issuer identifier "contains scheme,
    /// host, and optionally, port number and path components", so `userinfo`
    /// is refused, and the refusal masks it rather than quoting it.
    #[test]
    fn issuer_with_userinfo_is_a_boot_error_that_masks_it() {
        for issuer in [
            "https://idp:hunter2@idp.example/realms/ferroehr",
            "https://idp@idp.example/realms/ferroehr",
            "https://idp:hunter2@idp.example/r?tenant=a",
        ] {
            let err = validate(&section(issuer)).expect_err("userinfo in an issuer is refused");
            assert!(matches!(err, AuthConfigError::Section { .. }), "{err}");
            let shown = chain(&err);
            assert!(!shown.contains("hunter2"), "{shown}");
            assert!(!shown.contains("idp@"), "{shown}");
        }
    }

    /// A blank issuer and text that is no URL are refused, and a section with
    /// no issuer table and no Basic user has no mechanism.
    #[test]
    fn blank_and_unparsable_issuers_are_boot_errors() {
        assert!(matches!(
            validate(&section("   ")),
            Err(AuthConfigError::Section { .. })
        ));
        assert!(!has_bearer(&AuthSection::default()));
        assert!(matches!(
            require_mechanism(&AuthSection::default()),
            Err(AuthConfigError::NoMechanism)
        ));
        assert!(matches!(
            validate(&section("idp.example")),
            Err(AuthConfigError::Section { .. })
        ));
    }

    /// OWASP Password Storage Cheat Sheet §Argon2id: the minimum configuration
    /// is Argon2id with m=19456 KiB, t=2, p=1. The verifier reads its cost
    /// parameters from the stored PHC string, so a cheaper hash would verify.
    #[test]
    fn weak_argon2_params_are_a_boot_error() {
        for (m, t, p) in [(4_096, 2, 1), (16 * 1024, 2, 1), (FLOOR.0, 1, 1)] {
            assert!(
                matches!(
                    validate(&basic(&phc(m, t, p))),
                    Err(AuthConfigError::Section { .. })
                ),
                "m={m},t={t},p={p} is below the OWASP floor and must be refused"
            );
        }
        // `p=0` is not even a representable Argon2 parameter set, so it is
        // refused one step earlier, by the PHC parse itself.
        assert!(matches!(
            validate(&basic(&phc(FLOOR.0, FLOOR.1, 0))),
            Err(AuthConfigError::Section { .. })
        ));
        for (m, t, p) in [FLOOR, (65_536, 3, 4)] {
            assert!(
                validate(&basic(&phc(m, t, p))).is_ok(),
                "m={m},t={t},p={p} meets the floor"
            );
        }
    }

    #[test]
    fn a_non_argon2id_password_hash_is_a_boot_error() {
        assert!(matches!(
            validate(&basic(
                "$argon2i$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2hoYQ"
            )),
            Err(AuthConfigError::Section { .. })
        ));
        assert!(matches!(
            validate(&basic("not-a-phc-string")),
            Err(AuthConfigError::Section { .. })
        ));
    }

    /// RFC 9110 §11.6.1: a `401` challenge must name a scheme applicable to the
    /// target resource, so authentication with no mechanism cannot be honoured.
    #[test]
    fn auth_enabled_without_a_mechanism_is_a_boot_error() {
        let cfg = AuthSection::default();
        assert!(cfg.enabled && !has_bearer(&cfg) && !has_basic(&cfg));
        assert!(matches!(
            require_mechanism(&cfg),
            Err(AuthConfigError::NoMechanism)
        ));
        let floor = phc(FLOOR.0, FLOOR.1, FLOOR.2);
        assert!(require_mechanism(&basic(&floor)).is_ok());
        let disabled = AuthSection {
            enabled: false,
            ..AuthSection::default()
        };
        assert!(require_mechanism(&disabled).is_ok());
    }

    /// The policy grants answer which subjects are admitted without a scope
    /// and which clients a person signs in through, so a list set in code
    /// beside them is refused at boot, never read in two places.
    #[test]
    fn a_client_list_beside_the_policy_grants_is_refused() {
        let mut scopeless = valid();
        scopeless.scopeless_subjects = vec![ScopelessSubject {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(
                "https://idp.example/realms/ferroehr",
            ),
            sub: "svc".to_owned(),
        }];
        let mut first_party = valid();
        entry(&mut first_party).first_party_clients = Some(vec!["portal".to_owned()]);
        for cfg in [scopeless, first_party] {
            assert!(
                matches!(validate(&cfg), Err(AuthConfigError::Section { .. })),
                "{:?}",
                validate(&cfg)
            );
        }
    }

    /// An enabled section with Basic users and no issuer resolves to a
    /// Basic-only surface, and one that admits no credential is refused.
    #[test]
    fn a_basic_only_section_resolves_enabled_and_an_empty_one_is_refused() {
        let floor = phc(FLOOR.0, FLOOR.1, FLOOR.2);
        let resolved = resolve(&basic(&floor)).expect("a Basic-only section resolves");
        assert!(resolved.enabled);
        assert_eq!(
            resolved.issuers,
            Vec::<ferrohealth_smart::resource::Settings>::new()
        );
        assert!(resolved.basic.is_some());
        assert!(matches!(
            resolve(&AuthSection::default()),
            Err(AuthConfigError::Section { .. })
        ));
        assert!(
            validate(&AuthSection::default()).is_ok(),
            "no mechanism is require_mechanism's refusal, not validate's"
        );
    }

    /// The realm and the product scope fall back to FerroEHR's when unset, and
    /// a product scope outside the two SMART extension forms is refused.
    #[test]
    fn the_realm_and_the_product_scope_default_to_ferroehr_s() {
        let cfg = valid();
        assert_eq!(realm(&cfg), DEFAULT_REALM);
        assert_eq!(product_scope(&cfg), DEFAULT_PRODUCT_SCOPE);
        let mut named = valid();
        named.product_scope = "ferroehr/admin".to_owned();
        assert!(matches!(
            validate(&named),
            Err(AuthConfigError::Section { .. })
        ));
        named.product_scope = "__ferroehr.ops".to_owned();
        assert!(validate(&named).is_ok());
        assert_eq!(product_scope(&named), "__ferroehr.ops");
    }

    /// An assurance block mapping the given values.
    fn with_assurance(
        levels: &[(&str, AssuranceLevel)],
        minimum: Option<AssuranceLevel>,
    ) -> AssuranceSection {
        AssuranceSection {
            levels: levels
                .iter()
                .map(|(value, level)| ((*value).to_owned(), *level))
                .collect(),
            minimum,
            ..AssuranceSection::default()
        }
    }

    /// The default assurance block is off: no minimum, no map, nothing refused.
    #[test]
    fn the_assurance_block_is_off_by_default() {
        let assurance = AssuranceSection::default();
        assert_eq!(assurance.claim, "acr");
        assert!(assurance.levels.is_empty());
        assert_eq!(assurance.minimum, None);
        assert_eq!(assurance.accepted(), Vec::<&str>::new());
        assert!(with_assurance(&[], None).validate().is_ok());
    }

    /// A minimum with no mapped value would refuse every patient-data request,
    /// so it is refused at boot instead.
    #[test]
    fn a_minimum_without_levels_is_a_boot_error() {
        assert!(matches!(
            with_assurance(&[], Some(AssuranceLevel::Substantial)).validate(),
            Err(AssuranceError::Unreachable { minimum }) if minimum == "substantial"
        ));
    }

    /// A minimum no mapped value reaches is the same dead configuration.
    #[test]
    fn a_minimum_no_value_reaches_is_a_boot_error() {
        assert!(matches!(
            with_assurance(
                &[("urn:loa:low", AssuranceLevel::Low)],
                Some(AssuranceLevel::High)
            )
            .validate(),
            Err(AssuranceError::Unreachable { minimum }) if minimum == "high"
        ));
    }

    /// RFC 9470 §3 names the accepted values in one space-separated quoted
    /// `acr_values` parameter, so a value that cannot sit in it is refused.
    #[test]
    fn an_unquotable_level_value_is_a_boot_error() {
        for bad in ["", "two words", "quo\"te", "back\\slash"] {
            assert!(
                matches!(
                    with_assurance(&[(bad, AssuranceLevel::High)], Some(AssuranceLevel::High))
                        .validate(),
                    Err(AssuranceError::Value(_))
                ),
                "{bad:?}"
            );
        }
    }

    /// A blank claim has nothing to read the level from.
    #[test]
    fn a_blank_assurance_claim_is_a_boot_error() {
        let mut cfg = with_assurance(&[("x", AssuranceLevel::High)], None);
        cfg.claim = " ".to_owned();
        assert!(matches!(cfg.validate(), Err(AssuranceError::Claim { .. })));
    }

    /// The accepted values are those at or above the minimum, in map order.
    #[test]
    fn accepted_values_are_those_at_or_above_the_minimum() {
        let cfg = with_assurance(
            &[
                ("loa-high", AssuranceLevel::High),
                ("loa-low", AssuranceLevel::Low),
                ("loa-substantial", AssuranceLevel::Substantial),
            ],
            Some(AssuranceLevel::Substantial),
        );
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.accepted(), vec!["loa-high", "loa-substantial"]);
        assert!(AssuranceLevel::High > AssuranceLevel::Substantial);
        assert!(AssuranceLevel::Substantial > AssuranceLevel::Low);
    }

    /// The assurance block deserializes from TOML in the documented spelling.
    #[test]
    fn the_blocks_deserialize_from_toml() {
        let assurance: AssuranceSection = toml::from_str(
            r#"
            minimum = "substantial"
            [levels]
            "http://eidas.europa.eu/LoA/substantial" = "substantial"
            "http://eidas.europa.eu/LoA/high" = "high"
            "#,
        )
        .expect("parses");
        assert_eq!(assurance.minimum, Some(AssuranceLevel::Substantial));
        assert_eq!(assurance.levels.len(), 2);
    }
}
