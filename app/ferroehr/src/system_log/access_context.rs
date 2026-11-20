// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The request-scoped purpose of use, published by the protocol adapter and
//! read by platform-layer emitters.
//!
//! The audit middleware stamps the declared purpose onto the records IT
//! builds, but a record the service layer emits by itself — a pseudonymisation
//! boundary crossing, which no HTTP operation id describes — is built where
//! the request headers are already gone. This carries that one fact across, by
//! the same means [`crate::versioning::committer`] carries the authenticated
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
    static REQUEST_ORIGIN: RequestOrigin;
    static DECLARED_PURPOSE: Option<String>;
    static DECLARED_PERMIT: Option<String>;
    static REQUEST_ROLES: Vec<String>;
    static ACCESS_DELAY: AccessDelay;
    static COMMITTED_FACILITIES: FacilitySink;
    static REQUEST_BINDING: Option<crate::system_log::binding::BindingRecord>;
}

/// Where the facilities the compositions of one request name are gathered,
/// for the binding of a system caller's access record.
pub type FacilitySink =
    std::sync::Arc<std::sync::Mutex<Vec<crate::system_log::binding::Identified>>>;

/// Run `fut` gathering into `sink` the facilities its commits name.
pub async fn with_facilities<F>(sink: FacilitySink, fut: F) -> F::Output
where
    F: Future,
{
    COMMITTED_FACILITIES.scope(sink, fut).await
}

/// Record that the current request committed a composition at the facility
/// `issuer` and `id`; outside a gathering scope nothing is recorded.
pub fn record_facility(issuer: &str, id: &str) {
    // NOTE: no specification governs this: our own design; outside a request no record is
    // being gathered, so the facility has nowhere to go.
    let _outside = COMMITTED_FACILITIES.try_with(|sink| {
        sink.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(crate::system_log::binding::Identified {
                issuer: issuer.to_owned(),
                id: id.to_owned(),
            });
    });
}

/// Where the current request came from, as every record of it names it: the
/// server's request id and the client's network address.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestOrigin {
    /// The request id the server minted for the request.
    pub request_id: Option<String>,
    /// The client network address the protocol adapter believed.
    pub client_address: Option<String>,
}

/// The origin of the current request; empty outside a request scope.
#[must_use]
pub fn current_origin() -> RequestOrigin {
    REQUEST_ORIGIN.try_with(Clone::clone).unwrap_or_default()
}

/// Run `fut` with `origin` published as the request's origin, so a record the
/// service layer writes itself names the request and the client as the
/// adapter's own records do.
pub async fn with_origin<F>(origin: RequestOrigin, fut: F) -> F::Output
where
    F: Future,
{
    REQUEST_ORIGIN.scope(origin, fut).await
}

/// The facilities the running request's commits identified so far; empty
/// outside a request.
#[must_use]
pub fn current_facilities() -> Vec<crate::system_log::binding::Identified> {
    COMMITTED_FACILITIES
        .try_with(|sink| {
            sink.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        })
        .unwrap_or_default()
}

/// How the authenticated caller of the current request was bound.
///
/// A record the service layer writes itself names it as the adapter's own
/// records do (EHDS Annex II 3.2(a), (b); #5638); `None` outside a request
/// scope or for a caller no binding names.
#[must_use]
pub fn current_binding() -> Option<crate::system_log::binding::BindingRecord> {
    REQUEST_BINDING.try_with(Clone::clone).ok().flatten()
}

/// Run `fut` with `binding` published as the request's caller binding.
pub async fn with_binding<F>(
    binding: Option<crate::system_log::binding::BindingRecord>,
    fut: F,
) -> F::Output
where
    F: Future,
{
    REQUEST_BINDING.scope(binding, fut).await
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

/// Whether the EHDS Art. 3(3) delay of the person's own access holds for the
/// current request: data a professional has not yet released then reads as
/// absent.
///
/// The protocol adapter publishes `true` for a patient-context request (a
/// SMART `patient/` grant or the configured patient claim) on a deployment
/// that enables `[privacy.access_delay]`, and `false` for every other caller, so a
/// professional reads the record as before. Outside a request scope it is
/// `false` (`docs/law/eu/ehds/text.html` Art. 3(3)). No openEHR spec governs a
/// delay of the person's access: our own design/extension.
#[must_use]
pub fn access_delay_enforced() -> bool {
    access_delay() == AccessDelay::Person
}

/// Whether the EHDS Art. 3(3) delay holds for the current request on `ehr_id`.
///
/// It holds on every EHR for a request that acts for the person, and on the
/// EHRs the request found to be the caller's own records by the party binding
/// (#6108).
#[must_use]
pub fn access_delay_enforced_on(ehr_id: crate::ids::EhrId) -> bool {
    match access_delay() {
        AccessDelay::Off => false,
        AccessDelay::Person => true,
        AccessDelay::Records => crate::access_eval::condition::current()
            .is_some_and(|access| access.is_record(Some(ehr_id))),
    }
}

/// The EHDS Art. 3(3) delay the current request runs under; [`AccessDelay::Off`]
/// outside a request scope.
#[must_use]
pub fn access_delay() -> AccessDelay {
    ACCESS_DELAY
        .try_with(|delay| *delay)
        .unwrap_or(AccessDelay::Off)
}

/// Where the EHDS Art. 3(3) delay of the person's own access holds in one
/// request. No openEHR spec governs it: our own design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccessDelay {
    /// Nowhere: the deployment delays nothing, or the caller is no person.
    #[default]
    Off,
    /// On every EHR: the request acts for the person.
    Person,
    /// On each EHR whose subject is the `PERSON` the caller is bound to, which
    /// it reads as that record's person (#6106).
    Records,
}

/// Run `fut` with the EHDS Art. 3(3) delay of the person's access `delay`.
pub async fn with_access_delay<F>(delay: AccessDelay, fut: F) -> F::Output
where
    F: Future,
{
    ACCESS_DELAY.scope(delay, fut).await
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
    async fn the_request_origin_is_seen_only_inside_its_scope() {
        let origin = super::RequestOrigin {
            request_id: Some("req-1".to_owned()),
            client_address: Some("192.0.2.7".to_owned()),
        };
        assert_eq!(super::current_origin(), super::RequestOrigin::default());
        let seen = super::with_origin(origin.clone(), async { super::current_origin() }).await;
        assert_eq!(seen, origin);
        assert_eq!(super::current_origin(), super::RequestOrigin::default());
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
