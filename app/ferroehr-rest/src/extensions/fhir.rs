// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the **FHIR R4 inbound connector** + mapping-store CRUD
//! over the `ferroehr::service::FhirConnectorAdapter` seam.
//!
//! No openEHR spec governs this — our own enterprise feature, excluded from the
//! ITS-REST drift check. It is a persistence-boundary connector, distinct from
//! the SM Subject Proxy Service (master10): SPS reads subject variables through
//! data-binding frames, whereas this connector commits inbound FHIR resources as
//! COMPOSITIONs and serves them back.
//!
//! Three surfaces, all config-gated (`AppConfig::fhir_api_enabled`, default
//! `false`), answering `404` as an `OperationOutcome` when disabled:
//!
//! * `POST /fhir/r4/{resource_type}` — the inbound connector: the resource's
//!   mapping is resolved by type and `meta.profile`, and a COMPOSITION is built
//!   and committed through the normal validated path with `FEEDER_AUDIT`
//!   provenance. Only `STARTER_RESOURCES` is supported; anything else is a typed
//!   `501`.
//! * `GET /fhir/r4/{resource_type}?patient=…[&_count=N]` — the read façade:
//!   the enabled mappings for the type run a template-bound COMPOSITION query
//!   scoped to the patient, returning a `searchset` Bundle of reverse-mapped
//!   resources. The `patient` scope is mandatory (a missing one is a typed
//!   `400`) and an out-of-scope type is the same `501` as inbound.
//! * `/admin/fhir_mapping[/{id}]` — CRUD over the deployable mapping artefacts,
//!   mounted under `/admin/`, so the coarse RBAC gate classes it `Admin`.
//!
//! Every error on this surface is a FHIR `OperationOutcome` rather than the
//! openEHR error body: this is the FHIR boundary. Validator rejections surface
//! the openEHR validator's message verbatim in `diagnostics` — the CDR's rules
//! win, so a resource mapping to an invalid COMPOSITION is rejected `422` rather
//! than partially stored.

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
use uuid::Uuid;

use ferroehr::service::response::ServiceResponse;
use ferroehr::service::status::SmError;
use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::extensions::access::authz::roles::RbacDecision;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// FHIR R4 media type for the `OperationOutcome` / connector responses.
const FHIR_JSON: &str = "application/fhir+json";

/// The starter resource set the inbound connector maps;
/// anything else is a typed `501 OperationOutcome`.
pub(crate) const STARTER_RESOURCES: &[&str] =
    &["Patient", "Observation", "Condition", "DocumentReference"];

/// The FHIR-connector routes as a native `utoipa-axum` router (group-relative
/// paths; nested under `base_path`). The inbound/façade routes live under
/// `/fhir/r4`, the mapping store under `/admin`. Served through
/// [`guarded_dispatch`] → [`dispatch`]. No openEHR spec governs FHIR interop —
/// our own extension.
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (handlers in a single call must share the path;
    // mixing paths panics at router build with "Overlapping method route").
    OpenApiRouter::new()
        .routes(routes!(fhir_ingest, fhir_search))
        .routes(routes!(fhir_validate))
        // The static /fhir/r4/AuditEvent route wins over the dynamic
        // /fhir/r4/{resource_type} façade route (axum static-first matching).
        .routes(routes!(audit_event_search))
        .routes(routes!(fhir_mapping_list, fhir_mapping_create))
        .routes(routes!(
            fhir_mapping_get,
            fhir_mapping_update,
            fhir_mapping_delete
        ))
}

/// Inbound connector: commit a FHIR R4 resource as an openEHR COMPOSITION
/// (`POST /fhir/r4/{resource_type}`).
///
/// Only the starter set (Patient, Observation, Condition, `DocumentReference`)
/// is supported; anything else is `501`. EVERY response (success and error) is
/// a FHIR `OperationOutcome` in `application/fhir+json`. Config-gated on the
/// FHIR connector: `404` when `fhir_api_enabled` is off.
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines a FHIR connector, a FHIR read facade, or a mapping store,
/// so the whole group (paths, payloads, status codes) is our own design.
#[utoipa::path(
    post, path = "/fhir/r4/{resource_type}", tag = "fhir",
    params(("resource_type" = String, Path, description = "The FHIR R4 resource type (starter set only).")),
    request_body(content = serde_json::Value, description = "A FHIR R4 resource (JSON)."),
    responses(
        (status = 201, description = "Committed as a COMPOSITION (informational OperationOutcome + ETag/Location pointing at the openEHR COMPOSITION).", content_type = "application/fhir+json"),
        (status = 400, description = "The request body is not valid JSON, or a mapping precondition failed (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 404, description = "The FHIR connector is disabled (`fhir_api_enabled` off) (OperationOutcome). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", content_type = "application/fhir+json"),
        (status = 422, description = "Mapped COMPOSITION failed validation (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 501, description = "Resource type outside the starter set (OperationOutcome).", content_type = "application/fhir+json")
    )
)]
pub(crate) async fn fhir_ingest(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "fhir_ingest", parts, dispatch).await
}

/// The ingest door's dry twin: validate a FHIR R4 resource against its
/// mapping WITHOUT committing (`POST /fhir/r4/{resource_type}/$validate`).
///
/// The wire convention is HL7 FHIR R4's own validation operation
/// (`resource-operation-validate`,
/// <https://hl7.org/fhir/R4/resource-operation-validate.html>): the sibling
/// `$validate` path on the ingest door, returning an `OperationOutcome`. A
/// COMPLETED validation is `200` whichever way the verdict falls — the
/// verdict rides the issues: the commit path's rejections VERBATIM as
/// `error` issues, or the valid verdict plus the EHR disposition
/// (`would commit into …` / `would create …` — resolved, never created) as
/// `information` issues. Operation-level failures mirror the ingest door's
/// statuses. Same starter-set scope, same config gate, same access class as
/// the ingest door (its dry twin exists for mapping development).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines a FHIR connector; the operation's wire shape follows
/// HL7 FHIR R4 (official external documentation).
#[utoipa::path(
    post, path = "/fhir/r4/{resource_type}/$validate", tag = "fhir",
    params(("resource_type" = String, Path, description = "The FHIR R4 resource type (starter set only).")),
    request_body(content = serde_json::Value, description = "A FHIR R4 resource (JSON)."),
    responses(
        (status = 200, description = "Validation completed — the OperationOutcome carries the verdict: `information` issues (valid + the EHR disposition) or `error` issues with the commit path's rejections verbatim. Nothing is committed either way.", content_type = "application/fhir+json"),
        (status = 400, description = "The request body is not valid JSON, or a mapping precondition failed (OperationOutcome) — the same class the ingest door refuses `400`.", content_type = "application/fhir+json"),
        (status = 404, description = "No enabled mapping matches the resource type (the ingest door's `404`), or the FHIR connector is disabled (`fhir_api_enabled` off) (OperationOutcome). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", content_type = "application/fhir+json"),
        (status = 501, description = "Resource type outside the starter set (OperationOutcome).", content_type = "application/fhir+json")
    )
)]
pub(crate) async fn fhir_validate(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "fhir_validate", parts, dispatch).await
}

/// Read façade: a patient-scoped FHIR searchset Bundle of reverse-mapped
/// resources (`GET /fhir/r4/{resource_type}`).
///
/// `patient` is mandatory — the façade serves only this explicit scope, never
/// generic FHIR Search. The success Bundle is `application/fhir+json`; errors
/// are FHIR `OperationOutcome`. Config-gated on the FHIR connector: `404` when
/// `fhir_api_enabled` is off.
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines a FHIR connector, a FHIR read facade, or a mapping store,
/// so the whole group (paths, payloads, status codes) is our own design.
#[utoipa::path(
    get, path = "/fhir/r4/{resource_type}", tag = "fhir",
    params(
        ("resource_type" = String, Path, description = "The FHIR R4 resource type (starter set only)."),
        ("patient" = String, Query, description = "The patient scope (EHR subject or id) — required, non-empty."),
        ("_count" = Option<i64>, Query, description = "Optional page size.")
    ),
    responses(
        (status = 200, description = "A FHIR searchset Bundle of reverse-mapped resources.", content_type = "application/fhir+json"),
        (status = 400, description = "The `patient` scope is missing or blank (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 404, description = "The FHIR connector is disabled (`fhir_api_enabled` off) (OperationOutcome). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", content_type = "application/fhir+json"),
        (status = 501, description = "Resource type outside the starter set (OperationOutcome).", content_type = "application/fhir+json")
    )
)]
pub(crate) async fn fhir_search(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "fhir_search", parts, dispatch).await
}

/// The RESTful-ATNA **ITI-81 Retrieve ATNA Audit Event** transaction
/// (`GET /fhir/r4/AuditEvent`): a FHIR search over the local Audit Record
/// Repository, returning a `searchset`
/// Bundle of stored FHIR R4 `AuditEvent` documents (IHE BALP shape). Gated by
/// the local store (`[audit.store]`; 404 when off — independent of the FHIR
/// connector gate) and admin-only under RBAC (the node's security log is an
/// operator surface). Supported parameter subset: `date` (`ge`/`le`
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

/// List the FHIR mapping artefacts — mapping-as-data (`GET /admin/fhir_mapping`).
///
/// Config-gated on the FHIR connector: `404` (a FHIR `OperationOutcome`) when
/// `fhir_api_enabled` is off. The success list is `application/json`.
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines a FHIR connector, a FHIR read facade, or a mapping store,
/// so the whole group (paths, payloads, status codes) is our own design.
#[utoipa::path(
    get, path = "/admin/fhir_mapping", tag = "fhir",
    responses(
        (status = 200, description = "The mapping records.", body = serde_json::Value),
        (status = 404, description = "The FHIR connector is disabled (`fhir_api_enabled` off) (OperationOutcome). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", content_type = "application/fhir+json")
    )
)]
pub(crate) async fn fhir_mapping_list(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "fhir_mapping_list", parts, dispatch).await
}

/// Create a FHIR mapping artefact (`POST /admin/fhir_mapping`).
///
/// Body: `{name, definition, template_id?}`. The success record is
/// `application/json`; every error is a FHIR `OperationOutcome`.
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines a FHIR connector, a FHIR read facade, or a mapping store,
/// so the whole group (paths, payloads, status codes) is our own design.
#[utoipa::path(
    post, path = "/admin/fhir_mapping", tag = "fhir",
    request_body(content = serde_json::Value, description = "The mapping definition (canonical JSON)."),
    responses(
        (status = 201, description = "Created; the stored mapping record is returned.", body = serde_json::Value),
        (status = 400, description = "`name`/`definition` is missing or malformed, the `template_id` is unknown (FK), or the body is not valid JSON (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 409, description = "A mapping with that name already exists (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 415, description = "The request `Content-Type` is not `application/json` (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 404, description = "The FHIR connector is disabled (`fhir_api_enabled` off) (OperationOutcome). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", content_type = "application/fhir+json")
    )
)]
pub(crate) async fn fhir_mapping_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "fhir_mapping_create", parts, dispatch).await
}

/// Read one FHIR mapping artefact by id
/// (`GET /admin/fhir_mapping/{mapping_id}`).
///
/// The success record is `application/json`; errors are FHIR `OperationOutcome`.
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines a FHIR connector, a FHIR read facade, or a mapping store,
/// so the whole group (paths, payloads, status codes) is our own design.
#[utoipa::path(
    get, path = "/admin/fhir_mapping/{mapping_id}", tag = "fhir",
    params(("mapping_id" = String, Path, description = "The mapping UUID.")),
    responses(
        (status = 200, description = "The mapping record.", body = serde_json::Value),
        (status = 400, description = "`mapping_id` is not a valid UUID (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 404, description = "No mapping with that id, or the FHIR connector is disabled (OperationOutcome). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", content_type = "application/fhir+json")
    )
)]
pub(crate) async fn fhir_mapping_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "fhir_mapping_get", parts, dispatch).await
}

/// Update one FHIR mapping artefact (`PUT /admin/fhir_mapping/{mapping_id}`).
///
/// The success record is `application/json`; every error is a FHIR
/// `OperationOutcome`.
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines a FHIR connector, a FHIR read facade, or a mapping store,
/// so the whole group (paths, payloads, status codes) is our own design.
#[utoipa::path(
    put, path = "/admin/fhir_mapping/{mapping_id}", tag = "fhir",
    params(("mapping_id" = String, Path, description = "The mapping UUID.")),
    request_body(content = serde_json::Value, description = "The updated mapping definition (canonical JSON)."),
    responses(
        (status = 200, description = "Updated; the stored mapping record is returned.", body = serde_json::Value),
        (status = 400, description = "`mapping_id` is not a valid UUID, the definition is malformed, the `template_id` is unknown (FK), or the body is not valid JSON (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 409, description = "A mapping with that name already exists (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 415, description = "The request `Content-Type` is not `application/json` (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 404, description = "No mapping with that id, or the FHIR connector is disabled (OperationOutcome). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", content_type = "application/fhir+json")
    )
)]
pub(crate) async fn fhir_mapping_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "fhir_mapping_update", parts, dispatch).await
}

/// Delete one FHIR mapping artefact
/// (`DELETE /admin/fhir_mapping/{mapping_id}`).
///
/// Errors are FHIR `OperationOutcome`.
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines a FHIR connector, a FHIR read facade, or a mapping store,
/// so the whole group (paths, payloads, status codes) is our own design.
#[utoipa::path(
    delete, path = "/admin/fhir_mapping/{mapping_id}", tag = "fhir",
    params(("mapping_id" = String, Path, description = "The mapping UUID.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 400, description = "`mapping_id` is not a valid UUID (OperationOutcome).", content_type = "application/fhir+json"),
        (status = 404, description = "No mapping with that id, or the FHIR connector is disabled (OperationOutcome). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", content_type = "application/fhir+json")
    )
)]
pub(crate) async fn fhir_mapping_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "fhir_mapping_delete", parts, dispatch).await
}

pub(crate) fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move { run(state, op, parts).await })
}

async fn run(state: AppState, op: &'static str, parts: RequestParts) -> Response {
    // The ITI-81 retrieval is the AUDIT surface, not the FHIR connector: its
    // gate is the local Audit Record Repository, independent of
    // `fhir_api_enabled`.
    if op == "audit_event_search" {
        return audit_search(&state, &parts).await;
    }

    // Config gate: opt-in. When disabled every route answers 404 (as an
    // `OperationOutcome`) without consulting the backend.
    if !state.config().fhir_api_enabled {
        return operation_outcome(
            StatusCode::NOT_FOUND,
            "not-supported",
            "the FHIR connector is disabled",
        );
    }

    match op {
        "fhir_ingest" => ingest(&state, &parts).await,
        "fhir_validate" => validate(&state, &parts).await,
        "fhir_search" => search(&state, &parts).await,
        _ => mapping_crud(&state, op, &parts).await,
    }
}

/// The `fhir_mapping_*` CRUD surface over the stored template→resource
/// mappings.
///
/// No openEHR spec governs this — our own design/extension (the FHIR
/// connector's configuration surface).
async fn mapping_crud(state: &AppState, op: &'static str, parts: &RequestParts) -> Response {
    match op {
        "fhir_mapping_list" => mapping_list(state, parts).await,
        "fhir_mapping_create" => mapping_create(state, parts).await,
        "fhir_mapping_get" => mapping_get(state, parts).await,
        "fhir_mapping_update" => mapping_update(state, parts).await,
        "fhir_mapping_delete" => mapping_delete(state, parts).await,
        other => operation_outcome(
            StatusCode::INTERNAL_SERVER_ERROR,
            "exception",
            &format!("unrouted FHIR connector operation: {other}"),
        ),
    }
}

/// `GET /admin/fhir_mapping` — every stored template→resource mapping.
async fn mapping_list(state: &AppState, parts: &RequestParts) -> Response {
    match state.backend().fhir_mapping_list().await {
        Ok(items) => negotiate::respond(&parts.headers, StatusCode::OK, &items),
        Err(e) => sm_error_outcome(e),
    }
}

/// `POST /admin/fhir_mapping` — register one template→resource mapping.
async fn mapping_create(state: &AppState, parts: &RequestParts) -> Response {
    let body = match negotiate::json_value(&parts.headers, &parts.body) {
        Ok(b) => b,
        Err(e) => return api_error_outcome(&e),
    };
    match state.backend().fhir_mapping_create(body).await {
        Ok(created) => negotiate::respond(&parts.headers, StatusCode::CREATED, &created),
        Err(e) => sm_error_outcome(e),
    }
}

/// `GET /admin/fhir_mapping/{mapping_id}` — read one stored mapping.
async fn mapping_get(state: &AppState, parts: &RequestParts) -> Response {
    let id = match mapping_id(parts) {
        Ok(id) => id,
        Err(e) => return api_error_outcome(&e.0),
    };
    match state.backend().fhir_mapping_get(id).await {
        Ok(item) => negotiate::respond(&parts.headers, StatusCode::OK, &item),
        Err(e) => sm_error_outcome(e),
    }
}

/// `PUT /admin/fhir_mapping/{mapping_id}` — replace one stored mapping.
async fn mapping_update(state: &AppState, parts: &RequestParts) -> Response {
    let id = match mapping_id(parts) {
        Ok(id) => id,
        Err(e) => return api_error_outcome(&e.0),
    };
    let body = match negotiate::json_value(&parts.headers, &parts.body) {
        Ok(b) => b,
        Err(e) => return api_error_outcome(&e),
    };
    match state.backend().fhir_mapping_update(id, body).await {
        Ok(updated) => negotiate::respond(&parts.headers, StatusCode::OK, &updated),
        Err(e) => sm_error_outcome(e),
    }
}

/// `DELETE /admin/fhir_mapping/{mapping_id}` — drop one stored mapping.
async fn mapping_delete(state: &AppState, parts: &RequestParts) -> Response {
    let id = match mapping_id(parts) {
        Ok(id) => id,
        Err(e) => return api_error_outcome(&e.0),
    };
    match state.backend().fhir_mapping_delete(id).await {
        Ok(()) => negotiate::empty(StatusCode::NO_CONTENT),
        Err(e) => sm_error_outcome(e),
    }
}

/// Resolve the `{resource_type}` path param + enforce the starter-scope gate
///: a missing param is a routing bug (`500`), an out-of-scope type
/// is a typed `501` before the backend is touched. Shared by inbound + façade.
#[expect(
    clippy::result_large_err,
    reason = "the Err variant is a ready-to-return axum `Response`, which is large \
              by nature; boxing it would only move the allocation"
)]
fn scoped_resource_type(parts: &RequestParts) -> Result<String, Response> {
    let Some(resource_type) = parts.path.get("resource_type").cloned() else {
        return Err(operation_outcome(
            StatusCode::INTERNAL_SERVER_ERROR,
            "exception",
            "missing path parameter `resource_type`",
        ));
    };
    if !STARTER_RESOURCES.contains(&resource_type.as_str()) {
        return Err(operation_outcome(
            StatusCode::NOT_IMPLEMENTED,
            "not-supported",
            &format!(
                "FHIR resource type '{resource_type}' is not supported by the connector \
                 (starter set: {})",
                STARTER_RESOURCES.join(", ")
            ),
        ));
    }
    Ok(resource_type)
}

/// `POST /fhir/r4/{resource_type}` — the inbound connector.
async fn ingest(state: &AppState, parts: &RequestParts) -> Response {
    let resource_type = match scoped_resource_type(parts) {
        Ok(rt) => rt,
        Err(resp) => return resp,
    };
    let body = match negotiate::json_value(&parts.headers, &parts.body) {
        Ok(b) => b,
        Err(e) => return api_error_outcome(&e),
    };
    let profile = first_profile(&body);
    match state
        .backend()
        .fhir_ingest(resource_type, profile, body)
        .await
    {
        Ok(resp) => ingest_created(state, &resp),
        Err(e) => sm_error_outcome(e),
    }
}

/// `POST /fhir/r4/{resource_type}/$validate` — the ingest door's dry twin.
async fn validate(state: &AppState, parts: &RequestParts) -> Response {
    let resource_type = match scoped_resource_type(parts) {
        Ok(rt) => rt,
        Err(resp) => return resp,
    };
    let body = match negotiate::json_value(&parts.headers, &parts.body) {
        Ok(b) => b,
        Err(e) => return api_error_outcome(&e),
    };
    let profile = first_profile(&body);
    match state
        .backend()
        .fhir_validate(resource_type, profile, body)
        .await
    {
        // A completed validation is 200 whichever way the verdict fell (the
        // OperationOutcome carries it); only operation-level failures take
        // the ingest door's statuses.
        Ok(outcome) => fhir_json(StatusCode::OK, &outcome),
        Err(e) => sm_error_outcome(e),
    }
}

/// `GET /fhir/r4/{resource_type}?patient=…[&_count=N]` — the read façade.
async fn search(state: &AppState, parts: &RequestParts) -> Response {
    let resource_type = match scoped_resource_type(parts) {
        Ok(rt) => rt,
        Err(resp) => return resp,
    };
    let q = parts.query.as_deref();
    // `patient` is mandatory: the façade serves only this explicit scope,
    // never generic FHIR Search.
    let Some(patient) =
        crate::overview::params::query_param(q, "patient").filter(|p| !p.is_empty())
    else {
        return operation_outcome(
            StatusCode::BAD_REQUEST,
            "required",
            "the `patient` query parameter is required (the FHIR read façade serves only \
             the explicit patient scope, not generic Search)",
        );
    };
    let count =
        crate::overview::params::query_param(q, "_count").and_then(|c| c.parse::<i64>().ok());
    match state
        .backend()
        .fhir_search(resource_type, patient, count)
        .await
    {
        Ok(bundle) => fhir_json(StatusCode::OK, &bundle),
        Err(e) => sm_error_outcome(e),
    }
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

/// The first `meta.profile` canonical URL on the resource, if any.
fn first_profile(resource: &Value) -> Option<String> {
    resource
        .get("meta")
        .and_then(|m| m.get("profile"))
        .and_then(|p| p.get(0))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The `201` response for a committed resource: an information `OperationOutcome`
/// plus the `ETag`/`Location` headers pointing at the openEHR COMPOSITION (so a
/// client can read it back through the openEHR surface).
fn ingest_created(state: &AppState, resp: &ServiceResponse) -> Response {
    let mut out = operation_outcome(
        StatusCode::CREATED,
        "informational",
        "the FHIR resource was committed as an openEHR COMPOSITION",
    );
    if let Some(meta) = &resp.meta {
        negotiate::set_resource_headers(
            &mut out,
            &state.config().server.base_path,
            Some("composition"),
            meta,
        );
    }
    out
}

/// Parse the `{mapping_id}` path parameter as a UUID → `400` when malformed (a
/// missing param is a routing bug → `500`).
#[expect(
    clippy::map_err_ignore,
    reason = "`uuid::Error` carries only \"this is not a UUID\", which the 400 body \
              already states"
)]
fn mapping_id(parts: &RequestParts) -> Result<Uuid, RestError> {
    let raw = parts.path.get("mapping_id").ok_or_else(|| {
        RestError(ApiError::Internal(
            "missing path parameter `mapping_id`".to_owned(),
        ))
    })?;
    raw.parse::<Uuid>().map_err(|_| {
        RestError(ApiError::BadRequest(format!(
            "invalid FHIR mapping id `{raw}`"
        )))
    })
}

/// Render a FHIR resource (the read-façade Bundle) as `application/fhir+json`.
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

/// Map an [`ApiError`] (a wire-parse failure) to a FHIR `OperationOutcome`.
fn api_error_outcome(e: &ApiError) -> Response {
    let status = e.status();
    operation_outcome(status, fhir_issue_code(status), &e.to_string())
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
