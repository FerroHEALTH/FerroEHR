// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 / ADL 2 **archetype + artefact** wire — **our own extension**.
//!
//! No openEHR ITS-REST operation governs any route in this module: the released
//! Definition API provisions operational templates only, declaring no archetype
//! resource, generic artefact resource, count, or DELETE. The SM does declare
//! those operations (`SM/docs/UML/classes/i_definition_adl14.adoc` and
//! `i_definition_adl2.adoc`), so these routes realize a service basis with no
//! wire basis, for ADL 1.4 and ADL 2 alike.
//!
//! Every route here is therefore excluded from ITS-REST wire conformance and
//! gates the `Adl14ArchetypeProvisioning` / `Adl2ArchetypeProvisioning`
//! capability verdicts only. The envelope mirrors the released template surface
//! — same base segment layout, same status vocabulary — so clients see one
//! consistent Definition API, but every branch is ours.
//!
//! The routes mount inside the ITS-REST API router, so the full paths are
//! `{base_path}/definition/{archetype,artefact}/…`, which keeps the auth,
//! ATNA-audit and ABAC middleware stack uniform. There is no separate config
//! gate: the archetype surface lives and dies with the DEFINITION group.
//!
//! NOTE: no openEHR spec governs role semantics on an unspecified route — our
//! own design/extension. The shared authentication and RBAC layer answers before
//! any handler runs: no valid principal is `401`, the configured read-only role
//! is `403` on the writes, and the coarse operation class is `Clinical`.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::{HeaderValue, StatusCode, header};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use ferroehr::service::list::Page;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::{negotiate, params};

/// The archetype/artefact extension routes as a native `utoipa-axum` router —
/// **no ITS-REST contract** (see the module docs). Group-relative paths
/// (nested under `base_path`); every operation runs through
/// [`guarded_dispatch`] with [`dispatch`].
pub(crate) fn archetype_routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (the macro composes one method-router — handlers
    // in a single call must share the path; mixing paths panics at build with
    // "Overlapping method route").
    OpenApiRouter::new()
        .routes(routes!(
            definition_archetype_adl14_list,
            definition_archetype_adl14_upload
        ))
        .routes(routes!(
            definition_archetype_adl14_get,
            definition_archetype_adl14_delete
        ))
        .routes(routes!(definition_archetype_adl2_list))
        .routes(routes!(definition_archetype_adl2_count))
        .routes(routes!(definition_artefact_adl2_list))
        .routes(routes!(definition_artefact_adl2_count))
        .routes(routes!(definition_artefact_adl2_delete))
}

/// The `text/plain` media type ADL source text is exchanged in on these routes.
///
/// NOTE (no openEHR spec governs an ADL-source media type — our own
/// design/extension): ADL 1.4 source is a plain-text serialization (AM ADL 1.4
/// `ADL2/master02-overview.adoc` — ADL is "a formal language for expressing
/// archetypes"), and the released Definition API never puts an archetype on the
/// wire, so no registered openEHR media type exists for it. `text/plain` is the
/// truthful declaration for an opaque UTF-8 text artefact.
const ADL_TEXT: &str = "text/plain; charset=utf-8";

// ── ADL 1.4 archetypes (SM I_DEFINITION_ADL14) ───────────────────────────────

/// List the stored ADL 1.4 source archetypes
/// (`GET /definition/archetype/adl1.4`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_DEFINITION_ADL14.list_archetypes`
/// (`i_definition_adl14.adoc`), cursored by the SM list convention
/// (`master02-overview.adoc` §List Handling: `item_offset` / `items_to_fetch`),
/// spelled `offset` / `fetch` here to match the released template list's own
/// parameter names.
#[utoipa::path(
    get, path = "/definition/archetype/adl1.4", tag = "definition-archetype",
    params(
        ("offset" = Option<u64>, Query,
         description = "0-based offset into the id list (SM `item_offset`; \
                        absent = from the first item). Extension route — the \
                        cursor convention is SM `master02-overview.adoc` \
                        §List Handling, not a released wire parameter.",
         example = 0),
        ("fetch" = Option<u64>, Query,
         description = "Number of ids to return from `offset` (SM \
                        `items_to_fetch`; absent or `0` = all).",
         example = 20)
    ),
    responses(
        (status = 200, description = "The stored ADL 1.4 `ARCHETYPE_ID`s, \
                                      ascending. An empty store answers `200` \
                                      with `[]` — an empty collection is a \
                                      successful read, not a `404`.",
         body = Vec<String>,
         example = json!(["openEHR-EHR-COMPOSITION.prescription.v1"])),
        (status = 400, description = "`offset`/`fetch` is not a non-negative \
                                      integer.", body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Refused before the store is \
                                      read.", body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      an id list has no canonical-XML or \
                                      Simplified representation, so it is \
                                      served as `application/json` only.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_archetype_adl14_list(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_archetype_adl14_list", parts, dispatch).await
}

/// Store an ADL 1.4 source archetype (`POST /definition/archetype/adl1.4`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_DEFINITION_ADL14.upload_archetype`, whose
/// declared semantics are replace-or-create ("If an archetype with the same id
/// already exists, replace it. The archetype must be valid to succeed." —
/// `i_definition_adl14.adoc`), and whose `Pre_valid_archetype` is enforced by
/// the `openehr-adl` engine judging the source **as 1.4** (ADL 1.4 `master08`
/// §Validity Rules + the AOM 1.4 class invariants).
#[utoipa::path(
    post, path = "/definition/archetype/adl1.4", tag = "definition-archetype",
    params(
        ("Content-Type" = Option<String>, Header,
         description = "`text/plain` — ADL 1.4 source text (the only body form \
                        this route accepts). An absent header declares nothing \
                        to refuse and reads as ADL text.",
         example = "text/plain")
    ),
    request_body(content = String, content_type = "text/plain",
                 description = "ADL 1.4 source. The `ARCHETYPE_ID` is read out \
                                of the source's own `archetype` header — the \
                                resource identity is the client's, never \
                                server-assigned."),
    responses(
        (status = 201, description = "Stored. The body is the stored \
                                      `ARCHETYPE_ID`. A re-upload of an \
                                      existing id REPLACES it and answers `201` \
                                      too — the SM operation is \
                                      replace-or-create, so there is no \
                                      conflict branch to report.",
         body = String,
         headers(
             ("Location" = String,
              description = "`<base_path>/definition/archetype/adl1.4/<archetype_id>` \
                             — the URL of the stored archetype. Extension \
                             route: the convention is borrowed from the \
                             overview's §Location rule, not inherited from it.")
         ),
         example = json!("openEHR-EHR-COMPOSITION.prescription.v1")),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Refused before the body is \
                                      read.", body = serde_json::Value),
        (status = 403, description = "The authenticated principal carries the \
                                      configured read-only role: this route \
                                      writes the definition store, so it is \
                                      refused regardless of the payload.",
         body = serde_json::Value),
        (status = 415, description = "The request declares a `Content-Type` \
                                      other than `text/plain`; the body cannot \
                                      be processed as ADL source.",
         body = serde_json::Value),
        (status = 422, description = "The source does not parse as ADL 1.4, or \
                                      fails the ADL 1.4 / AOM 1.4 phase-1 \
                                      validity catalogue — SM \
                                      `Pre_valid_archetype`; the offending \
                                      rule-code mnemonics are carried as the \
                                      error detail.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_archetype_adl14_upload(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_archetype_adl14_upload", parts, dispatch).await
}

/// Retrieve one ADL 1.4 source archetype
/// (`GET /definition/archetype/adl1.4/{archetype_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_DEFINITION_ADL14.get_archetype`; the
/// `200`-vs-`404` of this read is also the wire realization of the interface's
/// boolean `has_archetype` (the same existence-as-presence realization the
/// released party read gives `I_PARTY.has_party`).
#[utoipa::path(
    get, path = "/definition/archetype/adl1.4/{archetype_id}", tag = "definition-archetype",
    params(
        ("archetype_id" = String, Path,
         description = "The `ARCHETYPE_ID`. Matched case-insensitively — BASE \
                        `master05` §\"Composite Identifiers and Case\": two \
                        identifiers \"identical apart from case … identify the \
                        same thing\".",
         example = "openEHR-EHR-COMPOSITION.prescription.v1")
    ),
    responses(
        (status = 200, description = "The stored ADL 1.4 source, byte-for-byte \
                                      as uploaded, as `text/plain`.",
         body = String,
         content_type = "text/plain"),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal).", body = serde_json::Value),
        (status = 404, description = "No archetype with that id — SM \
                                      `artefact_does_not_exist`. This is also \
                                      the answer for a syntactically malformed \
                                      `ARCHETYPE_ID`: the store key is an \
                                      opaque, case-insensitively matched \
                                      string with no syntactic gate on the \
                                      read path, so an unparseable id is \
                                      simply an id nothing is stored under.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_archetype_adl14_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_archetype_adl14_get", parts, dispatch).await
}

/// Delete one ADL 1.4 source archetype
/// (`DELETE /definition/archetype/adl1.4/{archetype_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_DEFINITION_ADL14.delete_archetype`
/// (`Pre_artefact_exists` / `Post_archetype_removed`). This is a definition-store
/// removal, not a clinical delete: no VERSION is produced, so the success is a
/// bodyless `204`.
#[utoipa::path(
    delete, path = "/definition/archetype/adl1.4/{archetype_id}", tag = "definition-archetype",
    params(
        ("archetype_id" = String, Path,
         description = "The `ARCHETYPE_ID` (case-insensitive, per BASE \
                        `master05` §\"Composite Identifiers and Case\").",
         example = "openEHR-EHR-COMPOSITION.prescription.v1")
    ),
    responses(
        (status = 204, description = "Removed. No body — the definition store \
                                      holds no version history for source \
                                      archetypes, so nothing survives to \
                                      return."),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal).", body = serde_json::Value),
        (status = 403, description = "The authenticated principal carries the \
                                      configured read-only role: this route \
                                      writes the definition store, so it is \
                                      refused before the store is touched.",
         body = serde_json::Value),
        (status = 404, description = "No archetype with that id — SM \
                                      `Pre_artefact_exists` /\
                                      `artefact_does_not_exist`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_archetype_adl14_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_archetype_adl14_delete", parts, dispatch).await
}

// ── ADL 2 archetypes + artefacts (SM I_DEFINITION_ADL2) ──────────────────────

/// List the stored ADL 2 archetypes (`GET /definition/archetype/adl2`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_DEFINITION_ADL2.list_archetypes` — the
/// artefacts whose concrete AOM2 type is `AUTHORED_ARCHETYPE` (AM AOM2
/// §Archetypes), as opposed to the templates and OPTs the released
/// `/definition/template/adl2` list already serves.
#[utoipa::path(
    get, path = "/definition/archetype/adl2", tag = "definition-archetype",
    params(
        ("offset" = Option<u64>, Query,
         description = "0-based offset into the id list (SM `item_offset`).",
         example = 0),
        ("fetch" = Option<u64>, Query,
         description = "Number of ids to return from `offset` (SM \
                        `items_to_fetch`; absent or `0` = all).",
         example = 20)
    ),
    responses(
        (status = 200, description = "The stored `AUTHORED_ARCHETYPE` HRIDs, \
                                      ascending; `[]` on an empty store.",
         body = Vec<String>,
         example = json!(["openEHR-EHR-OBSERVATION.cnf_count.v1.0.0"])),
        (status = 400, description = "`offset`/`fetch` is not a non-negative \
                                      integer.", body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal).", body = serde_json::Value),
        (status = 406, description = "An id list is served as \
                                      `application/json` only.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_archetype_adl2_list(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_archetype_adl2_list", parts, dispatch).await
}

/// Count the stored ADL 2 archetypes (`GET /definition/archetype/adl2/count`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_DEFINITION_ADL2.archetypes_count`. The
/// operation returns an `Integer`, so the response body is a bare JSON number —
/// the count IS the resource, and wrapping it in an object would invent a
/// schema no spec defines.
#[utoipa::path(
    get, path = "/definition/archetype/adl2/count", tag = "definition-archetype",
    responses(
        (status = 200, description = "The total number of stored \
                                      `AUTHORED_ARCHETYPE`s, as a bare JSON \
                                      number.",
         body = i64, example = json!(0)),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal).", body = serde_json::Value),
        (status = 406, description = "The count is served as \
                                      `application/json` only.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_archetype_adl2_count(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_archetype_adl2_count", parts, dispatch).await
}

/// List every stored ADL 2 artefact (`GET /definition/artefact/adl2`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_DEFINITION_ADL2.list_artefacts` — the whole
/// AOM2 artefact union (archetypes + templates + OPTs), which no released route
/// exposes: `/definition/template/adl2` lists templates only.
#[utoipa::path(
    get, path = "/definition/artefact/adl2", tag = "definition-archetype",
    params(
        ("offset" = Option<u64>, Query,
         description = "0-based offset into the id list (SM `item_offset`).",
         example = 0),
        ("fetch" = Option<u64>, Query,
         description = "Number of ids to return from `offset` (SM \
                        `items_to_fetch`; absent or `0` = all).",
         example = 20)
    ),
    responses(
        (status = 200, description = "Every stored AOM2 artefact HRID, \
                                      ascending; `[]` on an empty store.",
         body = Vec<String>,
         example = json!(["openEHR-EHR-COMPOSITION.cnf_minimal.v1.0.0"])),
        (status = 400, description = "`offset`/`fetch` is not a non-negative \
                                      integer.", body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal).", body = serde_json::Value),
        (status = 406, description = "An id list is served as \
                                      `application/json` only.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_artefact_adl2_list(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_artefact_adl2_list", parts, dispatch).await
}

/// Count every stored ADL 2 artefact (`GET /definition/artefact/adl2/count`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_DEFINITION_ADL2.artefacts_count`; the body
/// is a bare JSON number, as for the archetype count.
#[utoipa::path(
    get, path = "/definition/artefact/adl2/count", tag = "definition-archetype",
    responses(
        (status = 200, description = "The total number of stored AOM2 \
                                      artefacts, as a bare JSON number.",
         body = i64, example = json!(0)),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal).", body = serde_json::Value),
        (status = 406, description = "The count is served as \
                                      `application/json` only.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_artefact_adl2_count(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_artefact_adl2_count", parts, dispatch).await
}

/// Delete one stored ADL 2 artefact
/// (`DELETE /definition/artefact/adl2/{artefact_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_DEFINITION_ADL2.delete_artefact`
/// (`Pre_artefact_exists`, error `artefact_does_not_exist`) over the whole AOM2
/// artefact union — archetype, template or OPT. The released ADL 2 group has no
/// DELETE at all.
#[utoipa::path(
    delete, path = "/definition/artefact/adl2/{artefact_id}", tag = "definition-archetype",
    params(
        ("artefact_id" = String, Path,
         description = "The AOM2 artefact HRID (case-insensitive, per BASE \
                        `master05` §\"Composite Identifiers and Case\").",
         example = "openEHR-EHR-COMPOSITION.cnf_minimal.v1.0.0")
    ),
    responses(
        (status = 204, description = "Removed. No body — the definition store \
                                      keeps no version history for AOM2 \
                                      artefacts."),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal).", body = serde_json::Value),
        (status = 403, description = "The authenticated principal carries the \
                                      configured read-only role: this route \
                                      writes the definition store, so it is \
                                      refused before the store is touched.",
         body = serde_json::Value),
        (status = 404, description = "No artefact with that id — SM \
                                      `artefact_does_not_exist`. This is also \
                                      the answer for a syntactically malformed \
                                      HRID: the store key is opaque and \
                                      case-insensitively matched, so an \
                                      unparseable id is simply an id nothing \
                                      is stored under.",
         body = serde_json::Value),
        (status = 409, description = "The artefact is a template kind that \
                                      compositions still reference (the count \
                                      is named in the body): deleting it would \
                                      orphan committed content, so the store \
                                      refuses. Archived content counts too.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_artefact_adl2_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_artefact_adl2_delete", parts, dispatch).await
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
    let q = parts.query.as_deref();

    match op {
        "definition_archetype_adl14_list" => {
            let ids = state.backend().list_archetypes_adl14(page(q)?).await?;
            Ok(negotiate::respond(h, StatusCode::OK, &ids))
        }
        "definition_archetype_adl14_upload" => {
            // The archetype arrives as ADL source text — the one body form of
            // this route. A payload DECLARING another media type cannot be
            // processed as ADL, so it is refused before parsing; an absent
            // `Content-Type` declares nothing to refuse.
            negotiate::require_text_plain(h)?;
            let adl = negotiate::text_body(&parts.body)?;
            let archetype_id = state.backend().upload_archetype(adl).await?;
            let location = format!(
                "{}/definition/archetype/adl1.4/{}",
                state.config().server.base_path,
                urlencoding::encode(&archetype_id)
            );
            let mut resp = negotiate::respond(h, StatusCode::CREATED, &archetype_id);
            if let Ok(value) = HeaderValue::from_str(&location) {
                resp.headers_mut().insert(header::LOCATION, value);
            }
            Ok(resp)
        }
        "definition_archetype_adl14_get" => {
            let archetype_id = path_segment(&parts, "archetype_id")?;
            let adl = state.backend().get_archetype(archetype_id).await?;
            Ok(adl_text_response(adl))
        }
        "definition_archetype_adl14_delete" => {
            let archetype_id = path_segment(&parts, "archetype_id")?;
            state.backend().delete_archetype(archetype_id).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "definition_archetype_adl2_list" => {
            let ids = state.backend().list_archetypes_adl2(page(q)?).await?;
            Ok(negotiate::respond(h, StatusCode::OK, &ids))
        }
        "definition_archetype_adl2_count" => {
            let count = state.backend().archetypes_count_adl2().await?;
            Ok(negotiate::respond(h, StatusCode::OK, &count))
        }
        "definition_artefact_adl2_list" => {
            let ids = state.backend().list_artefacts(page(q)?).await?;
            Ok(negotiate::respond(h, StatusCode::OK, &ids))
        }
        "definition_artefact_adl2_count" => {
            let count = state.backend().artefacts_count().await?;
            Ok(negotiate::respond(h, StatusCode::OK, &count))
        }
        "definition_artefact_adl2_delete" => {
            let artefact_id = path_segment(&parts, "artefact_id")?;
            state.backend().delete_artefact(artefact_id).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted definition archetype operation: {other}"
        )))),
    }
}

/// Render stored ADL source as `text/plain` — the media type the module's
/// [`ADL_TEXT`] note grounds.
fn adl_text_response(adl: String) -> Response {
    let mut resp = (StatusCode::OK, adl).into_response();
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(ADL_TEXT));
    resp
}

/// Read the SM list cursor (`master02-overview.adoc` §List Handling) from the
/// `offset` / `fetch` query parameters; a non-numeric value is a `400`.
fn page(query: Option<&str>) -> Result<Page, RestError> {
    Ok(Page {
        item_offset: cursor_param(query, "offset")?,
        items_to_fetch: cursor_param(query, "fetch")?,
    })
}

/// One optional non-negative cursor parameter.
#[expect(
    clippy::map_err_ignore,
    reason = "`ParseIntError` adds only \"invalid digit\"/\"out of range\" to a 400 \
              body that already names the parameter and echoes the rejected value"
)]
fn cursor_param(query: Option<&str>, key: &str) -> Result<Option<u64>, RestError> {
    match params::query_param(query, key) {
        None => Ok(None),
        Some(raw) => raw.parse::<u64>().map(Some).map_err(|_| {
            RestError(ApiError::BadRequest(format!(
                "query parameter `{key}` must be a non-negative integer, got {raw:?}"
            )))
        }),
    }
}

/// Read a required path segment (a missing segment is impossible for a matched
/// route, so it is a routing bug → `500`).
fn path_segment(parts: &RequestParts, key: &str) -> Result<String, RestError> {
    parts.path.get(key).cloned().ok_or_else(|| {
        RestError(ApiError::Internal(format!(
            "missing path parameter `{key}`"
        )))
    })
}
