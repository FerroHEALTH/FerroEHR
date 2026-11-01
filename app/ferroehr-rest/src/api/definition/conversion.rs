// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 to ADL 2 **conversion** wire: two read-only routes that convert
//! a stored ADL 1.4 archetype or operational template and answer the ADL 2
//! sources with the conversion log.
//!
//! No openEHR specification defines a conversion operation: the released
//! Definition API declares none, and no openEHR spec governs 1.4 to 2
//! conversion. Both routes are therefore our own design, mounted under the
//! extensions root beside the archetype store, and excluded from ITS-REST wire
//! conformance.
//!
//! The answer is one JSON document of the ADL 2 sources and the conversion
//! log, so a client reads both from one response.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::Serialize;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_adl::adl14::opt::{ConvertedRoot, FillEdge, OptConversion};
use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

use super::archetype::path_segment;

// NOTE: AM `ADL2/master01-preface.adoc` §Backward Compatibility requires a 1.4 to 2
// conversion but defines no route or algorithm, so no specification governs
// these routes: our own design.
/// The conversion routes as a native `utoipa-axum` router (paths relative to
/// the extensions root); every operation runs through [`guarded_dispatch`].
pub(crate) fn conversion_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(definition_archetype_adl14_convert))
        .routes(routes!(definition_template_adl14_convert))
}

/// The answer of a conversion: one ADL 2 source per archetype root, each with
/// its log, and the slot fills that joined the roots.
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct Conversion {
    /// One converted source per archetype root; for a template the top root
    /// comes first, then the embedded roots in document order.
    roots: Vec<Root>,
    /// The slot fills that joined the roots in the operational template;
    /// empty for an archetype.
    structure: Vec<Fill>,
}

/// One converted archetype root.
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct Root {
    /// The archetype id of the root, as the ADL 1.4 source states it.
    archetype_id: String,
    /// The ADL 2 source text.
    adl2: String,
    /// The codes the converter synthesised and its provenance notes.
    log: Log,
    /// Every construct of the root the conversion dropped or changed, the
    /// converted archetype's `RESOURCE_DESCRIPTION.conversion_details`.
    conversion_details: BTreeMap<String, String>,
}

/// The conversion log of one root.
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct Log {
    /// External terminology code (`terminology::code`) to the at-code the
    /// converter minted for it.
    external_at_codes: BTreeMap<String, String>,
    /// Value-set signature to the ac-code the converter minted for it.
    value_sets: BTreeMap<String, String>,
    /// Notes on the decisions the conversion took.
    notes: Vec<String>,
}

/// A parent-to-child slot fill of the operational template, which no single
/// converted archetype states.
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct Fill {
    /// The archetype id of the root that holds the slot.
    parent_archetype_id: String,
    /// The path within the parent archetype to the slot.
    parent_path: String,
    /// The at-code of the slot.
    slot_node_id: String,
    /// The archetype id of the root that filled the slot.
    child_archetype_id: String,
}

impl From<OptConversion> for Conversion {
    fn from(conversion: OptConversion) -> Self {
        Self {
            roots: conversion.roots.into_iter().map(Root::from).collect(),
            structure: conversion.structure.into_iter().map(Fill::from).collect(),
        }
    }
}

impl From<ConvertedRoot> for Root {
    fn from(root: ConvertedRoot) -> Self {
        Self {
            archetype_id: root.archetype_id,
            adl2: root.adl2,
            log: Log {
                external_at_codes: root.log.external_at_codes,
                value_sets: root.log.value_sets,
                notes: root.log.notes,
            },
            conversion_details: root.conversion_details,
        }
    }
}

impl From<FillEdge> for Fill {
    fn from(edge: FillEdge) -> Self {
        Self {
            parent_archetype_id: edge.parent_archetype_id,
            parent_path: edge.parent_path,
            slot_node_id: edge.slot_node_id,
            child_archetype_id: edge.child_archetype_id,
        }
    }
}

/// Convert a stored ADL 1.4 archetype to ADL 2
/// (`GET /definition/archetype/adl1.4/{archetype_id}/adl2`).
///
/// **Our own extension: no ITS-REST operation governs this**, and no openEHR
/// specification defines a conversion operation (module docs). The store is only read.
#[utoipa::path(
    get, path = "/definition/archetype/adl1.4/{archetype_id}/adl2", tag = "definition-archetype",
    params(
        ("archetype_id" = String, Path,
         description = "The `ARCHETYPE_ID` of a stored ADL 1.4 archetype, matched \
                        case-insensitively (BASE `master05` §\"Composite \
                        Identifiers and Case\").",
         example = "openEHR-EHR-COMPOSITION.prescription.v1")
    ),
    responses(
        (status = 200, description = "The ADL 2 conversion: one root, its ADL 2 \
                                      source, the conversion log and the \
                                      `conversion_details`, and an empty \
                                      `structure`. No openEHR specification \
                                      governs this conversion: our own design.",
         body = Conversion),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal).", body = serde_json::Value),
        (status = 404, description = "No archetype with that id.",
         body = serde_json::Value),
        (status = 406, description = "The conversion is served as \
                                      `application/json` only.",
         body = serde_json::Value),
        (status = 422, description = "The stored source no longer converts: it \
                                      does not parse, or the result carries a \
                                      node ADL 2 has no syntax for. The same \
                                      status an archetype upload answers for a \
                                      source it cannot accept.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_archetype_adl14_convert(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_archetype_adl14_convert", parts, dispatch).await
}

/// Convert a stored ADL 1.4 operational template to ADL 2
/// (`GET /definition/template/adl1.4/{template_id}/adl2`).
///
/// **Our own extension: no ITS-REST operation governs this**, and no openEHR
/// specification defines a conversion operation (module docs). The template is decomposed into one source per
/// embedded archetype root; the store is only read.
#[utoipa::path(
    get, path = "/definition/template/adl1.4/{template_id}/adl2", tag = "definition-archetype",
    params(
        ("template_id" = String, Path,
         description = "The `template_id` of a stored ADL 1.4 operational \
                        template, matched case-insensitively (BASE `master05` \
                        §\"Composite Identifiers and Case\").",
         example = "IDCR Allergies List.v0")
    ),
    responses(
        (status = 200, description = "The ADL 2 conversion: one root per \
                                      embedded archetype root, the template's \
                                      top root first, each with its ADL 2 \
                                      source, conversion log and \
                                      `conversion_details`, and the slot fills \
                                      that joined them in `structure`. No \
                                      openEHR specification governs this \
                                      conversion: our own design.",
         body = Conversion),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal).", body = serde_json::Value),
        (status = 404, description = "No operational template with that id.",
         body = serde_json::Value),
        (status = 406, description = "The conversion is served as \
                                      `application/json` only.",
         body = serde_json::Value),
        (status = 422, description = "The stored template no longer converts: \
                                      it does not parse, or a root does not \
                                      convert or print as ADL 2.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_template_adl14_convert(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "definition_template_adl14_convert", parts, dispatch).await
}

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
    let conversion = match op {
        "definition_archetype_adl14_convert" => {
            let archetype_id = path_segment(&parts, "archetype_id")?;
            state.backend().adl14_convert_to_adl2(archetype_id).await?
        }
        "definition_template_adl14_convert" => {
            let template_id = path_segment(&parts, "template_id")?;
            state
                .backend()
                .adl14_convert_opt_to_adl2(template_id)
                .await?
        }
        other => {
            return Err(RestError::from(ApiError::Internal(format!(
                "unrouted definition conversion operation: {other}"
            ))));
        }
    };
    Ok(negotiate::respond(
        &parts.headers,
        StatusCode::OK,
        &Conversion::from(conversion),
    ))
}
