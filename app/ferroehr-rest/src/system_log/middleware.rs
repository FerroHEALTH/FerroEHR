// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The IHE ATNA audit tower layer: request, resolved [`AuditEvent`], record.
//!
//! One middleware installed outermost on the API router, wrapping the auth layer
//! so it observes every response, including the auth rejections the inner
//! handlers never see. ATNA requires one record per audited access describing
//! who acted, on what, with what outcome, from where and when; this layer
//! assembles that and hands it to the platform's system log, which follows IHE
//! ATNA. The DICOM PS3.15
//! §A.5 rendering and the syslog transport belong to `ferroehr::system_log`.
//!
//! It reads four response extensions the rest of the stack sets, so no handler
//! carries audit code: [`AuditOpId`] (the matched operation id, from the generic
//! dispatch), [`AuditObject`] (the resource ids, from the `ResourceMeta` the
//! dispatch holds), [`Principal`] (the caller, republished by the auth
//! middleware) and [`ActorAuthentication`] (how the caller authenticated and the
//! natural person behind the credential, from the same middleware). Every
//! mounted operation is classified in [`crate::system_log::classify`]; the
//! response of an operation without a class is refused rather than recorded
//! under a guessed one. Where no `ResourceMeta` exists the participant-object
//! id comes from the request path. An operation whose act wrote its own
//! record ([`EntryScope::Written`]) gets no second operation record, so the
//! store holds one record of it.
//!
//! A genuine authentication additionally records a login (DICOM `EventID`
//! 110114, `EventTypeCode` 110122) unless `suppress_login_events` is on. Genuine
//! means a real authentication event: the auth layer marks it only on a Basic
//! verified-credential cache miss, where the credentials were actually checked.
//! Auth rejections always record a failure, with a refused credential's
//! caller `UNKNOWN` (a patient-data step-up `401` carries its caller) —
//! surveillance of failed access is the point of ATNA.
//!
//! Every record of the request is written durably, in one transaction of the
//! local audit store, before the response leaves; a record that cannot be
//! written turns the response into `503`, so no response reports an access
//! the log does not hold (#5747).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6): FHIR resources are an external standard \
              with no RM type (typed-FHIR evaluation tracked separately)"
)]

use std::sync::Arc;

use axum::extract::{FromRequestParts, RawPathParams, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ferrohealth_server::client_address::ClientAddress;
use http::{HeaderValue, StatusCode, header};
use openehr_its::rest::runtime::ApiError;

use ferroehr::access_eval::condition::RequestAccess;
use ferroehr::system_log::access_context;
use ferroehr::system_log::categories::{AccessedContent, ResourceKind};
use ferroehr::system_log::event::{
    AccessDecision, AccessDomain, ActorAuthentication, AuditEvent, EntryScope, EventActionCode,
    EventOutcome, EventType, ObjectClass, QueryNaming,
};
use ferroehr::system_log::recorder::WriteRecorder;

use crate::extensions::access::authn::{FreshAuthentication, Principal};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::system_log::classify::audit_for;

/// The matched ITS-REST operation id, inserted by the dispatch layer onto the
/// response. Present only when a request reached the generic dispatcher.
///
/// The extension routers are mounted through the same `crate::api::mount` as the
/// generated groups, which inserts this uniformly, so extension traffic arrives
/// carrying its operation id, each classified in
/// [`crate::system_log::classify`].
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
    /// The query the operation ran, named and never quoted.
    pub query: Option<QueryNaming>,
    /// Whether the operation's record writes an access-log entry by its
    /// domain or as a read of the access log itself, or whether the act wrote
    /// wrote the record itself ([`EntryScope::Written`]), so the
    /// middleware writes none.
    pub entry: EntryScope,
}

/// The access-logging facts every record of one request shares: the request
/// correlation id, the declared purpose of use and the declared data permit.
#[derive(Debug, Clone, Default)]
pub(super) struct AccessContext {
    request_id: Option<String>,
    purpose: Option<String>,
    permit: Option<String>,
}

/// The facts of one request every operation record of it names: where it
/// came from, what it addressed, when it arrived and its access context.
#[derive(Debug, Clone)]
pub(super) struct RequestFacts {
    pub(super) client_ip: Option<String>,
    pub(super) path: String,
    pub(super) route_ehr_id: Option<String>,
    pub(super) timestamp: jiff::Timestamp,
    pub(super) access: AccessContext,
}

/// The caller as the authentication layer published it on the response: the
/// principal, and how it authenticated.
#[derive(Debug, Clone, Copy)]
pub(super) struct Caller<'a> {
    pub(super) principal: Option<&'a Principal>,
    pub(super) actor: Option<&'a ActorAuthentication>,
    pub(super) binding: Option<&'a ferroehr::system_log::binding::BindingRecord>,
    /// The access evaluator of the request, whose decisions each record
    /// names ([`ferroehr::access_eval::record`]).
    pub(super) access: Option<&'a Arc<RequestAccess>>,
}

impl<'a> Caller<'a> {
    /// Reads the caller off a response.
    fn of(resp: &'a Response) -> Self {
        Self {
            principal: resp.extensions().get::<Arc<Principal>>().map(Arc::as_ref),
            actor: resp.extensions().get::<ActorAuthentication>(),
            binding: resp
                .extensions()
                .get::<ferroehr::system_log::binding::BindingRecord>(),
            access: resp.extensions().get::<Arc<RequestAccess>>(),
        }
    }

    /// Stamp `event` with the decision the access evaluator gave the EHR it
    /// names; nothing for a request the evaluator did not judge.
    pub(super) fn stamp_decision(self, event: &mut AuditEvent) {
        if let Some(access) = self.access {
            ferroehr::access_eval::record::stamp(event, access);
        }
    }
}

/// Stamp the shared access-logging fields onto one record.
///
/// The legal basis is a deployment fact rather than a request one, so it is
/// read from configuration here rather than carried per request.
pub(super) fn fill_access(event: &mut AuditEvent, access: &AccessContext, state: &AppState) {
    event.request_id.clone_from(&access.request_id);
    event.purpose.clone_from(&access.purpose);
    event.permit.clone_from(&access.permit);
    event.legal_basis = state.backend().audit_legal_basis().map(str::to_owned);
}

/// Stamp what the handler said it touched onto the operation record: the
/// EHR, the object, the served count and origins, the query, the domain and
/// whether the record writes an access-log entry. The route's EHR and the
/// path's object stand in where the handler said nothing.
fn fill_object(
    event: &mut AuditEvent,
    object: Option<&AuditObject>,
    route_ehr_id: Option<&str>,
    op: &str,
    path: &str,
) {
    // NOTE: EHDS Art. 9(2)(c) (`docs/law/eu/ehds/text.html`), the record names
    // whose data was accessed, so the subject-scoped ITI-81 retrieval finds it.
    event.ehr_id = object
        .and_then(|o| o.ehr_id.clone())
        .or_else(|| route_ehr_id.map(str::to_owned));
    event.object_id = object
        .and_then(|o| o.uid.clone())
        .or_else(|| object_id_from_path(op, path));
    let Some(object) = object else {
        return;
    };
    event.result_count = object.result_count;
    event.record_origins(object.origins.clone(), object.origin_count);
    event.query.clone_from(&object.query);
    event.entry = object.entry;
    // A handler that knows its domain better than the resource class does
    // says so; the class decides for everything else.
    if let Some(domain) = object.domain {
        event.domain = domain;
    }
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

/// The data permit the caller declared it acts under (`[audit] permit_header`),
/// in the form [`ferroehr::system_log::access_context::permit_identifier`]
/// accepts; anything else is recorded as absent.
fn declared_permit(req: &Request, backend: &ferroehr::service::FerroEhrService) -> Option<String> {
    let header = backend.audit_permit_header()?;
    let declared = req.headers().get(header)?.to_str().ok()?;
    access_context::permit_identifier(declared)
}

/// The ATNA audit middleware, routing emission through the platform's
/// `SystemLog` component.
pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // The system-log master switch: disabled means no per-request work.
    if !state.backend().audit_enabled() {
        return next.run(req).await;
    }

    // Capture request-side facts before the request is consumed.
    let client_ip = client_ip(&req);
    let path = req.uri().path().to_owned();
    let (req, route_ehr_id) = route_ehr_id(req).await;
    let timestamp = jiff::Timestamp::now();
    let purpose = declared_purpose(&req, state.backend());
    let permit = declared_permit(&req, state.backend());
    // The server's own request id, which the family stack mints for every
    // request and the request log names; a client's `X-Request-Id` is echoed
    // to it and never recorded.
    let request_id = req
        .extensions()
        .get::<ferrohealth_telemetry::request_log::RequestId>()
        .map(|id| id.id().to_owned());

    let facts = RequestFacts {
        client_ip,
        path,
        route_ehr_id,
        timestamp,
        access: AccessContext {
            request_id,
            purpose,
            permit,
        },
    };
    let recorder = WriteRecorder::default();
    recorder.prime(crate::system_log::written::builder(&state, &facts));
    let resp = run_recorded(recorder.clone(), req, next, &facts).await;
    let status = resp.status();

    let op = resp.extensions().get::<AuditOpId>().copied();
    let caller = Caller::of(&resp);
    let object = resp.extensions().get::<AuditObject>().cloned();

    let fresh_auth = resp.extensions().get::<FreshAuthentication>().is_some();
    let refusal = crate::system_log::refusals::is_refusal(&resp);

    let mut records = Vec::new();
    // NOTE: our own design (#5602, #5762), the act or its commit wrote the operation record
    // itself, so a second one would be a second record of one event.
    let written = recorder.holds_operation_record()
        || object
            .as_ref()
            .is_some_and(|object| object.entry == EntryScope::Written);
    let mut op_recorded = written;

    let classified = op.map(|AuditOpId(op)| (op, audit_for(op)));
    if let Some((op, Some(classes))) = classified
        && !written
    {
        op_recorded = true;
        records.push(operation_record(
            &state,
            op,
            classes,
            status,
            refusal,
            caller,
            &facts,
            object.as_ref(),
        ));
        records.extend(crate::system_log::served::records(
            &state,
            &resp,
            op,
            caller,
            facts.client_ip.as_deref(),
            facts.timestamp,
            &facts.access,
        ));
    }

    // The authentication record: a rejected access attempt is a failed
    // user-authentication event (DICOM PS3.15 §A.5.1 `EventID` 110114 with
    // `EventTypeCode` 110122), and the success record fires only on a genuine
    // authentication event, gated by `suppress_login_events`.
    if refusal {
        records.extend(crate::system_log::refusals::refused(
            &state,
            &resp,
            caller,
            facts.client_ip,
            facts.timestamp,
            &facts.access,
            facts.route_ehr_id,
            op_recorded,
        ));
    } else if fresh_auth && !state.backend().suppress_login_events() {
        records.push(login(caller, facts.client_ip, facts.timestamp));
    }

    // NOTE: our own design (#5762), the records of what the request committed are held until
    // here, and are released whatever this response becomes, since the writes committed.
    state.backend().confirm_reserved(&recorder.reserved());
    if let Some((op, None)) = classified {
        return unclassified(op);
    }
    // NOTE: our own design (#5747), an access whose record cannot be written durably is
    // refused, so no response reports an access the log does not hold; the sender logs why.
    if state.backend().record_events(records).await.is_err() {
        let mut resp = RestError::from(ApiError::ServiceUnavailable(
            "audit trail unavailable".to_owned(),
        ))
        .into_response();
        resp.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        return resp;
    }
    resp
}

/// The operation record of `op`, classified as `classes`, for a response of
/// `status`: the caller, what the handler said it touched and the request's
/// shared facts.
#[expect(
    clippy::too_many_arguments,
    reason = "the record's parts as the middleware and the commit choke point both hold them"
)]
pub(super) fn operation_record(
    state: &AppState,
    op: &'static str,
    (action, object_class): (EventActionCode, ObjectClass),
    status: StatusCode,
    refusal: bool,
    caller: Caller<'_>,
    facts: &RequestFacts,
    object: Option<&AuditObject>,
) -> AuditEvent {
    let mut event = AuditEvent::new(action, object_class, outcome_from_status(status));
    // The concrete operation is the `EventTypeCode` (DICOM PS3.15 §A.5
    // EventIdentification); the id is ours.
    event.event_type = Some(EventType::rest(op));
    if refusal {
        event.decision = AccessDecision::Refused;
    }
    fill_common(&mut event, caller, facts.client_ip.clone(), facts.timestamp);
    fill_object(
        &mut event,
        object,
        facts.route_ehr_id.as_deref(),
        op,
        &facts.path,
    );
    caller.stamp_decision(&mut event);
    // NOTE: our own design (#5606), an evaluator refusal refuses EHR data, so its record
    // writes an access-log entry whatever the operation's resource class.
    if event.decided.as_ref().is_some_and(|d| d.refused.is_some())
        && event.domain != AccessDomain::Ehr
        && event.entry == EntryScope::ByDomain
    {
        event.entry = EntryScope::Refusal;
    }
    categorise(&mut event, state, op, object);
    fill_access(&mut event, &facts.access, state);
    event
}

/// Run the rest of the stack under the request's write `recorder`
/// ([`run_published`]).
async fn run_recorded(
    recorder: WriteRecorder,
    req: Request,
    next: Next,
    facts: &RequestFacts,
) -> Response {
    // NOTE: boxed, so the stack below the audit layer carries a pointer, not the request.
    Box::pin(crate::system_log::written::run(
        recorder,
        Box::pin(run_published(
            req,
            next,
            &facts.access,
            facts.client_ip.clone(),
        )),
    ))
    .await
}

/// Run the rest of the stack with the request's origin and declared purpose
/// and permit published for its task scope, so a record the service layer
/// emits by itself (a pseudonymisation boundary crossing, or an act that
/// writes its own record) names them as this middleware's own records do
/// (`ferroehr::system_log::access_context`).
async fn run_published(
    req: Request,
    next: Next,
    access: &AccessContext,
    client_address: Option<String>,
) -> Response {
    let origin = access_context::RequestOrigin {
        request_id: access.request_id.clone(),
        client_address,
    };
    let declared =
        access_context::with_declared(access.purpose.clone(), access.permit.clone(), next.run(req));
    access_context::with_origin(origin, declared).await
}

/// Stamp the EHDS priority category of what a successful operation `op`
/// touched onto its record (Annex II 3.2(c)).
fn categorise(event: &mut AuditEvent, state: &AppState, op: &str, object: Option<&AuditObject>) {
    if event.outcome != EventOutcome::Success {
        return;
    }
    let content = object.and_then(|o| o.content.clone());
    if let Some(accessed) = accessed_content(op, event.object, content) {
        event.category = state.backend().audit_classify(&accessed, event.action);
    }
}

/// The record of a genuine authentication (DICOM PS3.15 §A.5.1 `EventID`
/// 110114 with `EventTypeCode` 110122 "Login").
fn login(caller: Caller<'_>, client_ip: Option<String>, at: jiff::Timestamp) -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Authentication,
        EventOutcome::Success,
    );
    event.event_type = Some(EventType::Login);
    fill_common(&mut event, caller, client_ip, at);
    event
}

/// The refusal of the response of `op`, an operation without an audit
/// classification: it is never recorded under a guessed class (#5602).
fn unclassified(op: &str) -> Response {
    tracing::error!(
        operation = op,
        "the operation has no audit classification; its response is refused"
    );
    RestError::from(ApiError::Internal(format!(
        "the operation {op} has no audit classification"
    )))
    .into_response()
}

/// What an operation touched, for the record's EHDS priority category
/// (Annex II 3.2(c), #3621).
///
/// Item tags and revision histories are marked by the operation, whatever the
/// handler attached; then the handler's own content decides; then the resource
/// class marks the EHR itself and its directory. `EHR`, `EHR_STATUS`, `EHR_ACCESS`, the
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
    if op.starts_with("versioned_ehr_access") {
        return Some(AccessedContent::ResourceKind(ResourceKind::EhrAccess));
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
pub(super) fn outcome_from_status(status: StatusCode) -> EventOutcome {
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
pub(super) fn fill_common(
    event: &mut AuditEvent,
    caller: Caller<'_>,
    client_ip: Option<String>,
    timestamp: jiff::Timestamp,
) {
    let principal = caller.principal;
    event.user_id = principal.map(|p| p.subject.clone()).unwrap_or_default();
    // The token's issuer and client (RFC 7519 §4.1.1, RFC 9068 §2.2): who
    // vouched for the person and which application they used, as the BALP
    // `agent:user` names them.
    event.issuer = principal.and_then(|p| claim(p, "iss"));
    event.client_id = principal.and_then(recorded_client);
    // The roles the person held (NEN 7513's role element, #3239): the RFC 9068
    // §2.2.3.1 claim carriers for a bearer, the user definition for Basic.
    event.roles = principal.map(|p| p.roles.clone()).unwrap_or_default();
    // The purposes and roles the token states (IHE ITI TF-2 §3.71.4.2.2.1.1).
    event.token = principal.and_then(crate::extensions::access::authn::purpose::token_attributes);
    event.user_is_requestor = true;
    // The bearer token's `jti` (RFC 7519 §4.1.7) is the minimal token identity
    // IHE BALP `OAUTHaccessTokenUse.Minimal` records; token contents are never
    // logged.
    event.token_id = principal.and_then(|principal| claim(principal, "jti"));
    // How the person authenticated and who that person is (EHDS Annex II 3.1
    // and 3.2(b)), as the authentication layer judged it.
    event.actor = caller.actor.cloned().unwrap_or_default();
    event.binding = caller.binding.cloned();
    event.client_ip = client_ip;
    event.timestamp = timestamp;
}

/// The client a record names: the one the caller's issuer profile reads from
/// `client_claims`, which the classification and the policy grants admitted,
/// never another client claim the token carries (#5194).
fn recorded_client(principal: &Principal) -> Option<String> {
    principal
        .access
        .caller()
        .and_then(|caller| caller.client_id())
        .map(str::to_owned)
}

/// Returns the string claim `name` of a Bearer principal's validated token,
/// if present and non-empty.
fn claim(principal: &Principal, name: &str) -> Option<String> {
    principal
        .claims
        .get(name)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Returns the client network address for the DICOM `NetworkAccessPointID`:
/// the address the family stack named (`ferrohealth_server::client_address`),
/// which believes a forwarded hop only from a `[server] trusted_proxies` peer
/// (RFC 7239 §8.1), and `None` for a request served over no connection.
fn client_ip(req: &Request) -> Option<String> {
    req.extensions()
        .get::<ClientAddress>()
        .map(|ClientAddress(address)| address.to_string())
}

/// The route parameter every route that addresses one EHR names it by.
const EHR_PARAMETER: &str = "ehr_id";

/// The EHR the matched route names: its `{ehr_id}` path parameter.
///
/// The ITS-REST routes under `/ehr/{ehr_id}` and the extension routes that
/// address one EHR (`/admin/ehr/{ehr_id}`, `/message/export/{ehr_id}`,
/// `/message/import/{ehr_id}`) all name it by that parameter, so the router's
/// own match decides and a literal segment such as `/admin/ehr/all` is never
/// taken for an id. `POST /ehr` has none; its EHR id comes from the response
/// `AuditObject`.
///
/// The request is handed back whole, for the layers inside to serve.
async fn route_ehr_id(req: Request) -> (Request, Option<String>) {
    let (mut parts, body) = req.into_parts();
    // NOTE: no specification governs this: our own design; a rejection means the
    // route matched no parameter or a non-UTF-8 one the handler refuses itself.
    let ehr_id = RawPathParams::from_request_parts(&mut parts, &())
        .await
        .ok()
        .and_then(|params| {
            params
                .iter()
                .find(|(name, _)| *name == EHR_PARAMETER)
                .map(|(_, value)| value.to_owned())
        });
    (Request::from_parts(parts, body), ehr_id)
}

/// Derives the participant-object id from the request path for operations whose
/// dispatch carries no `ResourceMeta`: template ids, qualified stored-query
/// names, and demographic party uids, the physical party delete's included.
///
/// `None` for list-style operations and for ad-hoc queries, which have no single
/// object.
fn object_id_from_path(op: &str, path: &str) -> Option<String> {
    match op {
        _ if op.starts_with("definition_template_adl1.4") => segment_after(path, "adl1.4"),
        "definition_template_adl14_convert" => segment_after(path, "adl1.4"),
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
        "admin_party_delete" => segment_after(path, "party"),
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
mod tests;
