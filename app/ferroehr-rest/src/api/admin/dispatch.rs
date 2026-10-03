// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the `admin` API group, over the concrete
//! `ferroehr::service::FerroEhrService` admin methods.
//!
//! The ITS-REST Admin API is `DEVELOPMENT` (`specifications/docs/admin/
//! Description.md` §Status) and mounts exactly two operations, both physical EHR
//! delete: `admin_ehr_delete` (`operations/admin_ehr_delete.yaml` — a physical
//! cascade of every owned resource, `204` sync / `202` async / `404` for an
//! unknown id, matching SM `I_ADMIN_SERVICE.physical_ehr_delete`) and
//! `admin_ehr_delete_all` (`operations/admin_ehr_delete_all.yaml` — all EHRs or
//! the `ehr_id` subset, same status set). Beside them sit three of our own
//! extension routes that no ITS-REST operation governs: template delete,
//! stored-query-version delete, and the redacted config read.
//!
//! The group is config-gated (`AppConfig::admin.enabled`, default false): while
//! disabled every admin route answers `405 Method Not Allowed` with an empty
//! `Allow`, without touching the backend — declared for the bulk delete by
//! `responses/405.yaml`, and for the rest by the cross-cutting SHOULD in
//! `docs/overview/Requests_and_responses.md` §"HTTP Methods".

use axum::Json;
use axum::response::{IntoResponse, Response};
use http::StatusCode;

use openehr_its::rest::generated::admin::AdminEhrDeleteParams;
use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts};
use crate::overview::error::RestError;

use crate::negotiate;
use crate::state::AppState;

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
    // This `405` comes from a matched handler, so axum's allow-header machinery
    // never runs and the `Allow` RFC 9110 §15.5.6 mandates is stated as the
    // empty field value — §10.2.1's case for a resource "temporarily disabled by
    // configuration".
    if let Some(refusal) = admin_group_gate(&state) {
        return Ok(refusal);
    }
    let h = &parts.headers;
    let q = parts.query.as_deref();

    match op {
        "admin_ehr_delete" => {
            let p = AdminEhrDeleteParams::from_request(
                &parts.route_for(AdminEhrDeleteParams::PARAMS),
                q,
                h,
            )?;
            // SM physical_ehr_delete → 204 No Content; unknown EHR → 404
            // (the service maps `ehr_id_does_not_exist` to NotFound).
            state.backend().admin_ehr_delete(p.ehr_id).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        // NOTE: no openEHR spec governs the routes below — our own extensions;
        // both mirror `admin_ehr_delete` (204, 404, same admin gate).
        "admin_template_delete" => {
            let template_id = path_segment(&parts, "template_id")?;
            // A template still referenced by a committed version is a 409:
            // never orphan clinical data.
            state.backend().admin_template_delete(template_id).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_query_delete" => {
            let qualified_name = path_segment(&parts, "qualified_query_name")?;
            let version = path_segment(&parts, "version")?;
            state
                .backend()
                .admin_query_delete(qualified_name, version)
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_config" => {
            // NOTE: no openEHR spec governs configuration — our own extension.
            // The binary builds this snapshot at boot, so every secret leaf is
            // already redacted by its own `Secret`/`SecretUrl` type.
            let snapshot = state.observability().env_snapshot.as_ref().clone();
            Ok((StatusCode::OK, Json(snapshot)).into_response())
        }
        "admin_ehr_delete_all" => {
            // The generated `AdminEhrDeleteAllParams.ehr_id: Option<String>`
            // under-models the RFC 6570 `{?ehr_id*}` list, so the full list is
            // read from the raw query. `ehr_id` is optional
            // (`parameters/query/ehr_id_Admin.yaml`), so an empty list means
            // "delete ALL EHRs" (`operations/admin_ehr_delete_all.yaml`), which
            // is the semantics the service seam honours.
            let ids = ehr_id_list(q);
            state.backend().admin_ehr_delete_all(ids).await?;
            // The only declared success responses are a bodyless `204` (sync)
            // and `202` (async); this server is synchronous.
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted admin operation: {other}"
        )))),
    }
}

/// The ADMIN group's config gate, shared by every dispatcher mounted under
/// `/admin/` — this one plus the [`archive`](super::archive) and
/// [`report`](super::report) extension groups.
///
/// `Some(refusal)` while `AppConfig::admin.enabled` is off, `None` when the
/// group serves. One decision in one place, so no route under `/admin/` can
/// drift out of the gate.
pub(super) fn admin_group_gate(state: &AppState) -> Option<Response> {
    (!state.config().admin.enabled).then(|| {
        crate::overview::error::method_not_allowed_response(
            "",
            "the admin API is disabled on this server",
        )
    })
}

/// Reads a required path segment for the admin extension routes, which no
/// generated params type models.
///
/// A missing segment is impossible for a matched route, but is mapped to a `400`
/// rather than panicking.
fn path_segment(parts: &RequestParts, key: &str) -> Result<String, RestError> {
    parts.path.get(key).cloned().ok_or_else(|| {
        RestError(ApiError::BadRequest(format!(
            "missing path parameter '{key}'"
        )))
    })
}

/// Collects every `ehr_id` value from the raw query string, splitting each on
/// commas, so both the repeated and the comma-separated forms yield the full
/// list. Blank entries are dropped.
///
/// A plain query walk rather than percent-decoding: `ehr_id`s are UUIDs, so no
/// decoding is needed.
fn ehr_id_list(query: Option<&str>) -> Vec<String> {
    let Some(q) = query else {
        return Vec::new();
    };
    q.split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(k, _)| *k == "ehr_id")
        .flat_map(|(_, v)| v.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::ehr_id_list;

    #[test]
    fn parses_comma_separated_and_repeated() {
        assert!(ehr_id_list(None).is_empty());
        assert!(ehr_id_list(Some("")).is_empty());
        assert_eq!(ehr_id_list(Some("ehr_id=a")), vec!["a".to_owned()]);
        assert_eq!(
            ehr_id_list(Some("ehr_id=a,b")),
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert_eq!(
            ehr_id_list(Some("ehr_id=a&ehr_id=b")),
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert_eq!(
            ehr_id_list(Some("ehr_id=a, b &other=x&ehr_id=c")),
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
        );
        // A present-but-empty value contributes nothing.
        assert!(ehr_id_list(Some("ehr_id=")).is_empty());
    }
}
