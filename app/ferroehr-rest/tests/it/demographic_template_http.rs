// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for demographic operational templates: a ROLE and a
//! PERSON template uploaded through `POST /definition/template/adl1.4`, the
//! parties built against them, their Web Templates and their examples.
//!
//! `Role.v2` is a deployed template (`corpus/fixtures/opt/valid/role.v2.opt`)
//! whose `capabilities` and `credentials` attributes carry no cardinality while
//! their children state occurrences `0..*`. ADL 1.4 `master05-cadl.adoc`
//! §Occurrences calls that "not illegal": the value of a single-valued
//! attribute can only be `0..1` or `1..1`. `Person.v2` here is a synthetic
//! PERSON template with a `relationships` node, standing in for the PERSON
//! template a deployment pairs with it.
//!
//! The party model is RM demographic `master02-demographic_package.adoc`: a
//! patient is a PERSON whose `relationships` holds a `PARTY_RELATIONSHIP` to a
//! shared ROLE; `ROLE.performer` is mandatory
//! (`UML/classes/org.openehr.rm.demographic.role.adoc`, `performer` 1..1).
#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use ferroehr::config::rest::RestConfig;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::AuthSection;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const ROLE_TEMPLATE: &str = "Role.v2";
const PERSON_TEMPLATE: &str = "Person.v2";

fn corpus_opt(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/opt/valid")
        .join(name);
    std::fs::read_to_string(path).expect("OPT fixture under corpus/fixtures/opt/valid")
}

fn config() -> AppConfig {
    AppConfig {
        server: common::server(),
        rest: RestConfig {
            base_path: BASE.to_owned(),
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            cors_permissive: false,
            ..Default::default()
        },
        auth: AuthSection {
            enabled: false,
            basic: None,
            ..AuthSection::default()
        },
        ..Default::default()
    }
}

async fn app() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(config(), service).await)
}

/// A router with both demographic templates uploaded through the wire.
async fn app_with_templates() -> (testkit::TestDb, Router) {
    let (pg, app) = app().await;
    for name in ["role.v2.opt", "person.v2.opt"] {
        let (status, body) = upload(&app, corpus_opt(name)).await;
        assert_eq!(status, StatusCode::CREATED, "{name} uploads: {body}");
    }
    (pg, app)
}

struct Answer {
    status: StatusCode,
    content_type: Option<String>,
    etag: Option<String>,
    body: String,
}

async fn send(app: &Router, req: Request<Body>) -> Answer {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let header_text = |name: header::HeaderName| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let content_type = header_text(header::CONTENT_TYPE);
    let etag = header_text(header::ETAG);
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    Answer {
        status,
        content_type,
        etag,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn upload(app: &Router, opt: String) -> (StatusCode, String) {
    let req = common::req("POST", format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(opt)
        .build();
    let answer = send(app, req).await;
    (answer.status, answer.body)
}

async fn get(app: &Router, uri: &str, accept: &str) -> Answer {
    let req = common::req("GET", uri).accept(accept).build();
    send(app, req).await
}

/// Commits `body` on `demographic/{seg}` and returns its `version_uid`.
async fn create(app: &Router, seg: &str, body: &Value) -> String {
    let req = common::req("POST", common::full(&format!("/demographic/{seg}")))
        .json(body.to_string())
        .build();
    let answer = send(app, req).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "create {seg}: {}",
        answer.body
    );
    weak_etag_value(answer.etag.as_deref())
}

fn weak_etag_value(etag: Option<&str>) -> String {
    etag.expect("ETag present")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

fn container_of(version_uid: &str) -> &str {
    version_uid
        .split("::")
        .next()
        .expect("version container uuid")
}

fn party_ref(kind: &str, id: &str) -> Value {
    json!({
        "_type": "PARTY_REF",
        "namespace": "demographic",
        "type": kind,
        "id": { "_type": "HIER_OBJECT_ID", "value": id }
    })
}

fn archetyped(archetype_id: &str, template_id: Option<&str>) -> Value {
    let mut details = json!({
        "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype_id },
        "rm_version": "1.1.0"
    });
    if let Some(template_id) = template_id {
        details.as_object_mut().expect("an object").insert(
            "template_id".to_owned(),
            json!({ "_type": "TEMPLATE_ID", "value": template_id }),
        );
    }
    details
}

/// The organisation that performs the shared ROLE.
fn organisation_body() -> Value {
    json!({
        "_type": "ORGANISATION",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1", None),
        "name": { "_type": "DV_TEXT", "value": "Example care provider" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "value": { "_type": "DV_TEXT", "value": "Example care provider" }
                }]
            }
        }]
    })
}

/// The shared ROLE "Patient", built against `Role.v2`.
fn patient_role_body(performer: &str) -> Value {
    json!({
        "_type": "ROLE",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.Role.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-ROLE.Role.v1", Some(ROLE_TEMPLATE)),
        "name": { "_type": "DV_TEXT", "value": "Patient" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.role_name.v1",
            "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PARTY_IDENTITY.role_name.v1", None),
            "name": {
                "_type": "DV_CODED_TEXT",
                "value": "Officiële naam",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                    "code_string": "at0025"
                }
            },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0001",
                "name": { "_type": "DV_TEXT", "value": "Detail" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0010",
                    "name": { "_type": "DV_TEXT", "value": "Naam" },
                    "value": { "_type": "DV_TEXT", "value": "Patient" }
                }]
            }
        }],
        "performer": party_ref("ORGANISATION", performer)
    })
}

/// A PERSON built against `Person.v2`, with the `role` relationship to the
/// ROLE container `role` when given.
fn person_body(source: Option<&str>, role: Option<&str>) -> Value {
    let mut body = json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v2",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PERSON.person.v2", Some(PERSON_TEMPLATE)),
        "name": { "_type": "DV_TEXT", "value": "Person" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "Name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "Structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "Family name" },
                    "value": { "_type": "DV_TEXT", "value": "Example" }
                }]
            }
        }]
    });
    if let (Some(source), Some(role)) = (source, role) {
        body.as_object_mut().expect("an object").insert(
            "relationships".to_owned(),
            json!([{
                "_type": "PARTY_RELATIONSHIP",
                "archetype_node_id": "at0004",
                "name": { "_type": "DV_TEXT", "value": "role" },
                "source": party_ref("PERSON", source),
                "target": party_ref("ROLE", role)
            }]),
        );
    }
    body
}

fn template_uri(template_id: &str) -> String {
    format!("{BASE}/definition/template/adl1.4/{template_id}")
}

fn example_uri(template_id: &str, level: &str, kind: &str) -> String {
    format!(
        "{}/example?detail_level={level}&type={kind}",
        template_uri(template_id)
    )
}

#[tokio::test]
async fn role_template_uploads_and_lists_its_root() {
    let (_pg, app) = app().await;
    let (status, body) = upload(&app, corpus_opt("role.v2.opt")).await;
    assert_eq!(status, StatusCode::CREATED, "Role.v2 uploads: {body}");

    let listed = get(
        &app,
        &format!("{BASE}/definition/template/adl1.4"),
        "application/json",
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK);
    let rows: Value = serde_json::from_str(&listed.body).unwrap();
    let row = rows
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["template_id"] == ROLE_TEMPLATE))
        .expect("Role.v2 is listed");
    assert_eq!(row["archetype_id"], "openEHR-DEMOGRAPHIC-ROLE.Role.v1");

    let source = get(&app, &template_uri(ROLE_TEMPLATE), "application/xml").await;
    assert_eq!(source.status, StatusCode::OK);
    assert!(source.body.contains("<rm_type_name>ROLE</rm_type_name>"));
}

/// The Web Template of `Role.v2`: a ROLE root whose `capabilities` child
/// admits one `CAPABILITY`, the cap a single-valued attribute puts on it.
#[tokio::test]
async fn role_template_serves_its_web_template() {
    let (_pg, app) = app_with_templates().await;
    let wt = get(
        &app,
        &template_uri(ROLE_TEMPLATE),
        "application/openehr.wt+json",
    )
    .await;
    assert_eq!(wt.status, StatusCode::OK, "{}", wt.body);
    let wt: Value = serde_json::from_str(&wt.body).unwrap();
    assert_eq!(wt["tree"]["rmType"], "ROLE");
    let capability = wt["tree"]["children"]
        .as_array()
        .and_then(|c| c.iter().find(|n| n["rmType"] == "CAPABILITY"))
        .expect("the CAPABILITY node is in the tree");
    assert_eq!(capability["max"], 1, "{capability}");
}

/// The flow a deployment uses for patients: a shared ROLE "Patient" and a
/// PERSON whose `relationships` points at it, both read back.
#[tokio::test]
async fn a_person_holds_the_patient_role_through_its_relationships() {
    let (_pg, app) = app_with_templates().await;

    let organisation = create(&app, "organisation", &organisation_body()).await;
    let role = create(
        &app,
        "role",
        &patient_role_body(container_of(&organisation)),
    )
    .await;
    let role_id = container_of(&role).to_owned();

    let person = create(&app, "person", &person_body(None, None)).await;
    let person_id = container_of(&person).to_owned();
    let update = common::req("PUT", format!("{BASE}/demographic/person/{person_id}"))
        .if_match(format!("\"{person}\""))
        .prefer_representation()
        .json(person_body(Some(&person_id), Some(&role_id)).to_string())
        .build();
    let updated = send(&app, update).await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    let updated: Value = serde_json::from_str(&updated.body).unwrap();
    assert_eq!(updated["relationships"][0]["target"]["type"], "ROLE");

    let read_role = get(
        &app,
        &format!("{BASE}/demographic/role/{role_id}"),
        "application/json",
    )
    .await;
    assert_eq!(read_role.status, StatusCode::OK);
    let read_role: Value = serde_json::from_str(&read_role.body).unwrap();
    assert_eq!(read_role["name"]["value"], "Patient");
    assert_eq!(
        read_role["archetype_details"]["template_id"]["value"],
        ROLE_TEMPLATE
    );

    let read_person = get(
        &app,
        &format!("{BASE}/demographic/person/{person_id}"),
        "application/json",
    )
    .await;
    assert_eq!(read_person.status, StatusCode::OK);
    let read_person: Value = serde_json::from_str(&read_person.body).unwrap();
    let relationship = &read_person["relationships"][0];
    assert_eq!(relationship["name"]["value"], "role");
    assert_eq!(relationship["target"]["type"], "ROLE");
    assert_eq!(relationship["target"]["id"]["value"], role_id.as_str());
}

/// The example of a demographic template is a party of its root class, in
/// canonical JSON at every detail level and in both forms.
#[tokio::test]
async fn a_demographic_template_example_is_a_party() {
    let (_pg, app) = app_with_templates().await;
    for (template_id, class) in [(ROLE_TEMPLATE, "ROLE"), (PERSON_TEMPLATE, "PERSON")] {
        for level in ["required", "medium", "complete"] {
            for kind in ["input", "output"] {
                let uri = example_uri(template_id, level, kind);
                let answer = get(&app, &uri, "application/json").await;
                assert_eq!(answer.status, StatusCode::OK, "{uri}: {}", answer.body);
                assert_eq!(answer.content_type.as_deref(), Some("application/json"));
                let party: Value = serde_json::from_str(&answer.body).unwrap();
                assert_eq!(party["_type"], class, "{uri}");
                assert_eq!(
                    party.get("uid").is_some(),
                    kind == "output",
                    "{uri}: only the output form carries a uid"
                );
                if class == "ROLE" {
                    assert_eq!(party["performer"]["_type"], "PARTY_REF", "{uri}");
                }
            }
        }
    }
}

#[tokio::test]
async fn a_demographic_template_example_serves_canonical_xml() {
    let (_pg, app) = app_with_templates().await;
    for (template_id, class) in [(ROLE_TEMPLATE, "ROLE"), (PERSON_TEMPLATE, "PERSON")] {
        let uri = example_uri(template_id, "medium", "input");
        let answer = get(&app, &uri, "application/xml").await;
        assert_eq!(answer.status, StatusCode::OK, "{uri}: {}", answer.body);
        assert!(
            answer
                .content_type
                .as_deref()
                .is_some_and(|ct| ct.starts_with("application/xml")),
            "{uri}: {:?}",
            answer.content_type
        );
        // ITS-XML `ALL/Structure.xsd` publishes `items` (abstract `LOCATABLE`)
        // as the only element a party can be rooted at.
        assert!(answer.body.contains("<items "), "{uri}: {}", answer.body);
        assert!(
            answer.body.contains(&format!(r#"xsi:type="{class}""#)),
            "{uri}: {}",
            answer.body
        );
    }
}

/// The Simplified Formats RM mapping (ITS-REST
/// `simplified_formats/master05-rm_mapping.adoc`) maps no demographic party
/// class, so a party example refuses FLAT and STRUCTURED as the `406` the
/// operation declares.
#[tokio::test]
async fn a_demographic_template_example_refuses_the_simplified_formats() {
    let (_pg, app) = app_with_templates().await;
    for accept in [
        "application/openehr.wt.flat+json",
        "application/openehr.wt.structured+json",
    ] {
        let uri = example_uri(PERSON_TEMPLATE, "required", "input");
        let answer = get(&app, &uri, accept).await;
        assert_eq!(
            answer.status,
            StatusCode::NOT_ACCEPTABLE,
            "{accept}: {}",
            answer.body
        );
    }
}
