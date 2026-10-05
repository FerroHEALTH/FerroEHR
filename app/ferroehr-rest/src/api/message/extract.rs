// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_EHR_EXTRACT_SERVICE` over the MESSAGE extension wire — **our own
//! extension** (see [`super`] for the whole group's spec-silence flag).
//!
//! The four SM operations (`i_ehr_extract_service.adoc`) and the routes that
//! realize them:
//!
//! | SM operation | route |
//! |---|---|
//! | `export_ehrs(an_ehr_id)` | `GET /message/export/{ehr_id}` |
//! | `export_ehr_extracts(extract_spec)` | `POST /message/export` |
//! | `import_ehr(an_ehr_id[0..1], an_extract)` | `POST /message/import{?ehr_id}` |
//! | `import_ehr_extract(an_ehr_id, an_extract)` | `POST /message/import/{ehr_id}` |
//!
//! Both exports return `List<EXTRACT>`, so both answer `200` with a JSON array
//! of canonical `EXTRACT`s (RM `ehr_extract` `master05` — the array IS the
//! resource; no spec defines an envelope, so none is invented). `import_ehr`
//! creates an EHR and answers `201` naming it; `import_ehr_extract` adds
//! versions to an existing one and answers `204`.
//!
//! NOTE (no openEHR spec governs role semantics on an unspecified route — our
//! own design/extension): the shared authentication + RBAC layer answers before
//! any handler runs, so every route carries `401` and every route the coarse
//! gate classifies as a WRITE carries `403` for the configured read-only role.
//! `POST /message/export` is not one: SM `I_EHR_EXTRACT_SERVICE.export_ehr_extracts`
//! commits nothing, so it is pinned as a read
//! (`ferroehr-rest::extensions::access::authz::EXTENSION_READ_ROUTES`), like the
//! released ad-hoc AQL `POST`; the import routes stay writes.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;
use openehr_rm::v1_2::ehr_extract::common::extract::Extract;
use openehr_rm::v1_2::ehr_extract::common::extract_spec::ExtractSpec;

use ferroehr::ids::EhrId;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::params;
use crate::state::AppState;

/// The EHR-Extract extension routes as a native `utoipa-axum` router — **no
/// ITS-REST contract** (see the module docs). Group-relative paths (nested
/// under `base_path`); every operation runs through [`guarded_dispatch`] with
/// [`dispatch`].
pub(crate) fn extract_routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (handlers in a single call must share the path).
    OpenApiRouter::new()
        .routes(routes!(message_export_ehrs))
        .routes(routes!(message_export_ehr_extracts))
        .routes(routes!(message_import_ehr))
        .routes(routes!(message_import_ehr_extract))
}

/// Export one whole EHR as EXTRACTs (`GET /message/export/{ehr_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_EHR_EXTRACT_SERVICE.export_ehrs`.
#[utoipa::path(
    get, path = "/message/export/{ehr_id}", tag = "message",
    params(("ehr_id" = String, Path, description = "The EHR to export.")),
    responses(
        (status = 200, description = "The SM `List<EXTRACT>` — one canonical \
                                      `EXTRACT` carrying every versioned object \
                                      of the EHR, latest versions only \
                                      (`extract_version_spec.adoc`: \"By \
                                      default, only latest versions are \
                                      included\").",
         body = Vec<serde_json::Value>),
        (status = 400, description = "`ehr_id` is not a well-formed UUID — SM \
                                      `precondition_violation`. Refused before \
                                      any lookup: a malformed identifier is \
                                      never resolved against the store.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Refused before the path \
                                      parameter is even parsed.",
         body = serde_json::Value),
        (status = 404, description = "SM `ehr_id_does_not_exist`.",
         body = serde_json::Value),
        (status = 406, description = "No representation satisfies `Accept` \
                                      (the extract list is JSON only).",
         body = serde_json::Value)
    )
)]
pub(crate) async fn message_export_ehrs(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "message_export_ehrs", parts, dispatch).await
}

/// Export EXTRACTs by `EXTRACT_SPEC` (`POST /message/export`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM
/// `I_EHR_EXTRACT_SERVICE.export_ehr_extracts`. A read modelled as `POST`
/// because its selector is a whole `EXTRACT_SPEC` structure, exactly as the
/// released ad-hoc AQL read is (`query_execute_post`).
#[utoipa::path(
    post, path = "/message/export", tag = "message",
    request_body(content = serde_json::Value,
                 description = "A canonical `EXTRACT_SPEC` (RM `ehr_extract` \
                                `extract_spec.adoc`): the mandatory `manifest` \
                                naming the entities (each by `ehr_id` or \
                                `subject_id`, optionally narrowed by \
                                `item_list`), the `extract_type` coded term, \
                                and the optional `version_spec`.",
                 example = json!({
                     "_type": "EXTRACT_SPEC",
                     "extract_type": {
                         "_type": "DV_CODED_TEXT",
                         "value": "openEHR EHR",
                         "defining_code": {
                             "_type": "CODE_PHRASE",
                             "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                             "code_string": "803"
                         }
                     },
                     "include_multimedia": false,
                     "priority": 0,
                     "link_depth": 0,
                     "manifest": {
                         "_type": "EXTRACT_MANIFEST",
                         "entities": [ {
                             "_type": "EXTRACT_ENTITY_MANIFEST",
                             "extract_id_key": "7d44b88c-4199-4bad-97dc-d78268e01398",
                             "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398"
                         } ]
                     }
                 })),
    responses(
        (status = 200, description = "The SM `List<EXTRACT>` — one canonical \
                                      `EXTRACT` per manifest entity, in \
                                      manifest order (`EXTRACT_MANIFEST.\
                                      entities` is `1..*`, so the list is \
                                      never empty).",
         body = Vec<serde_json::Value>),
        (status = 400, description = "The body is not a well-formed \
                                      `EXTRACT_SPEC`, an entity names neither \
                                      `ehr_id` nor `subject_id`, an \
                                      `extract_type` outside the \
                                      extract-content-type codes RM ehr_extract \
                                      `master04-common_package.adoc` names \
                                      (`openehr-ehr`, `openehr-demographic`, \
                                      `openehr-synchronisation`, \
                                      `openehr-generic`, `generic-emr`, plus \
                                      the catch-all `other`), or a \
                                      selection this service does not support \
                                      (`criteria`, an unsupported \
                                      `commit_time_interval`) — SM \
                                      `precondition_violation`.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 404, description = "An entity's `ehr_id`/`subject_id` \
                                      resolves to no EHR (SM \
                                      `ehr_id_does_not_exist`), or an \
                                      `item_list` entry names no version \
                                      container in that EHR (SM \
                                      `versioned_object_does_not_exist`).",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is neither \
                                      `application/json` nor `application/xml`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn message_export_ehr_extracts(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "message_export_ehr_extracts", parts, dispatch).await
}

/// Import a whole EHR from an EXTRACT (`POST /message/import{?ehr_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_EHR_EXTRACT_SERVICE.import_ehr`, whose
/// `an_ehr_id` is `[0..1]` — hence a query parameter rather than a path
/// segment: the resource being created is the EHR, and the caller MAY fix its
/// identifier (RM common `master06` §Copying Case 1).
#[utoipa::path(
    post, path = "/message/import", tag = "message",
    params(("ehr_id" = Option<String>, Query,
            description = "Optional: create the clone under THIS identifier \
                           (the SM's \"same patient in other EHR services\" \
                           case). Absent ⇒ the source EHR id the extract \
                           carries is re-used (RM common `master06` §Copying \
                           Case 1).")),
    request_body(content = serde_json::Value,
                 description = "A canonical `EXTRACT` (RM `ehr_extract` \
                                `extract.adoc`) carrying the whole EHR: it \
                                MUST include an `EHR_STATUS` versioned object, \
                                since `EHR.ehr_status` is `1..1`.",
                 example = json!({ "_type": "EXTRACT" })),
    responses(
        (status = 201, description = "The EHR was cloned. The body names the \
                                      created EHR (`{ \"uid\": … }`), which a \
                                      caller that supplied no `ehr_id` cannot \
                                      otherwise know.",
         body = serde_json::Value,
         example = json!({ "uid": "7d44b88c-4199-4bad-97dc-d78268e01398" })),
        (status = 400, description = "The body is not a well-formed `EXTRACT`, \
                                      it carries no `EHR_STATUS` versioned \
                                      object, it names no source EHR id while \
                                      `ehr_id` is absent, or a content item / \
                                      `ORIGINAL_VERSION` is malformed — SM \
                                      `precondition_violation`.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "The authenticated principal carries the \
                                      configured read-only role: an import \
                                      writes, so it is refused before the body \
                                      is read.",
         body = serde_json::Value),
        (status = 409, description = "SM `ehr_create_fail_duplicate_id` — an \
                                      EHR with the target id already exists \
                                      (\"import EHRs with duplicate EHR ids \
                                      will fail\") — or the imported \
                                      `EHR_STATUS` names a subject another EHR \
                                      already holds (one EHR per subject, RM \
                                      ehr `master04` §EHR Status).",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is neither \
                                      `application/json` nor `application/xml`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn message_import_ehr(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "message_import_ehr", parts, dispatch).await
}

/// Import an EXTRACT into an existing EHR
/// (`POST /message/import/{ehr_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM
/// `I_EHR_EXTRACT_SERVICE.import_ehr_extract` (RM common `master06` §Copying
/// Cases 2/3).
#[utoipa::path(
    post, path = "/message/import/{ehr_id}", tag = "message",
    params(("ehr_id" = String, Path,
            description = "The EHR the extract's content lands in (mandatory: \
                           SM `an_ehr_id: UUID [1]`).")),
    request_body(content = serde_json::Value,
                 description = "A canonical `EXTRACT` whose content items \
                                become new versions of the addressed EHR's \
                                versioned objects.",
                 example = json!({ "_type": "EXTRACT" })),
    responses(
        (status = 204, description = "The extract's versions landed. No body: \
                                      the SM operation returns nothing, and \
                                      the affected resources are read through \
                                      their own released endpoints."),
        (status = 400, description = "The body is not a well-formed `EXTRACT`, \
                                      `ehr_id` is not a well-formed UUID, or a \
                                      content item / `ORIGINAL_VERSION` is \
                                      malformed — SM \
                                      `precondition_violation`.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "The authenticated principal carries the \
                                      configured read-only role: an import \
                                      writes, so it is refused before the body \
                                      is read.",
         body = serde_json::Value),
        (status = 404, description = "SM `ehr_id_does_not_exist`.",
         body = serde_json::Value),
        (status = 409, description = "The EHR already holds an \
                                      `EHR_STATUS`/`EHR_ACCESS` under a \
                                      different object id, or the imported \
                                      status names a subject another EHR \
                                      holds.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is neither \
                                      `application/json` nor `application/xml`.",
         body = serde_json::Value),
        (status = 422, description = "A version in the extract is semantically \
                                      invalid (template/RM/terminology \
                                      validation).",
         body = serde_json::Value)
    )
)]
pub(crate) async fn message_import_ehr_extract(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "message_import_ehr_extract", parts, dispatch).await
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub(crate) fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        run(state, op, parts)
            .await
            .unwrap_or_else(IntoResponse::into_response)
    })
}

async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;

    match op {
        "message_export_ehrs" => {
            let ehr_id = super::path_ehr_id(&parts)?;
            let extracts = state.backend().extract_ehrs(ehr_id).await?;
            Ok(negotiate::respond(h, StatusCode::OK, &extracts))
        }
        "message_export_ehr_extracts" => {
            let spec = negotiate::rm_value::<ExtractSpec>(h, &parts.body)?;
            let extracts = state.backend().export_ehr_extracts(spec).await?;
            Ok(negotiate::respond(h, StatusCode::OK, &extracts))
        }
        "message_import_ehr" => {
            let requested = query_ehr_id(&parts)?;
            let extract = read_extract(&parts)?;
            let created = state.backend().import_ehr(requested, extract).await?;
            Ok(negotiate::identifier_response(
                h,
                StatusCode::CREATED,
                &created.to_string(),
            ))
        }
        "message_import_ehr_extract" => {
            let ehr_id = super::path_ehr_id(&parts)?;
            let extract = read_extract(&parts)?;
            state.backend().import_ehr_extract(ehr_id, extract).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted message extract operation: {other}"
        )))),
    }
}

/// Decode the request body as a canonical `EXTRACT` (JSON or XML, per the
/// shared RM body negotiation).
fn read_extract(parts: &RequestParts) -> Result<Extract, RestError> {
    Ok(negotiate::rm_value::<Extract>(&parts.headers, &parts.body)?)
}

/// The OPTIONAL `ehr_id` query parameter of `import_ehr` (SM `[0..1]`).
fn query_ehr_id(parts: &RequestParts) -> Result<Option<EhrId>, RestError> {
    let Some(raw) = params::query_param(parts.query.as_deref(), "ehr_id") else {
        return Ok(None);
    };
    raw.parse::<EhrId>().map(Some).map_err(|e| {
        RestError(ApiError::BadRequest(format!(
            "query parameter `ehr_id` is not a well-formed EHR identifier: {e}"
        )))
    })
}
