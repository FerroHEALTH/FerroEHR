// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The versioned-resource responses: the weak `ETag`, `Last-Modified` and
//! `Location` headers (ITS-REST overview `Requests_and_responses.md`
//! §"`ETag` and Last-Modified", §Location) and the read, write and delete
//! renderers that carry them.

use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use openehr_its::rest::runtime::ApiError;
use openehr_its::xml::runtime::ToXml;
use serde::de::DeserializeOwned;

use ferroehr::service::response::{ResourceMeta, ServiceResponse};

use super::body::{empty, respond, respond_rm};
use super::prefer::write_negotiated;

/// Format an HTTP-date (RFC 7231 IMF-fixdate, always GMT) for `Last-Modified`.
pub(crate) fn http_date(at: jiff::Timestamp) -> String {
    at.strftime("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

/// The tree of the REST surface a versioned resource lives under, which
/// decides its `Location` and the domain of its access record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tree {
    /// An EHR or a resource under `/ehr/{ehr_id}`.
    Ehr,
    /// A party, a party relationship or a demographic contribution under
    /// `/demographic`, which no EHR scopes.
    Demographic,
}

/// Build the (path-absolute) `Location` URL for a resource of `tree`.
pub(crate) fn location(
    base_path: &str,
    tree: Tree,
    ehr_id: &str,
    segment: Option<&str>,
    uid: &str,
) -> String {
    match (tree, segment) {
        (Tree::Ehr, Some(seg)) => format!("{base_path}/ehr/{ehr_id}/{seg}/{uid}"),
        (Tree::Ehr, None) => format!("{base_path}/ehr/{ehr_id}"),
        (Tree::Demographic, Some(seg)) => format!("{base_path}/demographic/{seg}/{uid}"),
        (Tree::Demographic, None) => format!("{base_path}/demographic/{uid}"),
    }
}

/// The `ETag` header value for a resource identifier: the weak form `W/"{uid}"`
/// (overview §"`ETag` and Last-Modified" — the `W/` weakness indicator is a MUST).
pub(crate) fn resource_etag(uid: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!("W/\"{uid}\"")).ok()
}

/// Set the weak `ETag` of a resource identifier on a response (overview
/// §"`ETag` and Last-Modified" — the value "is usually taken from e.g.
/// `VERSIONED_OBJECT.uid.value`, `VERSION.uid.value`, `EHR.ehr_id.value`").
/// The single place the `W/"…"` header is written.
pub(crate) fn set_etag(resp: &mut Response, uid: &str) {
    if let Some(etag) = resource_etag(uid) {
        resp.headers_mut().insert(header::ETAG, etag);
    }
}

/// Set the `Last-Modified` header to `at`, an HTTP-date (RFC 9110 §8.8.2).
pub(crate) fn set_last_modified(resp: &mut Response, at: jiff::Timestamp) {
    if let Ok(lm) = HeaderValue::from_str(&http_date(at)) {
        resp.headers_mut().insert(header::LAST_MODIFIED, lm);
    }
}

/// Set the versioning headers on a response: the weak `ETag` and — when the
/// metadata carries a commit time — `Last-Modified` (overview §"`ETag` and
/// Last-Modified": both SHOULD accompany `VERSION`/`VERSIONED_OBJECT`
/// responses). No `Location`: overview §Location scopes it to creation and
/// redirect responses, and §"Deprecated headers" deprecates it on `GET` and
/// `DELETE`. Also stamps the access record's object, in the domain of `tree`.
pub(crate) fn set_versioning_headers(resp: &mut Response, meta: &ResourceMeta, tree: Tree) {
    set_etag(resp, &meta.uid);
    if let Some(at) = meta.last_modified {
        set_last_modified(resp, at);
    }
    let (ehr_id, domain) = match tree {
        Tree::Ehr => (Some(meta.ehr_id.clone()), None),
        Tree::Demographic => (
            None,
            Some(ferroehr::system_log::event::AccessDomain::Demographic),
        ),
    };
    resp.extensions_mut()
        .insert(crate::system_log::middleware::AuditObject {
            ehr_id,
            uid: Some(meta.uid.clone()),
            result_count: None,
            domain,
            origins: meta.origins.clone(),
            origin_count: (!meta.origins.is_empty())
                .then(|| u64::try_from(meta.origins.len()).unwrap_or(u64::MAX)),
            content: (!meta.content.is_empty()).then(|| {
                ferroehr::system_log::categories::AccessedContent::Objects(meta.content.clone())
            }),
            ..crate::system_log::middleware::AuditObject::default()
        });
}

/// Set the full create/update response headers: the versioning headers plus
/// the `Location` of the version this request created (overview §Location:
/// "resource creation … or redirect responses"; an openEHR update commits a
/// new VERSION, which is a newly created resource at that URL).
pub(crate) fn set_resource_headers(
    resp: &mut Response,
    base_path: &str,
    tree: Tree,
    segment: Option<&str>,
    meta: &ResourceMeta,
) {
    set_versioning_headers(resp, meta, tree);
    let url = location(base_path, tree, &meta.ehr_id, segment, &meta.uid);
    if let Ok(loc) = HeaderValue::from_str(&url) {
        resp.headers_mut().insert(header::LOCATION, loc);
    }
}

/// Render a create/update response honouring `Prefer` and setting the
/// versioning + `Location` headers.
pub(crate) fn write_rm<T>(
    headers: &HeaderMap,
    base_path: &str,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    segment: Option<&str>,
    resp: &ServiceResponse,
    root_tag: &str,
) -> Response
where
    T: DeserializeOwned + ToXml,
{
    let mut out = write_negotiated(
        headers,
        minimal_status,
        repr_status,
        resp.meta.as_ref().map(|m| m.uid.as_str()),
        |status| respond_rm::<T>(headers, status, &resp.body, root_tag),
    );
    if let Some(meta) = &resp.meta {
        set_resource_headers(&mut out, base_path, Tree::Ehr, segment, meta);
    }
    out
}

/// As [`write_rm`] but for a JSON-only payload (no canonical-XML shape, e.g. a
/// CONTRIBUTION wrapper).
pub(crate) fn write_json(
    headers: &HeaderMap,
    base_path: &str,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    segment: Option<&str>,
    resp: &ServiceResponse,
) -> Response {
    let mut out = write_negotiated(
        headers,
        minimal_status,
        repr_status,
        resp.meta.as_ref().map(|m| m.uid.as_str()),
        |status| respond(headers, status, &resp.body),
    );
    if let Some(meta) = &resp.meta {
        set_resource_headers(&mut out, base_path, Tree::Ehr, segment, meta);
    }
    out
}

/// Render a `200 OK` read of a single spec-typed RM object, additionally
/// setting the weak `ETag`/`Last-Modified`. No `Location` (overview §Location).
pub(crate) fn read_rm<T>(headers: &HeaderMap, resp: &ServiceResponse, root_tag: &str) -> Response
where
    T: DeserializeOwned + ToXml,
{
    let mut out = respond_rm::<T>(headers, StatusCode::OK, &resp.body, root_tag);
    if let Some(meta) = &resp.meta {
        set_versioning_headers(&mut out, meta, Tree::Ehr);
    }
    out
}

/// A `204 No Content` delete outcome carrying the deleted version's weak
/// `ETag`/`Last-Modified`. No `Location` (overview §Location).
pub(crate) fn deleted_with_headers(resp: &ServiceResponse) -> Response {
    let mut out = empty(StatusCode::NO_CONTENT);
    if let Some(meta) = &resp.meta {
        set_versioning_headers(&mut out, meta, Tree::Ehr);
    }
    out
}

/// Render an error response, additionally setting the latest-version `ETag` the
/// spec requires on a `409`/`412` (overview §"If-Match and accidental
/// overwrites": the service "SHOULD return also latest `version_uid` in the
/// `ETag` response headers"). No `Location` on the error path.
pub(crate) fn error_with_meta(
    error: ApiError,
    meta: Option<&ResourceMeta>,
    tree: Tree,
) -> Response {
    let mut out = crate::overview::error::RestError::from(error).into_response();
    if let Some(meta) = meta {
        set_versioning_headers(&mut out, meta, tree);
    }
    out
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "wire assertions are raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]
mod tests {
    use super::*;
    use crate::overview::negotiate::APPLICATION_JSON;

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

    fn content_type(resp: &Response) -> Option<String> {
        resp.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    const BASE: &str = "/ferroehr/rest/openehr/v1";

    fn etag(resp: &Response) -> Option<String> {
        resp.headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    fn loc(resp: &Response) -> Option<String> {
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    fn preference_applied(resp: &Response) -> Option<String> {
        resp.headers()
            .get("preference-applied")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    #[test]
    fn location_builds_per_segment() {
        assert_eq!(
            location(BASE, Tree::Ehr, "e1", None, "e1"),
            format!("{BASE}/ehr/e1")
        );
        assert_eq!(
            location(BASE, Tree::Ehr, "e1", Some("composition"), "v::s::1"),
            format!("{BASE}/ehr/e1/composition/v::s::1")
        );
    }

    fn meta(ehr: &str, uid: &str) -> ResourceMeta {
        ResourceMeta::new(ehr.to_owned(), uid.to_owned())
    }

    #[test]
    fn write_rm_minimal_is_headers_only() {
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::1"));
        let out = write_rm::<Composition>(
            &HeaderMap::new(),
            BASE,
            StatusCode::CREATED,
            StatusCode::CREATED,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(out.status(), StatusCode::CREATED);
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::1\""));
        assert_eq!(
            loc(&out).as_deref(),
            Some(&*format!("{BASE}/ehr/e1/composition/v::s::1"))
        );
        assert_eq!(content_type(&out), None);
        assert_eq!(preference_applied(&out).as_deref(), Some("return=minimal"));
    }

    #[test]
    fn write_rm_representation_returns_body() {
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::2"));
        let h = headers(&[("prefer", "return=representation")]);
        let out = write_rm::<Composition>(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(out.status(), StatusCode::OK);
        assert_eq!(content_type(&out).as_deref(), Some(APPLICATION_JSON));
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::2\""));
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=representation")
        );
    }

    #[test]
    fn deleted_with_headers_is_204_with_weak_etag_no_location() {
        let resp = ServiceResponse::deleted(meta("e1", "v::s::3"));
        let out = deleted_with_headers(&resp);
        assert_eq!(out.status(), StatusCode::NO_CONTENT);
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::3\""));
        assert!(loc(&out).is_none());
    }

    #[test]
    fn error_with_meta_sets_latest_version_etag_only() {
        let out = error_with_meta(
            ApiError::PreconditionFailed("stale".to_owned()),
            Some(&meta("e1", "v::s::5")),
            Tree::Ehr,
        );
        assert_eq!(out.status(), StatusCode::PRECONDITION_FAILED);
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::5\""));
        assert!(loc(&out).is_none());
    }

    #[test]
    fn read_rm_emits_weak_etag_and_no_location() {
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::7"));
        let out = read_rm::<Composition>(&HeaderMap::new(), &resp, "composition");
        assert_eq!(out.status(), StatusCode::OK);
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::7\""));
        assert!(loc(&out).is_none());
    }

    #[test]
    fn resource_etag_is_weak() {
        let v = resource_etag("8849182c::openEHRSys::2").expect("etag");
        assert_eq!(v.to_str().unwrap(), "W/\"8849182c::openEHRSys::2\"");
    }

    #[tokio::test]
    async fn write_rm_identifier_returns_uid_body() {
        use http_body_util::BodyExt;
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::9"));
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_rm::<Composition>(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(out.status(), StatusCode::OK);
        assert_eq!(content_type(&out).as_deref(), Some(APPLICATION_JSON));
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::9\""));
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=identifier")
        );
        let bytes = out.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(body, serde_json::json!({ "uid": "v::s::9" }));
    }

    #[tokio::test]
    async fn write_rm_identifier_on_create_is_201_with_uid_body() {
        use http_body_util::BodyExt;
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::1"));
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_rm::<Composition>(
            &h,
            BASE,
            StatusCode::CREATED,
            StatusCode::CREATED,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(out.status(), StatusCode::CREATED);
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=identifier")
        );
        let bytes = out.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(body, serde_json::json!({ "uid": "v::s::1" }));
    }

    /// A write that yields no resource metadata cannot produce the identifier
    /// body the preference is defined by, so the server applies AND declares
    /// the default `return=minimal` — never `Preference-Applied:
    /// return=identifier` over an empty body (overview §Representation details
    /// negotiation: the header indicates the preference that "has been
    /// honored").
    #[test]
    fn write_rm_identifier_without_meta_declares_minimal() {
        use openehr_rm::prelude::Composition;
        let resp = ServiceResponse::plain(serde_json::Value::Null);
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_rm::<Composition>(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=minimal"),
            "an unapplied preference is never claimed"
        );
        assert_eq!(out.status(), StatusCode::NO_CONTENT);
        assert!(etag(&out).is_none());
    }

    #[tokio::test]
    async fn write_json_identifier_is_never_204() {
        use http_body_util::BodyExt;
        let value = serde_json::json!({"_type": "CONTRIBUTION"});
        let resp = ServiceResponse::new(value, meta("e1", "c::s::1"));
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_json(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("contribution"),
            &resp,
        );
        assert_eq!(
            out.status(),
            StatusCode::OK,
            "the identifier variant is 200/201, never 204 \
             (overview §Prefer only identifier)"
        );
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=identifier")
        );
        let bytes = out.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(body, serde_json::json!({ "uid": "c::s::1" }));
    }

    #[test]
    fn write_json_identifier_without_meta_declares_minimal() {
        let resp = ServiceResponse::plain(serde_json::Value::Null);
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_json(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("contribution"),
            &resp,
        );
        assert_eq!(preference_applied(&out).as_deref(), Some("return=minimal"));
        assert_eq!(out.status(), StatusCode::NO_CONTENT);
    }

    #[test]
    fn http_date_is_imf_fixdate() {
        let ts: jiff::Timestamp = "2009-07-22T19:15:56Z".parse().unwrap();
        assert_eq!(http_date(ts), "Wed, 22 Jul 2009 19:15:56 GMT");
    }

    #[test]
    fn set_resource_headers_emits_last_modified() {
        let ts: jiff::Timestamp = "2024-03-04T05:06:07Z".parse().unwrap();
        let m = ResourceMeta::new("e1".to_owned(), "v::s::1".to_owned()).with_last_modified(ts);
        let mut resp = empty(StatusCode::OK);
        set_resource_headers(&mut resp, BASE, Tree::Ehr, Some("composition"), &m);
        let lm = resp
            .headers()
            .get(header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok());
        assert_eq!(lm, Some("Mon, 04 Mar 2024 05:06:07 GMT"));
    }

    #[test]
    fn set_resource_headers_omits_last_modified_when_absent() {
        let m = ResourceMeta::new("e1".to_owned(), "v::s::1".to_owned());
        let mut resp = empty(StatusCode::OK);
        set_resource_headers(&mut resp, BASE, Tree::Ehr, Some("composition"), &m);
        assert!(resp.headers().get(header::LAST_MODIFIED).is_none());
    }
}
