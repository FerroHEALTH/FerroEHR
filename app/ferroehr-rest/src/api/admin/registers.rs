// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **operational-policy registers** wire: the audit category map,
//! the EHDS Art. 3(3) access delay and the Cedar policy set, **our own
//! extension**.
//!
//! Each register is one JSON document, versioned with its committer
//! ([`ferroehr::policy_register`]). A read answers the newest version with
//! its version as the `ETag`; a write replaces the whole document, names the
//! version it replaces in `If-Match`, and is refused with `412` when another
//! write came first; the history lists every version. A change applies on the
//! next request on every replica, with no restart. No openEHR ITS-REST
//! operation governs any of it.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`admin_role`, `401`/`403`) and the `AppConfig::api.admin`
//! config gate (`405` with an empty `Allow` when off) unchanged, and the
//! audit trail records every read and write.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the admin register bodies are genuinely open \
              operational JSON, not RM content"
)]

use axum::response::Response;
use http::{HeaderValue, StatusCode, header};
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use ferroehr::policy_register::Register;
use ferroehr::service::admin::policy_register::RegisterVersion;

use super::body::AdminRefusal;
use crate::api::RequestParts;
use crate::api::guarded_handler;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The operation id of a register read.
pub(crate) const READ: &str = "admin_policy_register_read";
/// The operation id of a register write.
pub(crate) const WRITE: &str = "admin_policy_register_write";
/// The operation id of a register history read.
pub(crate) const HISTORY: &str = "admin_policy_register_history";

/// The policy register routes — **no ITS-REST contract** (module docs).
pub(crate) fn register_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            admin_policy_register_read,
            admin_policy_register_write
        ))
        .routes(routes!(admin_policy_register_history))
}

guarded_handler! {
    /// Read one operational-policy register
    /// (`GET /admin/registers/{register}`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/registers/{register}", tag = "admin-registers",
        params(("register" = String, Path,
                description = "`audit-categories`, `access-delay` or `cedar-policies`.")),
        responses(
            (status = 200, description = "`{ \"register\": …, \"version\": …, \
                                          \"committer\": …, \"committed_at\": …, \
                                          \"document\": … }`: the newest version, or \
                                          version 0, the default no one committed. \
                                          The `ETag` is the version.",
             body = serde_json::Value,
             example = json!({
                 "register": "access-delay",
                 "version": 2,
                 "committer": "admin@example.org",
                 "committed_at": "2026-11-12T09:15:00Z",
                 "document": {
                     "enabled": true,
                     "categories": ["Laboratory-Reports"],
                     "max_delay": "72h"
                 }
             })),
            (status = 404, description = "No such register.", body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_policy_register_read as READ => |state, parts| super::body::gated(state, parts, read)
}

guarded_handler! {
    /// Replace one operational-policy register whole
    /// (`PUT /admin/registers/{register}`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        put, path = "/admin/registers/{register}", tag = "admin-registers",
        params(
            ("register" = String, Path,
             description = "`audit-categories`, `access-delay` or `cedar-policies`."),
            ("If-Match" = String, Header,
             description = "The version this write replaces, as the read's `ETag` gave it.")
        ),
        request_body(content = serde_json::Value,
                     description = "The whole register document. `audit-categories`: \
                                    `{ \"shipped_map\": …, \"national\": [...], \
                                    \"templates\": {...}, \"archetypes\": {...} }`. \
                                    `access-delay`: `{ \"enabled\": …, \
                                    \"categories\": [...], \"max_delay\": \"72h\" }`. \
                                    `cedar-policies`: `{ \"policies\": \"<Cedar text>\" }`.",
                     example = json!({
                         "enabled": true,
                         "categories": ["Laboratory-Reports"],
                         "max_delay": "72h"
                     })),
        responses(
            (status = 200, description = "The new version, as a read answers it, with \
                                          its version as the `ETag`. The next request \
                                          on every replica runs under it.",
             body = serde_json::Value),
            (status = 400, description = "`If-Match` is missing or no version, or the \
                                          document is refused whole: not the register's \
                                          shape, a category map that does not compile \
                                          or classifies nothing under the production \
                                          floor, an access delay with an unusable limit \
                                          or an unknown category, or a Cedar policy set \
                                          that does not parse or validate. The version \
                                          in force stays.",
             body = serde_json::Value),
            (status = 404, description = "No such register.", body = serde_json::Value),
            (status = 412, description = "`If-Match` names a version that is no longer \
                                          the newest: read the register again.",
             body = serde_json::Value),
            (status = 415, description = "The body is not `application/json`.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_policy_register_write as WRITE => |state, parts| super::body::gated(state, parts, write)
}

guarded_handler! {
    /// Read every version of one operational-policy register
    /// (`GET /admin/registers/{register}/history`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/registers/{register}/history", tag = "admin-registers",
        params(("register" = String, Path,
                description = "`audit-categories`, `access-delay` or `cedar-policies`.")),
        responses(
            (status = 200, description = "`{ \"register\": …, \"versions\": [...] }`: \
                                          every version as a read answers it, newest \
                                          first, ending with version 0, the default.",
             body = serde_json::Value),
            (status = 404, description = "No such register.", body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_policy_register_history as HISTORY => |state, parts| {
        super::body::gated(state, parts, history)
    }
}

/// The register the path names.
fn register(parts: &RequestParts) -> Result<Register, RestError> {
    let name = parts.path.get("register").ok_or_else(|| {
        RestError::from(ApiError::Internal(
            "missing path parameter `register`".to_owned(),
        ))
    })?;
    name.parse::<Register>()
        .map_err(|unknown| RestError::from(ApiError::NotFound(unknown.to_string())))
}

/// One version as the wire carries it.
fn version_body(version: &RegisterVersion) -> Result<serde_json::Value, RestError> {
    let document: serde_json::Value = serde_json::from_str(&version.document).map_err(|e| {
        RestError::from(crate::overview::error::internal_fault(
            "read a stored register document",
            &e,
        ))
    })?;
    Ok(serde_json::json!({
        "register": version.register.as_str(),
        "version": version.version,
        "committer": version.committer,
        "committed_at": version.committed_at.map(|at| at.to_string()),
        "document": document,
    }))
}

/// `version` as a response, with its version as the `ETag`.
fn answer(parts: &RequestParts, version: &RegisterVersion) -> Result<Response, RestError> {
    let mut resp =
        negotiate::body::respond(&parts.headers, StatusCode::OK, &version_body(version)?);
    if let Ok(etag) = HeaderValue::from_str(&format!("\"{}\"", version.version)) {
        resp.headers_mut().insert(header::ETAG, etag);
    }
    Ok(resp)
}

/// The `admin_policy_register_read` operation.
async fn read(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let register = register(&parts)?;
    let version = state.backend().policy_register(register).await?;
    answer(&parts, &version)
}

/// The `admin_policy_register_write` operation.
async fn write(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let register = register(&parts)?;
    let replaces = crate::overview::version_id::if_match(&parts.headers)
        .ok_or_else(|| {
            RestError::from(ApiError::BadRequest(
                "If-Match is required: name the register version this write replaces".to_owned(),
            ))
        })?
        .parse::<i64>()
        .map_err(|_not_a_version| {
            RestError::from(ApiError::BadRequest(
                "If-Match must be a register version, as the read's ETag gave it".to_owned(),
            ))
        })?;
    let document = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let document = serde_json::to_string(&document).map_err(|e| {
        RestError::from(crate::overview::error::internal_fault(
            "write the register document",
            &e,
        ))
    })?;
    state
        .backend()
        .put_policy_register(register, replaces, &document)
        .await?;
    let version = state.backend().policy_register(register).await?;
    answer(&parts, &version)
}

/// The `admin_policy_register_history` operation.
async fn history(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let register = register(&parts)?;
    let versions = state
        .backend()
        .policy_register_history(register)
        .await?
        .iter()
        .map(version_body)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::json!({ "register": register.as_str(), "versions": versions }),
    ))
}
