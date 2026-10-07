// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The IHE ATNA audit tower layer: request, resolved [`AuditEvent`], emit.
//!
//! One middleware installed outermost on the API router, wrapping the auth layer
//! so it observes every response, including the auth rejections the inner
//! handlers never see. ATNA requires one record per audited access describing
//! who acted, on what, with what outcome, from where and when; this layer
//! assembles that and hands it to the SM System Log component, the only
//! normative openEHR requirement being "System Log | IHE ATNA-compliant system
//! log" (SM `master02-overview.adoc` §openEHR Platform Model). The DICOM PS3.15
//! §A.5 rendering and the syslog transport belong to `ferroehr::system_log`.
//!
//! It reads three response extensions the rest of the stack sets, so no handler
//! carries audit code: [`AuditOpId`] (the matched operation id, from the generic
//! dispatch), [`AuditObject`] (the resource ids, from the `ResourceMeta` the
//! dispatch holds) and [`Principal`] (the caller, republished by the auth
//! middleware). Every generated operation is classified in
//! [`crate::system_log::classify`], and an id with no explicit entry fails closed
//! to the audited default rather than being dropped; where no `ResourceMeta`
//! exists the participant-object id comes from the request path.
//!
//! A genuine authentication additionally emits a login record (DICOM `EventID`
//! 110114, `EventTypeCode` 110122) unless `suppress_login_events` is on. Genuine
//! means a real authentication event: the auth layer marks it only on a Basic
//! verified-credential cache miss, where the credentials were actually checked.
//! Auth rejections always emit a failure record, with a 401's caller `UNKNOWN` —
//! surveillance of failed access is the point of ATNA.
//!
//! Emission is non-blocking (`try_send` onto a bounded queue), so the request
//! path never blocks on auditing. Under `fail_mode=closed` a rejected operation
//! record turns the response into `503`; the auth records never gate it.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6): FHIR resources are an external standard \
              with no RM type (typed-FHIR evaluation tracked separately)"
)]

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::{HeaderValue, StatusCode, header};
use openehr_its::rest::runtime::ApiError;

use ferroehr::system_log::categories::{AccessedContent, ContentIds, ResourceKind};
use ferroehr::system_log::event::{
    AccessDomain, AuditEvent, EmitOutcome, EventActionCode, EventOutcome, EventType, ObjectClass,
};

use crate::extensions::access::authn::{FreshAuthentication, Principal};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::system_log::classify::audit_for;

/// The matched ITS-REST operation id, inserted by the dispatch layer onto the
/// response. Present only when a request reached the generic dispatcher.
///
/// The extension routers are mounted through the same `crate::api::mount` as the
/// generated groups, which inserts this uniformly, so extension traffic arrives
/// carrying its operation id. Those ids have no explicit entry in
/// [`crate::system_log::classify`], which fails an unrecognised id closed to the
/// audited default, so the operation record still fires.
#[derive(Debug, Clone, Copy)]
pub struct AuditOpId(pub &'static str);

/// The resource ids a handler touched, populated from the `ResourceMeta` the
/// dispatch already holds. Present for header-bearing operations.
#[derive(Debug, Clone, Default)]
pub struct AuditObject {
    /// The owning EHR id (for subject enrichment + the patient participant).
    pub ehr_id: Option<String>,
    /// The resource identifier (version uid / contribution uid / object URI).
    pub uid: Option<String>,
    /// How many records the operation served, for an operation that serves a
    /// countable set: AQL result rows, search entries. NEN 7513 and EHDS
    /// Art. 9 ask what was accessed, and for a query the answer is a volume as
    /// much as a resource.
    pub result_count: Option<u64>,
    /// The pseudonymisation domain this response read, when the handler knows
    /// it better than the resource class does.
    ///
    /// The released OAS reuses one CONTRIBUTION `operationId` across the EHR
    /// and DEMOGRAPHIC bundles, so the operation alone cannot say which domain
    /// a contribution read touched; the demographic group sets this and the
    /// class decides for everything else.
    pub domain: Option<AccessDomain>,
    /// The distinct origins of the data this response served and their true
    /// count (EHDS Annex II 3.2(e), #3212), read from the version metadata the
    /// handler holds; empty when the operation serves no version body.
    pub origins: Vec<String>,
    /// The true number of distinct origins, when the set above was capped.
    pub origin_count: Option<u64>,
    /// What the response served or wrote, for the record's EHDS priority
    /// category (Annex II 3.2(c), #3621): the template and root archetype ids
    /// of the objects, or a query's positive constraints. `None` leaves the
    /// operation and the resource class to decide.
    pub content: Option<AccessedContent>,
}

/// The EHRs a query served, each with its served-row count and the
/// identifiers of the versions it served from that EHR.
///
/// A response extension the query dispatch sets and the audit middleware
/// reads: an AQL statement is one operation but potentially many accesses, and
/// NEN 7513 and EHDS Art. 9 both ask which records were accessed. Carrying it
/// here rather than emitting from the service keeps the caller identity in the
/// one place that has it.
#[derive(Debug, Clone)]
pub struct AuditServedEhrs {
    /// The served EHRs with their served-row counts, in first-served order.
    pub served: Vec<(String, u64)>,
    /// The identifiers of the versions served from each EHR, keyed by EHR id.
    pub content: BTreeMap<String, Vec<ContentIds>>,
}

/// The access-logging facts every record of one request shares: the request
/// correlation id, the declared purpose of use, and the organisation the
/// caller acted for.
#[derive(Debug, Clone, Default)]
struct AccessContext {
    request_id: Option<String>,
    purpose: Option<String>,
    organisation: Option<String>,
}

/// Stamp the shared access-logging fields onto one record.
///
/// The legal basis is a deployment fact rather than a request one, so it is
/// read from configuration here rather than carried per request.
fn fill_access(event: &mut AuditEvent, access: &AccessContext, state: &AppState) {
    event.request_id.clone_from(&access.request_id);
    event.purpose.clone_from(&access.purpose);
    event.organisation.clone_from(&access.organisation);
    event.legal_basis = state.backend().audit_legal_basis().map(str::to_owned);
}

/// Emit one access record per EHR a query served, beside the statement's own
/// operation record.
///
/// The statement record answers "who ran what"; these answer "whose record was
/// disclosed", which is the question NEN 7513 and EHDS Art. 9 put per record.
/// Returns whether any record was refused under `fail_mode = closed`.
fn emit_served_ehr_records(
    state: &AppState,
    resp: &Response,
    op: &'static str,
    principal: Option<&Principal>,
    client_ip: Option<&str>,
    timestamp: jiff::Timestamp,
    access: &AccessContext,
) -> bool {
    let Some(AuditServedEhrs { served, content }) = resp.extensions().get::<AuditServedEhrs>()
    else {
        return false;
    };
    let mut rejected = false;
    for (ehr_id, rows) in served {
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        event.event_type = Some(EventType::RestOperation(op));
        fill_common(
            &mut event,
            principal,
            client_ip.map(str::to_owned),
            timestamp,
        );
        event.ehr_id = Some(ehr_id.clone());
        event.object_id = Some(ehr_id.clone());
        event.result_count = Some(*rows);
        event.category = content.get(ehr_id).and_then(|objects| {
            state
                .backend()
                .audit_classify(&AccessedContent::Objects(objects.clone()))
        });
        fill_access(&mut event, access, state);
        rejected |= state.backend().emit(event) == EmitOutcome::Rejected;
    }
    rejected
}

/// The purpose of use the caller declared, when the deployment records it.
///
/// The header name and the accepted vocabulary are deployment configuration
/// (`[audit] purpose_header` / `purpose_codes`). A declared code outside a
/// configured vocabulary is recorded as absent rather than verbatim: an
/// unagreed string in the trail reads at review time as though a purpose was
/// established when none was.
fn declared_purpose(req: &Request, backend: &ferroehr::service::FerroEhrService) -> Option<String> {
    let header = backend.audit_purpose_header()?;
    let declared = req.headers().get(header)?.to_str().ok()?.trim();
    backend
        .audit_accepts_purpose(declared)
        .then(|| declared.to_owned())
}

/// The organisation the caller acted for, from the identity token.
///
/// The claim is `[authz.abac] organization_claim`, resolved the same way the
/// policy-enforcement point resolves it ([`crate::extensions::access::pep`]).
/// One setting serves both: a second one could name a different claim than
/// authorization decides on, and the trail would then record an organisation
/// the deployment never granted anything to.
///
/// The setting is shared; the switch is not. It is read from the adapter's
/// configuration rather than off the live ABAC gate, so the trail records the
/// organisation whether or not that gate is enabled — an access log that went
/// blank because a policy layer was disabled would answer a different question
/// than the one it is kept for. A Basic-authenticated caller carries no claims,
/// so it resolves to `None`, as does an unconfigured claim — never a guess.
fn caller_organisation(state: &AppState, principal: Option<&Principal>) -> Option<String> {
    let principal = principal?;
    let claim = state.config().audit_organization_claim.as_deref()?;
    crate::extensions::access::authz::roles::claim_string(&principal.claims, claim)
}

/// The ATNA audit middleware, routing emission through the platform's SM
/// `SystemLog` component.
pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // The SM System Log master switch: disabled means no per-request work.
    if !state.backend().audit_enabled() {
        return next.run(req).await;
    }

    // Capture request-side facts before the request is consumed.
    let client_ip = client_ip(&req);
    let path = req.uri().path().to_owned();
    let timestamp = jiff::Timestamp::now();
    let purpose = declared_purpose(&req, state.backend());
    // The correlation id `SetRequestIdLayer::x_request_id` stamped on the request;
    // the response gains it only from `PropagateRequestIdLayer`, which runs
    // outside this layer.
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    // Published for the handler's task scope so a record the SERVICE layer
    // emits by itself — a pseudonymisation boundary crossing, which no
    // operation id describes — carries the same declared purpose this
    // middleware stamps on its own records
    // (`ferroehr::system_log::access_context`).
    let resp =
        ferroehr::system_log::access_context::with_purpose(purpose.clone(), next.run(req)).await;
    let status = resp.status();

    let op = resp.extensions().get::<AuditOpId>().copied();
    let principal = resp.extensions().get::<Principal>().cloned();
    let object = resp.extensions().get::<AuditObject>().cloned();

    let access = AccessContext {
        request_id,
        purpose,
        organisation: caller_organisation(&state, principal.as_ref()),
    };

    let fresh_auth = resp.extensions().get::<FreshAuthentication>().is_some();

    let mut op_rejected = false;

    // The operation record: `audit_for` fails an unrecognised op id closed to
    // the audited default, so only a deliberate opt-out yields `None`.
    if let Some(AuditOpId(op)) = op
        && let Some((action, object_class)) = audit_for(op)
    {
        let mut event = AuditEvent::new(action, object_class, outcome_from_status(status));
        // The concrete operation is the `EventTypeCode` (DICOM PS3.15 §A.5
        // EventIdentification); the id is ours.
        event.event_type = Some(EventType::RestOperation(op));
        fill_common(&mut event, principal.as_ref(), client_ip.clone(), timestamp);
        event.ehr_id = object
            .as_ref()
            .and_then(|o| o.ehr_id.clone())
            .or_else(|| ehr_id_from_path(&path));
        event.object_id = object
            .as_ref()
            .and_then(|o| o.uid.clone())
            .or_else(|| object_id_from_path(op, &path));
        event.result_count = object.as_ref().and_then(|o| o.result_count);
        if let Some(object) = object.as_ref() {
            event.record_origins(object.origins.clone(), object.origin_count);
        }
        if event.outcome == EventOutcome::Success
            && let Some(accessed) = accessed_content(
                op,
                object_class,
                object.as_ref().and_then(|o| o.content.clone()),
            )
        {
            event.category = state.backend().audit_classify(&accessed);
        }
        // A handler that knows its domain better than the resource class does
        // says so; the class decides for everything else.
        if let Some(domain) = object.as_ref().and_then(|o| o.domain) {
            event.domain = domain;
        }
        fill_access(&mut event, &access, &state);
        op_rejected = state.backend().emit(event) == EmitOutcome::Rejected;
        op_rejected |= emit_served_ehr_records(
            &state,
            &resp,
            op,
            principal.as_ref(),
            client_ip.as_deref(),
            timestamp,
            &access,
        );
    }

    // The authentication record: a rejected access attempt is a failed
    // user-authentication event (DICOM PS3.15 §A.5.1 `EventID` 110114 with
    // `EventTypeCode` 110122), and the success record fires only on a genuine
    // authentication event, gated by `suppress_login_events`.
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        let mut event = AuditEvent::new(
            EventActionCode::Execute,
            ObjectClass::Authentication,
            outcome_from_status(status),
        );
        event.event_type = Some(EventType::Login);
        // A 401 has no principal, so the caller is `UNKNOWN`; a 403 has one.
        fill_common(&mut event, principal.as_ref(), client_ip, timestamp);
        let _ = state.backend().emit(event);
    } else if fresh_auth && !state.backend().suppress_login_events() {
        let mut event = AuditEvent::new(
            EventActionCode::Execute,
            ObjectClass::Authentication,
            EventOutcome::Success,
        );
        event.event_type = Some(EventType::Login);
        fill_common(&mut event, principal.as_ref(), client_ip, timestamp);
        let _ = state.backend().emit(event);
    }

    if op_rejected {
        // Fail-closed: an auditable operation whose record cannot be delivered
        // must not be reported as having succeeded. No openEHR spec governs the
        // error-body shape (a MAY, `Requests_and_responses.md` §HTTP status
        // codes) — our own design keeps every error path consistent.
        let mut resp = RestError(ApiError::ServiceUnavailable(
            "audit trail unavailable (fail-closed)".to_owned(),
        ))
        .into_response();
        resp.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        return resp;
    }
    resp
}

/// What an operation touched, for the record's EHDS priority category
/// (Annex II 3.2(c), #3621).
///
/// Item tags and revision histories are marked by the operation, whatever the
/// handler attached; then the handler's own content decides; then the resource
/// class marks the EHR itself and its directory. `EHR`, `EHR_STATUS`, the
/// directory `FOLDER`, item tags and revision histories hold no
/// priority-category data. `None` for an operation outside the clinical
/// domain, or one that touched no classifiable content. No openEHR spec
/// governs access-log classification — our own design/extension.
fn accessed_content(
    op: &str,
    object_class: ObjectClass,
    content: Option<AccessedContent>,
) -> Option<AccessedContent> {
    if AccessDomain::of(object_class) != AccessDomain::Ehr || object_class == ObjectClass::Extract {
        return None;
    }
    if op.contains("_tags_") {
        return Some(AccessedContent::ResourceKind(ResourceKind::ItemTag));
    }
    if op.ends_with("revision_history") {
        return Some(AccessedContent::ResourceKind(ResourceKind::RevisionHistory));
    }
    if op.starts_with("versioned_ehr_status") {
        return Some(AccessedContent::ResourceKind(ResourceKind::EhrStatus));
    }
    match (content, object_class) {
        (Some(content), _) => Some(content),
        (None, ObjectClass::Ehr) => Some(AccessedContent::ResourceKind(ResourceKind::Ehr)),
        (None, ObjectClass::Directory) => Some(AccessedContent::ResourceKind(ResourceKind::Folder)),
        (None, _) => None,
    }
}

/// Maps an HTTP status to the DICOM `EventOutcomeIndicator` (PS3.15 §A.5.1).
///
/// 1xx to 3xx is `0` success (`304 Not Modified` is a successful conditional
/// read, RFC 9110 §15.4), 4xx is `4` minor failure, 5xx is `8` serious failure.
/// `12` major failure denotes a system-level abnormal termination and is not
/// inferable from an HTTP status, so it is never emitted here.
fn outcome_from_status(status: StatusCode) -> EventOutcome {
    if status.is_informational() || status.is_success() || status.is_redirection() {
        EventOutcome::Success
    } else if status.is_server_error() {
        EventOutcome::SeriousFailure
    } else {
        EventOutcome::MinorFailure
    }
}

/// Fills the participant-common fields: the requesting user (the DICOM source
/// `ActiveParticipant`), its network address, and the event time.
///
/// An absent principal leaves `user_id` empty, which the emitter renders as the
/// configured value-if-missing.
fn fill_common(
    event: &mut AuditEvent,
    principal: Option<&Principal>,
    client_ip: Option<String>,
    timestamp: jiff::Timestamp,
) {
    event.user_id = principal.map(|p| p.subject.clone()).unwrap_or_default();
    // The roles the person held (NEN 7513's role element, #3239): the RFC 9068
    // §2.2.3.1 claim carriers for a bearer, the user definition for Basic.
    event.roles = principal.map(|p| p.roles.clone()).unwrap_or_default();
    event.user_is_requestor = true;
    // The bearer token's `jti` (RFC 7519 §4.1.7) is the minimal token identity
    // IHE BALP `OAUTHaccessTokenUse.Minimal` records; token contents are never
    // logged.
    event.token_id = principal.and_then(token_id);
    event.client_ip = client_ip;
    event.timestamp = timestamp;
}

/// Returns the `jti` claim of a Bearer principal's validated token, if present
/// and a non-empty string.
fn token_id(principal: &Principal) -> Option<String> {
    principal
        .claims
        .get("jti")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Returns the client network address for the DICOM `NetworkAccessPointID`: the
/// `X-Forwarded-For` first hop, else the TCP peer address, else `None`.
fn client_ip(req: &Request) -> Option<String> {
    if let Some(xff) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        && let Some(first) = xff.split(',').next()
    {
        let trimmed = first.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
}

/// Extracts the `ehr_id` path parameter, the segment after the first exact `ehr`
/// segment.
///
/// `POST /ehr` has none, so its EHR id comes from the response `AuditObject`.
fn ehr_id_from_path(path: &str) -> Option<String> {
    segment_after(path, "ehr")
}

/// Derives the participant-object id from the request path for operations whose
/// dispatch carries no `ResourceMeta`: template ids, qualified stored-query
/// names, and demographic party uids.
///
/// `None` for list-style operations and for ad-hoc queries, which have no single
/// object.
fn object_id_from_path(op: &str, path: &str) -> Option<String> {
    match op {
        _ if op.starts_with("definition_template_adl1.4") => segment_after(path, "adl1.4"),
        _ if op.starts_with("definition_template_adl2") => segment_after(path, "adl2"),
        // /definition/query/{name} and /query/{name} — but never /query/aql.
        _ if op.starts_with("definition_query") => segment_after(path, "query"),
        "query_execute_stored_query"
        | "query_execute_stored_query_body"
        | "query_execute_stored_query_version"
        | "query_execute_stored_query_version_body" => segment_after(path, "query"),
        _ if op.starts_with("agent_") => segment_after(path, "agent"),
        _ if op.starts_with("group_") => segment_after(path, "group"),
        _ if op.starts_with("organisation_") => segment_after(path, "organisation"),
        _ if op.starts_with("person_") => segment_after(path, "person"),
        _ if op.starts_with("role_") => segment_after(path, "role"),
        _ if op.starts_with("versioned_party_") => segment_after(path, "versioned_party"),
        _ => None,
    }
}

/// Returns the path segment immediately following the first exact `marker`.
fn segment_after(path: &str, marker: &str) -> Option<String> {
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    while let Some(seg) = segments.next() {
        if seg == marker {
            return segments.next().filter(|s| !s.is_empty()).map(str::to_owned);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_codes_track_http_status() {
        assert_eq!(outcome_from_status(StatusCode::OK), EventOutcome::Success);
        assert_eq!(
            outcome_from_status(StatusCode::NO_CONTENT),
            EventOutcome::Success
        );
        // 304 Not Modified is a successful conditional read (RFC 9110 §15.4.5);
        // redirection is not a failed action.
        assert_eq!(
            outcome_from_status(StatusCode::NOT_MODIFIED),
            EventOutcome::Success
        );
        assert_eq!(
            outcome_from_status(StatusCode::BAD_REQUEST),
            EventOutcome::MinorFailure
        );
        assert_eq!(
            outcome_from_status(StatusCode::UNAUTHORIZED),
            EventOutcome::MinorFailure
        );
        assert_eq!(
            outcome_from_status(StatusCode::FORBIDDEN),
            EventOutcome::MinorFailure
        );
        assert_eq!(
            outcome_from_status(StatusCode::NOT_FOUND),
            EventOutcome::MinorFailure
        );
        assert_eq!(
            outcome_from_status(StatusCode::INTERNAL_SERVER_ERROR),
            EventOutcome::SeriousFailure
        );
        assert_eq!(
            outcome_from_status(StatusCode::SERVICE_UNAVAILABLE),
            EventOutcome::SeriousFailure
        );
    }

    #[test]
    fn ehr_id_from_various_paths() {
        assert_eq!(
            ehr_id_from_path("/ferroehr/rest/openehr/v1/ehr/abc-123/composition/x"),
            Some("abc-123".to_owned())
        );
        assert_eq!(
            ehr_id_from_path("/ferroehr/rest/openehr/v1/ehr/abc-123"),
            Some("abc-123".to_owned())
        );
        // Create has no id segment.
        assert_eq!(ehr_id_from_path("/ferroehr/rest/openehr/v1/ehr"), None);
        // Admin delete carries the id after `ehr`.
        assert_eq!(
            ehr_id_from_path("/ferroehr/rest/openehr/v1/admin/ehr/zzz"),
            Some("zzz".to_owned())
        );
        // "ehr_status" / "versioned_ehr_status" are not the exact `ehr` segment.
        assert_eq!(ehr_id_from_path("/definition/template/adl1.4"), None);
    }

    #[test]
    fn object_id_from_template_paths() {
        assert_eq!(
            object_id_from_path(
                "definition_template_adl1.4_get",
                "/x/definition/template/adl1.4/vital_signs.v1"
            ),
            Some("vital_signs.v1".to_owned())
        );
        // Upload/list have no id segment.
        assert_eq!(
            object_id_from_path(
                "definition_template_adl1.4_upload",
                "/x/definition/template/adl1.4"
            ),
            None
        );
        assert_eq!(
            object_id_from_path(
                "definition_template_adl2_version_get",
                "/x/definition/template/adl2/t.v1/1.0.0"
            ),
            Some("t.v1".to_owned())
        );
    }

    #[test]
    fn object_id_from_query_paths() {
        // Stored query execution: the qualified name is the search criteria.
        assert_eq!(
            object_id_from_path("query_execute_stored_query", "/x/query/eu.ferroehr::q1"),
            Some("eu.ferroehr::q1".to_owned())
        );
        assert_eq!(
            object_id_from_path("definition_query_store.yaml", "/x/definition/query/org::q2"),
            Some("org::q2".to_owned())
        );
        // Ad-hoc queries stay UNKNOWN (no extraction), and so does the cohort
        // extension: `cohort` is a route segment, not a stored-query name.
        assert_eq!(
            object_id_from_path("query_execute_adhoc_query", "/x/query/aql"),
            None
        );
        assert_eq!(
            object_id_from_path("query_execute_cohort", "/x/query/cohort"),
            None
        );
    }

    #[test]
    fn object_id_from_demographic_paths() {
        assert_eq!(
            object_id_from_path("person_get", "/x/demographic/person/p-1"),
            Some("p-1".to_owned())
        );
        assert_eq!(
            object_id_from_path("role_tags_delete", "/x/demographic/role/r-9/tags/k"),
            Some("r-9".to_owned())
        );
        assert_eq!(
            object_id_from_path("versioned_party_get", "/x/demographic/versioned_party/vp-3"),
            Some("vp-3".to_owned())
        );
        // Create has no id segment.
        assert_eq!(
            object_id_from_path("person_create", "/x/demographic/person"),
            None
        );
    }
}
