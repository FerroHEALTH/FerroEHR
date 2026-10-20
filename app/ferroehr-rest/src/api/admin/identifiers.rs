// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **protected-identifier read** wire — **our own extension**.
//!
//! No openEHR ITS-REST operation and no SM interface governs this route. The
//! RM models an identifier as a plain `DV_IDENTIFIER` (RM `data_types`
//! `UML/classes/org.openehr.rm.data_types.dv_identifier.adoc`) and says nothing
//! about protecting its value; `[demographic.identifier_protection]` seals the
//! national identifiers of the configured schemes, and this route is the one
//! supported way to see one again (#3776).
//!
//! Gating: mounted under `/admin/`, so the route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::admin.enabled` config gate
//! (`405` with an empty `Allow` when off). Beside that it needs its own
//! permission, `[authz.rbac] identifier_reader_role`, so holding the admin role
//! alone reveals no national identifier. Every read writes an access record
//! per scheme expanded, naming the reader, the party and the scheme, never the
//! value (`ferroehr::service::demographic::identifier`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the expanded party version is served \
              as the canonical JSON the version read returns, wrapped in an operational \
              envelope"
)]

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::{negotiate, params};

/// The protected-identifier read as a native `utoipa-axum` router — **no
/// ITS-REST contract** (see the module docs).
pub(crate) fn identifier_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(admin_party_identifiers))
}

/// Read a party version with its protected identifiers expanded
/// (`GET /admin/party/identifiers`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    get, path = "/admin/party/identifiers", tag = "admin-identifiers",
    params(("version_uid" = String, Query,
            description = "The `OBJECT_VERSION_ID` of the party version to read. Any \
                           version may be named, so a historical version shows the \
                           values it committed.")),
    responses(
        (status = 200, description = "`{ \"version_uid\": …, \"expanded_schemes\": [...], \
                                      \"version\": … }`: the party's \
                                      `ORIGINAL_VERSION` with each protected \
                                      identifier put back to its value, and the \
                                      scheme of each value put back. The body is a \
                                      view: it no longer matches the stored, signed \
                                      version. Each scheme expanded is one access \
                                      record naming the reader, the party and the \
                                      scheme, never the value.",
         body = serde_json::Value),
        (status = 400, description = "`version_uid` is missing or not an \
                                      `OBJECT_VERSION_ID`, or the deployment \
                                      configures no identifier protection.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class, or not holding \
                                      `authz.rbac.identifier_reader_role` (unset: \
                                      nobody holds it).",
         body = serde_json::Value),
        (status = 404, description = "No party holds that version.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value),
        (status = 503, description = "The access record could not be written under \
                                      `audit.fail_mode = \"closed\"`; the values are \
                                      withheld.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_party_identifiers(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_party_identifiers", parts, dispatch).await
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
    if let Some(refusal) = super::dispatch::admin_group_gate(&state) {
        return Ok(refusal);
    }
    if op != "admin_party_identifiers" {
        return Err(RestError(ApiError::Internal(format!(
            "unrouted admin identifier operation: {op}"
        ))));
    }
    let roles = crate::extensions::access::authn::current_principal()
        .map(|principal| principal.roles)
        .unwrap_or_default();
    if !holds_identifier_reader_role(
        state.config().rbac.identifier_reader_role.as_deref(),
        &roles,
    ) {
        return Err(RestError(ApiError::Forbidden(
            "reading a party's protected identifiers needs the role \
             `authz.rbac.identifier_reader_role` names"
                .to_owned(),
        )));
    }
    let version_uid =
        params::query_param(parts.query.as_deref(), "version_uid").ok_or_else(|| {
            RestError(ApiError::BadRequest(
                "the `version_uid` query parameter is required".to_owned(),
            ))
        })?;
    let (version, schemes) = state.backend().read_party_identifiers(&version_uid).await?;
    Ok(negotiate::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::json!({
            "version_uid": version_uid,
            "expanded_schemes": schemes,
            "version": version,
        }),
    ))
}

/// Whether `roles` holds `role`, compared ASCII-case-insensitively like every
/// other role; an unset role is held by nobody.
fn holds_identifier_reader_role(role: Option<&str>, roles: &[String]) -> bool {
    role.is_some_and(|role| roles.iter().any(|held| held.eq_ignore_ascii_case(role)))
}

#[cfg(test)]
mod tests {
    use super::holds_identifier_reader_role;

    #[test]
    fn an_unset_role_is_held_by_nobody_and_a_set_one_by_its_holders() {
        let admin = vec!["ADMIN".to_owned()];
        assert!(!holds_identifier_reader_role(None, &admin));
        assert!(!holds_identifier_reader_role(
            Some("IDENTIFIER_READER"),
            &admin
        ));
        let reader = vec!["ADMIN".to_owned(), "IDENTIFIER_READER".to_owned()];
        assert!(holds_identifier_reader_role(
            Some("identifier_reader"),
            &reader
        ));
    }
}
