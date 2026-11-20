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

use axum::response::{IntoResponse as _, Response};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use uuid::Uuid;

use openehr_its::rest::runtime::ApiError;

use crate::api::RequestParts;
use crate::negotiate;
use crate::overview::error::RestError;
use ferroehr::access_log::view::LogStanding;
use ferroehr::ids::EhrId;
use ferroehr::service::query::request::{AqlQueryRequest, QueryAccess};
use ferroehr::service::query::result_set::ResultSetDoc;

/// The access pre-filter derived from the request: the caller's per-EHR
/// `EHR_ACCESS` decision and what the caller may read of the access log.
///
/// Both are applied uniformly to every normalized [`AqlQueryRequest`]
/// before execution. No openEHR spec governs how they reach a query — our own
/// access-control extension.
pub(super) struct QueryScope {
    /// What the caller may read of the access log (#5131).
    pub(super) log_standing: LogStanding,
}

impl QueryScope {
    /// Stamps the access-log standing onto a normalized request, and the
    /// running request's `EHR_ACCESS` condition (RM ehr `ehr_access.adoc`
    /// §`EHR_ACCESS` Class), which only a statement that reads EHR data
    /// judges, so a demographic query reads no settings (#6110).
    pub(super) fn apply(&self, mut request: AqlQueryRequest) -> AqlQueryRequest {
        request.ehr_access = QueryAccess::OfRequest;
        request.log_standing = self.log_standing.clone();
        request
    }
}

/// What a QUERY request body is called in the refusal of one that does not
/// decode.
const QUERY_BODY: &str = "query request body";

/// A QUERY body's members, as the canonical door judges them: the names of
/// its top-level members, and the members whose values are open JSON and may
/// hold a `_type`-tagged node.
pub(super) trait OpenMembers {
    /// The top-level member names the body carried.
    fn names(&self) -> Vec<&str>;
    /// The open members, each with the path of member names that leads to it.
    fn open(&self) -> Vec<(Vec<&str>, &serde_json::Value)>;
}

/// The open members of a body: `query_parameters` and the undeclared members.
fn open_members<'a>(
    parameters: Option<&'a std::collections::BTreeMap<String, serde_json::Value>>,
    undeclared: &'a std::collections::BTreeMap<String, serde_json::Value>,
) -> Vec<(Vec<&'a str>, &'a serde_json::Value)> {
    let parameters = parameters
        .into_iter()
        .flatten()
        .map(|(name, value)| (vec!["query_parameters", name.as_str()], value));
    let undeclared = undeclared
        .iter()
        .map(|(name, value)| (vec![name.as_str()], value));
    parameters.chain(undeclared).collect()
}

/// The names of a body's present members, then its undeclared ones.
fn member_names<'a>(
    present: &[(&'a str, bool)],
    undeclared: &'a std::collections::BTreeMap<String, serde_json::Value>,
) -> Vec<&'a str> {
    present
        .iter()
        .filter(|(_, carried)| *carried)
        .map(|(name, _)| *name)
        .chain(undeclared.keys().map(String::as_str))
        .collect()
}

/// Implements [`OpenMembers`] for a QUERY body DTO whose members are the
/// `$fixed` ones, the paging members and `query_parameters`.
macro_rules! paged_query_members {
    ($ty:ty, [$($fixed:literal),*]) => {
        impl OpenMembers for $ty {
            fn names(&self) -> Vec<&str> {
                let present = [
                    $(($fixed, true),)*
                    ("offset", self.offset.is_some()),
                    ("fetch", self.fetch.is_some()),
                    ("query_parameters", self.query_parameters.is_some()),
                ];
                member_names(&present, &self.additional_properties)
            }

            fn open(&self) -> Vec<(Vec<&str>, &serde_json::Value)> {
                open_members(self.query_parameters.as_ref(), &self.additional_properties)
            }
        }
    };
}

paged_query_members!(
    openehr_its::rest::generated::query::AdhocQueryExecute,
    ["q"]
);
paged_query_members!(openehr_its::rest::generated::query::Query, []);

/// Decodes a QUERY request body once, straight into its DTO `T`
/// ([`negotiate::decode::typed_json`]), and refuses a `_type`-tagged node in it
/// that carries a key its RM class does not declare, as the canonical door
/// does ([`openehr_its::json::reject_undeclared_keys`]).
///
/// # Errors
/// The decode's refusals, and a `400` for an undeclared key: a document the
/// reader cannot read (ITS-REST overview `Requests_and_responses.md` §HTTP
/// status codes).
pub(super) fn query_body<T>(h: &HeaderMap, body: &bytes::Bytes) -> Result<T, RestError>
where
    T: serde::de::DeserializeOwned + OpenMembers,
{
    let decoded: T = negotiate::decode::typed_json(h, body, QUERY_BODY)?;
    let refused = |e: openehr_its::json::JsonParseError| {
        RestError::from(ApiError::BadRequest(format!(
            "invalid canonical JSON body: {e}"
        )))
    };
    let names = decoded.names();
    let open = decoded.open();
    // NOTE: a body tagged at its root is judged on its member names alone, the only part of it the door reads.
    if let Some((_, tag)) = open.iter().find(|(path, _)| path.as_slice() == ["_type"]) {
        let root: serde_json::Map<String, serde_json::Value> = names
            .iter()
            .map(|name| {
                let value = if *name == "_type" {
                    (*tag).clone()
                } else {
                    serde_json::Value::Null
                };
                ((*name).to_owned(), value)
            })
            .collect();
        openehr_its::json::reject_undeclared_keys(&serde_json::Value::Object(root))
            .map_err(refused)?;
    }
    for (path, value) in &open {
        openehr_its::json::reject_undeclared_keys(value)
            .map_err(|e| refused(path.iter().rev().fold(e, |e, name| e.in_field(name))))?;
    }
    Ok(decoded)
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
/// [`ehr_scope`] parses the result into [`AqlQueryRequest::ehr_ids`], a single
/// wire `ehr_id` being the one-element case of the `ehr_ids` list scope.
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

/// The EHR scope of a query request: the one EHR [`ehr_id_from_request`]
/// resolves, parsed, as the one-element case of [`AqlQueryRequest::ehr_ids`].
///
/// # Errors
///
/// The [`ehr_id_from_request`] refusals, and [`ApiError::BadRequest`] when the
/// id is not a UUID.
#[expect(
    clippy::map_err_ignore,
    reason = "the mapped error already echoes the rejected token; the discarded \
              `uuid::Error` adds only its own wording, which is not part of the \
              wire contract"
)]
pub(super) fn ehr_scope(
    query_ehr_id: Option<String>,
    headers: &HeaderMap,
) -> Result<Vec<EhrId>, RestError> {
    ehr_id_from_request(query_ehr_id, headers)?
        .map(|id| {
            Uuid::parse_str(&id).map(EhrId).map_err(|_| {
                RestError::from(ApiError::BadRequest(format!("invalid ehr_id `{id}`")))
            })
        })
        .into_iter()
        .collect()
}

/// The `400` for a request whose `ehr_id` query parameter and `openehr-ehr-id`
/// header name different EHRs (see [`ehr_id_from_request`]).
fn conflicting_ehr_id(first: &str, second: &str) -> RestError {
    RestError::from(ApiError::BadRequest(format!(
        "conflicting EHR scope: the `ehr_id` query parameter and the `{H_EHR_ID}` \
         request header name different EHRs (`{first}` vs `{second}`)"
    )))
}

/// Returns a required path segment, read directly from the matched path because
/// the generated `*BodyParams` for POST carry only the name and version parts.
pub(super) fn path_segment(parts: &RequestParts, key: &str) -> Result<String, RestError> {
    parts.path.get(key).cloned().ok_or_else(|| {
        RestError::from(ApiError::BadRequest(format!(
            "missing path parameter `{key}`"
        )))
    })
}

/// Renders the assembled `RESULT_SET` as the `200 OK` response with the
/// spec-mandated `ETag`.
///
/// `200_Query.yaml` declares a weak `ETag` "identifier of the `RESULT_SET`"
/// (`headers/ETag_RESULT_SET.yaml`). The vendored `ResultSet` schema carries no
/// `id` field, so the tag is a digest of the document's result-determining
/// bytes as written ([`ResultSetDoc::write`]): identical result sets get
/// identical tags. The body is JSON only, the QUERY operations declaring no
/// canonical-XML representation, so an `Accept` that admits no JSON is a
/// `406`, which carries no `ETag`.
pub(super) fn respond_result_set(headers: &HeaderMap, result_set: &ResultSetDoc) -> Response {
    if negotiate::resolve_accept(
        headers,
        &[negotiate::WireFormat::CanonicalJson],
        negotiate::WireFormat::CanonicalJson,
    )
    .is_none()
    {
        return RestError::from(ApiError::NotAcceptable(
            "this response is available as application/json only".to_owned(),
        ))
        .into_response();
    }
    let written = match result_set.write() {
        Ok(written) => written,
        Err(e) => {
            return RestError::from(crate::overview::error::internal_fault(
                "write the RESULT_SET",
                &e,
            ))
            .into_response();
        }
    };
    let mut resp = negotiate::body::body(StatusCode::OK, negotiate::APPLICATION_JSON, written.body);
    if let Ok(value) = HeaderValue::from_str(&written.etag) {
        resp.headers_mut().insert(header::ETAG, value);
    }
    resp
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
                RestError::from(ApiError::BadRequest(format!(
                    "the `{key}` query parameter is not an integer: {raw:?}"
                )))
            })
        })
        .transpose()?;
    match (body, url) {
        (Some(b), Some(u)) if b != u => Err(RestError::from(ApiError::BadRequest(format!(
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
            return Err(RestError::from(ApiError::BadRequest(format!(
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
    fn respond_result_set_emits_etag_on_json_200() {
        // Empty Accept → JSON; the 200 carries the RESULT_SET ETag.
        let rs = ResultSetDoc::default();
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
        let rs = ResultSetDoc::default();
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
        assert!(
            matches!(err.error(), ApiError::BadRequest(_)),
            "got {:?}",
            err.error()
        );
    }

    #[test]
    fn ehr_id_repeated_headers_must_agree() {
        // Two field lines of the same header naming different EHRs is the same
        // self-contradiction; agreeing lines resolve to the one value.
        let mut conflicting = HeaderMap::new();
        conflicting.append(H_EHR_ID, HeaderValue::from_static("a"));
        conflicting.append(H_EHR_ID, HeaderValue::from_static("b"));
        assert!(matches!(
            ehr_id_from_request(None, &conflicting).unwrap_err().error(),
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
    fn a_query_body_parses_json_and_rejects_xml() {
        let json_h = headers(&[("content-type", "application/json")]);
        let value: serde_json::Value = negotiate::decode::typed_json(
            &json_h,
            &Bytes::from_static(br#"{"offset":0}"#),
            QUERY_BODY,
        )
        .unwrap();
        assert_eq!(value["offset"], 0);

        let xml_h = headers(&[("content-type", "application/xml")]);
        let err = negotiate::decode::typed_json::<serde_json::Value>(
            &xml_h,
            &Bytes::from_static(b"<q/>"),
            QUERY_BODY,
        )
        .unwrap_err();
        assert!(
            matches!(err, ApiError::UnsupportedMediaType(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn query_scope_apply_stamps_ehr_access_and_log_standing() {
        let scope = QueryScope {
            log_standing: LogStanding::Reviewer,
        };
        let req = scope.apply(AqlQueryRequest::default());
        assert_eq!(req.ehr_access, QueryAccess::OfRequest);
        assert_eq!(req.log_standing, LogStanding::Reviewer);
    }
}

// ── Query error-status mapping (the QUERY responses enumerate 400/404/408) ─────
//
// The status codes are assembled platform-side as typed `ServiceError`s and turned
// into the ITS-REST status by `crate::overview::service_error::api_error`; this
// renderer emits the success `RESULT_SET` (and its `meta`, including the
// parameter-substituted `_executed_aql`) verbatim. `ehr_id_does_not_exist` →
// `404` for a well-formed-but-absent EHR id, while a malformed UUID stays a
// `400` (`Request.md` §About the ehr_id parameter).
// NOTE: a query-execution timeout is `408` (`responses/408_Query.yaml`),
// bounded by the `FERROEHR__RECORD__QUERY__TIMEOUT` budget; unset, only the blunt
// global `TimeoutLayer` applies.
