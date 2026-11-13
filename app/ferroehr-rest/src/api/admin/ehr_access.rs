// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **EHR access settings** wire: the controller's route for the
//! person's access instructions, **our own extension**.
//!
//! The controller commits a new `EHR_ACCESS` version on the person's
//! instruction and reads every version of it. The EHDS Art. 8 restriction of
//! access by health professionals and the Art. 10 opt-out from the access
//! services (`docs/law/eu/ehds/text.html`) are `restriction` and `opt_out`
//! directives of its `access_directives` settings, so this is where they are
//! recorded, lifted and read. The EHR is named in the body and the query,
//! since a restricted EHR answers every path route that names it as an
//! unknown one. No openEHR ITS-REST operation governs these routes, and no
//! openEHR specification governs FerroEHR's access-control scheme.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::api.admin` config gate
//! (`405` with an empty `Allow` when off). Each commit writes its commit
//! audit and an access record, and each read an access record
//! (`ferroehr::service::admin::access_repair`). An `opt_out` directive is
//! refused while `[authz.grants] allowed` does not allow `access_service`,
//! since the CDR could then name no access service and the opt-out would hold
//! for nobody.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use axum::response::Response;
use ferroehr::party_binding::grants::operation::Operation;
use http::StatusCode;
use openehr_its::rest::runtime::ApiError;
use openehr_rm::draft::access_control::terminology::{DirectiveKind, Term as _};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::body::{AdminRefusal, RegisterReadRefusal, required_query, required_str};
use crate::api::{RequestParts, guarded_handler};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::parse_ehr_id;
use crate::state::AppState;

/// The EHR access settings routes — **no ITS-REST contract** (module docs).
pub(crate) fn ehr_access_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(admin_ehr_access_commit, admin_ehr_access_versions))
}

guarded_handler! {
    /// Commit an EHR's `EHR_ACCESS` settings on the person's instruction
    /// (`POST /admin/ehr-access`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/ehr-access", tag = "admin-marks",
        request_body(content = serde_json::Value,
                     description = "`{ \"ehr_id\": …, \"preceding_version_uid\": …, \
                                    \"settings\": … }`: the EHR, the `EHR_ACCESS` version \
                                    the change is made over, and the whole new \
                                    `EHR_ACCESS.settings`, an `ACCESS_DIRECTIVE_SETTINGS` \
                                    of the `access_directives` scheme, or `null` for none. \
                                    An EHDS Art. 8 restriction is a `restriction` deny \
                                    directive, of the whole EHR or of the objects its \
                                    `data` names; an Art. 10 opt-out is an `opt_out` deny \
                                    directive. Both set `emergency_override` true."),
        responses(
            (status = 200, description = "`{ \"ehr_id\": …, \"version_uid\": … }`: the \
                                          committed `EHR_ACCESS` version. Data a \
                                          restriction covers then reads as absent to \
                                          every professional route, query and export, \
                                          unless a caller holding \
                                          `authz.rbac.emergency_access_role` declares an \
                                          emergency purpose (EHDS Art. 11(5)); an EHR \
                                          an opt-out covers reads as absent to every \
                                          `access_service` grant holder, unless it \
                                          declares an emergency purpose and acts for no \
                                          person (EHDS Art. 10(2)). No status, header or \
                                          count says that either exists.",
             body = serde_json::Value),
            (status = 400, description = "The body is not the object above; the settings \
                                          are not settings of the `access_directives` \
                                          scheme whose every invariant holds; a \
                                          `restriction` or `opt_out` directive sets \
                                          `emergency_override` false; or an `opt_out` \
                                          directive is committed while \
                                          `authz.grants.allowed` does not allow \
                                          `access_service`. The problem names the first \
                                          defect.",
             body = serde_json::Value),
            (status = 404, description = "No such EHR.", body = serde_json::Value),
            (status = 412, description = "`preceding_version_uid` is not the current \
                                          `EHR_ACCESS` version.",
             body = serde_json::Value),
            (status = 415, description = "The body is not `application/json`.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_ehr_access_commit => |state, parts| {
            super::body::gated(state, parts, commit)
        }
}

guarded_handler! {
    /// Read every version of an EHR's `EHR_ACCESS` settings
    /// (`GET /admin/ehr-access`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/ehr-access", tag = "admin-marks",
        params(("ehr_id" = String, Query, description = "The EHR whose settings to read.")),
        responses(
            (status = 200, description = "`{ \"ehr_id\": …, \"versions\": [{ \
                                          \"version_uid\": …, \"settings\": … }] }`, \
                                          newest first: the person's instructions and \
                                          their history.",
             body = serde_json::Value),
            RegisterReadRefusal,
            AdminRefusal
        )
    )]
    admin_ehr_access_versions => |state, parts| super::body::gated(state, parts, versions)
}

/// The `admin_ehr_access_commit` operation.
async fn commit(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let ehr_id = parse_ehr_id(required_str(&body, "ehr_id")?)?;
    let preceding = required_str(&body, "preceding_version_uid")?;
    let Some(settings) = body
        .get("settings")
        .filter(|value| value.is_object() || value.is_null())
    else {
        return Err(RestError::from(ApiError::BadRequest(
            "request body must carry a `settings` object or null".to_owned(),
        )));
    };
    if opts_out(settings)
        && !state
            .backend()
            .grants()
            .ceiling()
            .allows(Operation::AccessService)
    {
        return Err(RestError::from(ApiError::BadRequest(
            "this deployment allows no EHDS Art. 10 access-service caller ([authz.grants] \
             allowed does not allow access_service), so an opt_out directive would hold for \
             nobody; allow access_service and grant it to the access services in the CDR first"
                .to_owned(),
        )));
    }
    let meta = state
        .backend()
        .commit_ehr_access_settings(ehr_id, preceding, settings.clone())
        .await?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::json!({ "ehr_id": meta.ehr_id, "version_uid": meta.uid }),
    ))
}

/// The `admin_ehr_access_versions` operation.
async fn versions(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let ehr_id = parse_ehr_id(&required_query(&parts, "ehr_id")?)?;
    let versions: Vec<serde_json::Value> = state
        .backend()
        .ehr_access_versions(ehr_id)
        .await?
        .into_iter()
        .map(|(uid, settings)| serde_json::json!({ "version_uid": uid, "settings": settings }))
        .collect();
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::json!({ "ehr_id": ehr_id.to_string(), "versions": versions }),
    ))
}

/// Whether `settings` hold an `opt_out` directive.
fn opts_out(settings: &serde_json::Value) -> bool {
    settings
        .get("directives")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|directives| {
            directives.iter().any(|directive| {
                directive
                    .pointer("/kind/defining_code/code_string")
                    .and_then(serde_json::Value::as_str)
                    == Some(DirectiveKind::OptOut.code())
            })
        })
}
