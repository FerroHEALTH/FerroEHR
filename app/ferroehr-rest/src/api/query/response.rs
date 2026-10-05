// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared request-decoding + response-rendering for the QUERY API
//! (`docs/query/Request.md`, `docs/query/Response.md`).
//!
//! The ad-hoc ([`super::adhoc`]) and stored ([`super::stored`]) execution paths
//! both normalize the request — the `ehr_id` scope, the `offset`/`fetch` paging
//! window and the `query_parameters` binds — and render the same `RESULT_SET`
//! document, so the decode helpers and the renderer live here and both paths
//! stay identical.
//!
//! `ehr_id` may arrive as the `ehr_id` query parameter or the `openehr-ehr-id`
//! request header (`Request.md` §About the `ehr_id` parameter), and the `200 OK`
//! response carries an `ETag` identifying the `RESULT_SET`
//! (`responses/200_Query.yaml` + `headers/ETag_RESULT_SET.yaml`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::Response;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use uuid::Uuid;

use openehr_its::rest::runtime::ApiError;

use crate::api::RequestParts;
use crate::negotiate;
use crate::overview::error::RestError;
use ferroehr::aql::sql::EhrAccessScope;
use ferroehr::service::query::request::AqlQueryRequest;

/// The access pre-filter derived from the request: the ABAC patient
/// subject-scope id, the touched-attribute collection flag, and the caller's
/// per-EHR `EHR_ACCESS` decision.
///
/// All three are applied uniformly to every normalized [`AqlQueryRequest`]
/// before execution. No openEHR spec governs how they reach a query — our own
/// access-control extension.
pub(super) struct QueryScope {
    /// The ABAC patient subject-scope id, if a scoped principal is configured.
    pub(super) subject_scope: Option<String>,
    /// Whether the executor must collect the touched EHR/template ids for the
    /// ABAC post-check.
    pub(super) collect: bool,
    /// The EHRs the caller's `EHR_ACCESS` decision lets the query read.
    pub(super) ehr_access: EhrAccessScope,
}

impl QueryScope {
    /// Stamps the ABAC scope, the collection flag and the per-EHR access
    /// decision onto a normalized request.
    pub(super) fn apply(&self, mut request: AqlQueryRequest) -> AqlQueryRequest {
        request.subject_scope.clone_from(&self.subject_scope);
        request.ehr_access.clone_from(&self.ehr_access);
        request.collect_attributes = self.collect;
        request
    }
}

/// Decodes a JSON request body into `T`.
///
/// The QUERY operations declare `application/json` only, so a non-JSON
/// `Content-Type` is rejected by [`negotiate::json_value`].
pub(super) fn decode_body<T: serde::de::DeserializeOwned>(
    h: &HeaderMap,
    body: &bytes::Bytes,
) -> Result<T, RestError> {
    let value = negotiate::json_value(h, body).map_err(RestError)?;
    serde_json::from_value(value).map_err(|e| {
        RestError(ApiError::BadRequest(format!(
            "invalid query request body: {e}"
        )))
    })
}

/// The canonical name of the `ehr_id` request header, spelled `openehr-ehr-id`
/// by `Request.md` §Common Headers and Query Parameters.
///
/// The pre-1.1.0 `openEHR-EHR-id` is the deprecated form of the same header and
/// remains available; HTTP field names are case-insensitive (RFC 9110 §5.1), so
/// both spellings resolve through this one lookup.
pub(super) const H_EHR_ID: &str = "openehr-ehr-id";

/// Resolves the single wire `ehr_id` scope of a query execution from the two
/// spec-sanctioned forms, identically for `GET` and `POST`: the `ehr_id` query
/// parameter and the [`H_EHR_ID`] request header, which clients "MAY supply
/// … as a query parameter `ehr_id` or alternatively as a request header"
/// (`Request.md` §About the `ehr_id` parameter).
///
/// The caller collects the result into [`AqlQueryRequest::ehr_ids`], a single
/// wire `ehr_id` being the one-element case of the SM `List<UUID>` scope.
///
/// The released text never says what happens when a request carries both forms,
/// so the handling is our own: one form, or both naming the same EHR, is the
/// scope; both naming different EHRs is a `400`, since the request is
/// self-contradictory and no released rule picks a winner
/// (`Requests_and_responses.md` §HTTP status codes).
///
/// # Errors
///
/// [`ApiError::BadRequest`] when the query parameter and the header name
/// different EHRs, or when repeated header field lines disagree.
pub(super) fn ehr_id_from_request(
    query_ehr_id: Option<String>,
    headers: &HeaderMap,
) -> Result<Option<String>, RestError> {
    let mut from_header: Option<&str> = None;
    for raw in headers.get_all(H_EHR_ID) {
        // RFC 9110 §5.5 permits an empty field value, which carries no EHR
        // identifier, so it is "not supplied" rather than a conflicting scope.
        let Some(value) = raw.to_str().ok().map(str::trim).filter(|v| !v.is_empty()) else {
            continue;
        };
        match from_header {
            Some(seen) if seen != value => return Err(conflicting_ehr_id(seen, value)),
            _ => from_header = Some(value),
        }
    }
    match (query_ehr_id, from_header) {
        (Some(from_query), Some(header)) if from_query != header => {
            Err(conflicting_ehr_id(&from_query, header))
        }
        (Some(from_query), _) => Ok(Some(from_query)),
        (None, header) => Ok(header.map(str::to_owned)),
    }
}

/// The `400` for a request whose `ehr_id` query parameter and `openehr-ehr-id`
/// header name different EHRs (see [`ehr_id_from_request`]).
fn conflicting_ehr_id(first: &str, second: &str) -> RestError {
    RestError(ApiError::BadRequest(format!(
        "conflicting EHR scope: the `ehr_id` query parameter and the `{H_EHR_ID}` \
         request header name different EHRs (`{first}` vs `{second}`)"
    )))
}

/// Returns a required path segment, read directly from the matched path because
/// the generated `*BodyParams` for POST carry only the name and version parts.
pub(super) fn path_segment(parts: &RequestParts, key: &str) -> Result<String, RestError> {
    parts.path.get(key).cloned().ok_or_else(|| {
        RestError(ApiError::BadRequest(format!(
            "missing path parameter `{key}`"
        )))
    })
}

/// Renders the assembled `RESULT_SET` as the `200 OK` response with the
/// spec-mandated `ETag`.
///
/// `200_Query.yaml` declares a weak `ETag` "identifier of the `RESULT_SET`"
/// (`headers/ETag_RESULT_SET.yaml`). The vendored `ResultSet` schema carries no
/// `id` field, so the tag is a stable digest of the document's
/// result-determining content ([`result_set_etag`]): identical result sets get
/// identical tags. The body is negotiated by [`negotiate::respond`] — JSON only,
/// the QUERY operations declaring no canonical-XML representation.
pub(super) fn respond_result_set(headers: &HeaderMap, result_set: &serde_json::Value) -> Response {
    let mut resp = negotiate::respond(headers, StatusCode::OK, result_set);
    // Only a genuine 200 carries a RESULT_SET identifier; a negotiated 406 is an
    // error body and gets no `ETag`.
    if resp.status().is_success()
        && let Ok(value) = HeaderValue::from_str(&result_set_etag(result_set))
    {
        resp.headers_mut().insert(header::ETAG, value);
    }
    resp
}

/// Returns a weak `ETag` for a `RESULT_SET`: a deterministic 128-bit digest of
/// the document's result-determining content, rendered as `W/"{uuid}"`.
///
/// The digest covers `name`, `q`, `meta._executed_aql`, `columns` and `rows`,
/// and not the response-stamped metadata: an `ETag` "changes as soon as the
/// resource changes" (`Requests_and_responses.md` §`ETag` and Last-Modified), so
/// a digest over the per-response `_created` would mint a fresh tag for an
/// unchanged result set. The executed AQL is included because two different
/// queries may coincidentally return the same rows and are not the same result
/// set.
fn result_set_etag(result_set: &serde_json::Value) -> String {
    let identity = serde_json::json!({
        "name": result_set.get("name"),
        "q": result_set.get("q"),
        "executed_aql": result_set.pointer("/meta/_executed_aql"),
        "columns": result_set.get("columns"),
        "rows": result_set.get("rows"),
    });
    let bytes = serde_json::to_vec(&identity).unwrap_or_default();
    format!("W/\"{}\"", stable_uuid(&bytes))
}

/// Returns a deterministic UUID from a byte digest: the first 128 bits of the
/// input's SHA-256 digest.
///
/// SHA-256 (the pinned `sha2` crate) is a fixed, published algorithm, so the
/// derived value is stable across program runs, Rust toolchains, and server
/// versions — a served `ETag` must never change for an unchanged `RESULT_SET`.
fn stable_uuid(bytes: &[u8]) -> Uuid {
    use sha2::Digest as _;
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    let (half, _) = digest.split_at(16);
    let mut arr = [0u8; 16];
    arr.copy_from_slice(half);
    Uuid::from_bytes(arr)
}

/// Merges a paging value carried in the POST body with the same-named URL query
/// parameter.
///
/// The docs-text SHOULD-list draws no GET/POST distinction (`Request.md` §Common
/// Headers and Query Parameters), so the URL forms are accepted on the POSTs
/// too; released text assigns no precedence, so a value carried in both places
/// must agree and a conflict is a `400`.
pub(super) fn merge_body_and_url_i64(
    body: Option<i64>,
    query: Option<&str>,
    key: &str,
) -> Result<Option<i64>, RestError> {
    #[expect(
        clippy::map_err_ignore,
        reason = "`ParseIntError` adds only \"invalid digit\"/\"out of range\" to a 400 \
                  body that already names the parameter and echoes the rejected value"
    )]
    let url = crate::params::query_param(query, key)
        .map(|raw| {
            raw.parse::<i64>().map_err(|_| {
                RestError(ApiError::BadRequest(format!(
                    "the `{key}` query parameter is not an integer: {raw:?}"
                )))
            })
        })
        .transpose()?;
    match (body, url) {
        (Some(b), Some(u)) if b != u => Err(RestError(ApiError::BadRequest(format!(
            "`{key}` is {b} in the request body but {u} in the URL — a request may \
             carry the value in either place, not two disagreeing ones"
        )))),
        (b, u) => Ok(b.or(u)),
    }
}

/// Merges the body `query_parameters` object with the URL's named `$parameter`
/// binds, the same named-binding law the GETs follow (`Request.md` §Query
/// parameters).
///
/// A parameter carried in both places must agree; a conflict is a `400`.
pub(super) fn merge_body_and_url_parameters(
    body: std::collections::BTreeMap<String, serde_json::Value>,
    query: Option<&str>,
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, RestError> {
    let url_only = crate::params::aql_binds(Some(crate::params::url_members(
        query,
        crate::params::QUERY_RESERVED_KEYS,
    )))?;
    // A disagreement between the two carriers is loud, never silently won.
    for (key, url_value) in &url_only {
        if let Some(body_value) = body.get(key)
            && body_value != url_value
        {
            return Err(RestError(ApiError::BadRequest(format!(
                "query parameter `{key}` differs between the request body and the URL"
            ))));
        }
    }
    let mut merged = body;
    for (key, url_value) in url_only {
        merged.insert(key, url_value);
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    fn etag(resp: &Response) -> Option<String> {
        resp.headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    #[test]
    fn stable_uuid_is_deterministic_and_content_sensitive() {
        assert_eq!(stable_uuid(b"abc"), stable_uuid(b"abc"));
        assert_ne!(stable_uuid(b"abc"), stable_uuid(b"abd"));
    }

    /// Pins a known `RESULT_SET` to its exact served `ETag` so a change to the
    /// digest algorithm (or the identity-document shape it hashes) fails
    /// loudly: a served `ETag` must never change for an unchanged `RESULT_SET`
    /// ("It changes as soon as the resource changes",
    /// `Requests_and_responses.md` §`ETag` and Last-Modified). The expected
    /// value is the first 128 bits of the SHA-256 digest of the identity
    /// document, computed independently with `openssl dgst -sha256`.
    #[test]
    fn result_set_etag_is_pinned_to_a_known_vector() {
        let rs = serde_json::json!({"rows": [["x"]]});
        assert_eq!(
            result_set_etag(&rs),
            "W/\"e9a0328f-52cf-352e-db43-ff1abe1de874\""
        );
    }

    #[test]
    fn result_set_etag_is_weak_and_stable() {
        let rs = serde_json::json!({"rows": [[1, 2], [3, 4]]});
        let tag = result_set_etag(&rs);
        // 200_Query.yaml / ETag_RESULT_SET.yaml: weak form `W/"…"`.
        assert!(tag.starts_with("W/\""), "weak ETag: {tag}");
        assert!(tag.ends_with('"'), "quoted: {tag}");
        // Stable for an identical RESULT_SET; different for a different one.
        assert_eq!(tag, result_set_etag(&rs));
        assert_ne!(tag, result_set_etag(&serde_json::json!({"rows": []})));
    }

    /// The tag identifies the RESULT SET, not the response instance: two
    /// executions of the same query over the same data differ only in the
    /// per-response `meta._created` stamp and MUST carry the same `ETag`
    /// ("It changes as soon as the resource changes",
    /// `Requests_and_responses.md` §`ETag` and Last-Modified) — while a genuine
    /// change in the executed AQL, the columns or the rows MUST change it
    /// ("A unique identifier of the resultSet", `Request.md` §Common Headers
    /// and Query Parameters).
    #[test]
    fn result_set_etag_ignores_the_per_response_created_stamp() {
        let execution = |created: &str, rows: serde_json::Value| {
            serde_json::json!({
                "meta": {
                    "_type": "RESULTSET",
                    "_schema_version": "1.0.0",
                    "_created": created,
                    "_executed_aql": "SELECT c/uid/value FROM COMPOSITION c",
                },
                "q": "SELECT c/uid/value FROM COMPOSITION c",
                "columns": [{ "name": "#0", "path": "/uid/value" }],
                "rows": rows,
            })
        };
        let rows = serde_json::json!([["8849182c-82ad-4088-a07f-48ead4180515::s::1"]]);
        assert_eq!(
            result_set_etag(&execution("2026-08-03T09:00:00Z", rows.clone())),
            result_set_etag(&execution("2026-08-03T11:30:42.117Z", rows.clone())),
            "re-executing the same query over the same data must yield the same tag"
        );
        assert_ne!(
            result_set_etag(&execution("2026-08-03T09:00:00Z", rows)),
            result_set_etag(&execution("2026-08-03T09:00:00Z", serde_json::json!([]))),
            "a different result set must yield a different tag"
        );
        // The executed AQL is part of the identity: identical rows from a
        // different query are not the same result set.
        let mut other_query = execution("2026-08-03T09:00:00Z", serde_json::json!([]));
        other_query["meta"]["_executed_aql"] =
            serde_json::json!("SELECT e/ehr_id/value FROM EHR e");
        assert_ne!(
            result_set_etag(&other_query),
            result_set_etag(&execution("2026-08-03T09:00:00Z", serde_json::json!([]))),
        );
    }

    #[test]
    fn respond_result_set_emits_etag_on_json_200() {
        // Empty Accept → JSON; the 200 carries the RESULT_SET ETag.
        let rs = serde_json::json!({"rows": []});
        let resp = respond_result_set(&HeaderMap::new(), &rs);
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            etag(&resp).is_some_and(|t| t.starts_with("W/\"")),
            "weak ETag present on 200"
        );
    }

    #[test]
    fn respond_result_set_no_etag_when_xml_only_accept() {
        // The query response has no canonical-XML shape → 406, and an error body
        // carries no RESULT_SET ETag.
        let rs = serde_json::json!({"rows": []});
        let resp = respond_result_set(&headers(&[("accept", "application/xml")]), &rs);
        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
        assert!(etag(&resp).is_none());
    }

    #[test]
    fn ehr_id_from_either_form() {
        // Request.md §About the ehr_id parameter: the query parameter OR the
        // `openehr-ehr-id` header; either alone is the scope.
        let h = headers(&[(H_EHR_ID, "from-header")]);
        assert_eq!(
            ehr_id_from_request(None, &h).unwrap().as_deref(),
            Some("from-header"),
            "the header alone scopes the execution"
        );
        assert_eq!(
            ehr_id_from_request(Some("from-query".to_owned()), &HeaderMap::new())
                .unwrap()
                .as_deref(),
            Some("from-query"),
            "the query parameter alone scopes the execution"
        );
        assert_eq!(
            ehr_id_from_request(None, &HeaderMap::new()).unwrap(),
            None,
            "neither form supplied is an unscoped (population) query"
        );
    }

    #[test]
    fn ehr_id_header_name_is_case_insensitive() {
        // Requests_and_responses.md §Deprecated headers pairs `openEHR-EHR-id`
        // with `openehr-ehr-id`; RFC 9110 §5.1 makes field names
        // case-insensitive, so the deprecated spelling resolves identically.
        let h = headers(&[("openEHR-EHR-id", "from-header")]);
        assert_eq!(
            ehr_id_from_request(None, &h).unwrap().as_deref(),
            Some("from-header")
        );
    }

    #[test]
    fn ehr_id_both_forms_agreeing_is_accepted() {
        // Both forms naming the SAME EHR: the request names one EHR, so there
        // is nothing to arbitrate.
        let h = headers(&[(H_EHR_ID, "same-ehr")]);
        assert_eq!(
            ehr_id_from_request(Some("same-ehr".to_owned()), &h)
                .unwrap()
                .as_deref(),
            Some("same-ehr")
        );
    }

    #[test]
    fn ehr_id_conflicting_forms_are_bad_request() {
        // Both forms naming DIFFERENT EHRs is self-contradictory → 400
        // (Requests_and_responses.md §HTTP status codes, row 400); no released
        // text assigns a precedence, so this handling is our own.
        let h = headers(&[(H_EHR_ID, "from-header")]);
        let err = ehr_id_from_request(Some("from-query".to_owned()), &h).unwrap_err();
        assert!(matches!(err.0, ApiError::BadRequest(_)), "got {:?}", err.0);
    }

    #[test]
    fn ehr_id_repeated_headers_must_agree() {
        // Two field lines of the same header naming different EHRs is the same
        // self-contradiction; agreeing lines resolve to the one value.
        let mut conflicting = HeaderMap::new();
        conflicting.append(H_EHR_ID, HeaderValue::from_static("a"));
        conflicting.append(H_EHR_ID, HeaderValue::from_static("b"));
        assert!(matches!(
            ehr_id_from_request(None, &conflicting).unwrap_err().0,
            ApiError::BadRequest(_)
        ));

        let mut agreeing = HeaderMap::new();
        agreeing.append(H_EHR_ID, HeaderValue::from_static("a"));
        agreeing.append(H_EHR_ID, HeaderValue::from_static("a"));
        assert_eq!(
            ehr_id_from_request(None, &agreeing).unwrap().as_deref(),
            Some("a")
        );
    }

    #[test]
    fn ehr_id_empty_header_value_is_not_supplied() {
        // RFC 9110 §5.5 permits empty field values; an empty `openehr-ehr-id`
        // carries no EHR identifier, so it neither scopes nor conflicts.
        let h = headers(&[(H_EHR_ID, "")]);
        assert_eq!(ehr_id_from_request(None, &h).unwrap(), None);
        assert_eq!(
            ehr_id_from_request(Some("from-query".to_owned()), &h)
                .unwrap()
                .as_deref(),
            Some("from-query")
        );
    }

    #[test]
    fn decode_body_parses_json_and_rejects_xml() {
        let json_h = headers(&[("content-type", "application/json")]);
        let value: serde_json::Value =
            decode_body(&json_h, &Bytes::from_static(br#"{"offset":0}"#)).unwrap();
        assert_eq!(value["offset"], 0);

        let xml_h = headers(&[("content-type", "application/xml")]);
        let err =
            decode_body::<serde_json::Value>(&xml_h, &Bytes::from_static(b"<q/>")).unwrap_err();
        assert!(
            matches!(err.0, ApiError::UnsupportedMediaType(_)),
            "got {:?}",
            err.0
        );
    }

    #[test]
    fn query_scope_apply_stamps_subject_collect_and_ehr_access() {
        let withheld = ferroehr::ids::EhrId(Uuid::nil());
        let scope = QueryScope {
            subject_scope: Some("patient-1".to_owned()),
            collect: true,
            ehr_access: EhrAccessScope::Excluding(vec![withheld]),
        };
        let req = scope.apply(AqlQueryRequest::default());
        assert_eq!(req.subject_scope.as_deref(), Some("patient-1"));
        assert!(req.collect_attributes);
        assert_eq!(req.ehr_access, EhrAccessScope::Excluding(vec![withheld]));
    }
}

// ── Query error-status mapping (the QUERY responses enumerate 400/404/408) ─────
//
// The status codes are assembled platform-side as typed `SmError`s and turned
// into the ITS-REST status by `crate::overview::error::sm_api_error`; this
// renderer emits the success `RESULT_SET` (and its `meta`, including the
// parameter-substituted `_executed_aql`) verbatim. `ehr_id_does_not_exist` →
// `404` for a well-formed-but-absent EHR id, while a malformed UUID stays a
// `400` (`Request.md` §About the ehr_id parameter).
// NOTE: a query-execution timeout is `408` (`responses/408_Query.yaml`),
// bounded by the `FERROEHR__QUERY__TIMEOUT_MS` budget; unset, only the blunt
// global `TimeoutLayer` applies.
