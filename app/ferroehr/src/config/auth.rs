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
//! never issues them. This module holds what FerroEHR adds on top: a
//! Basic-only surface, the Argon2id cost floor, the issuer `userinfo` rule,
//! the refusal of private or symmetric key material in a `jwks_file`, and the
//! patient-data assurance types of `[authz.assurance]` and
//! `[authz.professional]`. No openEHR spec governs authentication mechanics,
//! ITS-REST leaving the scheme open — our own design, with the IETF OAuth and
//! JOSE RFCs as the authority for every rule enforced here.

use std::collections::BTreeMap;
use std::path::Path;

use argon2::{Algorithm, Params, PasswordHash};
use ferrohealth_smart::config::{AuthSection, ConfigError, IssuerKey, Resolved};
use secrecy::ExposeSecret;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::secret::redact_userinfo;

/// The protection space every challenge names when `[auth] realm` is empty
/// (RFC 6750 §3, RFC 7617 §2).
pub const DEFAULT_REALM: &str = "ferroehr";

/// The product scope when `[auth] product_scope` is empty: the scope that
/// admits the surfaces no SMART on openEHR resource family covers.
///
/// SMART App Launch 2.2.0, Scopes and Launch Context, §Extensibility gives an
/// extension scope the form "any string starting with `__`".
pub const DEFAULT_PRODUCT_SCOPE: &str = "__ferroehr.platform";

/// The minimum Argon2 memory cost, in KiB (OWASP Password Storage Cheat Sheet
/// §Argon2id).
pub const MIN_ARGON2_M_COST: u32 = 19 * 1024;
/// The minimum Argon2 iteration count.
pub const MIN_ARGON2_T_COST: u32 = 2;
/// The minimum Argon2 parallelism.
pub const MIN_ARGON2_P_COST: u32 = 1;

/// Returns whether `[auth]` names at least one token issuer.
#[must_use]
pub fn has_bearer(section: &AuthSection) -> bool {
    match &section.issuer {
        IssuerKey::Flat(issuer) => !issuer.trim().is_empty(),
        IssuerKey::List(entries) => !entries.is_empty(),
    }
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

/// Resolves `[auth]` into the issuers and the Basic store the crate checks
/// credentials against.
///
/// A section with no issuer is a Basic-only surface: the crate refuses an
/// enabled section without one, so it is resolved with its issuers off and
/// its Basic users kept (no specification governs this: our own design).
///
/// # Errors
/// [`AuthConfigError::Section`] when the crate refuses the section.
pub fn resolve(section: &AuthSection) -> Result<Resolved, AuthConfigError> {
    if has_bearer(section) || !section.enabled {
        return section
            .resolve()
            .map_err(|source| AuthConfigError::Section { source });
    }
    let mut basic_only = section.clone();
    basic_only.enabled = false;
    let mut resolved = basic_only
        .resolve()
        .map_err(|source| AuthConfigError::Section { source })?;
    resolved.enabled = true;
    Ok(resolved)
}

/// Validates `[auth]` at boot: the rules FerroEHR adds, then the crate's.
///
/// # Errors
/// The [`AuthConfigError`] naming the first rule the section breaks.
pub fn validate(section: &AuthSection) -> Result<(), AuthConfigError> {
    if !section.scopeless_subjects.is_empty() {
        return Err(AuthConfigError::ScopelessSubjects);
    }
    for entry in section.issuers() {
        check_issuer_userinfo(&entry.issuer)?;
        if let Some(path) = entry.jwks_file.as_deref().filter(|path| !path.is_empty()) {
            check_jwks_file(Path::new(path))?;
        }
    }
    if !section.jwks_file.is_empty() {
        check_jwks_file(Path::new(&section.jwks_file))?;
    }
    if let Some(basic) = &section.basic {
        for user in &basic.users {
            if let Some(hash) = &user.password_hash {
                check_password_hash(&user.user, hash.expose_secret())?;
            }
        }
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

/// Returns `[auth]` as the redacted configuration views render it
/// (`ferroehr config check`, `/management/env`, `ferroehr report`): every key
/// with its value, a password hash as `***`.
///
/// The crate's section types implement no `Serialize`, so the tree is written
/// here, key for key.
#[must_use]
pub fn redacted(section: &AuthSection) -> serde_json::Value {
    let issuer = match &section.issuer {
        IssuerKey::Flat(issuer) => serde_json::Value::String(issuer.clone()),
        IssuerKey::List(entries) => entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "issuer": entry.issuer,
                    "audience": entry.audience,
                    "algorithms": entry.algorithms,
                    "token_profile": entry.token_profile,
                    "clock_skew_seconds": entry.clock_skew_seconds,
                    "jwks_max_age_seconds": entry.jwks_max_age_seconds,
                    "jwks_refresh_interval_seconds": entry.jwks_refresh_interval_seconds,
                    "jwks_file": entry.jwks_file,
                })
            })
            .collect(),
    };
    let basic = section.basic.as_ref().map(|basic| {
        serde_json::json!({
            "users": basic
                .users
                .iter()
                .map(|user| serde_json::json!({
                    "user": user.user,
                    "password_hash": user.password_hash.as_ref().map(|_set| "***"),
                    "password_hash_file": user.password_hash_file,
                    "scopes": user.scopes,
                }))
                .collect::<Vec<_>>(),
        })
    });
    serde_json::json!({
        "enabled": section.enabled,
        "issuer": issuer,
        "audience": section.audience,
        "algorithms": section.algorithms,
        "token_profile": section.token_profile,
        "clock_skew_seconds": section.clock_skew_seconds,
        "jwks_max_age_seconds": section.jwks_max_age_seconds,
        "jwks_refresh_interval_seconds": section.jwks_refresh_interval_seconds,
        "jwks_file": section.jwks_file,
        "timeout_ms": section.timeout_ms,
        "realm": section.realm,
        "scopeless_subjects": section.scopeless_subjects,
        "capabilities": section.capabilities,
        "scopes_supported": section.scopes_supported,
        "product_scope": section.product_scope,
        "basic": basic,
    })
}

/// An `[auth]`, `[authz.assurance]` or `[authz.professional]` setting the
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
    /// `[auth] scopeless_subjects` is set, and no FerroEHR route honours it.
    #[error(
        "auth.scopeless_subjects is set, but every FerroEHR route carries a SMART on openEHR \
         requirement, which the allowlist does not satisfy: grant the subject the openEHR \
         scopes it needs (for example system/composition-*.cruds) and remove the key"
    )]
    ScopelessSubjects,
    /// The issuer carries `userinfo`; the value is shown masked.
    #[error(
        "auth.issuer {0:?} must carry no userinfo: an issuer identifier holds only \
         scheme, host, port and path components (OpenID Connect Core 1.0 §1.2)"
    )]
    IssuerHasUserinfo(String),
    /// The `jwks_file` is not a JWK Set.
    #[error(
        "auth.jwks_file is not a JWK Set (RFC 7517 §5): {category:?} error at line {line}, \
         column {column}"
    )]
    JwksUnparsable {
        /// The JSON error class.
        category: serde_json::error::Category,
        /// The line the parse stopped at.
        line: usize,
        /// The column the parse stopped at.
        column: usize,
    },
    /// The `jwks_file` holds a symmetric key.
    #[error(
        "auth.jwks_file key #{index} (kid {kid:?}) is a symmetric key (kty \"oct\", \
         RFC 7517 §4.1, RFC 7518 §6.4): a key set handed to a resource server holds public \
         keys only, and the server accepts asymmetric algorithms only (RFC 8725 §3.1)"
    )]
    JwksSymmetricKey {
        /// The key's position in the set.
        index: usize,
        /// The key's `kid`, when it has one.
        kid: Option<String>,
    },
    /// The `jwks_file` holds private key material.
    #[error(
        "auth.jwks_file key #{index} (kid {kid:?}) carries the private-key member \
         {member:?} (RFC 7518 §6.2.2, §6.3.2, RFC 8037 §2): a verifier needs the public key \
         only, so private material is never configured here"
    )]
    JwksPrivateKeyMaterial {
        /// The key's position in the set.
        index: usize,
        /// The key's `kid`, when it has one.
        kid: Option<String>,
        /// The private member it carries.
        member: &'static str,
    },
    /// The assurance claim name is blank.
    #[error(
        "authz.assurance.claim is blank: name the token claim that carries the \
         authentication assurance level (usually `acr`, OpenID Connect Core 1.0 §2)"
    )]
    AssuranceClaimBlank,
    /// A minimum is set and no claim value maps to a level.
    #[error(
        "authz.assurance.minimum = {0:?} but authz.assurance.levels maps no claim \
         value to a level: every patient-data request would be refused"
    )]
    AssuranceMinimumWithoutLevels(String),
    /// No mapped value reaches the minimum.
    #[error(
        "authz.assurance.minimum = {0:?} but no value in authz.assurance.levels maps \
         to that level or above: every patient-data request would be refused"
    )]
    AssuranceMinimumUnreachable(String),
    /// A level value cannot be quoted in the step-up challenge.
    #[error(
        "authz.assurance.levels key {0:?} is blank or carries whitespace, a quote, a \
         backslash or a control character: the refusal names the accepted values in a \
         space-separated quoted `acr_values` parameter (RFC 9470 §3, RFC 9110 §5.6.4)"
    )]
    AssuranceValueUnquotable(String),
    /// The professional claim name is blank.
    #[error(
        "authz.professional.claim is blank: name the token claim that identifies the \
         natural person (for example `sub`)"
    )]
    ProfessionalClaimBlank,
    /// The acting-for claim name is blank.
    #[error(
        "authz.professional.acting_for_claim is blank: remove it, or name the claim a \
         client token carries the professional's identifier in"
    )]
    ActingForClaimBlank,
    /// A Basic user's hash is no PHC string.
    #[error("auth.basic.users[{username:?}].password_hash is not a PHC hash string: {reason}")]
    PasswordHashUnparsable {
        /// The user.
        username: String,
        /// What the PHC parser reported.
        reason: String,
    },
    /// A Basic user's hash is of another algorithm.
    #[error(
        "auth.basic.users[{username:?}].password_hash uses {algorithm:?}; only argon2id is \
         accepted (OWASP Password Storage Cheat Sheet §Argon2id)"
    )]
    PasswordHashNotArgon2id {
        /// The user.
        username: String,
        /// The algorithm the hash names.
        algorithm: String,
    },
    /// A Basic user's hash is cheaper than the floor.
    #[error(
        "auth.basic.users[{username:?}].password_hash has m={m},t={t},p={p}, below the minimum \
         m={min_m},t={min_t},p={min_p} (OWASP Password Storage Cheat Sheet §Argon2id)"
    )]
    PasswordHashTooWeak {
        /// The user.
        username: String,
        /// The memory cost.
        m: u32,
        /// The iteration count.
        t: u32,
        /// The parallelism.
        p: u32,
        /// The memory floor.
        min_m: u32,
        /// The iteration floor.
        min_t: u32,
        /// The parallelism floor.
        min_p: u32,
    },
}

/// Refuses an issuer that carries `userinfo`, before the crate's own refusal
/// could quote it.
fn check_issuer_userinfo(issuer: &str) -> Result<(), AuthConfigError> {
    // NOTE: OpenID Connect Core 1.0 §1.2, an issuer identifier holds scheme,
    // host, port and path only; a value that is no URL is the crate's to refuse.
    let Ok(url) = Url::parse(issuer.trim()) else {
        return Ok(());
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AuthConfigError::IssuerHasUserinfo(redact_userinfo(
            issuer.trim(),
        )));
    }
    Ok(())
}

/// The shape of a JWK Set, read only to refuse what a verifier must never
/// hold.
#[derive(Deserialize)]
struct JwkSetShape {
    /// The keys.
    keys: Vec<JwkShape>,
}

/// One JWK, by the members that make it secret.
#[derive(Deserialize)]
struct JwkShape {
    /// The key type.
    kty: String,
    /// The key id.
    #[serde(default)]
    kid: Option<String>,
    /// The RSA or EC private exponent.
    #[serde(default)]
    d: Option<IgnoredAny>,
    /// An RSA prime.
    #[serde(default)]
    p: Option<IgnoredAny>,
    /// An RSA prime.
    #[serde(default)]
    q: Option<IgnoredAny>,
    /// An RSA CRT exponent.
    #[serde(default)]
    dp: Option<IgnoredAny>,
    /// An RSA CRT exponent.
    #[serde(default)]
    dq: Option<IgnoredAny>,
    /// The RSA CRT coefficient.
    #[serde(default)]
    qi: Option<IgnoredAny>,
    /// Further RSA primes.
    #[serde(default)]
    oth: Option<IgnoredAny>,
    /// A symmetric key value.
    #[serde(default)]
    k: Option<IgnoredAny>,
}

impl JwkShape {
    /// The first private member the key carries.
    fn private_member(&self) -> Option<&'static str> {
        [
            ("d", self.d.is_some()),
            ("p", self.p.is_some()),
            ("q", self.q.is_some()),
            ("dp", self.dp.is_some()),
            ("dq", self.dq.is_some()),
            ("qi", self.qi.is_some()),
            ("oth", self.oth.is_some()),
            ("k", self.k.is_some()),
        ]
        .into_iter()
        .find_map(|(name, present)| present.then_some(name))
    }
}

/// Refuses a `jwks_file` holding a symmetric key or private key material.
///
/// A file that cannot be read is left to the crate, which refuses the start
/// when it reads the key set.
fn check_jwks_file(path: &Path) -> Result<(), AuthConfigError> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    check_jwks(&text)
}

/// Refuses a JWK Set holding a symmetric key or private key material, never
/// quoting the input.
fn check_jwks(text: &str) -> Result<(), AuthConfigError> {
    let set: JwkSetShape =
        serde_json::from_str(text).map_err(|e| AuthConfigError::JwksUnparsable {
            category: e.classify(),
            line: e.line(),
            column: e.column(),
        })?;
    for (index, key) in set.keys.into_iter().enumerate() {
        if key.kty == "oct" {
            return Err(AuthConfigError::JwksSymmetricKey {
                index,
                kid: key.kid,
            });
        }
        if let Some(member) = key.private_member() {
            return Err(AuthConfigError::JwksPrivateKeyMaterial {
                index,
                kid: key.kid,
                member,
            });
        }
    }
    Ok(())
}

/// Holds a Basic user's PHC string to Argon2id at the OWASP floor.
///
/// The verifier reads its cost parameters from the stored string, so a
/// cheaper hash would verify; the crate checks the algorithm, and the floor
/// is FerroEHR's.
fn check_password_hash(username: &str, phc: &str) -> Result<(), AuthConfigError> {
    let hash = PasswordHash::new(phc).map_err(|e| AuthConfigError::PasswordHashUnparsable {
        username: username.to_owned(),
        reason: e.to_string(),
    })?;
    if !matches!(
        Algorithm::try_from(hash.algorithm.as_str()),
        Ok(Algorithm::Argon2id)
    ) {
        return Err(AuthConfigError::PasswordHashNotArgon2id {
            username: username.to_owned(),
            algorithm: hash.algorithm.as_str().to_owned(),
        });
    }
    let params = Params::try_from(&hash).map_err(|e| AuthConfigError::PasswordHashUnparsable {
        username: username.to_owned(),
        reason: e.to_string(),
    })?;
    if params.m_cost() < MIN_ARGON2_M_COST
        || params.t_cost() < MIN_ARGON2_T_COST
        || params.p_cost() < MIN_ARGON2_P_COST
    {
        return Err(AuthConfigError::PasswordHashTooWeak {
            username: username.to_owned(),
            m: params.m_cost(),
            t: params.t_cost(),
            p: params.p_cost(),
            min_m: MIN_ARGON2_M_COST,
            min_t: MIN_ARGON2_T_COST,
            min_p: MIN_ARGON2_P_COST,
        });
    }
    Ok(())
}

/// An electronic-identification level of assurance.
///
/// The three levels are the eIDAS levels of Regulation (EU) No 910/2014
/// Art. 8(2): low, substantial and high. Implementing Regulation (EU)
/// 2026/2099 Art. 6(3) asks for substantial, and high from 26 March 2032,
/// when a health professional is authenticated for cross-border exchange
/// (`docs/law/eu/cross-border-identification-2026-2099/text.html Art. 6(3)`).
/// Ordered, so `High > Substantial > Low`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssuranceLevel {
    /// eIDAS level of assurance low.
    Low,
    /// eIDAS level of assurance substantial.
    Substantial,
    /// eIDAS level of assurance high.
    High,
}

impl AssuranceLevel {
    /// The configuration and storage spelling (`low`, `substantial`, `high`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AssuranceLevel::Low => "low",
            AssuranceLevel::Substantial => "substantial",
            AssuranceLevel::High => "high",
        }
    }
}

/// The authentication assurance level a bearer token carries, and the floor a
/// patient-data request must meet (`[authz.assurance]`).
///
/// Off by default: with no `minimum` nothing is refused, and with an empty
/// `levels` map no level is recorded. EHDS Annex II 3.1 asks an EHR system
/// used by health professionals for "reliable mechanisms for the
/// identification and authentication of health professionals"
/// (`docs/law/eu/ehds/text.html Annex II 3.1`). No openEHR spec governs this —
/// our own design/extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AssuranceConfig {
    /// The token claim carrying the authentication context class (default
    /// `acr`, OpenID Connect Core 1.0 §2); a dotted path reaches into a JSON
    /// object.
    pub claim: String,
    /// Maps each claim value the issuer emits to the level it stands for.
    ///
    /// A value the map does not name carries no level.
    pub levels: BTreeMap<String, AssuranceLevel>,
    /// The level a patient-data request must carry; unset refuses nothing.
    pub minimum: Option<AssuranceLevel>,
}

impl Default for AssuranceConfig {
    fn default() -> Self {
        Self {
            claim: "acr".to_owned(),
            levels: BTreeMap::new(),
            minimum: None,
        }
    }
}

impl AssuranceConfig {
    /// The claim values that meet [`Self::minimum`], in the map's order.
    ///
    /// Empty when no minimum is set.
    #[must_use]
    pub fn accepted_values(&self) -> Vec<&str> {
        let Some(minimum) = self.minimum else {
            return Vec::new();
        };
        self.levels
            .iter()
            .filter(|(_, level)| **level >= minimum)
            .map(|(value, _)| value.as_str())
            .collect()
    }

    /// Validates the table at boot.
    ///
    /// # Errors
    /// The [`AuthConfigError`] naming the first rule the table breaks.
    pub fn validate(&self) -> Result<(), AuthConfigError> {
        if self.minimum.is_none() && self.levels.is_empty() {
            return Ok(());
        }
        if self.claim.trim().is_empty() {
            return Err(AuthConfigError::AssuranceClaimBlank);
        }
        if let Some(value) = self.levels.keys().find(|value| {
            value.is_empty()
                || value
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\')
        }) {
            return Err(AuthConfigError::AssuranceValueUnquotable(value.clone()));
        }
        if let Some(minimum) = self.minimum {
            if self.levels.is_empty() {
                return Err(AuthConfigError::AssuranceMinimumWithoutLevels(
                    minimum.as_str().to_owned(),
                ));
            }
            if self.accepted_values().is_empty() {
                return Err(AuthConfigError::AssuranceMinimumUnreachable(
                    minimum.as_str().to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// How a client-credentials token is told apart from a token that names a
/// natural person (`authz.professional.client_tokens`).
///
/// A token that lacks [`ProfessionalConfig::claim`] names no natural person
/// under either rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientTokenRule {
    /// A token whose `sub` equals its `client_id` or `azp` names the client
    /// application: RFC 9068 §2.2 has `sub` identify the client when no
    /// resource owner is involved.
    #[default]
    SubIsClient,
    /// Only a token lacking the configured claim is a client token.
    ClaimAbsent,
}

/// How a bearer token names the natural person behind it
/// (`[authz.professional]`).
///
/// Present, a patient-data request whose token names no natural person is
/// refused, unless the token is a client token acting for a professional named
/// in [`Self::acting_for_claim`]. EHDS Annex II 3.2(b) asks the access record
/// for "the specific natural person or persons having accessed"
/// (`docs/law/eu/ehds/text.html Annex II 3.2(b)`). No openEHR spec governs
/// this — our own design/extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfessionalConfig {
    /// The claim identifying the natural person (default `sub`); a dotted path
    /// reaches into a JSON object.
    pub claim: String,
    /// How a client-credentials token is recognised (default `sub_is_client`).
    pub client_tokens: ClientTokenRule,
    /// The claim a client token carries the professional's identifier in, when
    /// the client acts for a named professional; a dotted path reaches into a
    /// JSON object (for example `act.sub`). Unset refuses every client token
    /// on patient data.
    pub acting_for_claim: Option<String>,
}

impl Default for ProfessionalConfig {
    fn default() -> Self {
        Self {
            claim: "sub".to_owned(),
            client_tokens: ClientTokenRule::SubIsClient,
            acting_for_claim: None,
        }
    }
}

impl ProfessionalConfig {
    /// Validates the table at boot.
    ///
    /// # Errors
    /// The [`AuthConfigError`] naming the first rule the table breaks.
    pub fn validate(&self) -> Result<(), AuthConfigError> {
        if self.claim.trim().is_empty() {
            return Err(AuthConfigError::ProfessionalClaimBlank);
        }
        if self
            .acting_for_claim
            .as_deref()
            .is_some_and(|claim| claim.trim().is_empty())
        {
            return Err(AuthConfigError::ActingForClaimBlank);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_smart::config::{BasicSection, BasicUser};
    use secrecy::SecretString;

    use super::*;

    /// An `[auth]` section trusting `issuer` for the `ferroehr` audience.
    fn section(issuer: &str) -> AuthSection {
        AuthSection {
            issuer: IssuerKey::Flat(issuer.to_owned()),
            audience: vec!["ferroehr".to_owned()],
            ..AuthSection::default()
        }
    }

    /// A section that satisfies every boot rule.
    fn valid() -> AuthSection {
        section("https://idp.example/realms/ferroehr")
    }

    /// Parses an `[auth]` table body.
    fn parse(body: &str) -> Result<AuthSection, toml::de::Error> {
        toml::from_str(body)
    }

    /// RFC 8725 §3.1, §3.2 and RFC 9068 §4: the resource server accepts
    /// asymmetric algorithms only, so an `HS*` algorithm, which an HMAC secret
    /// would have verified, is refused at boot whatever the key source.
    #[test]
    fn a_symmetric_algorithm_is_a_boot_error() {
        let cfg = parse(
            "issuer = \"https://idp.example\"\naudience = [\"ferroehr\"]\nalgorithms = [\"HS256\"]\n",
        )
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
        let err = check_jwks(&format!(
            r#"{{"keys":[{{"kty":"oct","kid":"shared","k":"{secret_k}"}}]}}"#
        ))
        .expect_err("an oct key must be refused");
        assert!(matches!(
            err,
            AuthConfigError::JwksSymmetricKey { index: 0, .. }
        ));
        assert!(
            !err.to_string().contains(secret_k),
            "the refusal must not quote the key"
        );

        let dir = std::env::temp_dir().join(format!("ferroehr-jwks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("oct.json");
        std::fs::write(
            &path,
            format!(r#"{{"keys":[{{"kty":"oct","kid":"shared","k":"{secret_k}"}}]}}"#),
        )
        .expect("write");
        let mut cfg = valid();
        cfg.jwks_file = path.display().to_string();
        assert!(
            matches!(
                validate(&cfg),
                Err(AuthConfigError::JwksSymmetricKey { .. })
            ),
            "the [auth] section refuses it too"
        );
    }

    /// RFC 7518 §6.3.2: a private RSA member makes the key a private key, which
    /// a verifier never needs; a public key set passes.
    #[test]
    fn private_key_material_in_the_jwks_file_is_refused_and_public_keys_pass() {
        let err = check_jwks(
            r#"{"keys":[{"kty":"RSA","kid":"k1","n":"AQAB","e":"AQAB","d":"cHJpdmF0ZQ"}]}"#,
        )
        .expect_err("a private member must be refused");
        assert!(matches!(
            err,
            AuthConfigError::JwksPrivateKeyMaterial { member: "d", .. }
        ));
        assert!(
            !err.to_string().contains("cHJpdmF0ZQ"),
            "the refusal must not quote the private member"
        );
        assert!(check_jwks(r#"{"keys":[{"kty":"RSA","kid":"k1","n":"AQAB","e":"AQAB"}]}"#).is_ok());
        assert!(matches!(
            check_jwks("not json"),
            Err(AuthConfigError::JwksUnparsable { .. })
        ));
        // A parse failure never quotes the value it choked on.
        let err =
            check_jwks(r#"{"keys":"c2VjcmV0LWtleS1tYXRlcmlhbA"}"#).expect_err("not a JWK Set");
        assert!(
            !err.to_string().contains("c2VjcmV0"),
            "the refusal must not quote the input"
        );
    }

    /// RFC 8725 §3.2 / RFC 9068 §4 step 5: an unsigned token proves nothing, and
    /// `alg: none` is the canonical JWT forgery, so no spelling of it parses as
    /// an accepted algorithm.
    #[test]
    fn algorithm_none_is_unconfigurable() {
        for spelling in ["none", "None", "NONE"] {
            let body = format!(
                "issuer = \"https://idp.example\"\naudience = [\"ferroehr\"]\nalgorithms = [\"{spelling}\"]\n"
            );
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
                    scopes: vec!["user/composition-*.cruds".to_owned()],
                }],
            }),
            ..AuthSection::default()
        }
    }

    /// A Basic entry with no user-id, or with no hash, is refused at boot
    /// rather than passing as a usable credential.
    #[test]
    fn a_partial_basic_user_is_a_boot_error() {
        let floor = phc(MIN_ARGON2_M_COST, MIN_ARGON2_T_COST, MIN_ARGON2_P_COST);
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
            cfg.audience = audience;
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
        cfg.clock_skew_seconds = 301;
        assert!(matches!(
            validate(&cfg),
            Err(AuthConfigError::Section { .. })
        ));
        cfg.clock_skew_seconds = 300;
        assert!(validate(&cfg).is_ok(), "the cap itself is accepted");
        assert_eq!(AuthSection::default().clock_skew_seconds, 60);
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
            assert!(
                matches!(err, AuthConfigError::IssuerHasUserinfo(_)),
                "{err}"
            );
            let shown = err.to_string();
            assert!(!shown.contains("hunter2"), "{shown}");
            assert!(!shown.contains("idp@"), "{shown}");
        }
    }

    /// A blank issuer names no issuer, so an enabled section without a Basic
    /// user has no mechanism; text that is no URL is refused.
    #[test]
    fn blank_and_unparsable_issuers_are_boot_errors() {
        let blank = section("   ");
        assert!(!has_bearer(&blank));
        assert!(matches!(
            require_mechanism(&blank),
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
        for (m, t, p) in [(4_096, 2, 1), (16 * 1024, 2, 1), (MIN_ARGON2_M_COST, 1, 1)] {
            assert!(
                matches!(
                    validate(&basic(&phc(m, t, p))),
                    Err(AuthConfigError::PasswordHashTooWeak { .. })
                ),
                "m={m},t={t},p={p} is below the OWASP floor and must be refused"
            );
        }
        // `p=0` is not even a representable Argon2 parameter set, so it is
        // refused one step earlier, by the PHC parse itself.
        assert!(matches!(
            validate(&basic(&phc(MIN_ARGON2_M_COST, MIN_ARGON2_T_COST, 0))),
            Err(AuthConfigError::PasswordHashUnparsable { .. })
        ));
        for (m, t, p) in [
            (MIN_ARGON2_M_COST, MIN_ARGON2_T_COST, MIN_ARGON2_P_COST),
            (65_536, 3, 4),
        ] {
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
            Err(AuthConfigError::PasswordHashNotArgon2id { .. })
        ));
        assert!(matches!(
            validate(&basic("not-a-phc-string")),
            Err(AuthConfigError::PasswordHashUnparsable { .. })
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
        let floor = phc(MIN_ARGON2_M_COST, MIN_ARGON2_T_COST, MIN_ARGON2_P_COST);
        assert!(require_mechanism(&basic(&floor)).is_ok());
        let disabled = AuthSection {
            enabled: false,
            ..AuthSection::default()
        };
        assert!(require_mechanism(&disabled).is_ok());
    }

    /// The crate's scopeless allowlist admits a token on SMART on FHIR routes
    /// only, and FerroEHR's routes are SMART on openEHR, so a configured
    /// allowlist would promise access no route grants: it is refused.
    #[test]
    fn a_scopeless_subject_allowlist_is_a_boot_error() {
        let mut cfg = valid();
        cfg.scopeless_subjects = vec!["svc".to_owned()];
        assert!(matches!(
            validate(&cfg),
            Err(AuthConfigError::ScopelessSubjects)
        ));
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
    ) -> AssuranceConfig {
        AssuranceConfig {
            levels: levels
                .iter()
                .map(|(value, level)| ((*value).to_owned(), *level))
                .collect(),
            minimum,
            ..AssuranceConfig::default()
        }
    }

    /// The default assurance block is off: no minimum, no map, nothing refused.
    #[test]
    fn the_assurance_block_is_off_by_default() {
        let assurance = AssuranceConfig::default();
        assert_eq!(assurance.claim, "acr");
        assert!(assurance.levels.is_empty());
        assert_eq!(assurance.minimum, None);
        assert!(assurance.accepted_values().is_empty());
        assert!(with_assurance(&[], None).validate().is_ok());
    }

    /// A minimum with no mapped value would refuse every patient-data request,
    /// so it is refused at boot instead.
    #[test]
    fn a_minimum_without_levels_is_a_boot_error() {
        assert!(matches!(
            with_assurance(&[], Some(AssuranceLevel::Substantial)).validate(),
            Err(AuthConfigError::AssuranceMinimumWithoutLevels(level)) if level == "substantial"
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
            Err(AuthConfigError::AssuranceMinimumUnreachable(level)) if level == "high"
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
                    Err(AuthConfigError::AssuranceValueUnquotable(value)) if value == bad
                ),
                "{bad:?}"
            );
        }
    }

    /// A blank claim with a declared map has nothing to read the level from.
    #[test]
    fn a_blank_assurance_claim_is_a_boot_error() {
        let mut cfg = with_assurance(&[("x", AssuranceLevel::High)], None);
        cfg.claim = " ".to_owned();
        assert!(matches!(
            cfg.validate(),
            Err(AuthConfigError::AssuranceClaimBlank)
        ));
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
        assert_eq!(cfg.accepted_values(), vec!["loa-high", "loa-substantial"]);
        assert!(AssuranceLevel::High > AssuranceLevel::Substantial);
        assert!(AssuranceLevel::Substantial > AssuranceLevel::Low);
    }

    /// The professional block refuses blank claim names at boot.
    #[test]
    fn blank_professional_claims_are_boot_errors() {
        let professional = |claim: &str, acting_for: Option<&str>| ProfessionalConfig {
            claim: claim.to_owned(),
            acting_for_claim: acting_for.map(str::to_owned),
            ..ProfessionalConfig::default()
        };
        assert!(matches!(
            professional("", None).validate(),
            Err(AuthConfigError::ProfessionalClaimBlank)
        ));
        assert!(matches!(
            professional("sub", Some(" ")).validate(),
            Err(AuthConfigError::ActingForClaimBlank)
        ));
        assert!(professional("sub", Some("act.sub")).validate().is_ok());
        assert_eq!(
            ProfessionalConfig::default().client_tokens,
            ClientTokenRule::SubIsClient
        );
    }

    /// Both blocks deserialize from TOML in the documented spelling.
    #[test]
    fn the_blocks_deserialize_from_toml() {
        let assurance: AssuranceConfig = toml::from_str(
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
        let professional: ProfessionalConfig = toml::from_str(
            r#"
            claim = "professional_id"
            client_tokens = "claim_absent"
            acting_for_claim = "act.sub"
            "#,
        )
        .expect("parses");
        assert_eq!(professional.client_tokens, ClientTokenRule::ClaimAbsent);
        assert_eq!(professional.acting_for_claim.as_deref(), Some("act.sub"));
    }
}
