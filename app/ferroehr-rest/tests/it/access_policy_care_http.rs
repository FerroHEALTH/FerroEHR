// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's facts about care relationships in the `ACCESS_POLICY`
//! at the wire (#6072): the self-declared and attested paths, the caseload
//! switch, conflict allowances, concealed levels and value-set bindings
//! round-trip through `PUT` and `GET`; a policy that breaks its template or
//! names a code outside a value set it binds is refused `422` naming the
//! attribute (ITS-REST overview `Requests_and_responses.md` §HTTP status
//! codes, 422 "semantic errors"); one that breaks the draft model stays
//! `400`. A care relationship's role is checked against the bound
//! `care_role` value set (#6055). No openEHR spec governs the access policy: our own
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
/// The value set the tests bind organisation types to.
const ORGANISATION_TYPE_VS: &str = "http://terminology.example/ValueSet/organisation-types";

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
    body["attested_care"] = json!([{ "_type": "DV_IDENTIFIER",
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

/// A terminology server whose `value_set` holds `members` alone.
async fn terminology(value_set: &str, members: &[&str]) -> MockServer {
    let server = MockServer::start().await;
    for member in members {
        Mock::given(method("POST"))
            .and(path("/ValueSet/$validate-code"))
            .and(body_string_contains(format!(
                r#""name":"url","valueUri":"{value_set}""#
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
    }
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
    let server = terminology(KINSHIP_VS, &["BRO"]).await;
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

/// The value set the tests bind care roles to.
const CARE_ROLE_VS: &str = "http://terminology.example/ValueSet/care-role";

/// A PERSON created by the administrator; its container.
async fn person(app: &Router) -> String {
    let req = common::req("POST", format!("{BASE}/demographic/person"))
        .auth(common::basic_credential("root"))
        .prefer_representation()
        .json(crate::demographic_http::person_body().to_string())
        .build();
    let (status, headers, body) = common::send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    common::etag_uid(&headers)
        .split("::")
        .next()
        .expect("a versioned-object id")
        .to_owned()
}

/// A care relationship of its own from `patient` to `carer`'s `PERSON` in
/// the role `role` of `HL7_ParticipationType`.
fn care_of(patient: &str, carer: &str, role: &str) -> Value {
    let reference = |id: &str| {
        json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": id } })
    };
    json!({ "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.care.v1",
        "name": access("under_care_of"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "details" },
            "items": [
                { "_type": "ELEMENT", "archetype_node_id": "at0002", "name": access("care_role"),
                  "value": coded("HL7_ParticipationType", role) },
                { "_type": "ELEMENT", "archetype_node_id": "at0003", "name": access("provider"),
                  "value": { "_type": "DV_IDENTIFIER", "issuer": "urn:example:providers",
                             "id": "hospital", "type": "ORGANISATION" } }] },
        "time_validity": { "_type": "DV_INTERVAL",
            "lower": { "_type": "DV_DATE", "value": "2026-11-01" },
            "upper": { "_type": "DV_DATE", "value": "2027-11-01" },
            "lower_unbounded": false, "upper_unbounded": false },
        "source": reference(patient),
        "target": reference(carer) })
}

/// A care relationship's `care_role` is checked against the value set the
/// access policy binds for it, on the terminology server: a role outside it
/// is refused `422` naming the element, one inside it commits, here written
/// outside any request (draft access control package, Care Relationships;
/// #6055).
#[tokio::test]
async fn a_care_role_outside_the_bound_value_set_is_refused() {
    let (_db, pool) = common::migrated_pool().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .and(body_string_contains(format!(
            r#""name":"url","valueUri":"{CARE_ROLE_VS}""#
        )))
        .and(body_string_contains(r#""name":"code","valueCode":"RESP""#))
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
    let (svc, app) = replica(&pool, Some(&server.uri())).await;
    create(&app, &binding(policy(false), "care_role", CARE_ROLE_VS)).await;
    let (patient, carer) = (person(&app).await, person(&app).await);
    let post = |body: Value| {
        common::req("POST", format!("{EXT}/demographic/party_relationship"))
            .auth(common::basic_credential("root"))
            .prefer_representation()
            .json(body.to_string())
            .build()
    };
    let (status, answer) = common::send_json(&app, post(care_of(&patient, &carer, "ATND"))).await;
    names(status, &answer, "/details/care_role");
    let held = openehr_its::json::from_canonical_value(&care_of(&patient, &carer, "RESP"))
        .expect("a PARTY_RELATIONSHIP");
    let mut write = ferroehr::service::version_update::WriteRequest::new(held);
    write.lifecycle_state = Some("532".to_owned());
    Box::pin(svc.party_relationship_create(write))
        .await
        .expect("a role inside the bound value set commits");
}

/// [`policy`] naming the organisation type `GP` of a synthetic code system
/// in a rule, its exception and a conflict rule.
fn with_organisation_types() -> Value {
    let gp = || {
        json!([{ "_type": "ACCESS_ACTOR_SCOPE",
        "organisation_type": coded("urn:example:organisation-types", "GP") }])
    };
    let rule = |effect: &str, extra: Value| {
        let mut rule = json!({ "_type": "ACCESS_DIRECTIVE",
            "name": { "_type": "DV_TEXT", "value": "Exchange rule" },
            "archetype_node_id": "id15",
            "kind": access("controller_rule"),
            "effect": coded("HL7_ConsentProvisionType", effect),
            "applies_to": gp(),
            "emergency_override": false });
        if let (Some(rule), Some(extra)) = (rule.as_object_mut(), extra.as_object()) {
            rule.extend(extra.clone());
        }
        rule
    };
    let mut body = policy(false);
    body["rules"] = json!([rule(
        "deny",
        json!({ "exceptions": [rule("permit", json!({}))] })
    )]);
    body["conflict_rules"] = json!([{ "_type": "ACCESS_CONFLICT_RULE",
        "conflict": access("self"), "applies_to": gp() }]);
    body
}

/// The draft binds `organisation_type` to no code system ("none: the
/// controller binds one before a rule names a type", draft access control
/// Terminology, Value Sets), so a policy naming a type it binds no value set
/// for is refused `422` naming every place it names one; bound to a value
/// set holding the type, the same policy commits (#6096).
#[tokio::test]
async fn an_organisation_type_the_policy_binds_no_value_set_for_is_refused() {
    let (_db, pool) = common::migrated_pool().await;
    let server = terminology(ORGANISATION_TYPE_VS, &["GP"]).await;
    let (_svc, app) = replica(&pool, Some(&server.uri())).await;
    let (status, answer) = put(
        &app,
        (header::IF_NONE_MATCH, "*"),
        &with_organisation_types(),
    )
    .await;
    for attribute in [
        "/rules/0/applies_to/0/organisation_type",
        "/rules/0/exceptions/0/applies_to/0/organisation_type",
        "/conflict_rules/0/applies_to/0/organisation_type",
    ] {
        names(status, &answer, attribute);
    }
    let (status, _, _) = call(&app, "GET", "root", "", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the refused write left none");

    let bound = binding(
        with_organisation_types(),
        "organisation_type",
        ORGANISATION_TYPE_VS,
    );
    let (status, answer) = put(&app, (header::IF_NONE_MATCH, "*"), &bound).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
}

/// [`policy`] with a rule whose actor scope names a care relationship in
/// the role `role` by the path `direct`, care purposes `purposes` and the
/// time zone `zone`.
fn with_care_rule(role: &Value, purposes: &[Value], zone: &str) -> Value {
    let mut body = policy(true);
    body["rules"] = json!([{ "_type": "ACCESS_DIRECTIVE", "archetype_node_id": "id9",
        "name": { "_type": "DV_TEXT", "value": "Only the responsible practitioner" },
        "kind": access("controller_rule"),
        "effect": coded("HL7_ConsentProvisionType", "deny"),
        "emergency_override": false,
        "exceptions": [{ "_type": "ACCESS_DIRECTIVE", "archetype_node_id": "id10",
            "name": { "_type": "DV_TEXT", "value": "responsible, directly" },
            "kind": access("controller_rule"),
            "effect": coded("HL7_ConsentProvisionType", "permit"),
            "emergency_override": false,
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "relationship": access("care"),
                "care_roles": [role], "care_path": access("direct") }] }] }]);
    body["care_purposes"] = json!(purposes);
    body["time_zone"] = json!(zone);
    body
}

/// A rule's care roles and path, the care purposes and the time zone are
/// decided, so they round-trip; a care role outside `HL7_ParticipationType`
/// or the bound `care_role` value set, and a care purpose outside
/// `HL7_ActReason`, are refused `422` naming the attribute; a time zone the
/// zone database does not hold is `400` (#6075).
#[tokio::test]
async fn a_rules_care_roles_purposes_and_time_zone_are_stored_or_refused() {
    let (_db, pool) = common::migrated_pool().await;
    let care_role_vs = "http://terminology.example/ValueSet/care-role";
    let server = terminology(care_role_vs, &["RESP"]).await;
    let (_svc, app) = replica(&pool, Some(&server.uri())).await;
    let treat = coded("HL7_ActReason", "TREAT");
    let written = with_care_rule(
        &coded("HL7_ParticipationType", "RESP"),
        std::slice::from_ref(&treat),
        "Europe/Amsterdam",
    );
    let first = create(&app, &written).await;
    let (status, _, read) = call(&app, "GET", "root", "", None, None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    for attribute in ["rules", "care_purposes", "time_zone"] {
        assert_eq!(read[attribute], written[attribute], "{attribute}");
    }
    let scope = "/rules/0/exceptions/0/applies_to/0/care_roles/0";
    let local_role = with_care_rule(
        &coded("urn:example:roles", "RESP"),
        std::slice::from_ref(&treat),
        "UTC",
    );
    let (status, answer) = replace(&app, &first, &local_role).await;
    names(status, &answer, scope);
    let bound = |role: &str| {
        binding(
            with_care_rule(
                &coded("urn:example:roles", role),
                std::slice::from_ref(&treat),
                "UTC",
            ),
            "care_role",
            care_role_vs,
        )
    };
    let (status, answer) = replace(&app, &first, &bound("SPRF")).await;
    names(status, &answer, scope);
    let research = with_care_rule(
        &coded("HL7_ParticipationType", "RESP"),
        &[coded("urn:example:purposes", "HRESCH")],
        "UTC",
    );
    let (status, answer) = replace(&app, &first, &research).await;
    names(status, &answer, "/care_purposes/0");
    let atlantis = with_care_rule(
        &coded("HL7_ParticipationType", "RESP"),
        std::slice::from_ref(&treat),
        "Europe/Atlantis",
    );
    let (status, answer) = replace(&app, &first, &atlantis).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    let (status, answer) = replace(&app, &first, &bound("RESP")).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}
