// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The request-scoped purpose of use, published by the protocol adapter and
//! read by platform-layer emitters.
//!
//! The audit middleware stamps the declared purpose onto the records IT
//! builds, but a record the service layer emits by itself — a pseudonymisation
//! boundary crossing, which no HTTP operation id describes — is built where
//! the request headers are already gone. This carries that one fact across, by
//! the same means [`crate::service::committer`] carries the authenticated
//! committer: a task-local the adapter publishes for the request's scope.
//!
//! The value is the code the deployment's own `[audit] purpose_codes`
//! vocabulary accepted, never a caller's free text, because the adapter has
//! already filtered it ("an unagreed string in the trail reads at review time
//! as though a purpose was established when none was").
//!
//! **No openEHR spec governs this — our own design/extension.** The purpose of
//! use is required of an access record by NEN 7513 and EHDS Art. 9, not by
//! openEHR.

use tokio::task_local;

task_local! {
    static DECLARED_PURPOSE: Option<String>;
    static DECLARED_PERMIT: Option<String>;
    static REQUEST_ROLES: Vec<String>;
    static ACCESS_RESTRICTION_ENFORCED: bool;
    static ACCESS_DELAY_ENFORCED: bool;
}

/// The roles the authenticated caller holds for the current request (#3239).
///
/// Published by the authentication layer beside the committer identity; empty
/// outside a request scope or for an unauthenticated caller, and the record
/// says so rather than guessing.
#[must_use]
pub fn current_roles() -> Vec<String> {
    REQUEST_ROLES.try_with(Clone::clone).unwrap_or_default()
}

/// Run `fut` with `roles` published as the request's caller roles.
pub async fn with_roles<F>(roles: Vec<String>, fut: F) -> F::Output
where
    F: Future,
{
    REQUEST_ROLES.scope(roles, fut).await
}

/// The purpose of use declared for the current request, if any.
///
/// Outside a request scope — a background task, a test, or a deployment that
/// collects no purpose — this is `None`, and the record says truthfully that
/// no purpose was declared rather than guessing one.
#[must_use]
pub fn current_purpose() -> Option<String> {
    DECLARED_PURPOSE.try_with(Clone::clone).ok().flatten()
}

/// Run `fut` with `purpose` published as the request's declared purpose of use.
pub async fn with_purpose<F>(purpose: Option<String>, fut: F) -> F::Output
where
    F: Future,
{
    DECLARED_PURPOSE.scope(purpose, fut).await
}

/// The data permit declared for the current request, if any; `None` outside a
/// request scope and when the caller declared none.
#[must_use]
pub fn current_permit() -> Option<String> {
    DECLARED_PERMIT.try_with(Clone::clone).ok().flatten()
}

/// Run `fut` with `permit` published as the request's declared data permit.
pub async fn with_permit<F>(permit: Option<String>, fut: F) -> F::Output
where
    F: Future,
{
    DECLARED_PERMIT.scope(permit, fut).await
}

/// Run `fut` with `purpose` and `permit` published as the request's declared
/// purpose of use and data permit.
pub async fn with_declared<F>(purpose: Option<String>, permit: Option<String>, fut: F) -> F::Output
where
    F: Future,
{
    with_purpose(purpose, with_permit(permit, fut)).await
}

/// The longest data permit identifier the trail records.
const PERMIT_MAX_LEN: usize = 128;

/// The declared data permit as the trail records it: the trimmed value when it
/// is 1 to 128 characters of ASCII letters, digits and `.` `_` `:` `/` `#`
/// `-`, else `None`.
///
/// A permit identifier names a decision of a health data access body
/// (`docs/law/eu/ehds/text.html` Art. 68); a value outside that form is
/// recorded as absent rather than verbatim, so a header cannot carry free text
/// or personal data into the trail. No specification fixes the form: our own
/// design.
#[must_use]
pub fn permit_identifier(declared: &str) -> Option<String> {
    let value = declared.trim();
    let allowed =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '#' | '-');
    (!value.is_empty() && value.len() <= PERMIT_MAX_LEN && value.chars().all(allowed))
        .then(|| value.to_owned())
}

/// Whether the EHDS Art. 8 restriction of access holds for the current
/// request: restricted data then reads as absent.
///
/// The protocol adapter publishes `true` for the request of a health
/// professional or a healthcare provider, and `false` for the controller's
/// own surfaces and for an emergency access in the vital interest of the
/// person (EHDS Art. 11(5), `docs/law/eu/ehds/text.html`). Outside a request
/// scope (a background task, the admin dump, an erase) it is `false`: the
/// restriction limits access by professionals, never storage or the
/// controller's processing. No openEHR spec governs restriction of access —
/// our own design/extension.
#[must_use]
pub fn access_restriction_enforced() -> bool {
    ACCESS_RESTRICTION_ENFORCED
        .try_with(|enforced| *enforced)
        .unwrap_or(false)
}

/// Run `fut` with the EHDS Art. 8 restriction of access enforced or not.
pub async fn with_access_restriction<F>(enforced: bool, fut: F) -> F::Output
where
    F: Future,
{
    ACCESS_RESTRICTION_ENFORCED.scope(enforced, fut).await
}

/// Whether the EHDS Art. 3(3) delay of the person's own access holds for the
/// current request: data a professional has not yet released then reads as
/// absent.
///
/// The protocol adapter publishes `true` for a patient-context request (a
/// SMART `patient/` grant or the configured patient claim) on a deployment
/// that enables `[access_delay]`, and `false` for every other caller, so a
/// professional reads the record as before. Outside a request scope it is
/// `false` (`docs/law/eu/ehds/text.html` Art. 3(3)). No openEHR spec governs a
/// delay of the person's access: our own design/extension.
#[must_use]
pub fn access_delay_enforced() -> bool {
    ACCESS_DELAY_ENFORCED
        .try_with(|enforced| *enforced)
        .unwrap_or(false)
}

/// Run `fut` with the EHDS Art. 3(3) delay of the person's access enforced or
/// not.
pub async fn with_access_delay<F>(enforced: bool, fut: F) -> F::Output
where
    F: Future,
{
    ACCESS_DELAY_ENFORCED.scope(enforced, fut).await
}

#[cfg(test)]
mod tests {
    use super::permit_identifier;

    #[test]
    fn a_permit_identifier_is_recorded_only_in_its_form() {
        assert_eq!(
            permit_identifier(" hdab-nl/2026/0042 ").as_deref(),
            Some("hdab-nl/2026/0042")
        );
        assert_eq!(
            permit_identifier("urn:hdab:permit#7").as_deref(),
            Some("urn:hdab:permit#7")
        );
        assert_eq!(permit_identifier(""), None);
        assert_eq!(permit_identifier("   "), None);
        assert_eq!(permit_identifier("permit for J. Jansen"), None);
        assert_eq!(permit_identifier("vergunning-é"), None);
        assert_eq!(permit_identifier(&"p".repeat(129)), None);
        assert_eq!(
            permit_identifier(&"p".repeat(128)).map(|p| p.len()),
            Some(128)
        );
    }

    #[tokio::test]
    async fn the_declared_permit_is_seen_only_inside_its_scope() {
        assert_eq!(super::current_permit(), None);
        let seen =
            super::with_permit(Some("hdab-1".to_owned()), async { super::current_permit() }).await;
        assert_eq!(seen.as_deref(), Some("hdab-1"));
        assert_eq!(super::current_permit(), None);
    }
}
