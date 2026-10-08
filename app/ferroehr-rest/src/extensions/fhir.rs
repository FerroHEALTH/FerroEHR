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

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6): FHIR resources are an external standard \
              with no RM type (typed-FHIR evaluation tracked separately)"
)]

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use http::header::{CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use ferroehr::service::status::SmError;

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
/// surface). Supported parameter subset: `date` (`ge`/`le`
/// prefixes), `patient`, `agent`, `entity`, `outcome`, `action`, `_count`,
/// `_offset`; other FHIR search parameters are ignored (lenient search).
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
        ("date" = Option<Vec<String>>, Query, description = "Event-time bound(s), `ge`/`le`-prefixed RFC 3339 instants (e.g. `date=ge2026-07-01T00:00:00Z&date=le2026-07-18T00:00:00Z`)."),
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
        (status = 400, description = "Malformed search parameter (OperationOutcome).", content_type = "application/fhir+json"),
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
                return operation_outcome(StatusCode::FORBIDDEN, "forbidden", &reason);
            }
        }
    }
    // Gate 2: the local store must be on.
    if !state.backend().audit_search_enabled() {
        return operation_outcome(
            StatusCode::NOT_FOUND,
            "not-supported",
            "the local audit record repository is disabled ([audit.store])",
        );
    }

    let filter = match audit_filter(parts.query.as_deref()) {
        Ok(filter) => filter,
        Err(message) => {
            return operation_outcome(StatusCode::BAD_REQUEST, "invalid", &message);
        }
    };
    // The subject-scoped grant is exactly that: a retrieval that names no
    // subject would be the unscoped one, which this role does not hold.
    if subject_scoped && filter.patient.is_none() {
        return operation_outcome(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the subject-scoped audit role reads the access log for one subject at a time: \
             the `patient` parameter is required (the unscoped retrieval needs the admin role)",
        );
    }
    match state.backend().audit_event_search(&filter).await {
        Ok((total, documents)) => {
            let entries: Vec<Value> = documents
                .into_iter()
                .map(|resource| json!({ "resource": resource, "search": { "mode": "match" } }))
                .collect();
            let bundle = json!({
                "resourceType": "Bundle",
                "type": "searchset",
                "total": total,
                "entry": entries,
            });
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

/// Parse the supported ITI-81 parameter subset from the query string. Unknown
/// parameters are ignored (FHIR lenient search); malformed values of the
/// supported ones are an error (`400`).
fn audit_filter(
    query: Option<&str>,
) -> Result<ferroehr::system_log::store::AuditSearchFilter, String> {
    let mut filter = ferroehr::system_log::store::AuditSearchFilter {
        count: 50,
        ..Default::default()
    };
    for (key, value) in query_pairs(query) {
        match key.as_str() {
            "date" => {
                // FHIR date-prefix grammar; the supported subset is ge/le.
                let (prefix, instant) = value.split_at_checked(2).unwrap_or(("", ""));
                let parsed = instant
                    .parse::<jiff::Timestamp>()
                    .map_err(|e| format!("invalid `date` value `{value}`: {e}"))?;
                match prefix {
                    "ge" => filter.from = Some(parsed),
                    "le" => filter.to = Some(parsed),
                    _ => {
                        return Err(format!(
                            "unsupported `date` prefix in `{value}` (supported: ge, le)"
                        ));
                    }
                }
            }
            "patient" => filter.patient = Some(value),
            "agent" => filter.agent = Some(value),
            "entity" => filter.entity = Some(value),
            "outcome" => {
                let outcome = value
                    .parse::<i16>()
                    .ok()
                    .filter(|o| [0, 4, 8, 12].contains(o))
                    .ok_or_else(|| {
                        format!("invalid `outcome` `{value}` (expected 0, 4, 8 or 12)")
                    })?;
                filter.outcome = Some(outcome);
            }
            "action" => {
                if !["C", "R", "U", "D", "E"].contains(&value.as_str()) {
                    return Err(format!("invalid `action` `{value}` (expected C/R/U/D/E)"));
                }
                filter.action = Some(value);
            }
            "_count" => {
                filter.count = value
                    .parse::<i64>()
                    .ok()
                    .filter(|c| *c > 0)
                    .ok_or_else(|| format!("invalid `_count` `{value}`"))?;
            }
            "_offset" => {
                filter.offset = value
                    .parse::<i64>()
                    .ok()
                    .filter(|o| *o >= 0)
                    .ok_or_else(|| format!("invalid `_offset` `{value}`"))?;
            }
            _ => {} // lenient search: unknown parameters are ignored
        }
    }
    Ok(filter)
}

/// Decode the query string into `(key, value)` pairs (percent-decoding via
/// the `urlencoding` crate — the house rule for all percent codecs).
fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            let key = urlencoding::decode(key).ok()?.into_owned();
            let value = urlencoding::decode(&value.replace('+', " "))
                .ok()?
                .into_owned();
            Some((key, value))
        })
        .collect()
}

/// Render a FHIR resource (the searchset Bundle) as `application/fhir+json`.
fn fhir_json(status: StatusCode, body: &Value) -> Response {
    let mut resp = (status, Json(body.clone())).into_response();
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(FHIR_JSON));
    resp
}

/// Build a FHIR `OperationOutcome` response (`severity`/`code`/`diagnostics`)
/// with `Content-Type: application/fhir+json` and the given status.
fn operation_outcome(status: StatusCode, code: &str, diagnostics: &str) -> Response {
    let severity = if status.is_success() {
        "information"
    } else {
        "error"
    };
    let body = json!({
        "resourceType": "OperationOutcome",
        "issue": [{ "severity": severity, "code": code, "diagnostics": diagnostics }],
    });
    let mut resp = (status, Json(body)).into_response();
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(FHIR_JSON));
    resp
}

/// Map an [`SmError`] to a FHIR `OperationOutcome`, reusing the maintained SM →
/// HTTP table ([`RestError`]) for the status and surfacing the message (e.g. a
/// validator rejection) verbatim in `diagnostics`.
fn sm_error_outcome(e: SmError) -> Response {
    let message = e.message.clone();
    let status = RestError::from(e).0.status();
    operation_outcome(status, fhir_issue_code(status), &message)
}

/// The FHIR `issue.code` (`IssueType`) for an HTTP status.
fn fhir_issue_code(status: StatusCode) -> &'static str {
    match status {
        StatusCode::NOT_FOUND => "not-found",
        StatusCode::UNPROCESSABLE_ENTITY | StatusCode::BAD_REQUEST => "invalid",
        StatusCode::CONFLICT | StatusCode::PRECONDITION_FAILED => "conflict",
        StatusCode::NOT_IMPLEMENTED
        | StatusCode::UNSUPPORTED_MEDIA_TYPE
        | StatusCode::NOT_ACCEPTABLE => "not-supported",
        StatusCode::UNAUTHORIZED => "login",
        StatusCode::FORBIDDEN => "forbidden",
        _ => "exception",
    }
}
