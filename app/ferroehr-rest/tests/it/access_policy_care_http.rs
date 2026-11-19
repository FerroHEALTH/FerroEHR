// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's facts about care relationships in the `ACCESS_POLICY`
//! at the wire (#6072): the self-declared and attested paths, the caseload
//! switch, conflict allowances, concealed levels and value-set bindings
//! round-trip through `PUT` and `GET`; a policy that breaks its template or
//! names a code outside a value set it binds is refused `422` naming the
//! attribute (ITS-REST overview `Requests_and_responses.md` §HTTP status
//! codes, 422 "semantic errors"); one that breaks the draft model stays
//! `400`. No openEHR spec governs the access policy: our own
//! design/extension over FerroEHR's draft access control package.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferroehr::service::terminology::config::{OnError, TerminologyServerConfig};
use ferroehr::service::terminology::fhir::FhirTerminologyProvider;
use http::{StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::access_policy_http::{call, create, policy, replica_of};
use crate::common;
use crate::common::{BASE, EXT};

/// The value set the tests bind kinship to.
const KINSHIP_VS: &str = "http://terminology.example/ValueSet/kinship";

/// A coded term of `terminology`.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// A term of the draft `openehr_access` terminology.
fn access(code: &str) -> Value {
    coded("openehr_access", code)
}

/// [`policy`] with every care fact the policy stores, its kinship coded
/// `kinship` and unbound.
fn with_care(kinship: &Value) -> Value {
    let mut body = policy(true);
    body["self_declared_care"] = json!({ "_type": "DV_DURATION", "value": "P1D" });
    body["attested_care"] = json!([{ "_type": "DV_IDENTIFIER", "issuer": "Regional exchange",
        "id": "urn:example:agreement:attested-care" }]);
    body["caseload"] = json!(false);
    body["conflict_rules"] = json!([{ "_type": "ACCESS_CONFLICT_RULE",
        "conflict": access("related"), "kinships": [kinship] }]);
    body["concealed_confidentialities"] = json!([coded("HL7_Confidentiality", "V")]);
    body
}

/// `body` with `binding` bound to the value set `url`.
fn binding(mut body: Value, binding: &str, url: &str) -> Value {
    let entry = json!({ "_type": "ACCESS_VALUE_SET", "binding": access(binding),
        "value_set": { "_type": "DV_URI", "value": url } });
    match body["value_sets"].as_array_mut() {
        Some(bound) => bound.push(entry),
        None => body["value_sets"] = json!([entry]),
    }
    body
}

/// A terminology server whose [`KINSHIP_VS`] holds `member` alone.
async fn terminology(member: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .and(body_string_contains(format!(
            r#""name":"url","valueUri":"{KINSHIP_VS}""#
        )))
        .and(body_string_contains(format!(
            r#""name":"code","valueCode":"{member}""#
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{ "name": "result", "valueBoolean": true }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{ "name": "result", "valueBoolean": false }]
        })))
        .mount(&server)
        .await;
    server
}

/// A replica over `pool`, asking the terminology server at `base` when one
/// is given, refusing a code it cannot check.
async fn replica(pool: &PgPool, base: Option<&str>) -> (Arc<FerroEhrService>, Router) {
    let mut svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool));
    if let Some(base) = base {
        let cfg = TerminologyServerConfig {
            endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
                url: ferrohealth_secret::secret_url::SecretUrl::new(base),
                timeout: ferrohealth_config::units::Duration::from_millis(800),
                ..ferrohealth_smart::config::endpoint::EndpointSection::default()
            },
            on_error: OnError::Refuse,
        };
        let server = FhirTerminologyProvider::new(&cfg).expect("the terminology client builds");
        svc = svc.with_terminology_server(Arc::new(server));
    }
    replica_of(svc, pool, false, true).await
}

/// `PUT` `body` under the precondition `condition`, asking for each
/// violation in the answer; the status and the answer.
async fn put(
    app: &Router,
    condition: (header::HeaderName, &str),
    body: &Value,
) -> (StatusCode, Value) {
    let req = common::req("PUT", format!("{EXT}/admin/access_policy"))
        .auth(common::basic_credential("root"))
        .header(condition.0, condition.1)
        .prefer_representation()
        .json(body.to_string())
        .build();
    common::send_json(app, req).await
}

/// `PUT` `body` over the version `etag`; the status and the answer.
async fn replace(app: &Router, etag: &str, body: &Value) -> (StatusCode, Value) {
    put(app, (header::IF_MATCH, etag), body).await
}

/// Asserts `answer` is a `422` whose text names `attribute`.
fn names(status: StatusCode, answer: &Value, attribute: &str) {
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert!(
        answer.to_string().contains(attribute),
        "{attribute}: {answer}"
    );
}

#[tokio::test]
async fn every_care_fact_round_trips_through_put_and_get() {
    let (_db, pool) = common::migrated_pool().await;
    let (_svc, app) = replica(&pool, None).await;
    let written = binding(
        with_care(&coded("openehr", "10")),
        "care_role",
        "http://terminology.example/ValueSet/care-role",
    );
    create(&app, &written).await;
    let (status, _, read) = call(&app, "GET", "root", "", None, None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    for attribute in [
        "self_declared_care",
        "attested_care",
        "caseload",
        "conflict_rules",
        "concealed_confidentialities",
        "value_sets",
    ] {
        assert_eq!(read[attribute], written[attribute], "{attribute}");
    }
}

#[tokio::test]
async fn a_code_outside_its_value_set_is_refused_naming_the_attribute() {
    let (_db, pool) = common::migrated_pool().await;
    let server = terminology("BRO").await;
    let (_svc, app) = replica(&pool, Some(&server.uri())).await;
    let first = create(&app, &policy(false)).await;

    let bound = |kinship| {
        binding(
            with_care(&coded("urn:example:kinship", kinship)),
            "kinship",
            KINSHIP_VS,
        )
    };
    let (status, answer) = replace(&app, &first, &bound("SIS")).await;
    names(status, &answer, "/conflict_rules/0/kinships/0");

    let (status, answer) = replace(&app, &first, &with_care(&coded("openehr", "999"))).await;
    names(status, &answer, "/conflict_rules/0/kinships/0");
    let (status, answer) = replace(
        &app,
        &first,
        &with_care(&coded("urn:example:kinship", "BRO")),
    )
    .await;
    names(status, &answer, "/conflict_rules/0/kinships/0");

    let mut unknown_level = with_care(&coded("openehr", "10"));
    unknown_level["concealed_confidentialities"] = json!([coded("HL7_Confidentiality", "X")]);
    let (status, answer) = replace(&app, &first, &unknown_level).await;
    names(status, &answer, "/concealed_confidentialities/0");

    let twice = binding(bound("BRO"), "kinship", KINSHIP_VS);
    let (status, answer) = replace(&app, &first, &twice).await;
    names(status, &answer, "/value_sets/1/binding");

    let (status, answer) = replace(&app, &first, &bound("BRO")).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

#[tokio::test]
async fn a_bound_code_no_terminology_server_can_check_is_refused() {
    let (_db, pool) = common::migrated_pool().await;
    let (_svc, app) = replica(&pool, None).await;
    let body = binding(
        with_care(&coded("urn:example:kinship", "BRO")),
        "kinship",
        KINSHIP_VS,
    );
    let (status, answer) = put(&app, (header::IF_NONE_MATCH, "*"), &body).await;
    names(status, &answer, "/conflict_rules/0/kinships/0");
    let (status, _, _) = call(&app, "GET", "root", "", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the refused write left none");
}

#[tokio::test]
async fn a_policy_that_breaks_the_draft_model_stays_bad_request() {
    let (_db, pool) = common::migrated_pool().await;
    let (_svc, app) = replica(&pool, None).await;
    let first = create(&app, &policy(false)).await;
    let mut self_with_kinships = with_care(&coded("openehr", "10"));
    self_with_kinships["conflict_rules"][0]["conflict"] = access("self");
    let mut instant_self_declared = with_care(&coded("openehr", "10"));
    instant_self_declared["self_declared_care"]["value"] = json!("PT0S");
    for body in [self_with_kinships, instant_self_declared] {
        let (status, answer) = replace(&app, &first, &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
}

/// An ADL2 operational template of the access policy, specialising the
/// default archetype, that requires `self_declared_care`.
fn profile_template() -> String {
    "operational_template (adl_version=2.0.6; rm_release=1.1.0)\n    \
     openEHR-EHR-ACCESS_POLICY.arcadia.v1.0.0\n\n\
     language\n    original_language = <[ISO_639-1::en]>\n\n\
     description\n    original_author = <[\"name\"] = <\"FerroEHR tests\">>\n    \
     lifecycle_state = <\"published\">\n    details = <\n        [\"en\"] = <\n            \
     language = <[ISO_639-1::en]>\n        >\n    >\n\n\
     definition\n    ACCESS_POLICY[id1] matches {\n        \
     self_declared_care existence matches {1..1} matches {\n            \
     DV_DURATION[id2]\n        }\n    }\n\n\
     terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
     [\"id1\"] = <text = <\"Arcadia access policy\"> description = <\"A profile.\">>\n            \
     [\"id2\"] = <text = <\"Self-declared care\"> description = <\"The longest period.\">>\n        \
     >\n    >\n"
        .to_owned()
}

/// `body` naming `template` in its `archetype_details`.
fn under(mut body: Value, template: &str) -> Value {
    body["archetype_details"]["template_id"] = json!({ "_type": "TEMPLATE_ID", "value": template });
    body
}

#[tokio::test]
async fn a_policy_is_checked_against_the_template_it_names() {
    let (_db, pool) = common::migrated_pool().await;
    let (_svc, app) = replica(&pool, None).await;
    let first = create(&app, &policy(false)).await;

    let (status, answer) = replace(&app, &first, &under(policy(true), "no.such.template.v1")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");

    let upload = common::req("POST", format!("{BASE}/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain")
        .auth(common::basic_credential("root"))
        .body(profile_template())
        .build();
    let (status, _, text) = common::send(&app, upload).await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    let template = "openEHR-EHR-ACCESS_POLICY.arcadia.v1.0.0";

    let (status, answer) = replace(&app, &first, &under(policy(true), template)).await;
    names(status, &answer, "self_declared_care");
    let (status, answer) = replace(
        &app,
        &first,
        &under(with_care(&coded("openehr", "10")), template),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}
