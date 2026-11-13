// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **access policy** wire: the controller's `ACCESS_POLICY`,
//! **our own extension**.
//!
//! The policy is one `VERSIONED_ACCESS_POLICY` of FerroEHR's draft access
//! control package ([`ferroehr::access_policy`]). A read answers the newest
//! version with its `OBJECT_VERSION_ID` as the weak `ETag` and its commit
//! time as `Last-Modified`, as the ITS-REST versioned resources do (overview
//! §"`ETag` and Last-Modified"); a write commits the whole policy as the next
//! version in its own contribution, naming the version it replaces in
//! `If-Match` or creating the first with `If-None-Match: *` (RFC 9110
//! §13.1), and is refused with `412` when another write came first; the
//! history lists every `ORIGINAL_VERSION` with its contribution and commit
//! audit. A change applies on the next request on every replica, with no
//! restart. ITS-REST defines no resource for the policy.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`admin_role`, `401`/`403`) and the `AppConfig::api.admin`
//! config gate (`405` with an empty `Allow` when off), and the audit trail
//! records every read and write. A write also needs a policy grant of
//! `write_access_policy` in the CDR, so the admin role alone changes no
//! policy.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the policy and its versions are served \
              as the canonical JSON the service renders"
)]

use axum::response::Response;
use http::{StatusCode, header};
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use ferroehr::party_binding::grants::capabilities::Capabilities;
use ferroehr::party_binding::grants::operation::Operation;
use ferroehr::service::admin::access_policy::{PolicyPrecondition, PolicyVersion};

use super::body::AdminRefusal;
use crate::api::RequestParts;
use crate::api::guarded_handler;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The operation id of a policy read.
pub(crate) const READ: &str = "admin_access_policy_read";
/// The operation id of a policy write.
pub(crate) const WRITE: &str = "admin_access_policy_write";
/// The operation id of a policy history read.
pub(crate) const HISTORY: &str = "admin_access_policy_history";
/// The operation id of the versioned policy read.
pub(crate) const VERSIONED: &str = "admin_access_policy_versioned";

/// The access policy routes — **no ITS-REST contract** (module docs).
pub(crate) fn access_policy_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_access_policy_read, admin_access_policy_write))
        .routes(routes!(admin_access_policy_history))
        .routes(routes!(admin_access_policy_versioned))
}

guarded_handler! {
    /// Read the access policy in force (`GET /admin/access_policy`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/access_policy", tag = "admin-access-policy",
        responses(
            (status = 200, description = "The newest `ACCESS_POLICY`, in canonical JSON. The \
                                          weak `ETag` is its `OBJECT_VERSION_ID` and \
                                          `Last-Modified` its commit time.",
             body = serde_json::Value),
            (status = 404, description = "No access policy is committed; nothing is \
                                          tightened.", body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_access_policy_read as READ => |state, parts| super::body::gated(state, parts, read)
}

guarded_handler! {
    /// Commit the next version of the access policy
    /// (`PUT /admin/access_policy`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        put, path = "/admin/access_policy", tag = "admin-access-policy",
        params(
            ("If-Match" = Option<String>, Header,
             description = "The `OBJECT_VERSION_ID` of the version this write replaces, as \
                            the read's `ETag` gave it."),
            ("If-None-Match" = Option<String>, Header,
             description = "`*` to create the first version.")
        ),
        request_body(content = serde_json::Value,
                     description = "The whole `ACCESS_POLICY` in canonical JSON, read strictly \
                                    and checked against the invariants of the draft access \
                                    control BMM."),
        responses(
            (status = 200, description = "The new version, as a read answers it, with its \
                                          `OBJECT_VERSION_ID` as the `ETag`. The next \
                                          request on every replica runs under it.",
             body = serde_json::Value),
            (status = 201, description = "The first version, as a read answers it.",
             body = serde_json::Value),
            (status = 400, description = "Neither `If-Match` nor `If-None-Match: *`, an \
                                          `If-Match` that is no `OBJECT_VERSION_ID`, or a \
                                          body refused whole: no canonical-JSON \
                                          `ACCESS_POLICY`, a broken invariant of the draft \
                                          BMM, a root that is no `ACCESS_POLICY` archetype \
                                          root, or a controller that is no `ORGANISATION` or \
                                          not the policy's. The version in force stays.",
             body = serde_json::Value),
            (status = 403, description = "Not in the Admin class, or holding no policy grant \
                                          of `write_access_policy`.",
             body = serde_json::Value),
            (status = 412, description = "`If-Match` names a version that is no longer the \
                                          newest, or none is committed; or \
                                          `If-None-Match: *` while one is.",
             body = serde_json::Value),
            (status = 415, description = "The body is not `application/json`.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_access_policy_write as WRITE => |state, parts| super::body::gated(state, parts, write)
}

guarded_handler! {
    /// Read every version of the access policy
    /// (`GET /admin/access_policy/history`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/access_policy/history", tag = "admin-access-policy",
        responses(
            (status = 200, description = "Every `ORIGINAL_VERSION` of the policy with its \
                                          `contribution` and `commit_audit`, newest first; \
                                          empty when none is committed.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_access_policy_history as HISTORY => |state, parts| {
        super::body::gated(state, parts, history)
    }
}

guarded_handler! {
    /// Read the versioned access policy
    /// (`GET /admin/access_policy/versioned`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/access_policy/versioned", tag = "admin-access-policy",
        responses(
            (status = 200, description = "The `VERSIONED_ACCESS_POLICY`: its `uid`, the \
                                          controller as `owner_id`, and `time_created`. The \
                                          weak `ETag` is its `uid`.",
             body = serde_json::Value),
            (status = 404, description = "No access policy is committed.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_access_policy_versioned as VERSIONED => |state, parts| {
        super::body::gated(state, parts, versioned)
    }
}

/// The refusal of a read while no version is committed.
fn none_committed() -> RestError {
    RestError::from(ApiError::NotFound(
        "no access policy is committed; nothing is tightened".to_owned(),
    ))
}

/// `version` as a response with `status`, its uid as the `ETag` and its
/// commit time as `Last-Modified`.
fn answer(parts: &RequestParts, status: StatusCode, version: &PolicyVersion) -> Response {
    let mut resp = negotiate::body::respond(&parts.headers, status, &version.data);
    negotiate::resource::set_etag(&mut resp, &version.version_uid);
    negotiate::resource::set_last_modified(&mut resp, version.time_committed);
    resp
}

/// The `admin_access_policy_read` operation.
async fn read(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let version = state
        .backend()
        .access_policy_current()
        .await?
        .ok_or_else(none_committed)?;
    Ok(answer(&parts, StatusCode::OK, &version))
}

/// What the write's conditional headers expect.
fn precondition(parts: &RequestParts) -> Result<PolicyPrecondition, RestError> {
    let creates = parts
        .headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|value| value.as_bytes() == b"*");
    match (
        crate::overview::version_id::if_match(&parts.headers),
        creates,
    ) {
        (Some(uid), false) => Ok(PolicyPrecondition::Replace(uid)),
        (None, true) => Ok(PolicyPrecondition::Create),
        (Some(_), true) => Err(RestError::from(ApiError::BadRequest(
            "name the version this write replaces in If-Match, or create the first version \
             with If-None-Match: *, not both"
                .to_owned(),
        ))),
        (None, false) => Err(RestError::from(ApiError::BadRequest(
            "If-Match is required: name the access policy version this write replaces, or \
             create the first version with If-None-Match: *"
                .to_owned(),
        ))),
    }
}

/// The `admin_access_policy_write` operation.
async fn write(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    if !Capabilities::current().holds(Operation::WriteAccessPolicy) {
        return Err(RestError::from(ApiError::Forbidden(
            "writing the access policy needs a policy grant of \
             ferroehr-operations::write_access_policy"
                .to_owned(),
        )));
    }
    let precondition = precondition(&parts)?;
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let version = state
        .backend()
        .put_access_policy(&precondition, &body)
        .await?;
    let status = match precondition {
        PolicyPrecondition::Create => StatusCode::CREATED,
        PolicyPrecondition::Replace(_) => StatusCode::OK,
    };
    Ok(answer(&parts, status, &version))
}

/// The `admin_access_policy_history` operation.
async fn history(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let versions: Vec<serde_json::Value> = state
        .backend()
        .access_policy_history()
        .await?
        .into_iter()
        .map(|version| version.original_version)
        .collect();
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::Value::Array(versions),
    ))
}

/// The `admin_access_policy_versioned` operation.
async fn versioned(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let versioned = state
        .backend()
        .access_policy_versioned()
        .await?
        .ok_or_else(none_committed)?;
    let mut resp = negotiate::body::respond(&parts.headers, StatusCode::OK, &versioned.body);
    negotiate::resource::set_etag(&mut resp, &versioned.uid);
    negotiate::resource::set_last_modified(&mut resp, versioned.last_modified);
    Ok(resp)
}
