// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the `admin` API group, over the concrete
//! `ferroehr::service::FerroEhrService` admin methods.
//!
//! The ITS-REST Admin API is `DEVELOPMENT` (`specifications/docs/admin/
//! Description.md` §Status) and mounts exactly two operations, both physical EHR
//! delete: `admin_ehr_delete` (`operations/admin_ehr_delete.yaml` — a physical
//! cascade of every owned resource, `204` sync / `202` async / `404` for an
//! unknown id) and
//! `admin_ehr_delete_all` (`operations/admin_ehr_delete_all.yaml` — all EHRs or
//! the `ehr_id` subset, same status set). Beside them sit four of our own
//! extension routes that no ITS-REST operation governs: template delete,
//! physical party delete,
//! stored-query-version delete, and the redacted config read.
//!
//! The group is config-gated (`AppConfig::api.admin`, default false): while
//! disabled every admin route answers `405 Method Not Allowed` with an empty
//! `Allow`, without touching the backend — declared for the bulk delete by
//! `responses/405.yaml`, and for the rest by the cross-cutting SHOULD in
//! `docs/overview/Requests_and_responses.md` §"HTTP Methods".

use std::fmt;
use std::str::FromStr;

use axum::Json;
use axum::response::{IntoResponse, Response};
use http::StatusCode;

use openehr_its::rest::decode::query_component_octets;
use openehr_its::rest::generated::admin::AdminEhrDeleteParams;
use openehr_its::rest::runtime::ApiError;
use openehr_its::rest::stored_query::{QualifiedQueryName, QueryVersion};

use crate::api::RequestParts;
use crate::overview::error::RestError;
use crate::overview::version_id::{parse_ehr_id, parse_party_id};

use crate::negotiate;
use crate::state::AppState;

/// The `admin_ehr_delete` operation.
pub(crate) async fn ehr_delete(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let p = parts.decode(
        AdminEhrDeleteParams::from_request,
        AdminEhrDeleteParams::PARAMS,
    )?;
    // Physical EHR delete → 204 No Content; unknown EHR → 404
    // (the service maps `ehr_id_does_not_exist` to NotFound).
    state
        .backend()
        .admin_ehr_delete(parse_ehr_id(&p.ehr_id)?)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The `admin_template_delete` operation.
pub(crate) async fn template_delete(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let template_id = path_segment(&parts, "template_id")?;
    // A template still referenced by a committed version is a 409:
    // never orphan clinical data.
    state.backend().admin_template_delete(&template_id).await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The `admin_party_delete` operation.
pub(crate) async fn party_delete(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let party_id = path_segment(&parts, "party_id")?;
    state
        .backend()
        .physical_party_delete(parse_party_id(&party_id)?)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The `admin_query_delete` operation.
pub(crate) async fn query_delete(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let qualified_name: QualifiedQueryName = parse_segment(&parts, "qualified_query_name")?;
    let version: QueryVersion = parse_segment(&parts, "version")?;
    state
        .backend()
        .admin_query_delete(&qualified_name, version)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The `admin_config` operation.
pub(crate) fn config(state: &AppState) -> Response {
    if let Some(refusal) = admin_group_gate(state) {
        return refusal;
    }
    // NOTE: no openEHR spec governs configuration — our own extension.
    // The binary builds this snapshot at boot, so every secret leaf is
    // already redacted by its own `Secret`/`SecretUrl` type.
    let snapshot = state.observability().env_snapshot.as_ref().clone();
    (StatusCode::OK, Json(snapshot)).into_response()
}

/// The `admin_ehr_delete_all` operation.
pub(crate) async fn ehr_delete_all(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let q = parts.query.as_deref();
    // The generated `AdminEhrDeleteAllParams.ehr_id: Option<String>`
    // under-models the RFC 6570 `{?ehr_id*}` list, so the full list is
    // read from the raw query. `ehr_id` is optional
    // (`parameters/query/ehr_id_Admin.yaml`), so an empty list means
    // "delete ALL EHRs" (`operations/admin_ehr_delete_all.yaml`), which
    // is the semantics the service seam honours.
    let ids = ehr_id_list(q)
        .iter()
        .map(|id| parse_ehr_id(id))
        .collect::<Result<Vec<_>, _>>()?;
    state.backend().admin_ehr_delete_all(&ids).await?;
    // The only declared success responses are a bodyless `204` (sync)
    // and `202` (async); this server is synchronous.
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// The ADMIN group's config gate, shared by every handler mounted under
/// `/admin/`: these, and those of the [`archive`](super::archive) and
/// [`report`](super::report) extension groups.
///
/// `Some(refusal)` while `AppConfig::api.admin` is off, `None` when the
/// group serves. One decision in one place, so no route under `/admin/` can
/// drift out of the gate.
///
/// The `405` comes from a matched handler, so axum's allow-header machinery
/// never runs and the `Allow` RFC 9110 §15.5.6 mandates is stated as the
/// empty field value: §10.2.1's case for a resource "temporarily disabled by
/// configuration".
pub(super) fn admin_group_gate(state: &AppState) -> Option<Response> {
    (!state.config().api.admin).then(|| {
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
        RestError::from(ApiError::BadRequest(format!(
            "missing path parameter '{key}'"
        )))
    })
}

/// The path parameter `key` read by its grammar.
///
/// # Errors
///
/// [`ApiError::BadRequest`] when the parameter is missing or breaks the
/// grammar, the overview `400` row for a malformed request.
fn parse_segment<T>(parts: &RequestParts, key: &str) -> Result<T, RestError>
where
    T: FromStr<Err: fmt::Display>,
{
    path_segment(parts, key)?.parse().map_err(|err| {
        RestError::from(ApiError::BadRequest(format!(
            "path parameter '{key}': {err}"
        )))
    })
}

/// Collects every `ehr_id` value from the raw query string, splitting each on
/// commas, so both the repeated and the comma-separated forms yield the full
/// list. Blank entries are dropped.
///
/// The value is split on its literal commas (the RFC 6570 §3.2.8 list
/// delimiter) before each item is percent-decoded as every other query
/// parameter is ([`query_component_octets`], RFC 3986 §2.1).
fn ehr_id_list(query: Option<&str>) -> Vec<String> {
    let Some(q) = query else {
        return Vec::new();
    };
    q.split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(k, _)| *k == "ehr_id")
        .flat_map(|(_, v)| v.split(','))
        .map(|item| {
            String::from_utf8_lossy(&query_component_octets(item))
                .trim()
                .to_owned()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::ehr_id_list;

    #[test]
    fn parses_comma_separated_and_repeated() {
        assert_eq!(ehr_id_list(None), Vec::<String>::new());
        assert_eq!(ehr_id_list(Some("")), Vec::<String>::new());
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
        assert_eq!(ehr_id_list(Some("ehr_id=")), Vec::<String>::new());
        // Each item is percent-decoded after the comma split, a plus kept (RFC 3986 §2.1).
        assert_eq!(
            ehr_id_list(Some("ehr_id=a%2Db,%20c%20,d+e")),
            vec!["a-b".to_owned(), "c".to_owned(), "d+e".to_owned()]
        );
    }
}
