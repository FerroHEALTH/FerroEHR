// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ITS-REST **stored-query** resource (`tags: Query`).
//!
//! Operations (`docs/specs/openehr/ITS-REST/specifications/operations/`):
//! `definition_query_list` / `definition_query_store.yaml` /
//! `definition_query_version_get` / `definition_query_version_store.yaml`.
//! Governing spec text:
//! `docs/specs/openehr/ITS-REST/specifications/docs/definition/`.
//!
//! Queries route through the wire-shaped `DefinitionAdapter`
//! (`query_list`/`query_version_get`/`query_store`) — the SM
//! `DefinitionQueryService` exchanges `QUERY_DESCRIPTOR`s, while the wire
//! returns/accepts the ITS-REST `StoredQuery` shapes. A store success is
//! `200 OK` (not `204`) with a `Location` for the stored resource
//! (`responses/200_StoredQuery_stored.yaml` + `headers/Location_Query.yaml`).

use axum::response::Response;
use http::StatusCode;

use openehr_its::rest::generated::definition::{
    DefinitionQueryListParams, DefinitionQueryStoreYamlParams, DefinitionQueryVersionGetParams,
    DefinitionQueryVersionStoreYamlParams,
};

use crate::api::RequestParts;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The default query formalism when `query_type` is absent
/// (`parameters/query/query_type.yaml`: `default: "AQL"`).
const DEFAULT_QUERY_TYPE: &str = "AQL";

/// `GET …/definition/query/{qualified_query_name}` — the registered queries
/// under the qualified name (a prefix pattern; wildcard on empty).
pub(super) async fn list(state: &AppState, parts: &RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        DefinitionQueryListParams::from_request,
        DefinitionQueryListParams::PARAMS,
    )?;
    Ok(negotiate::respond(
        h,
        StatusCode::OK,
        &state.backend().query_list(p.qualified_query_name).await?,
    ))
}

/// `GET …/definition/query` — every registered stored query (the empty
/// prefix of the same list). NOTE: the released ITS-REST text defines only
/// the `{qualified_query_name}` operation, whose own description says an
/// empty pattern "will be treated as \"wildcard\" in the search"
/// (`operations/definition_query_list.yaml`) yet leaves that clause with no
/// addressable form (the path parameter is required and no `/definition/query`
/// path exists); the bare listing is our own convenience extension realizing
/// it — no openEHR spec governs the bare form.
pub(super) async fn list_all(
    state: &AppState,
    parts: &RequestParts,
) -> Result<Response, RestError> {
    Ok(negotiate::respond(
        &parts.headers,
        StatusCode::OK,
        &state.backend().query_list(String::new()).await?,
    ))
}

/// Stores or upserts a query at a server-assigned SEMVER
/// (`PUT …/definition/query/{qualified_query_name}`).
///
/// The `query_type` query parameter (default `AQL`,
/// `parameters/query/query_type.yaml`) is threaded to the store, which persists
/// the declared formalism; per `QUERY_DESCRIPTOR.formalism` ("may be any other
/// string value") an unsupported non-AQL formalism gets an honest
/// unsupported-formalism reject rather than a blanket "invalid AQL" 400.
pub(super) async fn store(state: &AppState, parts: &RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        DefinitionQueryStoreYamlParams::from_request,
        DefinitionQueryStoreYamlParams::PARAMS,
    )?;
    let name = p.qualified_query_name.clone();
    let query_type = p
        .query_type
        .unwrap_or_else(|| DEFAULT_QUERY_TYPE.to_owned());
    // The store's single declared body type is `text/plain`
    // (`operations/definition_query_store.yaml`); a payload DECLARING another
    // media type is refused 415 before parsing (`Resources.md` §format rules
    // MUST), an absent `Content-Type` declares nothing to refuse.
    negotiate::require_text_plain(h)?;
    let body = negotiate::text_body(&parts.body)?;
    // The `DefinitionAdapter::query_store` impl honours `query_type`: an AQL
    // formalism runs the syntactic check, while an unsupported non-AQL
    // formalism is rejected as a distinct unsupported-formalism `400`
    // (`QUERY_DESCRIPTOR.formalism`, `parameters/query/query_type.yaml`).
    //
    // The store returns the SEMVER it actually wrote at, and the `Location`
    // names exactly that resource (`headers/Location_Query.yaml`) — never a
    // neighbouring version recovered by a post-hoc lookup.
    let version = state
        .backend()
        .query_store(name.clone(), None, query_type, body)
        .await?;
    let location = format!(
        "{}/definition/query/{name}/{version}",
        state.config().server.base_path
    );
    Ok(negotiate::empty_with_location(StatusCode::OK, &location))
}

/// `GET …/definition/query/{qualified_query_name}/{version}` — the registered
/// query at the SEMVER (or SEMVER prefix); `404` if absent.
pub(super) async fn version_get(
    state: &AppState,
    parts: &RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        DefinitionQueryVersionGetParams::from_request,
        DefinitionQueryVersionGetParams::PARAMS,
    )?;
    Ok(negotiate::respond(
        h,
        StatusCode::OK,
        &state
            .backend()
            .query_version_get(p.qualified_query_name, p.version)
            .await?,
    ))
}

/// `PUT …/definition/query/{qualified_query_name}/{version}` — store a query at
/// a specified SEMVER (an exact `major.minor.patch`; a prefix or malformed
/// segment is the released `400` branch); `409` on an existing `(name, version)`.
///
/// as [`store`], the `query_type` parameter is read and threaded through.
pub(super) async fn version_store(
    state: &AppState,
    parts: &RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        DefinitionQueryVersionStoreYamlParams::from_request,
        DefinitionQueryVersionStoreYamlParams::PARAMS,
    )?;
    let name = p.qualified_query_name.clone();
    let version = p.version.clone();
    let query_type = p
        .query_type
        .unwrap_or_else(|| DEFAULT_QUERY_TYPE.to_owned());
    // Same `text/plain`-only body type as the version-less store (the released
    // operation omits the Content-Type parameter but carries the identical
    // text/plain body) — a declared foreign media type is 415 (`Resources.md`
    // §format rules MUST).
    negotiate::require_text_plain(h)?;
    let body = negotiate::text_body(&parts.body)?;
    // As in `store`, the `query_store` impl honours `query_type`: the AQL
    // syntactic check runs only for the AQL formalism, and an unsupported
    // non-AQL formalism is an honest unsupported-formalism reject. The
    // returned SEMVER is the exact path version (the store requires an exact
    // `major.minor.patch`; a prefix or malformed value is its `400`).
    let stored = state
        .backend()
        .query_store(name.clone(), Some(version), query_type, body)
        .await?;
    let location = format!(
        "{}/definition/query/{name}/{stored}",
        state.config().server.base_path
    );
    Ok(negotiate::empty_with_location(StatusCode::OK, &location))
}
