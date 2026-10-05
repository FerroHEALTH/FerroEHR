// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Native `utoipa-axum` routing for the **Admin API group** — the two released
//! ITS-REST admin operations plus three of our own extension routes.
//!
//! Each `#[utoipa::path]` handler single-sources its route and its `OpenAPI`
//! path, then forwards to the group dispatcher ([`super::dispatch::dispatch`])
//! through [`guarded_dispatch`].
//!
//! The Admin API is `DEVELOPMENT`-state within ITS-REST Release-1.1.0
//! (`docs/admin/Description.md` §Status). That is a reporting qualifier only:
//! the BCP-14 requirement force of the released text is not state-qualified, so
//! every MUST and SHOULD below binds as it would on a STABLE group.
//!
//! `specifications/admin.openapi.yaml` mounts exactly two operations, `DELETE
//! /admin/ehr/{ehr_id}` and `DELETE /admin/ehr/all{?ehr_id*}`. The other three
//! routes below are our own extension, governed by no ITS-REST operation and
//! excluded from any conformance-profile claim.
//!
//! What the released responses declare shapes three properties here:
//!
//! - No released admin response carries a `headers:` block at all, so none is
//!   declared. The one response header this group emits is the `Allow` on the
//!   config-gate `405`, which comes from RFC 9110 rather than the openEHR text.
//! - `Location` is never declared on any route of the group: §"Deprecated
//!   headers" deprecates it on both `GET` and `DELETE` responses, and every
//!   route here is one or the other.
//! - Success is always `204`. Both operations declare a `202` for asynchronous
//!   execution; every delete here is synchronous, so that branch is never
//!   produced and is not declared — the served document describes only what this
//!   server emits.
//!
//! The group is config-gated (`AppConfig::admin.enabled`, default false), and
//! while off every route answers `405` with the openEHR error body before the
//! backend is touched. The ground differs per route and each declaration cites
//! its own: `admin_ehr_delete_all` has released provenance (its own NOTE plus
//! `responses/405.yaml`), while `admin_ehr_delete` and the three extensions rest
//! on the cross-cutting "If a method is recognized but not allowed for the
//! target resource, the response SHOULD be `405 Method Not Allowed`"
//! (`Requests_and_responses.md` §"HTTP Methods"). Because that `405` comes from
//! a matched handler, axum's allow-header machinery never runs, so the mandatory
//! `Allow` (RFC 9110 §15.5.6) is set explicitly, with the empty field value
//! §10.2.1 defines for a resource "temporarily disabled by configuration".
//!
//! Every released admin operation carries `security: []` and declares no
//! `401`/`403`, so the branches declared on all five routes are our own design:
//! everything under `/admin/` is `OperationClass::Admin` to the RBAC gate
//! ([`crate::extensions::access::authz`]), which runs before the config gate, so
//! an unauthenticated caller sees `401` and a non-admin principal `403` whether
//! or not the group is enabled.
//!
//! `parameters/query/ehr_id_Admin.yaml` is `explode: true` and optional, so an
//! absent or empty list means "delete ALL EHRs"; both the repeated form and a
//! comma-separated single value are accepted, read from the raw query string for
//! the reason `admin/dispatch.rs` gives.
//!
//! NOTE: the generated `admin_ehr_delete_all` route carries an RFC 6570
//! query-expansion suffix that is not part of the resource path, so the mounted
//! and documented path is the plain `/admin/ehr/all` — the normalization
//! [`crate::api::normalize_path`] applies to every generated template.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::api::guarded_dispatch;
use crate::state::AppState;

/// The Admin API group as a native `utoipa-axum` router (group-relative paths).
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_ehr_delete_all))
        .routes(routes!(admin_ehr_delete))
        .routes(routes!(admin_template_delete))
        .routes(routes!(admin_query_delete))
        .routes(routes!(admin_config))
}

/// The redacted effective configuration as a JSON tree (`GET /admin/config`).
///
/// **Our own extension — no ITS-REST operation governs this.** The released
/// Admin API (`specifications/admin.openapi.yaml`) defines exactly two
/// operations, both EHR deletes, and no openEHR specification models server
/// configuration at all; this route is excluded from any conformance-profile
/// claim. Same admin gate as the sibling deletes (`AppConfig::admin.enabled` →
/// `405` when off; RBAC Admin class by the `/admin/` path → `401`
/// unauthenticated / `403` non-admin).
///
/// Every secret-bearing leaf is redacted STRUCTURALLY by its `Secret` /
/// `SecretUrl` type before it ever reaches this handler — the binary builds the
/// snapshot once at boot with
/// [`ferroehr::config::FerroEhrConfig::to_redacted_json`], so no secret substring
/// is present here to leak.
#[utoipa::path(
    get, path = "/admin/config", tag = "ADMIN",
    params(
        ("Accept" = Option<String>, Header,
         description = "The tree is served as `application/json`. No openEHR \
                        format negotiation applies — this is not an RM \
                        resource, so neither canonical XML nor a Simplified \
                        format is meaningful for it.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The redacted effective configuration \
                                      (every secret leaf already `***`/\
                                      `scheme://***@…` by its `Secret`/\
                                      `SecretUrl` type). No response header \
                                      accompanies it — the tree is not a \
                                      versioned resource, so there is no `ETag` \
                                      or `Last-Modified` to derive, and \
                                      §\"Deprecated headers\" deprecates \
                                      `Location` on `GET`.",
         body = serde_json::Value,
         example = json!({
             "server": { "bind": "0.0.0.0:8080", "base_path": "/ferroehr/rest/openehr/v1" },
             "db": { "url": "postgres://***@db:5432/ferroehr", "max_connections": 20 },
             "admin": { "enabled": true }
         })),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released admin operations carry \
                                      `security: []` and declare no such branch \
                                      (see the module docs).",
         body = serde_json::Value,
         example = json!({ "error": "Unauthorized", "message": "no credentials supplied" })),
        (status = 403, description = "Authenticated but not in the Admin class \
                                      (`OperationClass::Admin`, keyed off the \
                                      `/admin/` path). Our own authorization \
                                      design.",
         body = serde_json::Value,
         example = json!({ "error": "Forbidden", "message": "operation requires the 'ADMIN' role" })),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false). This route has no released file, \
                                      so its ground is the cross-cutting \
                                      overview rule — \"If a method is \
                                      recognized but not allowed for the target \
                                      resource, the response SHOULD be `405 \
                                      Method Not Allowed` status code\" \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      Methods\") — not the bulk-delete NOTE, \
                                      which governs only that one operation.",
         body = serde_json::Value,
         headers(
             ("Allow" = String,
              description = "EMPTY. RFC 9110 §15.5.6 makes `Allow` mandatory on \
                             every `405`, and this one comes from a MATCHED \
                             handler, so axum's method-fallback machinery never \
                             supplies it. The empty field value is exactly what \
                             RFC 9110 §10.2.1 defines for this case: \"An empty \
                             Allow field value indicates that the resource \
                             allows no methods, which might occur in a 405 \
                             response if the resource has been temporarily \
                             disabled by configuration.\"")
         ),
         example = json!({
             "error": "Method Not Allowed",
             "message": "the admin API is disabled on this server"
         }))
    )
)]
pub(crate) async fn admin_config(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_config", parts, super::dispatch::dispatch).await
}

/// Delete multiple EHRs (`DELETE /admin/ehr/all`).
///
/// The released operation text (`operations/admin_ehr_delete_all.yaml`,
/// summary "Delete multiple EHRs"): "Deletes all or multiple EHRs, or a
/// specified subset of EHRs identified using the `ehr_id` query parameter."
///
/// > NOTE: This functionality is intended primarily for **development** or
/// > **testing** purposes and may be disabled in **production** environments,
/// > in which case server may respond with `405 Method Not Allowed`.
///
/// "All resources associated with or owned by the targeted EHRs (such as
/// `COMPOSITION`, `EHR_STATUS`, `ITEM_TAG`, `CONTRIBUTION`, and their historical
/// versions) will also be **permanently** and physically deleted, in compliance
/// with applicable data protection regulations (e.g., the GDPR in the European
/// Union)."
///
/// "The server may execute this operation asynchronously (e.g. in batches), in
/// which case returns status `202 Accepted`. If the deletion is processed
/// synchronously and completes successfully, the server returns status `204 No
/// Content`." — this server is SYNCHRONOUS, so `204` is the only success and
/// the `202` branch is never produced (hence not declared; see the module
/// docs).
///
/// This operation realizes NO SM interface operation:
/// `I_ADMIN_SERVICE.physical_ehr_delete` takes one `UUID[1]`
/// (`docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc`) and the
/// archive operations archive rather than delete — a released wire behaviour
/// with no service-model anchor to name it by.
///
/// The released generic `404` (`responses/404.yaml`: "`404 Not Found` is
/// returned when, based on the request parameters, the server did not find a
/// current representation of a target resource, or is not willing to disclose
/// that one exists.") is UNREACHABLE on this server, and is therefore NOT
/// declared: the released text defines no trigger for it on a mixed list, and
/// the shipped semantics are delete-what-exists → `204` (an id that names no
/// EHR deletes nothing and is not an error), with a malformed id rejected `400`
/// before any deletion runs.
#[utoipa::path(
    delete, path = "/admin/ehr/all", tag = "EHR",
    params(
        ("ehr_id" = Option<Vec<String>>, Query,
         description = "The EHR ids (UUIDs) to delete — \"An optional parameter \
                        to perform the operation on a subset of EHRs\" \
                        (`parameters/query/ehr_id_Admin.yaml`). OPTIONAL: an \
                        absent or empty list deletes ALL EHRs. Both the \
                        exploded/repeated form (`?ehr_id=a&ehr_id=b`, which is \
                        what the released `style: form` + `explode: true` \
                        means) and a comma-separated single value \
                        (`?ehr_id=a,b`) are accepted; blank entries are \
                        dropped.",
         example = json!([
             "7d44b88c-4199-4bad-97dc-d78268e01398",
             "297c3e91-7c17-4497-85dd-01e05aaae44e"
         ]))
    ),
    responses(
        (status = 204, description = "\"`204 No Content` is returned when the \
                                      requested operation succeeded and the \
                                      resource(s) identified by the request \
                                      parameters has been physically deleted \
                                      (i.e. hard-delete).\" \
                                      (`responses/204_deleted_hard.yaml`) — \
                                      synchronous and bodyless. No response \
                                      header: the released response file \
                                      declares none, the deleted EHRs have no \
                                      surviving version to tag, and \
                                      §\"Deprecated headers\" deprecates \
                                      `Location` on `DELETE`."),
        (status = 400, description = "An `ehr_id` in the list is not a \
                                      well-formed UUID. The whole bulk request \
                                      is rejected BEFORE any deletion runs \
                                      (nothing is deleted). The released \
                                      operation declares no `400`, so the \
                                      ground is the overview status table's \
                                      `400` row — \"malformed request syntax, \
                                      syntactically invalid content\" \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\").",
         body = serde_json::Value,
         example = json!({ "error": "Bad Request", "message": "invalid EHR id: not-a-uuid" })),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released operation carries \
                                      `security: []` and declares no such \
                                      branch (see the module docs).",
         body = serde_json::Value,
         example = json!({ "error": "Unauthorized", "message": "no credentials supplied" })),
        (status = 403, description = "Authenticated but not in the Admin class. \
                                      Our own authorization design.",
         body = serde_json::Value,
         example = json!({ "error": "Forbidden", "message": "operation requires the 'ADMIN' role" })),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false). This is the ONE route in the \
                                      group with its own released provenance: \
                                      the operation's NOTE — \"may be disabled \
                                      in **production** environments, in which \
                                      case server may respond with `405 Method \
                                      Not Allowed`\" — with \
                                      `responses/405.yaml` enumerated: \"`405 \
                                      Method Not Allowed` is returned when the \
                                      service knows the request method, but the \
                                      target resource doesn't support this \
                                      method (e.g. due to security \
                                      concerns).\"",
         body = serde_json::Value,
         headers(
             ("Allow" = String,
              description = "EMPTY — mandatory on every `405` (RFC 9110 \
                             §15.5.6) and emitted here explicitly because the \
                             status comes from a matched handler; the empty \
                             field value is RFC 9110 §10.2.1's own case for a \
                             resource \"temporarily disabled by \
                             configuration\".")
         ),
         example = json!({
             "error": "Method Not Allowed",
             "message": "the admin API is disabled on this server"
         }))
    )
)]
pub(crate) async fn admin_ehr_delete_all(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "admin_ehr_delete_all",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Delete EHR by id (`DELETE /admin/ehr/{ehr_id}`).
///
/// The released operation text (`operations/admin_ehr_delete.yaml`, summary
/// "Delete EHR by id"): "Deletes the EHR identified by `ehr_id`."
///
/// "All resources associated with or owned by the specified EHR (such as
/// `COMPOSITION`, `EHR_STATUS`, `ITEM_TAG`, `CONTRIBUTION`, and their historical
/// versions) will also be **permanently** and physically deleted, in compliance
/// with applicable data protection regulations (e.g., the GDPR in the European
/// Union)."
///
/// "The server may execute this operation asynchronously (e.g. in batches), in
/// which case returns status `202 Accepted`. If the deletion is processed
/// synchronously and completes successfully, the server returns status `204 No
/// Content`." — this server is SYNCHRONOUS, so `204` is the only success and
/// the `202` branch is never produced (hence not declared; see the module
/// docs).
///
/// Realizes SM `I_ADMIN_SERVICE.physical_ehr_delete`
/// (`docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc` —
/// precondition `has_ehr`, error `ehr_id_does_not_exist`). Note that the
/// released operation does NOT carry the bulk route's development/testing NOTE
/// and does NOT enumerate `405`: this single-EHR delete is not declared
/// disable-in-production by the released text, so the group's config gate
/// grounds its `405` on the overview §"HTTP Methods" rule instead.
#[utoipa::path(
    delete, path = "/admin/ehr/{ehr_id}", tag = "EHR",
    params(
        ("ehr_id" = String, Path,
         description = "\"EHR identifier taken from EHR.ehr_id.value.\" \
                        (`parameters/path/ehr_id.yaml`) — a UUID \
                        (`format: uuid`).",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398")
    ),
    responses(
        (status = 204, description = "\"`204 No Content` is returned when the \
                                      requested operation succeeded and the \
                                      resource(s) identified by the request \
                                      parameters has been physically deleted \
                                      (i.e. hard-delete).\" \
                                      (`responses/204_deleted_hard.yaml`) — \
                                      synchronous and bodyless. No response \
                                      header: the released response file \
                                      declares none, a physically deleted EHR \
                                      has no surviving version to tag, and \
                                      §\"Deprecated headers\" deprecates \
                                      `Location` on `DELETE`."),
        (status = 400, description = "`ehr_id` is not a well-formed UUID \
                                      (rejected before any deletion). The \
                                      released operation declares no `400`, so \
                                      the ground is the overview status table's \
                                      `400` row — \"malformed request syntax, \
                                      syntactically invalid content\" \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\").",
         body = serde_json::Value,
         example = json!({ "error": "Bad Request", "message": "invalid EHR id: not-a-uuid" })),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released operation carries \
                                      `security: []` and declares no such \
                                      branch (see the module docs).",
         body = serde_json::Value,
         example = json!({ "error": "Unauthorized", "message": "no credentials supplied" })),
        (status = 403, description = "Authenticated but not in the Admin class. \
                                      Our own authorization design.",
         body = serde_json::Value,
         example = json!({ "error": "Forbidden", "message": "operation requires the 'ADMIN' role" })),
        (status = 404, description = "\"`404 Not Found` is returned when an EHR \
                                      with `ehr_id` does not exist.\" — this \
                                      route's OWN released response file \
                                      (`responses/404_unknown_ehr_id.yaml`), \
                                      distinct from the generic \
                                      `responses/404.yaml` the bulk route \
                                      binds. It is the REST reading of the SM \
                                      precondition `has_ehr` failing \
                                      (`ehr_id_does_not_exist`, which \
                                      `i_admin_service.adoc` defines only \
                                      abstractly, with no HTTP binding).",
         body = serde_json::Value,
         example = json!({
             "error": "Not Found",
             "message": "EHR 7d44b88c-4199-4bad-97dc-d78268e01398"
         })),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false). The released operation enumerates \
                                      no `405` and carries no \
                                      disable-in-production NOTE — that NOTE \
                                      belongs to `admin_ehr_delete_all` alone — \
                                      so the ground here is the cross-cutting \
                                      overview rule: \"If a method is \
                                      recognized but not allowed for the target \
                                      resource, the response SHOULD be `405 \
                                      Method Not Allowed` status code\" \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      Methods\").",
         body = serde_json::Value,
         headers(
             ("Allow" = String,
              description = "EMPTY — mandatory on every `405` (RFC 9110 \
                             §15.5.6) and emitted here explicitly because the \
                             status comes from a matched handler; the empty \
                             field value is RFC 9110 §10.2.1's own case for a \
                             resource \"temporarily disabled by \
                             configuration\".")
         ),
         example = json!({
             "error": "Method Not Allowed",
             "message": "the admin API is disabled on this server"
         }))
    )
)]
pub(crate) async fn admin_ehr_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_ehr_delete", parts, super::dispatch::dispatch).await
}

/// Physically delete one operational template by its `template_id`
/// (`DELETE /admin/template/{template_id}`).
///
/// **Our own extension — no ITS-REST operation governs this.** The released
/// Admin API (`specifications/admin.openapi.yaml`) defines exactly two
/// operations, both EHR deletes; there is no `DELETE` on any
/// `/definition/template/adl1.4*` path either. This route is excluded from any
/// conformance-profile claim.
///
/// SM relation: it is the wire ANALOGUE of `I_DEFINITION_ADL14.delete_opt`
/// (`docs/specs/openehr/SM/docs/UML/classes/i_definition_adl14.adoc`), not a
/// realization of it — that SM operation has no released wire at all — and it
/// is keyed by the OPT's internal id whereas this route is
/// addressed by the wire `template_id` (matched case-insensitively, overview
/// §"Composite Identifiers and Case").
///
/// The delete is guarded: it refuses with `409` while any committed version
/// still references the template, so a physical delete can never orphan
/// committed clinical data (the `version.template_id` foreign key is the
/// underlying integrity guard). No openEHR spec governs that guard — our own
/// design.
#[utoipa::path(
    delete, path = "/admin/template/{template_id}", tag = "ADMIN",
    params(
        ("template_id" = String, Path,
         description = "The `template_id` — the wire address of the OPT, \
                        matched case-insensitively (overview §\"Composite \
                        Identifiers and Case\"), exactly as on the released \
                        `/definition/template/adl1.4/{template_id}` reads.",
         example = "openEHR-EHR-COMPOSITION.minimal.v1")
    ),
    responses(
        (status = 204, description = "Deleted (bodyless). Extension route — no \
                                      released response file governs this; the \
                                      status mirrors the released hard-delete \
                                      `204` (`responses/204_deleted_hard.yaml`). \
                                      No response header: a deleted template \
                                      has nothing to tag, and §\"Deprecated \
                                      headers\" deprecates `Location` on \
                                      `DELETE`."),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design.",
         body = serde_json::Value,
         example = json!({ "error": "Unauthorized", "message": "no credentials supplied" })),
        (status = 403, description = "Authenticated but not in the Admin class. \
                                      Our own authorization design.",
         body = serde_json::Value,
         example = json!({ "error": "Forbidden", "message": "operation requires the 'ADMIN' role" })),
        (status = 404, description = "No template with `template_id`. Extension \
                                      route — the status follows the overview \
                                      table's `404` row \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\"), by convention, not by \
                                      obligation.",
         body = serde_json::Value,
         example = json!({
             "error": "Not Found",
             "message": "template openEHR-EHR-COMPOSITION.minimal.v1"
         })),
        (status = 409, description = "A committed version still references the \
                                      template, so deleting it would orphan \
                                      clinical data. Our own guard — no openEHR \
                                      spec governs it; the status follows the \
                                      overview table's `409` row, a request \
                                      that \"might generate a duplicate or a \
                                      conflict\". The message names how many \
                                      committed versions still hold the \
                                      reference.",
         body = serde_json::Value,
         example = json!({
             "error": "Conflict",
             "message": "template 'openEHR-EHR-COMPOSITION.minimal.v1' is still referenced by 3 committed version(s); delete those compositions before deleting the template"
         })),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false). This route has no released file, \
                                      so its ground is the cross-cutting \
                                      overview rule — \"If a method is \
                                      recognized but not allowed for the target \
                                      resource, the response SHOULD be `405 \
                                      Method Not Allowed` status code\" \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      Methods\") — not the bulk-delete NOTE, \
                                      which governs only that one operation.",
         body = serde_json::Value,
         headers(
             ("Allow" = String,
              description = "EMPTY — mandatory on every `405` (RFC 9110 \
                             §15.5.6) and emitted here explicitly because the \
                             status comes from a matched handler; the empty \
                             field value is RFC 9110 §10.2.1's own case for a \
                             resource \"temporarily disabled by \
                             configuration\".")
         ),
         example = json!({
             "error": "Method Not Allowed",
             "message": "the admin API is disabled on this server"
         }))
    )
)]
pub(crate) async fn admin_template_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "admin_template_delete",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Physically delete one stored-query version — a single `(name, version)` row
/// (`DELETE /admin/query/{qualified_query_name}/{version}`).
///
/// **Our own extension — no ITS-REST operation governs this.** The released
/// Admin API (`specifications/admin.openapi.yaml`) defines exactly two
/// operations, both EHR deletes, and the released Definition API has no
/// `DELETE` on `/definition/query/…` either. This route is excluded from any
/// conformance-profile claim.
///
/// It does **NOT** realize SM `I_DEFINITION_QUERY.delete_query`
/// (`docs/specs/openehr/SM/docs/UML/classes/i_definition_query.adoc`): that
/// operation is keyed by NAME ALONE — `Pre_has_query: has_query(a_query_name)`,
/// `Post_query_deleted: not has_query (a_query_name)` — so it removes every
/// version of the query, whereas this route removes exactly one
/// `(name, version)` row and leaves the query's other versions in place. The
/// SM operation therefore stays unrealized on this server;
/// naming it here would be a false claim.
#[utoipa::path(
    delete, path = "/admin/query/{qualified_query_name}/{version}", tag = "ADMIN",
    params(
        ("qualified_query_name" = String, Path,
         description = "The qualified query name \
                        (`[{namespace}::]{query-name}`), matched \
                        case-insensitively — as on the released stored-query \
                        `PUT` (overview §\"Composite Identifiers and Case\").",
         example = "org.openehr::compositions"),
        ("version" = String, Path,
         description = "The exact stored SEMVER version to delete. Only this \
                        one row is removed; the query's other versions survive \
                        (which is why this is not SM `delete_query`).",
         example = "1.0.2")
    ),
    responses(
        (status = 204, description = "Deleted (bodyless). Extension route — no \
                                      released response file governs this; the \
                                      status mirrors the released hard-delete \
                                      `204` (`responses/204_deleted_hard.yaml`). \
                                      No response header: a deleted row has \
                                      nothing to tag, and §\"Deprecated \
                                      headers\" deprecates `Location` on \
                                      `DELETE`."),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design.",
         body = serde_json::Value,
         example = json!({ "error": "Unauthorized", "message": "no credentials supplied" })),
        (status = 403, description = "Authenticated but not in the Admin class. \
                                      Our own authorization design.",
         body = serde_json::Value,
         example = json!({ "error": "Forbidden", "message": "operation requires the 'ADMIN' role" })),
        (status = 404, description = "No stored query at that \
                                      `(name, version)` — an unknown name, or a \
                                      known name with no such version. \
                                      Extension route — the status follows the \
                                      overview table's `404` row \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\"), by convention, not by \
                                      obligation.",
         body = serde_json::Value,
         example = json!({
             "error": "Not Found",
             "message": "stored query org.openehr::compositions at version 1.0.2"
         })),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false). This route has no released file, \
                                      so its ground is the cross-cutting \
                                      overview rule — \"If a method is \
                                      recognized but not allowed for the target \
                                      resource, the response SHOULD be `405 \
                                      Method Not Allowed` status code\" \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      Methods\") — not the bulk-delete NOTE, \
                                      which governs only that one operation.",
         body = serde_json::Value,
         headers(
             ("Allow" = String,
              description = "EMPTY — mandatory on every `405` (RFC 9110 \
                             §15.5.6) and emitted here explicitly because the \
                             status comes from a matched handler; the empty \
                             field value is RFC 9110 §10.2.1's own case for a \
                             resource \"temporarily disabled by \
                             configuration\".")
         ),
         example = json!({
             "error": "Method Not Allowed",
             "message": "the admin API is disabled on this server"
         }))
    )
)]
pub(crate) async fn admin_query_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "admin_query_delete",
        parts,
        super::dispatch::dispatch,
    )
    .await
}
