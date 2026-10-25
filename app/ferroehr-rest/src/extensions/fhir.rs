// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the RESTful-ATNA **ITI-81** retrieval,
//! `GET /fhir/r4/AuditEvent`, over the local Audit Record Repository.
//!
//! No openEHR spec governs this — our own extension, excluded from the
//! ITS-REST drift check. The SM names `I_SYSTEM_LOG` as "IHE ATNA-compliant"
//! and defines no wire; the route realizes IHE ITI TF-2 transaction ITI-81
//! (Retrieve ATNA Audit Event). FHIR R4 resource mapping is not served here.
//!
//! The route is gated by the local store (`[audit.store]`) and answers `404` as
//! an `OperationOutcome` when it is off. Every error on this surface is a FHIR
//! `OperationOutcome` rather than the openEHR error body: this is a FHIR
//! boundary.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use http::header::{CONTENT_TYPE, HeaderValue};
use ihe_atna::retrieve::Criterion;
use ihe_atna::retrieve::Query;
use ihe_atna::retrieve::TokenSystem;
use ihe_atna::retrieve::Value;
use ihe_atna::retrieve::bundle::{IssueType, Match, refusal, searchset};
use ihe_atna::retrieve::date::Bound;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use ferroehr::service::status::SmError;
use ferroehr::system_log::store::{AuditCriterion, AuditElement, AuditToken, AuditTokenSystem};

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::extensions::access::authz::roles::RbacDecision;
use crate::overview::error::RestError;
use crate::state::AppState;

/// FHIR R4 media type for the `Bundle` / `OperationOutcome` responses.
const FHIR_JSON: &str = "application/fhir+json";

/// The ITI-81 route as a native `utoipa-axum` router (group-relative path;
/// nested under `base_path`). Served through [`guarded_dispatch`] →
/// [`dispatch`].
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(audit_event_search))
}

/// The RESTful-ATNA **ITI-81 Retrieve ATNA Audit Event** transaction
/// (`GET /fhir/r4/AuditEvent`): a FHIR search over the local Audit Record
/// Repository, returning a `searchset` Bundle of stored FHIR R4 `AuditEvent`
/// documents (IHE BALP shape). Gated by the local store (`[audit.store]`; 404
/// when off) and admin-only under RBAC (the node's security log is an operator
/// surface). `ihe-atna` reads the query: at least one `date` is required
/// (`RESTful` ATNA §3.81.4.1.2.1), and `patient`, `agent`, `entity` (or their
/// `.identifier` forms), `outcome`, `action`, `_count` and `_offset` are
/// applied; a parameter the store cannot apply is a `400` `not-supported`,
/// and one `ihe-atna` does not know is ignored (§3.81.4.1.3).
///
/// OUR OWN EXTENSION as far as openEHR is concerned — no openEHR spec governs
/// this: the ITS-REST resource set defines no audit-retrieval endpoint. Its
/// basis is IHE ITI TF-2, transaction **ITI-81 (Retrieve ATNA Audit Event)**,
/// whose RESTful-ATNA option this realizes over the local Audit Record
/// Repository; the SM only names `I_SYSTEM_LOG` as "IHE ATNA-compliant" and
/// defines no wire.
#[utoipa::path(
    get, path = "/fhir/r4/AuditEvent", tag = "audit",
    params(
        ("date" = Vec<String>, Query, description = "Required, at least one (RESTful ATNA §3.81.4.1.2.1): event-time bounds, FHIR date values with the `eq`, `gt`, `lt`, `ge`, `le`, `sa` or `eb` prefix (e.g. `date=ge2026-07-01&date=le2026-07-18T00:00:00Z`)."),
        ("patient" = Option<String>, Query, description = "The recorded patient (EHR subject) id. Required for a caller holding `authz.rbac.subject_audit_role`, which reads the log for one subject at a time."),
        ("agent" = Option<String>, Query, description = "The authenticated principal."),
        ("entity" = Option<String>, Query, description = "The touched resource id."),
        ("outcome" = Option<String>, Query, description = "The outcome indicator: 0, 4, 8 or 12."),
        ("action" = Option<String>, Query, description = "The action code: C, R, U, D or E."),
        ("_count" = Option<i64>, Query, description = "Page size (default 50, max 1000)."),
        ("_offset" = Option<i64>, Query, description = "Page offset.")
    ),
    responses(
        (status = 200, description = "A FHIR searchset Bundle of AuditEvent resources.", content_type = "application/fhir+json"),
        (status = 400, description = "No `date`, a malformed search parameter (`invalid`), or one the local store cannot apply (`not-supported`), as an OperationOutcome.", content_type = "application/fhir+json"),
        (status = 403, description = "Caller lacks the admin role, or holds only the subject-scoped audit role and named no `patient` (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 404, description = "The local audit record repository is disabled (OperationOutcome). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", content_type = "application/fhir+json")
    )
)]
pub(crate) async fn audit_event_search(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "audit_event_search", parts, dispatch).await
}

/// Dispatches the ITI-81 retrieval behind the shared request guard.
pub(crate) fn dispatch(state: AppState, _op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move { audit_search(&state, &parts).await })
}

/// `GET /fhir/r4/AuditEvent` — the ITI-81 retrieval over the local Audit
/// Record Repository.
async fn audit_search(state: &AppState, parts: &RequestParts) -> Response {
    // Gate 1, and it must stay FIRST: authorization precedes availability, or the
    // resource's state becomes a side channel for a caller who may not read this
    // surface at all (#2070). The trail is the node's security-surveillance
    // record (IHE ITI TF-1 §9): the unscoped retrieval is admin-only, and the
    // one narrower grant, `subject_audit_role`, reads it for ONE subject (the
    // `patient` parameter) so a portal serving GDPR Art. 15 / EHDS Art. 9 never
    // holds an admin credential (#3240). No openEHR spec governs it.
    let mut subject_scoped = false;
    if let Some(authz) = state.authz()
        && let Some(rbac) = authz.rbac()
    {
        let roles = crate::extensions::access::authn::current_principal()
            .map(|p| p.roles)
            .unwrap_or_default();
        if let RbacDecision::Deny(reason) = rbac.decide(
            crate::extensions::access::authz::classify::OperationClass::Admin,
            &roles,
        ) {
            if rbac.holds_subject_audit_role(&roles) {
                subject_scoped = true;
            } else {
                return operation_outcome(StatusCode::FORBIDDEN, IssueType::Forbidden, &reason);
            }
        }
    }
    // Gate 2: the local store must be on.
    if !state.backend().audit_search_enabled() {
        return operation_outcome(
            StatusCode::NOT_FOUND,
            IssueType::NotSupported,
            "the local audit record repository is disabled ([audit.store])",
        );
    }

    let filter = match audit_filter(parts.query.as_deref()) {
        Ok(filter) => filter,
        Err(Refused::Invalid(message)) => {
            return operation_outcome(StatusCode::BAD_REQUEST, IssueType::Invalid, &message);
        }
        Err(Refused::NotSupported(message)) => {
            return operation_outcome(StatusCode::BAD_REQUEST, IssueType::NotSupported, &message);
        }
    };
    // The subject-scoped grant is exactly that: a retrieval that names no
    // subject would be the unscoped one, which this role does not hold.
    if subject_scoped && filter.patient.is_none() {
        return operation_outcome(
            StatusCode::FORBIDDEN,
            IssueType::Forbidden,
            "the subject-scoped audit role reads the access log for one subject at a time: \
             the `patient` parameter is required (the unscoped retrieval needs the admin role)",
        );
    }
    match state.backend().audit_event_search(&filter.store).await {
        Ok((total, documents)) => {
            let total_count = u32::try_from(total).unwrap_or(u32::MAX);
            let served = u64::try_from(documents.len()).unwrap_or(u64::MAX);
            let matches = if filter.total_only {
                Vec::new()
            } else {
                match documents
                    .into_iter()
                    .map(|resource| {
                        serde_json::from_value(resource).map(|resource| Match {
                            full_url: None,
                            resource,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(matches) => matches,
                    Err(e) => {
                        tracing::error!(error = %e, "a stored AuditEvent is no FHIR R4 AuditEvent");
                        return operation_outcome(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            IssueType::Exception,
                            "a stored audit record could not be read",
                        );
                    }
                }
            };
            let query = parts.query.as_deref().unwrap_or_default();
            let self_url = format!("AuditEvent?{query}");
            let offset = u64::try_from(filter.store.offset).unwrap_or(0);
            let next_offset = offset.saturating_add(served);
            let next_url = (!filter.total_only
                && served > 0
                && next_offset < u64::try_from(total).unwrap_or(0))
            .then(|| format!("AuditEvent?{}", with_offset(query, next_offset)));
            let bundle = searchset(total_count, &self_url, next_url.as_deref(), matches);
            let mut resp = fhir_json(StatusCode::OK, &bundle);
            // Reading a person's access log is itself an access about that
            // person: the record of this call names whose log was read and how
            // many records it served.
            resp.extensions_mut()
                .insert(crate::system_log::middleware::AuditObject {
                    ehr_id: None,
                    uid: filter.patient.as_ref().map(|p| format!("audit-log:{p}")),
                    result_count: u64::try_from(total).ok(),
                    domain: Some(ferroehr::system_log::event::AccessDomain::System),
                    origins: Vec::new(),
                    origin_count: None,
                    content: None,
                });
            resp
        }
        Err(e) => sm_error_outcome(e),
    }
}

/// Why a query is refused.
enum Refused {
    /// The query does not read (`invalid`).
    Invalid(String),
    /// The query asks for a criterion the local store does not apply
    /// (`not-supported`).
    NotSupported(String),
}

/// The store filter of one query, and what the handler reads beside it.
struct Filter {
    store: ferroehr::system_log::store::AuditSearchFilter,
    patient: Option<String>,
    total_only: bool,
}

/// The largest page served; no specification governs it: our own design (FHIR
/// R4 Search §3.1.1.5.3 lets a server return fewer than `_count`).
const MAX_COUNT: i64 = 1000;

/// The `outcome` code system (FHIR R4 `AuditEvent.outcome`).
const OUTCOME_SYSTEM: &str = "http://hl7.org/fhir/audit-event-outcome";

/// The `action` code system (FHIR R4 `AuditEvent.action`).
const ACTION_SYSTEM: &str = "http://hl7.org/fhir/audit-event-action";

/// Reads the ITI-81 query with `ihe-atna` and turns it into the store
/// filter.
///
/// `ihe-atna` refuses a query with no `date` (`RESTful` ATNA
/// §3.81.4.1.2.1), a modifier, and a value outside its form. A `date` value
/// with no time is read in UTC, the zone the store records in (FHIR R4 Search
/// §3.1.1.4.7 has a server assume its own zone).
fn audit_filter(query: Option<&str>) -> Result<Filter, Refused> {
    let query = Query::parse(query.unwrap_or_default())
        .map_err(|e| Refused::Invalid(ferroehr::service::error::ErrorChain::new(&e).to_string()))?;
    let mut store = ferroehr::system_log::store::AuditSearchFilter {
        // NOTE: the page size when the query names none, our own design.
        count: 50,
        ..Default::default()
    };
    for date in query.dates() {
        match date
            .bounds(&jiff::tz::TimeZone::UTC)
            .map_err(|e| Refused::Invalid(format!("a `date` value is refused: {e}")))?
        {
            Bound::Within { from, until } => {
                if let Some(from) = from {
                    store.from = Some(store.from.map_or(from, |held| held.max(from)));
                }
                if let Some(until) = until {
                    store.until = Some(store.until.map_or(until, |held| held.min(until)));
                }
            }
            Bound::Outside { .. } => {
                return Err(Refused::NotSupported(
                    "the `ne` prefix of `date` is not supported by this repository".to_owned(),
                ));
            }
        }
    }
    for criterion in query.all_criteria() {
        store.criteria.push(criterion_of(criterion)?);
    }
    let patient = sole_patient(&query);
    // NOTE: our own design: `recorded_at` keeps microseconds, so a bound finer than that is
    // widened to the microsecond that holds it, never narrowed past a stored record.
    store.from = store
        .from
        .map(|from| to_micros(from, jiff::RoundMode::Floor));
    if let Some(until) = store.until {
        let rounded = to_micros(until, jiff::RoundMode::Ceil);
        store.until_inclusive = rounded != until;
        store.until = Some(rounded);
    }
    let total_only = query.count() == Some(0);
    if let Some(count) = query.count().filter(|count| *count > 0) {
        store.count = i64::from(count).min(MAX_COUNT);
    }
    if let Some(offset) = query.offset() {
        store.offset = i64::try_from(offset)
            .map_err(|_overflow| Refused::Invalid("`_offset` is out of range".to_owned()))?;
    }
    Ok(Filter {
        store,
        patient,
        total_only,
    })
}

/// The store criterion of one parameter of `RESTful` ATNA §3.81.4.1.2.2:
/// any of its values matches, and a repeated parameter is one more criterion
/// every record must also match (FHIR R4 Search §3.1.1.3).
fn criterion_of(criterion: &Criterion) -> Result<AuditCriterion, Refused> {
    let code = criterion.parameter().code();
    let values = criterion.any_of();
    let texts = || -> Vec<String> {
        values
            .iter()
            .filter_map(|value| match value {
                Value::Reference(text) | Value::String(text) | Value::Uri(text) => {
                    Some(text.clone())
                }
                Value::Token(_) => None,
            })
            .collect()
    };
    let element = |element: AuditElement| -> Result<AuditCriterion, Refused> {
        let tokens = values
            .iter()
            .map(|value| match value {
                Value::Token(token) => Ok(AuditToken {
                    system: match &token.system {
                        TokenSystem::Any => AuditTokenSystem::Any,
                        TokenSystem::Absent => AuditTokenSystem::Absent,
                        TokenSystem::Named(system) => AuditTokenSystem::Named(system.clone()),
                    },
                    code: token.code.clone(),
                }),
                Value::Reference(_) | Value::String(_) | Value::Uri(_) => {
                    Err(Refused::Invalid(format!("`{code}` is not a token")))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(AuditCriterion::Element(element, tokens))
    };
    match code {
        "patient" => Ok(AuditCriterion::Patient(texts())),
        "agent" => Ok(AuditCriterion::Agent(texts())),
        "entity" => Ok(AuditCriterion::Entity(texts())),
        "address" => Ok(AuditCriterion::Address(texts())),
        "patient.identifier" => element(AuditElement::PatientIdentifier),
        "agent.identifier" => element(AuditElement::AgentIdentifier),
        "entity.identifier" => element(AuditElement::EntityIdentifier),
        "source.identifier" => element(AuditElement::SourceIdentifier),
        "entity-type" => element(AuditElement::EntityType),
        "entity-role" => element(AuditElement::EntityRole),
        "type" => element(AuditElement::Type),
        "subtype" => element(AuditElement::Subtype),
        "outcome" => values
            .iter()
            .map(|value| {
                let outcome = token_code(code, value, OUTCOME_SYSTEM)?;
                outcome
                    .parse::<i16>()
                    .ok()
                    .filter(|o| [0, 4, 8, 12].contains(o))
                    .ok_or_else(|| {
                        Refused::Invalid(format!(
                            "invalid `outcome` `{outcome}` (expected 0, 4, 8 or 12)"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(AuditCriterion::Outcome),
        "action" => values
            .iter()
            .map(|value| {
                let action = token_code(code, value, ACTION_SYSTEM)?;
                if ["C", "R", "U", "D", "E"].contains(&action.as_str()) {
                    Ok(action)
                } else {
                    Err(Refused::Invalid(format!(
                        "invalid `action` `{action}` (expected C, R, U, D or E)"
                    )))
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map(AuditCriterion::Action),
        other => Err(Refused::NotSupported(format!(
            "the search parameter `{other}` is not supported by this repository"
        ))),
    }
}

/// The one patient the query is held to, which the subject-scoped audit role
/// requires and the access record names: the single value of the single
/// `patient` or `patient.identifier` parameter, a token naming no system.
fn sole_patient(query: &Query) -> Option<String> {
    let mut named = query.all_criteria().iter().filter(|criterion| {
        matches!(
            criterion.parameter().code(),
            "patient" | "patient.identifier"
        )
    });
    let criterion = named.next()?;
    if named.next().is_some() {
        return None;
    }
    match criterion.any_of() {
        [Value::Reference(text) | Value::String(text) | Value::Uri(text)] => Some(text.clone()),
        [Value::Token(token)] if !matches!(token.system, TokenSystem::Named(_)) => {
            token.code.clone()
        }
        _ => None,
    }
}

/// `at` rounded to the microsecond by `mode`; `at` itself when the rounding
/// overflows.
fn to_micros(at: jiff::Timestamp, mode: jiff::RoundMode) -> jiff::Timestamp {
    at.round(
        jiff::TimestampRound::new()
            .smallest(jiff::Unit::Microsecond)
            .mode(mode),
    )
    .unwrap_or(at)
}

/// The code of a token value, which names no system or `system` (any system
/// when `system` is empty).
fn token_code(code: &str, value: &Value, system: &str) -> Result<String, Refused> {
    let Value::Token(token) = value else {
        return Err(Refused::Invalid(format!("`{code}` is not a token")));
    };
    let system_ok = match &token.system {
        TokenSystem::Any | TokenSystem::Absent => true,
        TokenSystem::Named(named) => !system.is_empty() && named == system,
    };
    if !system_ok {
        return Err(Refused::NotSupported(format!(
            "`{code}` with a code system is not supported by this repository"
        )));
    }
    token.code.clone().ok_or_else(|| {
        Refused::NotSupported(format!(
            "`{code}` with no code is not supported by this repository"
        ))
    })
}

/// `query` with its `_offset` set to `offset`, for the `next` link.
fn with_offset(query: &str, offset: u64) -> String {
    let mut pairs: Vec<String> = query
        .split('&')
        .filter(|pair| !pair.is_empty() && !pair.starts_with("_offset="))
        .map(str::to_owned)
        .collect();
    pairs.push(format!("_offset={offset}"));
    pairs.join("&")
}

/// Render a FHIR resource as `application/fhir+json`.
fn fhir_json<T: serde::Serialize>(status: StatusCode, body: &T) -> Response {
    let mut resp = (status, Json(body)).into_response();
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(FHIR_JSON));
    resp
}

/// A FHIR `OperationOutcome` response of one `error` issue, written by
/// `ihe-atna`, with the given status.
fn operation_outcome(status: StatusCode, code: IssueType, diagnostics: &str) -> Response {
    fhir_json(status, &refusal(code, diagnostics))
}

/// Map an [`SmError`] to a FHIR `OperationOutcome`, reusing the maintained SM →
/// HTTP table ([`RestError`]) for the status and surfacing the message in
/// `diagnostics`.
fn sm_error_outcome(e: SmError) -> Response {
    let message = e.message.clone();
    let status = RestError::from(e).0.status();
    operation_outcome(status, issue_type(status), &message)
}

/// The FHIR `issue.code` (`IssueType`) for an HTTP status.
fn issue_type(status: StatusCode) -> IssueType {
    match status {
        StatusCode::NOT_FOUND => IssueType::NotFound,
        StatusCode::UNPROCESSABLE_ENTITY | StatusCode::BAD_REQUEST => IssueType::Invalid,
        StatusCode::NOT_IMPLEMENTED
        | StatusCode::UNSUPPORTED_MEDIA_TYPE
        | StatusCode::NOT_ACCEPTABLE => IssueType::NotSupported,
        StatusCode::UNAUTHORIZED => IssueType::Login,
        StatusCode::FORBIDDEN => IssueType::Forbidden,
        StatusCode::SERVICE_UNAVAILABLE => IssueType::Transient,
        _ => IssueType::Exception,
    }
}
