// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The treating professional's release of a composition held from the person
//! (EHDS Art. 3(3)), on the clinical surface, **our own extension**.
//!
//! Regulation (EU) 2025/327 Art. 3(3) delays a person's access "until a
//! health professional is able to properly communicate and explain" the data
//! (`docs/law/eu/ehds/text.html`). The professional who explains it is the
//! one who releases it, so beside the Admin-class
//! `POST /admin/access-delay/release` this route lets any caller who may
//! write the EHR release one composition of it: it carries the clinical
//! authentication class, the `EHR_ACCESS` gate, the ABAC check of a composition
//! update and the SMART `composition-` create grant of an EHR-scoped write. A
//! caller that acts for the person is refused, since the person cannot
//! release what is held from them. No openEHR ITS-REST operation governs it.

use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::extensions::access::authn::current_principal;
use crate::extensions::access::delay;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The operation id of the release; its `composition_` prefix and `_update`
/// verb make the ABAC check a composition update and the RBAC class a write.
pub(crate) const RELEASE: &str = "composition_access_delay_update";

/// The clinical access-delay route, mounted beside the openEHR base path.
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(composition_access_delay_update))
}

/// Release one composition held from the person
/// (`POST /ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/access-delay/release`).
///
/// **Our own extension, no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    post,
    path = "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/access-delay/release",
    tag = "COMPOSITION",
    params(
        ("ehr_id" = String, Path, description = "The EHR the composition is in."),
        ("versioned_object_uid" = String, Path,
         description = "The VERSIONED_COMPOSITION a health professional has explained \
                        to the person.")
    ),
    responses(
        (status = 204, description = "Every hold of the composition still in force \
                                      carries a release instant, and the person \
                                      reads it. Releasing nothing succeeds."),
        (status = 400, description = "An id is not a UUID.", body = serde_json::Value),
        (status = 401, description = "No valid credential.", body = serde_json::Value),
        (status = 403, description = "The caller may not write this EHR, or acts \
                                      for the person.",
         body = serde_json::Value),
        (status = 404, description = "No such EHR, or no such object in it.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn composition_access_delay_update(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, RELEASE, parts, dispatch).await
}

fn dispatch(state: AppState, _op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        match run(&state, &parts).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        }
    })
}

async fn run(state: &AppState, parts: &RequestParts) -> Result<Response, RestError> {
    // NOTE: EHDS Art. 3(3), a health professional releases held data; a token that may
    // be the person's is refused, as it is withheld from (the owner's ruling of 2026-10-31).
    if current_principal().is_some_and(|principal| delay::acts_for_the_person(&principal)) {
        return Err(RestError::from(ApiError::Forbidden(
            "access denied: a held composition is released by a health professional, \
             never by the person it is held from (EHDS Art. 3(3))"
                .to_owned(),
        )));
    }
    let ehr_id = parts.path.get("ehr_id").map_or("", String::as_str);
    let vo_id = parts
        .path
        .get("versioned_object_uid")
        .map_or("", String::as_str);
    state.backend().release_person_access(ehr_id, vo_id).await?;
    Ok(negotiate::empty(StatusCode::NO_CONTENT))
}
