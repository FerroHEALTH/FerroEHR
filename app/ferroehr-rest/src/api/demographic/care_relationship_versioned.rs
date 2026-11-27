// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The versioned care relationship reads: **an extension of ITS-REST**.
//!
//! The draft access control package (ITS-REST Resources, Care
//! Relationships) gives each relationship its versioned object, its revision
//! history and one version, after the `versioned_` resources of ITS-REST.
//! Each read is decided on the relationship's subject and writes one
//! access-log entry for it; a relationship the caller may not read answers
//! `404`. No ITS-REST operation governs these routes: our own extension,
//! excluded from any conformance-profile claim.

use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use ferroehr::versioning::object_version_id::parse_version_uid;
use openehr_its::rest::runtime::ApiError;

use super::care_relationship::audit_revealed;
use crate::api::{RequestParts, guarded_handler};
use crate::overview::error::RestError;
use crate::state::AppState;

/// The versioned care relationship routes, paths relative to the extensions
/// root.
pub(crate) fn versioned_care_relationship_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(versioned_care_relationship_get))
        .routes(routes!(care_relationship_revision_history))
        .routes(routes!(care_relationship_version_get_by_id))
}

guarded_handler! {
    /// Retrieve the versioned object of a care relationship
    /// (`GET /versioned_care_relationship/{relationship_uid}`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.**
    #[utoipa::path(
        get, path = "/versioned_care_relationship/{relationship_uid}", tag = "demographic-relationship",
        params(("relationship_uid" = String, Path, description = "The relationship's container id.")),
        responses(
            (status = 200, description = "The VERSIONED_OBJECT, with its ETag.", body = serde_json::Value),
            (status = 400, description = "A malformed uid.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 404, description = "No care relationship the caller may read has this uid.", body = serde_json::Value)
        )
    )]
    versioned_care_relationship_get => get
}

guarded_handler! {
    /// Retrieve the revision history of a care relationship
    /// (`GET /versioned_care_relationship/{relationship_uid}/revision_history`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.** One
    /// item per version, with the `AUDIT_DETAILS` of its commit.
    #[utoipa::path(
        get, path = "/versioned_care_relationship/{relationship_uid}/revision_history", tag = "demographic-relationship",
        params(("relationship_uid" = String, Path, description = "The relationship's container id.")),
        responses(
            (status = 200, description = "The REVISION_HISTORY.", body = serde_json::Value),
            (status = 400, description = "A malformed uid.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 404, description = "No care relationship the caller may read has this uid.", body = serde_json::Value)
        )
    )]
    care_relationship_revision_history => revision_history
}

guarded_handler! {
    /// Retrieve one version of a care relationship
    /// (`GET /versioned_care_relationship/{relationship_uid}/version/{version_uid}`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.**
    #[utoipa::path(
        get, path = "/versioned_care_relationship/{relationship_uid}/version/{version_uid}", tag = "demographic-relationship",
        params(
            ("relationship_uid" = String, Path, description = "The relationship's container id."),
            ("version_uid" = String, Path, description = "The version's uid.")
        ),
        responses(
            (status = 200, description = "The ORIGINAL_VERSION, with its ETag.", body = serde_json::Value),
            (status = 400, description = "A malformed uid.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 404, description = "No such version of a care relationship the caller may read.", body = serde_json::Value)
        )
    )]
    care_relationship_version_get_by_id => version_get_by_id
}

/// The `versioned_care_relationship_get` operation.
pub(crate) async fn get(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let raw = path_value(&parts, "relationship_uid")?;
    let read = state
        .backend()
        .versioned_care_relationship_get(super::object_of(raw)?)
        .await?;
    let mut out = super::read_versioned(&parts.headers, raw, &read.body.body, super::JSON_ONLY);
    audit_revealed(&mut out, read.revealed);
    Ok(out)
}

/// The `care_relationship_revision_history` operation.
pub(crate) async fn revision_history(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let raw = path_value(&parts, "relationship_uid")?;
    let read = state
        .backend()
        .care_relationship_revision_history(super::object_of(raw)?)
        .await?;
    let mut out = super::read_versioned(&parts.headers, raw, &read.body.body, super::JSON_ONLY);
    audit_revealed(&mut out, read.revealed);
    Ok(out)
}

/// The `care_relationship_version_get_by_id` operation.
pub(crate) async fn version_get_by_id(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let raw = path_value(&parts, "relationship_uid")?;
    let vo_id = super::object_of(raw)?;
    let (of, tree) = parse_version_uid(path_value(&parts, "version_uid")?)?;
    if of != vo_id {
        return Err(RestError::from(ApiError::NotFound(format!(
            "care relationship {vo_id} has no version of {of}"
        ))));
    }
    let read = state
        .backend()
        .care_relationship_version_get_by_id(vo_id, tree)
        .await?;
    let mut out = super::read_versioned(&parts.headers, raw, &read.body.body, super::JSON_ONLY);
    audit_revealed(&mut out, read.revealed);
    Ok(out)
}

/// The path parameter `name` of `parts`.
fn path_value<'p>(parts: &'p RequestParts, name: &str) -> Result<&'p str, RestError> {
    parts
        .path
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| RestError::from(ApiError::BadRequest(format!("the {name} is missing"))))
}
