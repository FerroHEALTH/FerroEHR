// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The patient scope: every caller is held to the EHR whose subject the
//! access-token claim `[auth] patient_claim` names.
//!
//! The scope applies only when the claim is configured. A token without the
//! claim is refused `403`; an operation that names another patient's EHR in
//! its path, its body or its result is refused `403`; and an AQL query reads
//! only the rows of the caller's patient. The claim is read in
//! `[auth] patient_namespace` when one is set, and names the subject by its id
//! alone otherwise, so an EHR whose subject id another namespace also holds is
//! refused. No openEHR spec governs the patient scope: our own extension.
#![expect(
    clippy::result_large_err,
    reason = "the Err variant of every gate in this module is a ready-to-return axum \
              `Response`, which is large by nature"
)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::response::Response;
use ferroehr::aql::sql::subject::SubjectScope;
use ferroehr::aql::subject::{EhrSubject, TouchedSubjects};
use ferroehr::ids::EhrId;

use crate::api::RequestParts;
use crate::api::operation::operation;
use crate::extensions::access::authn::{Principal, current_principal};
use crate::extensions::access::authz::classify::{Mode, ResourceKind};
use crate::extensions::access::pep::{forbidden, forbidden_unauthenticated, lookup_error};
use crate::params;
use crate::state::AppState;

/// A subject-lookup failure (a database error). Fail-closed: a lookup failure
/// refuses the request (`503` or `500`), never silently admits it.
#[derive(Debug, thiserror::Error)]
#[error("subject lookup failed: {context}")]
pub struct ResolveError {
    /// What was being looked up when the lookup failed.
    context: String,
    /// The lookup failure itself, an `sqlx` driver error or an identifier
    /// refusal, reachable through [`std::error::Error::source`].
    ///
    /// Kept out of the message: a lookup failure renders as a `500`, whose
    /// body must disclose no internal error value (OWASP REST Security Cheat
    /// Sheet §Error handling). The chain goes to the trace record instead.
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl ResolveError {
    /// A lookup failure: what was being looked up, and the error that broke it.
    #[must_use]
    pub fn new(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            context: context.into(),
            source: Box::new(source),
        }
    }
}

/// The future a [`SubjectFn`] returns.
pub type ResolverFuture<T> = Pin<Box<dyn Future<Output = Result<T, ResolveError>> + Send>>;

/// `ehr_id → EHR subject`, or `None` when no EHR has the id.
///
/// The subject is read as the AQL patient scope addresses it
/// ([`ferroehr::aql::subject::ehr_subject`]). Ids are passed as strings so the
/// REST layer stays `uuid`-free (the binary parses); the binary builds the
/// closure over its pool.
pub type SubjectFn = Arc<dyn Fn(String) -> ResolverFuture<Option<EhrSubject>> + Send + Sync>;

/// The configured patient scope: the claim, its namespace and the subject
/// lookup.
#[derive(Clone)]
pub struct PatientScope {
    /// The access-token claim carrying the patient id.
    claim: String,
    /// The subject namespace the claim's ids are read in; `None` reads the id
    /// alone.
    namespace: Option<String>,
    /// The EHR subject lookup.
    subject: SubjectFn,
}

impl std::fmt::Debug for PatientScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatientScope")
            .field("claim", &self.claim)
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl PatientScope {
    /// The scope of `claim`, read in `namespace`, looking EHR subjects up
    /// through `subject`.
    #[must_use]
    pub fn new(claim: &str, namespace: Option<&str>, subject: SubjectFn) -> Self {
        Self {
            claim: claim.to_owned(),
            namespace: namespace.map(str::to_owned),
            subject,
        }
    }

    /// The access-token claim carrying the patient id.
    #[must_use]
    pub fn claim(&self) -> &str {
        &self.claim
    }

    /// The patient id `principal`'s token carries.
    ///
    /// # Errors
    /// A ready `403` when the token lacks the claim: the server is working and
    /// the caller's token is what is insufficient.
    pub(crate) fn patient_of(&self, principal: &Principal) -> Result<String, Response> {
        ferrohealth_smart::roles::claim_text(&principal.claims, &self.claim)
            .map(str::to_owned)
            .ok_or_else(|| {
                forbidden(
                    principal,
                    &format!("the patient scope requires a '{}' claim", self.claim),
                )
            })
    }

    /// Holds the EHR `ehr_id` to `patient`: the claim, read in the configured
    /// namespace, must name the EHR's subject ([`EhrSubject::is_named_by`]),
    /// the rule the query patient scope applies.
    ///
    /// A subject-less EHR is refused; without a configured namespace, so is
    /// one whose subject id another namespace also holds. An id no EHR has
    /// passes, so the route answers it.
    ///
    /// # Errors
    /// A ready `403` on a refusal, a `503` when the subject lookup cannot
    /// obtain a database connection, or a `500` when it fails otherwise.
    pub(crate) async fn hold(
        &self,
        principal: &Principal,
        patient: &str,
        ehr_id: &str,
    ) -> Result<(), Response> {
        let subject = (self.subject)(ehr_id.to_owned())
            .await
            .map_err(|e| lookup_error(principal, "resolve the EHR subject", &e))?;
        match subject {
            None => Ok(()),
            Some(subject) if subject.is_named_by(patient, self.namespace.as_deref()) => Ok(()),
            Some(EhrSubject::Absent) => Err(forbidden(
                principal,
                "patient scope (the EHR records no subject)",
            )),
            Some(EhrSubject::Shared { .. }) if self.namespace.is_none() => Err(forbidden(
                principal,
                "patient scope (the subject id is held under several namespaces)",
            )),
            Some(EhrSubject::Unique { .. } | EhrSubject::Shared { .. }) => {
                Err(forbidden(principal, "patient scope (subject mismatch)"))
            }
        }
    }
}

/// The caller a configured scope holds, with the patient its token names.
type Bound = (PatientScope, Principal, String);

/// The configured patient scope and the authenticated caller it holds, or
/// `None` when no scope is configured.
///
/// # Errors
/// A ready `403` when a scope is configured and the request carries no
/// authenticated caller or the caller's token lacks the claim.
fn bound(state: &AppState) -> Result<Option<Bound>, Response> {
    let Some(scope) = state.authz().and_then(|handle| handle.patient().cloned()) else {
        return Ok(None);
    };
    let principal = current_principal().ok_or_else(|| {
        forbidden_unauthenticated("authentication required for the patient scope")
    })?;
    let patient = scope.patient_of(&principal)?;
    Ok(Some((scope, principal, patient)))
}

/// The patient scope of an operation checked before its handler runs: the
/// EHR its path names, and for `ehr_get_by_subject` the subject it asks for.
///
/// For `ehr_get_by_subject` an absent `subject_id` deliberately passes so the
/// route answers `400`, not `403`: the released specification declares the
/// parameter required (`specifications/parameters/query/subject_id.yaml`), so
/// the request is invalid for every caller (RFC 9110 §15.5.1).
///
/// # Errors
/// A ready `403`, `503` or `500`, as [`PatientScope::hold`].
pub(crate) async fn pre(
    state: &AppState,
    op: &'static str,
    parts: &RequestParts,
) -> Result<(), Response> {
    if operation(op).is_none_or(|row| row.pep != Mode::Pre || row.kind.is_none()) {
        return Ok(());
    }
    let Some((scope, principal, patient)) = bound(state)? else {
        return Ok(());
    };
    if op == "ehr_get_by_subject" {
        return match params::query_param(parts.query.as_deref(), "subject_id") {
            Some(subject) if subject != patient => {
                Err(forbidden(&principal, "patient scope (subject mismatch)"))
            }
            _ => Ok(()),
        };
    }
    match parts.path.get("ehr_id") {
        Some(ehr_id) => scope.hold(&principal, &patient, ehr_id).await,
        None => Ok(()),
    }
}

/// The patient scope of a successful response: the EHR the dispatch recorded
/// it served ([`crate::system_log::middleware::AuditObject`]).
pub(crate) async fn post(state: &AppState, op: &'static str, resp: Response) -> Response {
    if !resp.status().is_success()
        || operation(op).is_none_or(|row| row.pep != Mode::Post || row.kind.is_none())
    {
        return resp;
    }
    let ehr_id = resp
        .extensions()
        .get::<crate::system_log::middleware::AuditObject>()
        .and_then(|object| object.ehr_id.clone());
    let Some(ehr_id) = ehr_id else {
        return resp;
    };
    let (scope, principal, patient) = match bound(state) {
        Ok(Some(bound)) => bound,
        Ok(None) => return resp,
        Err(deny) => return deny,
    };
    match scope.hold(&principal, &patient, &ehr_id).await {
        Ok(()) => resp,
        Err(deny) => deny,
    }
}

/// Holds every EHR an operation names in its body, such as the manifest of
/// `POST /message/export`, to the caller's patient claim, before any of their
/// content is read: the pre-check reads only the path `ehr_id`, so the first
/// EHR outside the claim refuses the whole operation with the `403` its path
/// route would answer.
///
/// # Errors
/// A ready `403` on a refusal or a missing claim, and the `503` or `500` of a
/// failed subject lookup.
pub(crate) async fn each(state: &AppState, ehr_ids: &[EhrId]) -> Result<(), Box<Response>> {
    let Some((scope, principal, patient)) = bound(state).map_err(Box::new)? else {
        return Ok(());
    };
    for ehr_id in ehr_ids {
        scope
            .hold(&principal, &patient, &ehr_id.to_string())
            .await
            .map_err(Box::new)?;
    }
    Ok(())
}

/// Prepares a query execution: the patient subject scope the SQL filters
/// every row to, and whether the executor collects the touched EHRs for
/// [`query_post`].
///
/// `Ok((None, false))` when no patient scope is configured.
///
/// # Errors
/// A ready `403` for a missing caller or claim.
pub(crate) fn query_pre(
    state: &AppState,
    op: &'static str,
) -> Result<(Option<SubjectScope>, bool), Response> {
    if operation(op).and_then(|row| row.kind) != Some(ResourceKind::Query) {
        return Ok((None, false));
    }
    let Some((scope, _, patient)) = bound(state)? else {
        return Ok((None, false));
    };
    Ok((
        Some(SubjectScope {
            id: patient,
            namespace: scope.namespace.clone(),
        }),
        true,
    ))
}

/// Post-checks a query execution: without a configured namespace, a touched
/// subject whose id another namespace also holds is refused, as the direct
/// routes refuse it ([`PatientScope::hold`]); the SQL filter matched the bare
/// id, which names both subjects.
///
/// # Errors
/// A ready `403` when a touched subject id is ambiguous or the caller or
/// claim is missing.
pub(crate) fn query_post(
    state: &AppState,
    op: &'static str,
    outcome: &ferroehr::service::query::request::QueryOutcome,
) -> Result<(), Response> {
    if operation(op).and_then(|row| row.kind) != Some(ResourceKind::Query)
        || outcome.ehr_ids.is_empty()
    {
        return Ok(());
    }
    let Some((scope, principal, _)) = bound(state)? else {
        return Ok(());
    };
    let subjects = TouchedSubjects::fold(&outcome.subjects, scope.namespace.as_deref());
    if subjects.shared.is_empty() {
        Ok(())
    } else {
        Err(forbidden(
            &principal,
            "patient scope (a touched subject id is held under several namespaces)",
        ))
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_types,
        reason = "a test builds the Principal's claim map directly; the claim set \
                  is RFC 7519-open, so the fixtures name serde_json's types"
    )]

    use ferroehr::error::ServiceError;
    use ferrohealth_smart::resource::{Access, Honoured};
    use http::StatusCode;

    use super::*;

    /// A scope whose lookup answers `subject` for every EHR id (`None`: no EHR
    /// has the id).
    fn scope_with_subject(subject: Option<EhrSubject>) -> PatientScope {
        PatientScope::new(
            "patient_id",
            None,
            Arc::new(move |_ehr_id: String| {
                let subject = subject.clone();
                Box::pin(async move { Ok::<_, ResolveError>(subject) })
            }),
        )
    }

    /// A bearer principal carrying `claims`.
    fn bearer_principal(claims: serde_json::Map<String, serde_json::Value>) -> Principal {
        let caller = crate::test_support::person("alice", "").with_claims(claims);
        Principal::new(
            Access::Caller(Arc::new(caller)),
            vec!["USER".to_owned()],
            Honoured::default(),
        )
    }

    fn principal_with_patient(patient: &str) -> Principal {
        let mut claims = serde_json::Map::new();
        claims.insert("patient_id".to_owned(), serde_json::json!(patient));
        bearer_principal(claims)
    }

    /// A subject the claim id names under one namespace.
    fn unique(id: &str) -> EhrSubject {
        EhrSubject::Unique {
            id: id.to_owned(),
            namespace: Some("test.patients".to_owned()),
        }
    }

    #[tokio::test]
    async fn subject_mismatch_denies() {
        let scope = scope_with_subject(Some(unique("P2")));
        let principal = principal_with_patient("P1");
        let err = scope
            .hold(&principal, "P1", "ehr-1")
            .await
            .expect_err("mismatch must deny");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn subject_match_passes() {
        let scope = scope_with_subject(Some(unique("P1")));
        let principal = principal_with_patient("P1");
        assert!(scope.hold(&principal, "P1", "ehr-1").await.is_ok());
    }

    /// A subject-less EHR is outside every patient scope, as the query scope
    /// excludes it.
    #[tokio::test]
    async fn subject_less_ehr_is_refused() {
        let scope = scope_with_subject(Some(EhrSubject::Absent));
        let principal = principal_with_patient("P1");
        let err = scope
            .hold(&principal, "P1", "ehr-1")
            .await
            .expect_err("a subject-less EHR must deny");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    /// The claim carries no namespace, so an id another namespace also holds
    /// names neither subject.
    #[tokio::test]
    async fn subject_id_held_under_two_namespaces_is_refused() {
        let shared = EhrSubject::Shared {
            id: "P1".to_owned(),
            namespace: Some("test.patients".to_owned()),
        };
        let scope = scope_with_subject(Some(shared));
        let principal = principal_with_patient("P1");
        let err = scope
            .hold(&principal, "P1", "ehr-1")
            .await
            .expect_err("an ambiguous subject id must deny");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    /// A subject lookup that cannot obtain a database connection answers `503`
    /// with `Retry-After`: no statement ran, so the request can be repeated.
    #[tokio::test]
    async fn a_lookup_acquire_failure_answers_503() {
        let options = sqlx::postgres::PgConnectOptions::new()
            .host("ferroehr-acquire-test.invalid")
            .username("nobody")
            .database("nothing");
        let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy_with(options);
        pool.close().await;
        let scope = PatientScope::new(
            "patient_id",
            None,
            Arc::new(move |_ehr_id: String| {
                let pool = pool.clone();
                Box::pin(async move {
                    ferroehr::aql::subject::ehr_subject(&pool, EhrId(uuid::Uuid::nil()))
                        .await
                        .map_err(|e| ResolveError::new("ehr subject lookup", ServiceError::from(e)))
                })
            }),
        );
        let principal = principal_with_patient("P1");
        let err = scope
            .hold(&principal, "P1", "ehr-1")
            .await
            .expect_err("an unreachable database must not pass the scope");
        assert_eq!(err.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            err.headers().contains_key(http::header::RETRY_AFTER),
            "a 503 carries Retry-After"
        );
    }

    /// A lookup failure that is not a lost connection stays a `500`.
    #[tokio::test]
    async fn a_lookup_fault_answers_500() {
        let scope = PatientScope::new(
            "patient_id",
            None,
            Arc::new(|_ehr_id: String| {
                Box::pin(async move {
                    Err(ResolveError::new(
                        "ehr subject lookup",
                        ServiceError::exception("lookup failed"),
                    ))
                })
            }),
        );
        let principal = principal_with_patient("P1");
        let err = scope
            .hold(&principal, "P1", "ehr-1")
            .await
            .expect_err("a lookup fault must not pass the scope");
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// With a configured patient namespace the claim names the (id, namespace)
    /// pair, so an id another namespace also holds is no longer ambiguous.
    #[tokio::test]
    async fn a_configured_namespace_names_the_pair() {
        let shared = |namespace: &str| EhrSubject::Shared {
            id: "P1".to_owned(),
            namespace: Some(namespace.to_owned()),
        };
        let principal = principal_with_patient("P1");
        for (subject, passes) in [
            (shared("test.patients"), true),
            (shared("other.patients"), false),
            (unique("P1"), true),
        ] {
            let mut scope = scope_with_subject(Some(subject.clone()));
            scope.namespace = Some("test.patients".to_owned());
            let decided = scope.hold(&principal, "P1", "ehr-1").await;
            assert_eq!(decided.is_ok(), passes, "{subject:?}");
        }
    }

    /// An id no EHR has passes, so the route answers it.
    #[tokio::test]
    async fn unknown_ehr_passes() {
        let scope = scope_with_subject(None);
        let principal = principal_with_patient("P1");
        assert!(scope.hold(&principal, "P1", "ehr-1").await.is_ok());
    }

    #[test]
    fn missing_patient_claim_is_forbidden() {
        let scope = scope_with_subject(Some(unique("P1")));
        let principal = bearer_principal(serde_json::Map::new());
        let err = scope.patient_of(&principal).expect_err("missing claim");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    /// The scope never shows its lookup closure, and names its claim.
    #[test]
    fn debug_names_the_claim() {
        let rendered = format!("{:?}", scope_with_subject(None));
        assert!(rendered.contains("patient_id"), "{rendered}");
    }
}
