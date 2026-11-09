// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **re-key** wire: the deliberate change of a `PERSON`'s binding
//! key, **our own extension**.
//!
//! A present binding key is immutable on every other write (RFC 7643 §2.2
//! `mutability: "immutable"`). This route is the one change: it refuses a new
//! key that is the administrator's own login or is already bound, retires
//! the old key in the commit that binds the new one, and writes one access
//! record carrying both. No openEHR ITS-REST operation governs it.
//!
//! Gating: mounted under `/admin/`, so the route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::api.admin` config gate
//! (`405` with an empty `Allow` when off) unchanged.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the admin register bodies are genuinely open \
              operational JSON, not RM content"
)]

use axum::response::Response;
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use ferroehr::service::demographic::rekey::BindingKey;

use super::body::required_str;
use crate::api::RequestParts;
use crate::api::guarded_handler;
use crate::api::refusals::JsonBodyRequired;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::parse_vo_id;
use crate::state::AppState;

/// The re-key route — **no ITS-REST contract** (module docs).
pub(crate) fn binding_key_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(admin_rekey_party))
}

guarded_handler! {
    /// Change a `PERSON`'s binding key (`POST /admin/party/binding-key`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/party/binding-key", tag = "admin-identifiers",
        request_body(content = serde_json::Value,
                     description = "`{ \"party_id\": …, \"issuer\": …, \"id\": … }`: the \
                                    `PERSON`'s container and the new key, under the \
                                    `identifier_issuer` of its `[[auth.party]]` table.",
                     example = json!({
                         "party_id": "0d6c7f1e-3b7a-4c2e-9f61-2a8e5d4c3b21",
                         "issuer": "https://login.example/tenant",
                         "id": "5b0f2c1a-7d3e-4f6a-8b9c-0d1e2f3a4b5c"
                     })),
        responses(
            (status = 204, description = "The `PERSON` carries the new key in a new \
                                          version, the old key no longer binds, and one \
                                          access record carries both."),
            (status = 400, description = "The body is not the object above.",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 403, description = "Not in the Admin class, or the new key is the \
                                          administrator's own login.",
             body = serde_json::Value),
            (status = 404, description = "No live `PERSON` has that container.",
             body = serde_json::Value),
            (status = 405, description = "The admin API is disabled on this server.",
             body = serde_json::Value),
            (status = 409, description = "The `PERSON` is of no mapped template, carries \
                                          no key of that issuer, or the new key is \
                                          already bound to another party.",
             body = serde_json::Value),
            (status = 422, description = "The new key names no issuer or no value.",
             body = serde_json::Value),
            JsonBodyRequired
        )
    )]
    admin_rekey_party => |state, parts| super::body::gated(state, parts, rekey)
}

/// The `admin_rekey_party` operation.
async fn rekey(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let body = negotiate::decode::json_value(&parts.headers, &parts.body)?;
    let person = parse_vo_id(required_str(&body, "party_id")?)?;
    let key = BindingKey {
        issuer: required_str(&body, "issuer")?.to_owned(),
        value: required_str(&body, "id")?.to_owned(),
    };
    state.backend().rekey_party(person, key).await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}
