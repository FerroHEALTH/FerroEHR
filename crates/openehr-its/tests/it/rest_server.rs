// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::panic_in_result_fn,
    reason = "test assertions in the Book's Result-returning shape (.claude/rules/testing.md \
              §Test shapes); `allow`, not `expect`, because #[tokio::test] moves each body into \
              an async block, which this lint does not reach uniformly"
)]
//! Contract tests for the generated ITS-REST routers (`server::router`).
//!
//! Each test drives a router through `tower::ServiceExt::oneshot` — no socket —
//! and pins one wire behaviour: how a request becomes the operation's params
//! struct and body, and how the trait method's answer or `Refusal` becomes
//! the response. The statuses and headers per operation are the vendored OAS
//! (`crates/openehr-its/vendor/rest-oas/<group>-codegen.openapi.yaml`); the
//! `Prefer` and `Content-Type` semantics are the ITS-REST docs text
//! (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
//! §Prefer minimal, identifier or full representation response, and
//! `Resources.md` §JSON Format).

use std::error::Error;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use http::{Method, StatusCode};
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_its::rest::generated::common::Identifier;
use openehr_its::rest::generated::{admin, definition, demographic, ehr, query, system};
use openehr_its::rest::runtime::{ApiError, Payload, Refusal, Representation};
use openehr_its::rest::server;
use openehr_rm::v1_2::composition::composition::Composition;
use openehr_rm::v1_2::demographic::person::Person;
use openehr_rm::v1_2::ehr::ehr_status::EhrStatus;
use serde_json::{Value, json};
use tower::ServiceExt as _;

/// A test's plumbing error: every fallible step propagates with `?`.
type TestResult = Result<(), Box<dyn Error>>;

/// The EHR id every request names.
const EHR_ID: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";

/// A version uid (`OBJECT_VERSION_ID` form).
const VERSION_UID: &str = "8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1";

/// The version a precondition failure names as current.
const CURRENT_VERSION: &str = "8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::2";

/// An implementation of every group's trait that overrides nothing.
#[derive(Debug)]
struct Unimplemented;

impl admin::server::AdminApi for Unimplemented {}
impl definition::server::DefinitionApi for Unimplemented {}
impl demographic::server::DemographicApi for Unimplemented {}
impl ehr::server::EhrApi for Unimplemented {}
impl query::server::QueryApi for Unimplemented {}
impl system::server::SystemApi for Unimplemented {}

/// What the recording stub saw, one line per call.
#[derive(Debug, Default)]
struct Recorder {
    seen: Mutex<Vec<String>>,
}

impl Recorder {
    fn record(&self, line: String) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(line);
        }
    }

    fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl ehr::server::EhrApi for Recorder {
    async fn ehr_create(
        &self,
        params: ehr::EhrCreateParams,
        body: Option<Payload<EhrStatus, Value>>,
    ) -> Result<ehr::server::EhrCreateResponse, Refusal> {
        self.record(format!(
            "ehr_create prefer={:?} audit={:?} body={}",
            params.prefer,
            params.openehr_audit_details,
            body.is_some()
        ));
        let location = Some(format!("https://cdr.example.org/openehr/v1/ehr/{EHR_ID}"));
        let etag = Some(format!("\"{EHR_ID}\""));
        if params.prefer.as_deref() == Some("return=representation") {
            return Ok(ehr::server::EhrCreateResponse::Created {
                body: Some(Representation::Identifier(Identifier {
                    uid: EHR_ID.to_owned(),
                    additional_properties: std::collections::BTreeMap::new(),
                })),
                headers: ehr::EhrCreateCreatedHeaders { etag, location },
            });
        }
        Ok(ehr::server::EhrCreateResponse::NoContent {
            headers: ehr::EhrCreateNoContentHeaders { etag, location },
        })
    }

    async fn contribution_create(
        &self,
        _params: ehr::ContributionCreateParams,
        body: Payload<ehr::NewContribution, ehr::NewContribution<Value>>,
    ) -> Result<ehr::server::ContributionCreateResponse, Refusal> {
        let (form, preceding) = match &body {
            Payload::Canonical(c) => (
                "canonical",
                c.versions.first().map(|v| v.preceding_version_uid.clone()),
            ),
            Payload::Flat(c) => (
                "flat",
                c.versions.first().map(|v| v.preceding_version_uid.clone()),
            ),
            Payload::Structured(c) => (
                "structured",
                c.versions.first().map(|v| v.preceding_version_uid.clone()),
            ),
        };
        self.record(format!(
            "contribution_create {form} preceding={:?}",
            preceding.flatten().map(|uid| uid.value().to_owned())
        ));
        Ok(ehr::server::ContributionCreateResponse::NoContent {
            headers: ehr::ContributionCreateNoContentHeaders {
                etag: None,
                location: None,
                content_type: None,
            },
        })
    }

    async fn ehr_status_get_by_version_id(
        &self,
        params: ehr::EhrStatusGetByVersionIdParams,
    ) -> Result<ehr::server::EhrStatusGetByVersionIdResponse, Refusal> {
        self.record(format!(
            "ehr_status_get_by_version_id ehr_id={} version_uid={}",
            params.ehr_id, params.version_uid
        ));
        Err(ApiError::NotFound(format!("no EHR_STATUS version {}", params.version_uid)).into())
    }

    async fn composition_update(
        &self,
        params: ehr::CompositionUpdateParams,
        _body: Payload<Composition, Value>,
    ) -> Result<ehr::server::CompositionUpdateResponse, Refusal> {
        self.record(format!("composition_update if_match={}", params.if_match));
        // The current version is `…::2`, so any other precondition fails.
        Err(Refusal::new(ApiError::PreconditionFailed(
            "the composition has a newer version".to_owned(),
        ))
        .try_with_header("ETag", &format!("\"{CURRENT_VERSION}\""))?)
    }
}

#[async_trait::async_trait]
impl query::server::QueryApi for Recorder {
    async fn query_execute_adhoc_query(
        &self,
        params: query::QueryExecuteAdhocQueryParams,
    ) -> Result<query::server::QueryExecuteAdhocQueryResponse, Refusal> {
        self.record(format!(
            "adhoc q={} ehr_id={:?} offset={:?} fetch={:?} members={} accept={:?}",
            params.q,
            params.ehr_id,
            params.offset,
            params.fetch,
            serde_json::to_string(&params.query_parameters).unwrap_or_default(),
            params.accept,
        ));
        Ok(query::server::QueryExecuteAdhocQueryResponse::Ok {
            body: result_set(&params.q),
            headers: query::QueryExecuteAdhocQueryOkHeaders {
                etag: Some("\"rs-1\"".to_owned()),
                content_type: None,
            },
        })
    }

    async fn query_execute_adhoc_query_body(
        &self,
        _params: query::QueryExecuteAdhocQueryBodyParams,
        body: query::AdhocQueryExecute,
    ) -> Result<query::server::QueryExecuteAdhocQueryBodyResponse, Refusal> {
        self.record(format!("adhoc_body q={} fetch={:?}", body.q, body.fetch));
        Ok(query::server::QueryExecuteAdhocQueryBodyResponse::Ok {
            body: result_set(&body.q),
            headers: query::QueryExecuteAdhocQueryBodyOkHeaders::default(),
        })
    }
}

/// A one-row result set echoing the query text.
fn result_set(q: &str) -> query::ResultSet {
    query::ResultSet {
        meta: None,
        name: None,
        q: Some(q.to_owned()),
        columns: None,
        rows: vec![vec![json!(1)]],
        additional_properties: std::collections::BTreeMap::new(),
    }
}

/// The ehr and query routers over one recorder, plus the recorder.
fn recorder_app() -> (axum::Router, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let app = ehr::server::router(Arc::clone(&recorder))
        .merge(query::server::router(Arc::clone(&recorder)));
    (app, recorder)
}

/// A request of `method` to `uri` with `headers` and `body`.
fn request(
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<http::Request<Body>, Box<dyn Error>> {
    let mut builder = http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    Ok(builder.body(Body::from(body.to_owned()))?)
}

/// The answer to `request` from `app`: status, headers and body bytes.
async fn send(
    app: axum::Router,
    request: http::Request<Body>,
) -> Result<(StatusCode, http::HeaderMap, Vec<u8>), Box<dyn Error>> {
    let response = app.oneshot(request).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, headers, body.to_vec()))
}

/// The `message` of an ITS-REST `Error` body, asserting the body is one.
fn error_message(headers: &http::HeaderMap, body: &[u8]) -> Result<String, Box<dyn Error>> {
    assert_eq!(
        headers
            .get(http::header::CONTENT_TYPE)
            .map(http::HeaderValue::as_bytes),
        Some(&b"application/json"[..]),
        "an error answers as canonical JSON"
    );
    let error: Value = serde_json::from_slice(body)?;
    assert!(
        error.get("validationErrors").is_some_and(Value::is_array),
        "the ITS-REST `Error` body carries `validationErrors`: {error}"
    );
    Ok(error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned())
}

/// Every group's router mounts beside every other under one base path, and an
/// operation an implementation does not override answers `501` with the
/// ITS-REST `Error` body.
#[tokio::test]
async fn every_group_router_mounts_together_and_a_default_answers_501() -> TestResult {
    let api = Arc::new(Unimplemented);
    let all = axum::Router::new()
        .merge(admin::server::router(Arc::clone(&api)))
        .merge(definition::server::router(Arc::clone(&api)))
        .merge(demographic::server::router(Arc::clone(&api)))
        .merge(ehr::server::router(Arc::clone(&api)))
        .merge(query::server::router(Arc::clone(&api)))
        .merge(system::server::router(Arc::clone(&api)));
    let app = axum::Router::new().nest("/openehr/v1", all);
    for (method, uri) in [
        (Method::POST, "/openehr/v1/ehr"),
        (Method::GET, "/openehr/v1/query/aql?q=SELECT%201"),
        (Method::DELETE, "/openehr/v1/admin/ehr/all"),
        (Method::GET, "/openehr/v1/definition/template/adl1.4"),
        (Method::GET, "/openehr/v1/demographic/tags"),
    ] {
        let (status, headers, body) =
            send(app.clone(), request(method.clone(), uri, &[], "")?).await?;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{method} {uri}");
        assert_eq!(error_message(&headers, &body)?, "not implemented");
    }
    Ok(())
}

/// Query pairs reach their params fields typed, a pair that is not a declared
/// parameter becomes a member of the form-exploded `query_parameters` object,
/// and a header parameter is read by its wire name; the answer carries its
/// JSON body and its declared `ETag`.
#[tokio::test]
async fn query_and_header_parameters_reach_the_params_struct() -> TestResult {
    let (app, recorder) = recorder_app();
    let uri = format!(
        "/query/aql?q=SELECT%20c%20FROM%20COMPOSITION%20c&ehr_id={EHR_ID}&offset=5&fetch=10\
         &temperature=36.6&code=a%2Bb"
    );
    let (status, headers, body) = send(
        app,
        request(Method::GET, &uri, &[("Accept", "application/json")], "")?,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("etag").map(http::HeaderValue::as_bytes),
        Some(&b"\"rs-1\""[..])
    );
    assert_eq!(
        headers
            .get(http::header::CONTENT_TYPE)
            .map(http::HeaderValue::as_bytes),
        Some(&b"application/json"[..])
    );
    let rows: Value = serde_json::from_slice(&body)?;
    assert_eq!(
        rows,
        json!({ "q": "SELECT c FROM COMPOSITION c", "rows": [[1]] })
    );
    assert_eq!(
        recorder.seen(),
        [format!(
            "adhoc q=SELECT c FROM COMPOSITION c ehr_id=Some(\"{EHR_ID}\") offset=Some(5) \
             fetch=Some(10) members={{\"code\":\"a+b\",\"temperature\":36.6}} \
             accept=Some(\"application/json\")"
        )]
    );
    Ok(())
}

/// Path parameters are read by position into their wire-named fields,
/// percent-decoded; a sibling route that names a different parameter at the
/// same position stays its own operation.
#[tokio::test]
async fn path_parameters_reach_the_params_struct() -> TestResult {
    let (app, recorder) = recorder_app();
    let encoded = VERSION_UID.replace("::", "%3A%3A");
    let uri = format!("/ehr/{EHR_ID}/ehr_status/{encoded}");
    let (status, headers, body) = send(app.clone(), request(Method::GET, &uri, &[], "")?).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        error_message(&headers, &body)?,
        format!("not found: no EHR_STATUS version {VERSION_UID}")
    );
    assert_eq!(
        recorder.seen(),
        [format!(
            "ehr_status_get_by_version_id ehr_id={EHR_ID} version_uid={VERSION_UID}"
        )]
    );
    let tags = format!("/ehr/{EHR_ID}/ehr_status/{encoded}/tags");
    let (status, _, _) = send(app, request(Method::GET, &tags, &[], "")?).await?;
    assert_eq!(
        status,
        StatusCode::NOT_IMPLEMENTED,
        "the tags route is its own operation"
    );
    Ok(())
}

/// A create answers `201` with `Location`, `ETag` and the representation
/// under `Prefer: return=representation`, and `204` with the same headers and
/// no body otherwise; a list header arrives one item per field line.
#[tokio::test]
async fn a_create_answers_201_with_its_headers_or_204_minimal() -> TestResult {
    let (app, recorder) = recorder_app();
    let full = request(
        Method::POST,
        "/ehr",
        &[
            ("Prefer", "return=representation"),
            ("openehr-audit-details", "change_type=\"249\""),
            ("openehr-audit-details", "committer.name=\"Dr. A, B\""),
        ],
        "",
    )?;
    let (status, headers, body) = send(app.clone(), full).await?;
    assert_eq!(status, StatusCode::CREATED);
    let location = format!("https://cdr.example.org/openehr/v1/ehr/{EHR_ID}");
    assert_eq!(
        headers.get("location").map(http::HeaderValue::as_bytes),
        Some(location.as_bytes())
    );
    let etag = format!("\"{EHR_ID}\"");
    assert_eq!(
        headers.get("etag").map(http::HeaderValue::as_bytes),
        Some(etag.as_bytes())
    );
    let created: Value = serde_json::from_slice(&body)?;
    assert_eq!(created, json!({ "uid": EHR_ID }));

    let minimal = request(Method::POST, "/ehr", &[("Prefer", "return=minimal")], "")?;
    let (status, headers, body) = send(app, minimal).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        headers.get("location").map(http::HeaderValue::as_bytes),
        Some(location.as_bytes())
    );
    assert_eq!(
        headers.get("etag").map(http::HeaderValue::as_bytes),
        Some(etag.as_bytes())
    );
    assert!(body.is_empty(), "a 204 carries no body");
    assert!(headers.get(http::header::CONTENT_TYPE).is_none());
    assert_eq!(
        recorder.seen(),
        [
            "ehr_create prefer=Some(\"return=representation\") \
             audit=Some([\"change_type=\\\"249\\\"\", \"committer.name=\\\"Dr. A, B\\\"\"]) body=false"
                .to_owned(),
            "ehr_create prefer=Some(\"return=minimal\") audit=None body=false".to_owned(),
        ]
    );
    Ok(())
}

/// A required header the request does not carry answers `400` naming it,
/// before the trait method runs.
#[tokio::test]
async fn a_missing_required_header_answers_400_naming_it() -> TestResult {
    let (app, recorder) = recorder_app();
    let uri = format!("/ehr/{EHR_ID}/composition/{VERSION_UID}");
    let (status, headers, body) = send(app, request(Method::PUT, &uri, &[], "{}")?).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let message = error_message(&headers, &body)?;
    assert!(message.contains("`If-Match`"), "{message}");
    assert!(recorder.seen().is_empty(), "the trait method never ran");
    Ok(())
}

/// An integer query parameter that does not parse answers `400` naming it,
/// and so does a scalar parameter given twice.
#[tokio::test]
async fn an_unparseable_query_parameter_answers_400_naming_it() -> TestResult {
    let (app, recorder) = recorder_app();
    let (status, headers, body) = send(
        app.clone(),
        request(Method::GET, "/query/aql?q=SELECT%201&offset=five", &[], "")?,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let message = error_message(&headers, &body)?;
    assert!(message.contains("`offset`"), "{message}");

    let (status, headers, body) = send(
        app.clone(),
        request(
            Method::GET,
            "/query/aql?q=SELECT%201&fetch=1&fetch=2",
            &[],
            "",
        )?,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_message(&headers, &body)?.contains("`fetch`"));

    let (status, headers, body) =
        send(app, request(Method::GET, "/query/aql?offset=1", &[], "")?).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_message(&headers, &body)?.contains("`q`"));
    assert!(recorder.seen().is_empty(), "the trait method never ran");
    Ok(())
}

/// A canonical-JSON body decodes into its DTO; sent under another
/// `Content-Type` it answers `415`, and without one it is read as JSON.
#[tokio::test]
async fn a_json_body_decodes_and_another_content_type_answers_415() -> TestResult {
    let (app, recorder) = recorder_app();
    let body = r#"{"q":"SELECT c FROM COMPOSITION c","fetch":3}"#;
    let (status, headers, _) = send(
        app.clone(),
        request(
            Method::POST,
            "/query/aql",
            &[("Content-Type", "application/xml")],
            body,
        )?,
    )
    .await?;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(headers.get(http::header::CONTENT_TYPE).is_some());

    for content_type in [Some("application/json; charset=utf-8"), None] {
        let headers: Vec<(&str, &str)> = content_type
            .map(|ct| ("Content-Type", ct))
            .into_iter()
            .collect();
        let (status, _, _) = send(
            app.clone(),
            request(Method::POST, "/query/aql", &headers, body)?,
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "Content-Type {content_type:?}");
    }

    let (status, headers, reply) = send(
        app,
        request(Method::POST, "/query/aql", &[], r#"{"q": 7}"#)?,
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a body that is not the DTO"
    );
    assert!(error_message(&headers, &reply)?.contains("request body"));
    assert_eq!(
        recorder.seen(),
        [
            "adhoc_body q=SELECT c FROM COMPOSITION c fetch=Some(3)".to_owned(),
            "adhoc_body q=SELECT c FROM COMPOSITION c fetch=Some(3)".to_owned(),
        ]
    );
    Ok(())
}

/// A semantic-validation failure renders every violation as one
/// `validationErrors` item.
#[tokio::test]
async fn validation_errors_render_into_the_error_body() -> TestResult {
    use axum::response::IntoResponse as _;
    let error = ApiError::ValidationFailed(vec![openehr_its::rest::runtime::ValidationError {
        path: "/content[0]".to_owned(),
        message: "unknown archetype".to_owned(),
    }]);
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    let rendered: Value = serde_json::from_slice(&body)?;
    assert_eq!(
        rendered,
        json!({
            "message": "1 validation error(s)",
            "validationErrors": ["/content[0]: unknown archetype"],
        })
    );
    Ok(())
}

/// A demographic store whose every party has moved on to a newer version, or
/// is still referenced.
#[derive(Debug)]
struct Parties;

#[async_trait::async_trait]
impl demographic::server::DemographicApi for Parties {
    async fn person_update(
        &self,
        params: demographic::PersonUpdateParams,
        _body: Payload<Person, Value>,
    ) -> Result<demographic::server::PersonUpdateResponse, Refusal> {
        Err(Refusal::new(ApiError::PreconditionFailed(format!(
            "`{}` is not the latest version",
            params.if_match
        )))
        .with_headers(demographic::PersonUpdatePreconditionFailedHeaders {
            etag: Some(format!("\"{CURRENT_VERSION}\"")),
            location: Some(format!(
                "https://cdr.example.org/openehr/v1/demographic/person/{CURRENT_VERSION}"
            )),
        })?)
    }

    async fn person_delete(
        &self,
        params: demographic::PersonDeleteParams,
    ) -> Result<demographic::server::PersonDeleteResponse, Refusal> {
        Err(Refusal::new(ApiError::Conflict(format!(
            "`{}` is still referenced",
            params.uid_based_id
        )))
        .with_headers(demographic::PersonDeleteConflictHeaders {
            etag: Some(format!("\"{CURRENT_VERSION}\"")),
            location: Some(format!(
                "https://cdr.example.org/openehr/v1/demographic/person/{CURRENT_VERSION}"
            )),
        })?)
    }
}

/// A committed fixture under the shared `corpus/fixtures` tree.
fn corpus_fixture(relative: &str) -> Result<String, Box<dyn Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fixtures");
    Ok(std::fs::read_to_string(root.join(relative))?)
}

/// The `ETag` of an answer, as text.
fn etag(headers: &http::HeaderMap) -> Option<&str> {
    headers.get("etag").and_then(|value| value.to_str().ok())
}

/// A failed `If-Match` answers `412` with the `Error` body and the current
/// version in `ETag` (`Requests_and_responses.md` §If-Match and accidental
/// overwrites: "SHOULD return also latest `version_uid` in the `ETag` response
/// headers"), set field by field.
#[tokio::test]
async fn a_precondition_failure_carries_the_current_etag() -> TestResult {
    let (app, recorder) = recorder_app();
    let uri = format!("/ehr/{EHR_ID}/composition/{VERSION_UID}");
    let body = corpus_fixture("composition/minimal_event.v1.json")?;
    let stale = format!("\"{VERSION_UID}\"");
    let (status, headers, reply) = send(
        app,
        request(Method::PUT, &uri, &[("If-Match", stale.as_str())], &body)?,
    )
    .await?;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(
        etag(&headers),
        Some(format!("\"{CURRENT_VERSION}\"").as_str())
    );
    assert_eq!(
        error_message(&headers, &reply)?,
        "precondition failed: the composition has a newer version"
    );
    assert_eq!(
        recorder.seen(),
        [format!("composition_update if_match={stale}")]
    );
    Ok(())
}

/// The demographic `412` and `409` answers carry the `ETag` and `Location` the
/// OAS declares for them, set typed from the operation's headers struct.
#[tokio::test]
async fn demographic_refusals_carry_their_declared_headers() -> TestResult {
    let app = demographic::server::router(Arc::new(Parties));
    let location =
        format!("https://cdr.example.org/openehr/v1/demographic/person/{CURRENT_VERSION}");
    let current = format!("\"{CURRENT_VERSION}\"");
    let person = corpus_fixture("demographic/person.v1.json")?;
    let uri = format!("/demographic/person/{VERSION_UID}");
    let (status, headers, body) = send(
        app.clone(),
        request(Method::PUT, &uri, &[("If-Match", "\"stale\"")], &person)?,
    )
    .await?;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(etag(&headers), Some(current.as_str()));
    assert_eq!(
        headers.get("location").map(http::HeaderValue::as_bytes),
        Some(location.as_bytes())
    );
    assert!(error_message(&headers, &body)?.contains("not the latest version"));

    let (status, headers, body) = send(app, request(Method::DELETE, &uri, &[], "")?).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(etag(&headers), Some(current.as_str()));
    assert_eq!(
        headers.get("location").map(http::HeaderValue::as_bytes),
        Some(location.as_bytes())
    );
    assert!(error_message(&headers, &body)?.contains("still referenced"));
    Ok(())
}

/// A header value that is not legal on the wire is refused as the
/// implementation's own fault.
#[test]
fn an_illegal_refusal_header_is_a_header_error() {
    let refused = Refusal::new(ApiError::Conflict("x".to_owned())).try_with_header("ETag", "a\nb");
    assert!(refused.is_err());
    let refusal = Refusal::from(ApiError::Conflict("x".to_owned()));
    assert_eq!(refusal.status(), StatusCode::CONFLICT);
    assert!(refusal.headers().is_empty());
}

/// The whole-API router mounts every group without a route conflict, and
/// answers an unknown path `404` and an undeclared method `405` with `Allow`,
/// both with the `Error` body, at the root and nested under a base path.
#[tokio::test]
async fn the_fallbacks_answer_404_and_405_with_the_error_body() -> TestResult {
    let api = Arc::new(Unimplemented);
    let root = server::router(Arc::clone(&api));
    let nested = axum::Router::new().nest("/openehr/v1", server::router(api));
    for (app, base) in [(root, ""), (nested, "/openehr/v1")] {
        let missing = format!("{base}/no/such/resource");
        let (status, headers, body) =
            send(app.clone(), request(Method::GET, &missing, &[], "")?).await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
        assert!(error_message(&headers, &body)?.contains("`/no/such/resource`"));

        for (method, path, allow) in [
            (Method::DELETE, "/query/aql", "GET, POST"),
            (Method::PATCH, &*format!("/ehr/{EHR_ID}"), "GET, PUT"),
            (Method::POST, "/admin/ehr/all", "DELETE"),
        ] {
            let uri = format!("{base}{path}");
            let (status, headers, body) =
                send(app.clone(), request(method.clone(), &uri, &[], "")?).await?;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {uri}");
            assert_eq!(
                headers
                    .get(http::header::ALLOW)
                    .map(http::HeaderValue::as_bytes),
                Some(allow.as_bytes()),
                "{method} {uri}"
            );
            assert!(error_message(&headers, &body)?.contains(&method.to_string()));
        }

        let (status, _, _) = send(
            app.clone(),
            request(Method::POST, &format!("{base}/ehr"), &[], "")?,
        )
        .await?;
        assert_eq!(
            status,
            StatusCode::NOT_IMPLEMENTED,
            "a bound route still serves"
        );
    }
    Ok(())
}

/// A CONTRIBUTION whose envelope is canonical and whose version `data` is
/// `data` (ITS-REST `operations/contribution_create.yaml` §Simplified Formats).
fn contribution_body(data: &str) -> String {
    format!(
        r#"{{
  "versions": [{{
    "preceding_version_uid": {{ "_type": "OBJECT_VERSION_ID", "value": "{VERSION_UID}" }},
    "lifecycle_state": {{
      "_type": "DV_CODED_TEXT",
      "value": "complete",
      "defining_code": {{ "terminology_id": {{ "_type": "TERMINOLOGY_ID", "value": "openehr" }}, "code_string": "532" }}
    }},
    "commit_audit": {{
      "_type": "UPDATE_AUDIT",
      "change_type": {{
        "_type": "DV_CODED_TEXT",
        "value": "modification",
        "defining_code": {{ "terminology_id": {{ "_type": "TERMINOLOGY_ID", "value": "openehr" }}, "code_string": "251" }}
      }},
      "committer": {{ "_type": "PARTY_IDENTIFIED", "name": "A. Clinician" }}
    }},
    "data": {data}
  }}],
  "audit": {{
    "_type": "UPDATE_AUDIT",
    "change_type": {{
      "_type": "DV_CODED_TEXT",
      "value": "modification",
      "defining_code": {{ "terminology_id": {{ "_type": "TERMINOLOGY_ID", "value": "openehr" }}, "code_string": "251" }}
    }},
    "committer": {{ "_type": "PARTY_IDENTIFIED", "name": "A. Clinician" }}
  }}
}}"#
    )
}

/// A FLAT and a STRUCTURED CONTRIBUTION parse to the envelope over raw JSON
/// content, `preceding_version_uid` typed `OBJECT_VERSION_ID`, the content
/// left as sent.
#[test]
fn a_simplified_contribution_envelope_reads_with_typed_version_uids() -> TestResult {
    for data in [
        r#"{"vital_signs/language|code": "en", "vital_signs/body_temperature:0/any_event:0/temperature|magnitude": 37.1}"#,
        r#"{"vital_signs": {"language": [{"|code": "en"}]}}"#,
    ] {
        let parsed: ehr::NewContribution<Value> = serde_json::from_str(&contribution_body(data))?;
        let version = parsed.versions.first().ok_or("one version")?;
        assert_eq!(
            version
                .preceding_version_uid
                .as_ref()
                .map(ObjectVersionId::value),
            Some(VERSION_UID)
        );
        assert_eq!(version.data, Some(serde_json::from_str::<Value>(data)?));
    }
    Ok(())
}

/// `contribution_create` admits both Simplified Formats its `Content-Type`
/// parameter declares and hands the implementation the matching `Payload`;
/// canonical JSON stays canonical and XML is still `415`.
#[tokio::test]
async fn contribution_create_admits_the_simplified_formats() -> TestResult {
    let (app, recorder) = recorder_app();
    let uri = format!("/ehr/{EHR_ID}/contribution");
    let flat = contribution_body(r#"{"vital_signs/language|code": "en"}"#);
    for content_type in [
        "application/openehr.wt.flat+json",
        "application/openehr.wt.structured+json",
    ] {
        let (status, _, _) = send(
            app.clone(),
            request(Method::POST, &uri, &[("Content-Type", content_type)], &flat)?,
        )
        .await?;
        assert_eq!(status, StatusCode::NO_CONTENT, "{content_type}");
    }
    let (status, _, _) = send(
        app.clone(),
        request(
            Method::POST,
            &uri,
            &[("Content-Type", "application/xml")],
            &flat,
        )?,
    )
    .await?;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let (status, _, _) = send(
        app,
        request(
            Method::POST,
            &uri,
            &[("Content-Type", "application/json")],
            &flat,
        )?,
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "FLAT content is not a canonical version"
    );
    let preceding = format!("preceding=Some({VERSION_UID:?})");
    assert_eq!(
        recorder.seen(),
        [
            format!("contribution_create flat {preceding}"),
            format!("contribution_create structured {preceding}"),
        ]
    );
    Ok(())
}

/// A `201_EHR` body is `oneOf` `Ehr` and `Identifier` (ITS-REST
/// `responses/201_EHR.yaml`): the released `Ehr` schema example reads as the
/// full representation, the `Identifier` example as the identifier form, and
/// each writes back unchanged.
#[test]
fn a_201_ehr_body_reads_either_form() -> TestResult {
    // `schemas/ehr/Ehr.yaml` example.
    let full = json!({
        "system_id": { "value": "9624982A-9F42-41A5-9318-AE13D5F5031F" },
        "ehr_id": { "value": "7d44b88c-4199-4bad-97dc-d78268e01398" },
        "ehr_status": {
            "id": {
                "_type": "OBJECT_VERSION_ID",
                "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"
            },
            "namespace": "local",
            "type": "EHR_STATUS"
        },
        "ehr_access": {
            "id": {
                "_type": "OBJECT_VERSION_ID",
                "value": "59a8d0ac-140e-4feb-b2d6-af99f8e68af8::openEHRSys.example.com::1"
            },
            "namespace": "local",
            "type": "EHR_ACCESS"
        },
        "time_created": { "value": "2015-01-20T19:30:22.765+01:00" }
    });
    // `schemas/others/Identifier.yaml` example.
    let identifier =
        json!({ "uid": "6cb19121-4307-4648-9da0-d62e4d51f19b::openEHRSys.example.com::2" });
    let read: Representation<openehr_rm::v1_2::ehr::ehr::Ehr> =
        serde_json::from_value(full.clone())?;
    assert!(matches!(read, Representation::Full(_)), "{read:?}");
    let read: Representation<openehr_rm::v1_2::ehr::ehr::Ehr> =
        serde_json::from_value(identifier.clone())?;
    let Representation::Identifier(id) = &read else {
        return Err(format!("the identifier form read as {read:?}").into());
    };
    assert_eq!(
        id.uid,
        "6cb19121-4307-4648-9da0-d62e4d51f19b::openEHRSys.example.com::2"
    );
    assert_eq!(serde_json::to_value(&read)?, identifier);
    let neither = serde_json::from_value::<Representation<openehr_rm::v1_2::ehr::ehr::Ehr>>(
        json!({ "uid": 7 }),
    );
    assert!(neither.is_err(), "{neither:?}");
    Ok(())
}

/// A CONTRIBUTION may carry a logical deletion, a version with no `data` (RM
/// common master06 §Logical Deletion; `ORIGINAL_VERSION.data` is `0..1`), and
/// an audit typed `AUDIT_DETAILS`, which servers "SHOULD additionally accept"
/// (ITS-REST `operations/contribution_create.yaml`); the tag is kept on write.
#[test]
fn a_contribution_reads_a_deletion_and_an_audit_details_audit() -> TestResult {
    let body = contribution_body("null")
        .replace(r#""_type": "UPDATE_AUDIT""#, r#""_type": "AUDIT_DETAILS""#);
    let mut value: Value = serde_json::from_str(&body)?;
    let member = value
        .pointer_mut("/versions/0")
        .and_then(Value::as_object_mut)
        .ok_or("one version member")?;
    member.remove("data");
    member.insert(
        "lifecycle_state".to_owned(),
        json!({
            "_type": "DV_CODED_TEXT",
            "value": "deleted",
            "defining_code": { "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "523" }
        }),
    );
    let parsed: ehr::NewContribution = serde_json::from_value(value)?;
    let version = parsed.versions.first().ok_or("one version")?;
    assert!(version.data.is_none());
    let written = serde_json::to_value(&parsed)?;
    assert_eq!(
        written.pointer("/audit/_type"),
        Some(&json!("AUDIT_DETAILS"))
    );
    assert_eq!(written.pointer("/versions/0/data"), None);
    Ok(())
}
