// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::panic_in_result_fn,
    clippy::panic,
    reason = "test assertions/diagnostics/fixtures: Result-returning tests assert and name \
              the unexpected outcome (.claude/rules/testing.md §Test shapes); `allow`, not \
              `expect`, because #[tokio::test] moves each body into an async block, which \
              these lints do not reach uniformly"
)]
//! Contract tests for the generated ITS-REST clients over the `reqwest` engine.
//!
//! Every test stands a `wiremock` server in for the CDR and pins one wire
//! behaviour: the request the generated method builds (method, path, query
//! pairs, headers, body) and the outcome variant a documented answer becomes.
//! The documented status and header set per operation is the vendored OAS
//! (`crates/openehr-its/vendor/rest-oas/<group>-codegen.openapi.yaml`, by
//! `operationId`); the general statuses (`401`, `403`, `5xx`) and the `Prefer`,
//! `ETag` and `If-Match` semantics are the ITS-REST docs text
//! (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`).
//! Every mock carries `.expect(n)`: an unmatched request answers wiremock's own
//! `404`, so a `NotFound` outcome alone would prove nothing.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;
use std::time::Duration;

use http::StatusCode;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::{ObjectRef, ObjectRefData};
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::uid_based_id::UidBasedId;
use openehr_its::json::{from_canonical_json, to_canonical_value};
use openehr_its::rest::client::{
    Client, ClientError, Credentials, ReqwestTransport, RetryPolicy, TransportError,
};
use openehr_its::rest::generated::{admin, common, definition, demographic, ehr, query, system};
use openehr_rm::v1_2::common::tags::item_tag::ItemTag;
use openehr_rm::v1_2::composition::composition::Composition;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::demographic::person::Person;
use serde_json::json;
use wiremock::matchers::{body_json, body_string, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A test's plumbing error: every fallible step propagates with `?`.
type TestResult = Result<(), Box<dyn Error>>;

/// The EHR id every fixture names.
const EHR_ID: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";

/// A versioned-object uid (`HIER_OBJECT_ID` form).
const VO_UID: &str = "8849182c-82ad-4088-a07f-48ead4180515";

/// A version uid (`OBJECT_VERSION_ID` form, BASE `master05` §Syntaxes).
const VERSION_UID: &str = "8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1";

/// A client over the `reqwest` engine for the service rooted at `base`.
fn client_at(base: &str) -> Result<Client<ReqwestTransport>, Box<dyn Error>> {
    let transport = ReqwestTransport::with_timeout(Duration::from_secs(10))?;
    Ok(Client::new(transport, base.parse()?)?)
}

/// A client for the mock server's root.
fn client_for(server: &MockServer) -> Result<Client<ReqwestTransport>, Box<dyn Error>> {
    client_at(&server.uri())
}

/// A retry budget whose backoffs cost nothing measurable.
fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 3,
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(5),
    }
}

/// A committed fixture under the shared `corpus/fixtures` tree.
fn corpus_fixture(relative: &str) -> Result<String, Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fixtures");
    Ok(std::fs::read_to_string(root.join(relative))?)
}

/// A local `OBJECT_REF` of `type` to the versioned object `uid`.
fn object_ref(r#type: &str, uid: &str) -> Result<ObjectRef, Box<dyn Error>> {
    Ok(ObjectRef::ObjectRef(ObjectRefData {
        namespace: "local".to_owned(),
        r#type: r#type.to_owned(),
        id: ObjectId::HierObjectId(HierObjectId::new(uid)?),
    }))
}

/// The typed `EHR` the `200` fixtures serve.
fn ehr_fixture() -> Result<openehr_rm::v1_2::ehr::ehr::Ehr, Box<dyn Error>> {
    Ok(openehr_rm::v1_2::ehr::ehr::Ehr {
        system_id: HierObjectId::new("5f1b7c8e-2a44-4c1e-9d63-0a9e0c1f2b3d")?,
        ehr_id: HierObjectId::new(EHR_ID)?,
        contributions: None,
        ehr_status: object_ref("EHR_STATUS", "3a4c9f2e-6b1d-4e8a-a7c5-9d2f1e0b8c6a")?,
        ehr_access: object_ref("EHR_ACCESS", "b6e1d4c2-8f3a-4a9b-9c7d-2e5f0a1b3c4d")?,
        compositions: None,
        directory: None,
        time_created: DvDateTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: "2026-09-25T10:00:00Z".to_owned(),
        },
        folders: None,
        tags: None,
    })
}

/// `ehr_get_by_id` parameters for `ehr_id`, with no `Accept` set.
fn get_ehr(ehr_id: &str) -> ehr::EhrGetByIdParams {
    ehr::EhrGetByIdParams {
        ehr_id: ehr_id.to_owned(),
        accept: None,
    }
}

/// An ITS-REST `Error` body.
fn error_body(message: &str) -> common::Error {
    common::Error {
        message: message.to_owned(),
        validation_errors: vec!["error1".to_owned()],
        additional_properties: BTreeMap::new(),
    }
}

/// A minimal `NewContribution` with one audit and no versions.
fn new_contribution() -> Result<ehr::NewContribution, Box<dyn Error>> {
    Ok(serde_json::from_value(json!({
        "versions": [],
        "audit": {
            "change_type": {
                "_type": "DV_CODED_TEXT",
                "value": "creation",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "249"
                }
            },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Test" }
        }
    }))?)
}

/// A `ResultSet` answering `q` with one row.
fn result_set(q: &str) -> query::ResultSet {
    query::ResultSet {
        meta: None,
        name: None,
        q: Some(q.to_owned()),
        columns: Some(vec![query::ResultSetColumn {
            name: "#0".to_owned(),
            path: Some("/uid/value".to_owned()),
            additional_properties: BTreeMap::new(),
        }]),
        rows: vec![vec![json!(VERSION_UID)]],
        additional_properties: BTreeMap::new(),
    }
}

// ── ehr ─────────────────────────────────────────────────────────────────────

/// `ehr_create` under `Prefer: return=minimal` answers `201` with no body.
///
/// OAS `ehr-codegen` `ehr_create` `201` (the `201_EHR` response component)
/// declares the `ETag` and `Location` headers and carries a body only under
/// `return=representation` or `return=identifier`, so a minimal `201` decodes
/// to `None`.
#[tokio::test]
async fn ehr_create_minimal_answers_created_with_etag_and_location() -> TestResult {
    let server = MockServer::start().await;
    let etag = format!("W/\"{EHR_ID}\"");
    let location = format!("{}/ehr/{EHR_ID}", server.uri());
    Mock::given(method("POST"))
        .and(path("/ehr"))
        .and(header("Prefer", "return=minimal"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", etag.as_str())
                .insert_header("Location", location.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::EhrCreateParams {
        prefer: Some("return=minimal".to_owned()),
        accept: None,
        content_type: None,
        openehr_version: None,
        openehr_audit_details: None,
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_create(&params, None)
        .await?;
    let (body, headers) = match outcome {
        ehr::client::EhrCreateOutcome::Created { body, headers } => (body, headers),
        other => panic!("expected the 201 answer, got {other:?}"),
    };
    assert!(body.is_none(), "a minimal 201 carries no body: {body:?}");
    assert_eq!(headers.etag, Some(etag));
    assert_eq!(headers.location, Some(location));
    Ok(())
}

/// `ehr_get_by_id` `200` decodes the canonical-JSON `EHR` into the typed model.
///
/// OAS `ehr-codegen` `ehr_get_by_id` `200`: an `EHR` body with `Content-Type`.
#[tokio::test]
async fn ehr_get_by_id_decodes_the_typed_ehr() -> TestResult {
    let server = MockServer::start().await;
    let expected = ehr_fixture()?;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(to_canonical_value(&expected)))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await?;
    let (body, headers) = match outcome {
        ehr::client::EhrGetByIdOutcome::Ok { body, headers } => (body, headers),
        other @ ehr::client::EhrGetByIdOutcome::NotFound { .. } => {
            panic!("expected the 200 answer, got {other:?}")
        }
    };
    assert_eq!(body, expected);
    assert_eq!(headers.content_type.as_deref(), Some("application/json"));
    Ok(())
}

/// `ehr_get_by_id` `404` is the `NotFound` outcome, not an error.
///
/// OAS `ehr-codegen` `ehr_get_by_id` documents `404`.
#[tokio::test]
async fn ehr_get_by_id_not_found_is_an_outcome() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await?;
    assert!(
        matches!(outcome, ehr::client::EhrGetByIdOutcome::NotFound { .. }),
        "expected the 404 answer, got {outcome:?}"
    );
    Ok(())
}

/// `composition_update` sends `If-Match` verbatim and reads the `412` `ETag`.
///
/// OAS `ehr-codegen` `composition_update`: `If-Match` is a required header and
/// `412` declares `ETag`; the docs text §If-Match and accidental overwrites
/// says the service "SHOULD return also latest `version_uid` in the `ETag`".
#[tokio::test]
async fn composition_update_sends_if_match_and_reads_the_412_etag() -> TestResult {
    let server = MockServer::start().await;
    let if_match = format!("\"{VERSION_UID}\"");
    let latest = "W/\"8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::2\"";
    Mock::given(method("PUT"))
        .and(path(format!("/ehr/{EHR_ID}/composition/{VO_UID}")))
        .and(header("If-Match", if_match.as_str()))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(412).insert_header("ETag", latest))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let composition: Composition =
        from_canonical_json(&corpus_fixture("composition/minimal_event.v1.json")?)?;
    let params = ehr::CompositionUpdateParams {
        ehr_id: EHR_ID.to_owned(),
        uid_based_id: VO_UID.to_owned(),
        if_match,
        prefer: None,
        accept: None,
        content_type: None,
        openehr_item_tag: None,
        openehr_version_item_tag: None,
        openehr_version: None,
        openehr_audit_details: None,
        openehr_template_id: None,
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .composition_update(&params, &composition)
        .await?;
    let (body, headers) = match outcome {
        ehr::client::CompositionUpdateOutcome::PreconditionFailed { body, headers } => {
            (body, headers)
        }
        other => panic!("expected the 412 answer, got {other:?}"),
    };
    assert_eq!(headers.etag.as_deref(), Some(latest));
    assert!(body.is_empty(), "the 412 carried no body: {body:?}");
    Ok(())
}

/// `composition_update` puts the canonical JSON of the typed body on the wire.
///
/// OAS `ehr-codegen` `composition_update` request body: `application/json`.
#[tokio::test]
async fn composition_update_sends_the_canonical_json_body() -> TestResult {
    let server = MockServer::start().await;
    let composition: Composition =
        from_canonical_json(&corpus_fixture("composition/minimal_event.v1.json")?)?;
    Mock::given(method("PUT"))
        .and(path(format!("/ehr/{EHR_ID}/composition/{VO_UID}")))
        .and(body_json(to_canonical_value(&composition)))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::CompositionUpdateParams {
        ehr_id: EHR_ID.to_owned(),
        uid_based_id: VO_UID.to_owned(),
        if_match: format!("\"{VERSION_UID}\""),
        prefer: None,
        accept: None,
        content_type: None,
        openehr_item_tag: None,
        openehr_version_item_tag: None,
        openehr_version: None,
        openehr_audit_details: None,
        openehr_template_id: None,
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .composition_update(&params, &composition)
        .await?;
    assert!(
        matches!(
            outcome,
            ehr::client::CompositionUpdateOutcome::NoContent { .. }
        ),
        "expected the 204 answer, got {outcome:?}"
    );
    Ok(())
}

/// `contribution_create` `409` is the `Conflict` outcome.
///
/// OAS `ehr-codegen` `contribution_create` documents `409`.
#[tokio::test]
async fn contribution_create_conflict_is_an_outcome() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/ehr/{EHR_ID}/contribution")))
        .respond_with(ResponseTemplate::new(409))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::ContributionCreateParams {
        ehr_id: EHR_ID.to_owned(),
        prefer: None,
        accept: None,
        content_type: None,
        openehr_template_id: None,
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .contribution_create(&params, &new_contribution()?)
        .await?;
    let body = match outcome {
        ehr::client::ContributionCreateOutcome::Conflict { body } => body,
        other => panic!("expected the 409 answer, got {other:?}"),
    };
    assert!(body.is_empty(), "the 409 carried no body: {body:?}");
    Ok(())
}

/// `contribution_create` `400` decodes the `Error` body.
///
/// OAS `ehr-codegen` `contribution_create` `400` references the `Error`
/// schema; the `400` response component says the body MAY carry error details,
/// so the outcome carries the body as received and its decoded form.
#[tokio::test]
async fn contribution_create_bad_request_decodes_the_error_body() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/ehr/{EHR_ID}/contribution")))
        .respond_with(ResponseTemplate::new(400).set_body_json(error_body("versions is malformed")))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::ContributionCreateParams {
        ehr_id: EHR_ID.to_owned(),
        prefer: None,
        accept: None,
        content_type: None,
        openehr_template_id: None,
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .contribution_create(&params, &new_contribution()?)
        .await?;
    let body = match outcome {
        ehr::client::ContributionCreateOutcome::BadRequest { body } => body,
        other => panic!("expected the 400 answer, got {other:?}"),
    };
    let error = body.error().ok_or("the 400 answer carried an Error body")?;
    assert_eq!(error.message, "versions is malformed");
    assert_eq!(error.validation_errors, vec!["error1".to_owned()]);
    assert_eq!(body.message(), Some("versions is malformed"));
    assert_eq!(body.validation_errors(), ["error1"]);
    Ok(())
}

/// A `400` whose body carries `message` alone still decodes as the `Error`.
///
/// OAS `Error` lists `validationErrors` as required, but the `400` response
/// component makes the whole body a MAY; a service that sends the message
/// without the list still sends its diagnostics, and the client reads them
/// with an empty list rather than refusing the documented answer.
#[tokio::test]
async fn a_prose_bad_request_decodes_as_the_error() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/ehr/{EHR_ID}/contribution")))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_raw(r#"{"message":"the body is not JSON"}"#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::ContributionCreateParams {
        ehr_id: EHR_ID.to_owned(),
        prefer: None,
        accept: None,
        content_type: None,
        openehr_template_id: None,
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .contribution_create(&params, &new_contribution()?)
        .await?;
    let body = match outcome {
        ehr::client::ContributionCreateOutcome::BadRequest { body } => body,
        other => panic!("expected the 400 answer, got {other:?}"),
    };
    assert_eq!(body.message(), Some("the body is not JSON"));
    assert!(body.validation_errors().is_empty());
    assert_eq!(body.text(), Some(r#"{"message":"the body is not JSON"}"#));
    Ok(())
}

/// `composition_create` sends the committal-metadata headers as the docs text
/// writes them: `openehr-version` once, `openehr-audit-details` one field line
/// per value, `openehr-template-id` once.
///
/// Docs text §openehr-version and openehr-audit-details: "services MUST accept
/// `openehr-version` and `openehr-audit-details` custom request headers"; the
/// example there sends `openehr-audit-details` as several lines. Docs text
/// §openehr-template-id names the header for a Simplified Format commit.
#[tokio::test]
async fn composition_create_sends_the_commit_headers() -> TestResult {
    let server = MockServer::start().await;
    let composition: Composition =
        from_canonical_json(&corpus_fixture("composition/minimal_event.v1.json")?)?;
    Mock::given(method("POST"))
        .and(path(format!("/ehr/{EHR_ID}/composition")))
        .and(header(
            "openehr-version",
            "lifecycle_state.code_string=\"532\"",
        ))
        .and(header(
            "openehr-template-id",
            "IDCR - Vital Signs Encounter.v1",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::CompositionCreateParams {
        ehr_id: EHR_ID.to_owned(),
        prefer: Some("return=minimal".to_owned()),
        accept: None,
        content_type: None,
        openehr_item_tag: None,
        openehr_version_item_tag: None,
        openehr_version: Some("lifecycle_state.code_string=\"532\"".to_owned()),
        openehr_audit_details: Some(vec![
            "change_type.code_string=\"251\"".to_owned(),
            "description.value=\"triage\"".to_owned(),
        ]),
        openehr_template_id: Some("IDCR - Vital Signs Encounter.v1".to_owned()),
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .composition_create(&params, &composition)
        .await?;
    assert!(
        matches!(
            outcome,
            ehr::client::CompositionCreateOutcome::NoContent { .. }
        ),
        "expected the 204 answer, got {outcome:?}"
    );
    let requests = server.received_requests().await.ok_or("recording is on")?;
    let request = requests.first().ok_or("one request was recorded")?;
    let audit: Vec<&str> = request
        .headers
        .get_all("openehr-audit-details")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();
    assert_eq!(
        audit,
        [
            "change_type.code_string=\"251\"",
            "description.value=\"triage\""
        ],
        "one field line per audit-details value"
    );
    Ok(())
}

/// `composition_update` `422` carries the `Error` body with its
/// `validationErrors`, although the OAS attaches no schema to that status.
///
/// OAS `ehr-codegen` `composition_update` documents `422` with no content; the
/// `400` response component's "MAY contain error details" is the only body
/// the docs text describes for an error, and a service that sends one under
/// `422` is read the same way.
#[tokio::test]
async fn composition_update_unprocessable_carries_the_error_body() -> TestResult {
    let server = MockServer::start().await;
    let composition: Composition =
        from_canonical_json(&corpus_fixture("composition/minimal_event.v1.json")?)?;
    Mock::given(method("PUT"))
        .and(path(format!("/ehr/{EHR_ID}/composition/{VO_UID}")))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "error": "Unprocessable Entity",
            "message": "the composition does not validate against its template",
            "validationErrors": ["/content[0]: node not in template"]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::CompositionUpdateParams {
        ehr_id: EHR_ID.to_owned(),
        uid_based_id: VO_UID.to_owned(),
        if_match: format!("\"{VERSION_UID}\""),
        prefer: None,
        accept: None,
        content_type: None,
        openehr_item_tag: None,
        openehr_version_item_tag: None,
        openehr_version: None,
        openehr_audit_details: None,
        openehr_template_id: None,
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .composition_update(&params, &composition)
        .await?;
    let body = match outcome {
        ehr::client::CompositionUpdateOutcome::UnprocessableEntity { body } => body,
        other => panic!("expected the 422 answer, got {other:?}"),
    };
    assert_eq!(
        body.message(),
        Some("the composition does not validate against its template")
    );
    assert_eq!(
        body.validation_errors(),
        ["/content[0]: node not in template"]
    );
    Ok(())
}

/// `composition_tags_get` `200` decodes the typed `ITEM_TAG` array.
///
/// OAS `ehr-codegen` `composition_tags_get` `200`: an array of
/// `ItemTagOfComposition`. The version uid in the path is one segment whose
/// `::` separators travel literal: RFC 3986 §3.3 admits `:` in a `pchar`,
/// and the ITS-REST examples write a `version_uid` that way.
#[tokio::test]
async fn composition_tags_get_decodes_the_typed_tags() -> TestResult {
    let server = MockServer::start().await;
    let tags = vec![ItemTag::new(
        "priority".to_owned(),
        Some("high".to_owned()),
        UidBasedId::ObjectVersionId(ObjectVersionId::new(VERSION_UID)?),
        None,
        object_ref("VERSIONED_COMPOSITION", VO_UID)?,
    )?];
    Mock::given(method("GET"))
        .and(path(format!(
            "/ehr/{EHR_ID}/composition/{VERSION_UID}/tags"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(to_canonical_value(&tags)))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::CompositionTagsGetParams {
        ehr_id: EHR_ID.to_owned(),
        uid_based_id: VERSION_UID.to_owned(),
        accept: None,
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .composition_tags_get(&params)
        .await?;
    let body = match outcome {
        ehr::client::CompositionTagsGetOutcome::Ok { body, .. } => body,
        other @ ehr::client::CompositionTagsGetOutcome::NotFound { .. } => {
            panic!("expected the 200 answer, got {other:?}")
        }
    };
    assert_eq!(body, tags);
    Ok(())
}

// ── query ───────────────────────────────────────────────────────────────────

/// `query_execute_adhoc_query` sends every parameter as a query pair.
///
/// OAS `query-codegen` `query_execute_adhoc_query`: `q` and `fetch` are query
/// parameters and `query_parameters` is a `style: form, explode: true` object,
/// so each member is its own pair; `200` answers a `ResultSet`.
#[tokio::test]
async fn adhoc_query_get_sends_every_parameter_as_a_query_pair() -> TestResult {
    let server = MockServer::start().await;
    let aql = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = $ehr";
    Mock::given(method("GET"))
        .and(path("/query/aql"))
        .and(query_param("q", aql))
        .and(query_param("fetch", "10"))
        .and(query_param("ehr", EHR_ID))
        .and(query_param("min_systolic", "140"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(result_set(aql))
                .insert_header("ETag", "W/\"result-1\""),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let query_parameters = BTreeMap::from([
        ("ehr".to_owned(), json!(EHR_ID)),
        ("min_systolic".to_owned(), json!(140)),
    ]);
    let params = query::QueryExecuteAdhocQueryParams {
        q: aql.to_owned(),
        ehr_id: None,
        offset: None,
        fetch: Some(10),
        query_parameters: Some(query_parameters),
        accept: None,
    };
    let outcome = query::client::QueryClient::new(&client)
        .query_execute_adhoc_query(&params)
        .await?;
    let (body, headers) = match outcome {
        query::client::QueryExecuteAdhocQueryOutcome::Ok { body, headers } => (body, headers),
        other => panic!("expected the 200 answer, got {other:?}"),
    };
    assert_eq!(body.q.as_deref(), Some(aql));
    assert_eq!(body.rows.len(), 1);
    assert_eq!(headers.etag.as_deref(), Some("W/\"result-1\""));
    Ok(())
}

/// `query_execute_adhoc_query` `408` is the `RequestTimeout` outcome.
///
/// OAS `query-codegen` `query_execute_adhoc_query` documents `408`.
#[tokio::test]
async fn adhoc_query_request_timeout_is_an_outcome() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/query/aql"))
        .respond_with(ResponseTemplate::new(408))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = query::QueryExecuteAdhocQueryParams {
        q: "SELECT e FROM EHR e".to_owned(),
        ehr_id: None,
        offset: None,
        fetch: None,
        query_parameters: None,
        accept: None,
    };
    let outcome = query::client::QueryClient::new(&client)
        .query_execute_adhoc_query(&params)
        .await?;
    assert!(
        matches!(
            outcome,
            query::client::QueryExecuteAdhocQueryOutcome::RequestTimeout { .. }
        ),
        "expected the 408 answer, got {outcome:?}"
    );
    Ok(())
}

/// `query_execute_adhoc_query_body` posts the query as canonical JSON.
///
/// OAS `query-codegen` `query_execute_adhoc_query_body`: an
/// `application/json` `AdhocQueryExecute` request body, `200` a `ResultSet`.
#[tokio::test]
async fn adhoc_query_post_sends_the_json_body() -> TestResult {
    let server = MockServer::start().await;
    let execute = query::AdhocQueryExecute {
        q: "SELECT e/ehr_id/value FROM EHR e".to_owned(),
        offset: None,
        fetch: Some(5),
        query_parameters: None,
        additional_properties: BTreeMap::new(),
    };
    Mock::given(method("POST"))
        .and(path("/query/aql"))
        .and(header("Content-Type", "application/json"))
        .and(body_json(&execute))
        .respond_with(ResponseTemplate::new(200).set_body_json(result_set(&execute.q)))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = query::QueryExecuteAdhocQueryBodyParams {
        accept: None,
        content_type: None,
    };
    let outcome = query::client::QueryClient::new(&client)
        .query_execute_adhoc_query_body(&params, &execute)
        .await?;
    let body = match outcome {
        query::client::QueryExecuteAdhocQueryBodyOutcome::Ok { body, .. } => body,
        other => panic!("expected the 200 answer, got {other:?}"),
    };
    assert_eq!(body.q, Some(execute.q));
    Ok(())
}

// ── definition ──────────────────────────────────────────────────────────────

/// `definition_template_adl1.4_upload` sends the OPT 1.4 XML verbatim.
///
/// OAS `definition-codegen` `definition_template_adl1.4_upload`: an
/// `application/xml` request body; `201` declares `Location`, `ETag` and
/// `Content-Type`, and its body is returned as received.
#[tokio::test]
async fn template_upload_sends_the_xml_verbatim() -> TestResult {
    let server = MockServer::start().await;
    let opt = corpus_fixture("composition/minimal_persistent.opt")?;
    let location = format!(
        "{}/definition/template/adl1.4/minimal_persistent",
        server.uri()
    );
    Mock::given(method("POST"))
        .and(path("/definition/template/adl1.4"))
        .and(header("Content-Type", "application/xml"))
        .and(body_string(opt.as_str()))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_raw(opt.as_bytes().to_vec(), "application/xml")
                .insert_header("Location", location.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = definition::DefinitionTemplateAdl14UploadParams {
        prefer: None,
        accept: None,
        content_type: None,
    };
    let outcome = definition::client::DefinitionClient::new(&client)
        .definition_template_adl1_4_upload(&params, &opt)
        .await?;
    let (body, headers) = match outcome {
        definition::client::DefinitionTemplateAdl14UploadOutcome::Created { body, headers } => {
            (body, headers)
        }
        other => panic!("expected the 201 answer, got {other:?}"),
    };
    assert_eq!(body, opt.as_bytes());
    assert_eq!(headers.location, Some(location));
    assert_eq!(headers.content_type.as_deref(), Some("application/xml"));
    Ok(())
}

/// `definition_template_adl1.4_list` `200` decodes the typed `TemplateList`.
///
/// OAS `definition-codegen` `definition_template_adl1.4_list` `200`: a
/// `TemplateList` body.
#[tokio::test]
async fn template_list_decodes_the_typed_list() -> TestResult {
    let server = MockServer::start().await;
    let list: definition::TemplateList = vec![definition::TemplateMetadata {
        template_id: "minimal_persistent".to_owned(),
        version: None,
        concept: "minimal persistent".to_owned(),
        archetype_id: "openEHR-EHR-COMPOSITION.minimal.v1".to_owned(),
        created_timestamp: "2026-09-25T10:00:00Z".to_owned(),
        additional_properties: BTreeMap::new(),
    }];
    Mock::given(method("GET"))
        .and(path("/definition/template/adl1.4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = definition::DefinitionTemplateAdl14ListParams {
        accept: None,
        template_id: None,
        concept: None,
        version: None,
        offset: None,
        fetch: None,
    };
    let outcome = definition::client::DefinitionClient::new(&client)
        .definition_template_adl1_4_list(&params)
        .await?;
    let definition::client::DefinitionTemplateAdl14ListOutcome::Ok { body, .. } = outcome;
    assert_eq!(body.len(), 1);
    let first = body.first().ok_or("the list carries one template")?;
    assert_eq!(first.template_id, "minimal_persistent");
    assert_eq!(first.archetype_id, "openEHR-EHR-COMPOSITION.minimal.v1");
    Ok(())
}

// ── demographic ─────────────────────────────────────────────────────────────

/// `person_create` `422` is the `UnprocessableEntity` outcome.
///
/// OAS `demographic-codegen` `person_create` documents `422`.
#[tokio::test]
async fn person_create_unprocessable_is_an_outcome() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/demographic/person"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(422))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let person: Person = from_canonical_json(&corpus_fixture("demographic/person.v1.json")?)?;
    let params = demographic::PersonCreateParams {
        prefer: None,
        accept: None,
        content_type: None,
        openehr_item_tag: None,
        openehr_version_item_tag: None,
        openehr_version: None,
        openehr_audit_details: None,
    };
    let outcome = demographic::client::DemographicClient::new(&client)
        .person_create(&params, &person)
        .await?;
    assert!(
        matches!(
            outcome,
            demographic::client::PersonCreateOutcome::UnprocessableEntity { .. }
        ),
        "expected the 422 answer, got {outcome:?}"
    );
    Ok(())
}

/// `person_get` `204` is the `NoContent` outcome.
///
/// OAS `demographic-codegen` `person_get` documents `204`.
#[tokio::test]
async fn person_get_no_content_is_an_outcome() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/demographic/person/{VO_UID}")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = demographic::PersonGetParams {
        uid_based_id: VO_UID.to_owned(),
        version_at_time: None,
        accept: None,
    };
    let outcome = demographic::client::DemographicClient::new(&client)
        .person_get(&params)
        .await?;
    assert!(
        matches!(outcome, demographic::client::PersonGetOutcome::NoContent),
        "expected the 204 answer, got {outcome:?}"
    );
    Ok(())
}

// ── admin ───────────────────────────────────────────────────────────────────

/// `admin_ehr_delete` `204` is the `NoContent` outcome.
///
/// OAS `admin-codegen` `admin_ehr_delete` documents `204`.
#[tokio::test]
async fn admin_ehr_delete_no_content_is_an_outcome() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/admin/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = admin::AdminEhrDeleteParams {
        ehr_id: EHR_ID.to_owned(),
    };
    let outcome = admin::client::AdminClient::new(&client)
        .admin_ehr_delete(&params)
        .await?;
    assert!(
        matches!(outcome, admin::client::AdminEhrDeleteOutcome::NoContent),
        "expected the 204 answer, got {outcome:?}"
    );
    Ok(())
}

// ── system ──────────────────────────────────────────────────────────────────

/// `options` sends `OPTIONS /` and captures the `Allow` header.
///
/// OAS `system-codegen` `options` `200` declares `Allow` and an `Options`
/// body.
#[tokio::test]
async fn options_captures_the_allow_header() -> TestResult {
    let server = MockServer::start().await;
    let options = system::Options {
        solution: Some("FerroEHR".to_owned()),
        solution_version: None,
        vendor: None,
        restapi_specs_version: Some("1.1.0".to_owned()),
        conformance_profile: None,
        endpoints: Some(vec!["/ehr".to_owned()]),
        additional_properties: BTreeMap::new(),
    };
    Mock::given(method("OPTIONS"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(&options)
                .insert_header("Allow", "GET, POST, PUT, DELETE, OPTIONS"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let outcome = system::client::SystemClient::new(&client)
        .options(&system::OptionsParams { accept: None })
        .await?;
    let system::client::OptionsOutcome::Ok { body, headers } = outcome;
    assert_eq!(
        headers.allow.as_deref(),
        Some("GET, POST, PUT, DELETE, OPTIONS")
    );
    assert_eq!(body.solution, options.solution);
    assert_eq!(body.restapi_specs_version, options.restapi_specs_version);
    Ok(())
}

// ── the shared runtime ──────────────────────────────────────────────────────

/// A status the operation does not document is an `UndocumentedStatus` error.
///
/// OAS `ehr-codegen` `ehr_get_by_id` documents `200` and `404` only; `418` is
/// neither nor one of the general statuses the docs text §HTTP status codes
/// lists.
#[tokio::test]
async fn an_undocumented_status_is_an_error() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(418))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let refused = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await;
    assert!(
        matches!(
            refused,
            Err(ClientError::UndocumentedStatus { status, .. }) if status == StatusCode::IM_A_TEAPOT
        ),
        "expected an undocumented-status error, got {refused:?}"
    );
    Ok(())
}

/// `401` is an `Unauthorized` error carrying the `WWW-Authenticate` challenge.
///
/// Docs text §Authentication and authorization: services "MUST properly use
/// the `WWW-Authenticate` … response headers, returning … `401 Unauthorized`".
#[tokio::test]
async fn unauthorized_carries_the_challenge_and_detail() -> TestResult {
    let server = MockServer::start().await;
    let challenge = "Bearer realm=\"cdr\"";
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("WWW-Authenticate", challenge)
                .set_body_json(error_body("token expired")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let refused = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await;
    let (got_challenge, body) = match refused {
        Err(ClientError::Unauthorized {
            challenge, body, ..
        }) => (challenge, body),
        other => panic!("expected an unauthorized error, got {other:?}"),
    };
    assert_eq!(got_challenge.as_deref(), Some(challenge));
    assert_eq!(body.message(), Some("token expired"));
    Ok(())
}

/// `403` is a `Forbidden` error carrying the answer body as received.
///
/// Docs text §HTTP status codes: `403` "The service understood the request but
/// refuses to authorize it". The body is the service's own text, kept whole
/// even when it is not an ITS-REST `Error`.
#[tokio::test]
async fn forbidden_is_an_error_carrying_the_body() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/admin/ehr/{EHR_ID}")))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_raw("<html>the admin API is disabled</html>", "text/html"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let refused = admin::client::AdminClient::new(&client)
        .admin_ehr_delete(&admin::AdminEhrDeleteParams {
            ehr_id: EHR_ID.to_owned(),
        })
        .await;
    let body = match refused {
        Err(ClientError::Forbidden { body, .. }) => body,
        other => panic!("expected a forbidden error, got {other:?}"),
    };
    assert!(body.error().is_none(), "an HTML body is not an Error");
    assert_eq!(body.text(), Some("<html>the admin API is disabled</html>"));
    Ok(())
}

/// A `5xx` is a `ServiceFailure` error carrying the answer body.
///
/// Docs text §HTTP status codes lists `500` for every operation; the body the
/// service sent is kept so the caller can log the service's own account.
#[tokio::test]
async fn a_service_failure_carries_the_body() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(500).set_body_json(error_body("the pool is exhausted")))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?.with_retry(RetryPolicy {
        max_attempts: 1,
        ..fast_retry()
    });
    let refused = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await;
    let (status, body) = match refused {
        Err(ClientError::ServiceFailure { status, body, .. }) => (status, body),
        other => panic!("expected a service failure, got {other:?}"),
    };
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.message(), Some("the pool is exhausted"));
    Ok(())
}

/// A `GET` answered `503` is retried and succeeds on the second attempt.
///
/// RFC 9110 §9.2.2 makes `GET` idempotent, so a repeat after a service
/// failure cannot change the outcome.
#[tokio::test]
async fn an_idempotent_request_is_retried_after_a_service_failure() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(to_canonical_value(&ehr_fixture()?)))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?.with_retry(fast_retry());
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await?;
    assert!(
        matches!(outcome, ehr::client::EhrGetByIdOutcome::Ok { .. }),
        "expected the 200 answer on the retry, got {outcome:?}"
    );
    let requests = server.received_requests().await.ok_or("recording is on")?;
    assert_eq!(requests.len(), 2, "one failed attempt, one retry");
    Ok(())
}

/// A `POST` answered `503` is sent exactly once.
///
/// RFC 9110 §9.2.2 does not make `POST` idempotent, so a repeat could commit
/// twice; the failure is reported as a `ServiceFailure` error.
#[tokio::test]
async fn a_post_is_never_retried() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ehr"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?.with_retry(fast_retry());
    let params = ehr::EhrCreateParams {
        prefer: None,
        accept: None,
        content_type: None,
        openehr_version: None,
        openehr_audit_details: None,
    };
    let refused = ehr::client::EhrClient::new(&client)
        .ehr_create(&params, None)
        .await;
    assert!(
        matches!(
            refused,
            Err(ClientError::ServiceFailure { status, .. }) if status == StatusCode::SERVICE_UNAVAILABLE
        ),
        "expected a service failure, got {refused:?}"
    );
    let requests = server.received_requests().await.ok_or("recording is on")?;
    assert_eq!(requests.len(), 1, "a POST is sent exactly once");
    Ok(())
}

/// A request that times out in the engine is a retryable `Transport` error.
///
/// The engine's timeout is a failure that reached no answer, so an idempotent
/// request is sent again within the budget before the error surfaces.
#[tokio::test]
async fn an_engine_timeout_is_a_retried_transport_error() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .expect(2)
        .mount(&server)
        .await;
    let transport = ReqwestTransport::with_timeout(Duration::from_millis(100))?;
    let client = Client::new(transport, server.uri().parse()?)?.with_retry(RetryPolicy {
        max_attempts: 2,
        ..fast_retry()
    });
    let refused = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await;
    assert!(
        matches!(
            refused,
            Err(ClientError::Transport {
                source: TransportError::Timeout { .. },
                ..
            })
        ),
        "expected an engine timeout, got {refused:?}"
    );
    Ok(())
}

/// `Accept` defaults to `application/json`, sent once, when the params set none.
///
/// Docs text §Representation details negotiation: the canonical JSON form is
/// what the typed surface decodes.
#[tokio::test]
async fn accept_defaults_to_canonical_json() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .and(header("Accept", "application/json"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await?;
    assert!(
        matches!(outcome, ehr::client::EhrGetByIdOutcome::NotFound { .. }),
        "expected the 404 answer, got {outcome:?}"
    );
    Ok(())
}

/// An `Accept` the params set replaces the default.
///
/// OAS `ehr-codegen` `ehr_get_by_id` declares `Accept` as a header parameter.
#[tokio::test]
async fn an_explicit_accept_replaces_the_default() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .and(header("Accept", "application/xml"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let params = ehr::EhrGetByIdParams {
        ehr_id: EHR_ID.to_owned(),
        accept: Some("application/xml".to_owned()),
    };
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&params)
        .await?;
    assert!(
        matches!(outcome, ehr::client::EhrGetByIdOutcome::NotFound { .. }),
        "expected the 404 answer, got {outcome:?}"
    );
    Ok(())
}

/// `Credentials::Basic` sends the RFC 7617 `Authorization` header.
///
/// RFC 7617 §2: the credentials are `base64(user-id ":" password)`.
#[tokio::test]
async fn basic_credentials_send_the_rfc_7617_header() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .and(header("Authorization", "Basic YWxpY2U6aHVudGVyMg=="))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?.with_credentials(Credentials::basic("alice", "hunter2"));
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await?;
    assert!(
        matches!(outcome, ehr::client::EhrGetByIdOutcome::NotFound { .. }),
        "expected the 404 answer, got {outcome:?}"
    );
    Ok(())
}

/// `Credentials::Bearer` sends the RFC 6750 `Authorization` header.
///
/// RFC 6750 §2.1: `Authorization: Bearer <token>`.
#[tokio::test]
async fn bearer_credentials_send_the_rfc_6750_header() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/ehr/{EHR_ID}")))
        .and(header("Authorization", "Bearer eyJ.test.token"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?.with_credentials(Credentials::bearer("eyJ.test.token"));
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr(EHR_ID))
        .await?;
    assert!(
        matches!(outcome, ehr::client::EhrGetByIdOutcome::NotFound { .. }),
        "expected the 404 answer, got {outcome:?}"
    );
    Ok(())
}

/// A path parameter with a space travels percent-encoded in one segment.
///
/// RFC 3986 §2.1: a space is not a `pchar`, so it is sent as `%20`.
#[tokio::test]
async fn a_path_parameter_is_percent_encoded() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ehr/has%20space"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let outcome = ehr::client::EhrClient::new(&client)
        .ehr_get_by_id(&get_ehr("has space"))
        .await?;
    assert!(
        matches!(outcome, ehr::client::EhrGetByIdOutcome::NotFound { .. }),
        "expected the 404 answer, got {outcome:?}"
    );
    Ok(())
}

/// A base URL path is kept, with or without a trailing slash, and never doubled.
///
/// The ITS-REST paths hang under the service root, so `/openehr/v1` and
/// `/openehr/v1/` both resolve `/ehr/{ehr_id}` to `/openehr/v1/ehr/{ehr_id}`.
#[tokio::test]
async fn a_base_path_is_kept_without_doubling_the_slash() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/openehr/v1/ehr/{EHR_ID}")))
        .respond_with(ResponseTemplate::new(404))
        .expect(2)
        .mount(&server)
        .await;
    for base in [
        format!("{}/openehr/v1", server.uri()),
        format!("{}/openehr/v1/", server.uri()),
    ] {
        let client = client_at(&base)?;
        let outcome = ehr::client::EhrClient::new(&client)
            .ehr_get_by_id(&get_ehr(EHR_ID))
            .await?;
        assert!(
            matches!(outcome, ehr::client::EhrGetByIdOutcome::NotFound { .. }),
            "expected the 404 answer under {base}, got {outcome:?}"
        );
    }
    Ok(())
}

/// A body that is not the documented shape is a `Body` error naming the path.
///
/// OAS `system-codegen` `Options.endpoints` is an array of strings; the error's
/// source is the `serde_path_to_error` report that locates the defect.
#[tokio::test]
async fn an_undocumented_body_shape_is_a_body_error() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("OPTIONS"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(r#"{"endpoints":"not a list"}"#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server)?;
    let refused = system::client::SystemClient::new(&client)
        .options(&system::OptionsParams { accept: None })
        .await;
    let error = match refused {
        Err(error @ ClientError::Body { .. }) => error,
        other => panic!("expected a body error, got {other:?}"),
    };
    let source = error.source().ok_or("the body error carries its cause")?;
    let report = source
        .downcast_ref::<serde_path_to_error::Error<serde_json::Error>>()
        .ok_or("the body error's cause is the serde_path_to_error report")?;
    assert_eq!(report.path().to_string(), "endpoints");
    Ok(())
}
