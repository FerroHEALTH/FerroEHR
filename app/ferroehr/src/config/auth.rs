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

use crate::config::server::PublicRoot;

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

/// Returns the base URLs an issuer that names no `audience` is resolved
/// for: the openEHR REST API's `baseUrl` the SMART discovery document
/// publishes, `[server] public_base_url` with `server.base_path` and
/// `api.base_path`, or none when the deployment names no public URL.
///
/// An authorization server names the resource the app asked for in `aud`
/// (RFC 8707 §2), and SMART on openEHR has the app ask for that `baseUrl`
/// (master04 §Services).
#[must_use]
pub fn audience_bases(root: Option<&PublicRoot>, api_base_path: &str) -> Vec<String> {
    if api_base_path.starts_with("http://") || api_base_path.starts_with("https://") {
        return vec![api_base_path.to_owned()];
    }
    root.map(|root| root.at(api_base_path))
        .into_iter()
        .collect()
}

/// Resolves `[auth]` for a reader that classifies no token, each issuer
/// that names no audience resolved for `bases` ([`audience_bases`]).
///
/// The boot checks, the party mapping and the federation trust read the
/// issuers and the Basic store; a standing question asked of the result fails
/// ([`Unasked`]).
///
/// # Errors
/// [`AuthConfigError::Section`] when the crate refuses the section.
pub fn resolve(section: &AuthSection, bases: &[String]) -> Result<Resolved, AuthConfigError> {
    resolve_with(section, Arc::new(Unasked), bases)
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
    bases: &[String],
) -> Result<Resolved, AuthConfigError> {
    let bases: Vec<&str> = bases.iter().map(String::as_str).collect();
    ProductStandingSection(section.clone())
        .resolve_with_tls(&ferrohealth_smart::config::ServerTls, standing, &bases)
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
pub fn validate(section: &AuthSection, bases: &[String]) -> Result<(), AuthConfigError> {
    // NOTE: RFC 9110 §11.6.1, a section with no mechanism is refused by
    // `require_mechanism`, whose message names the keys that fix it.
    if require_mechanism(section).is_err() {
        return Ok(());
    }
    resolve(section, bases).map(|_resolved| ())
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
mod tests;
