// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for EHDS Art. 5 (`docs/law/eu/ehds/text.html`): what a
//! natural person inserts through a patient-context credential is "clearly
//! distinguishable as having been inserted by the natural person", and the
//! person "shall not be able to directly alter the electronic health data and
//! related information inserted by health professionals".
//!
//! A person's commit records `PARTY_SELF` as its committer, the RM's "committer
//! is the record subject" (RM common master04 §Audit Details), with the login
//! in its `external_ref`; a read, an AQL query and the contribution list tell
//! it apart. A `patient/`-only token may update or delete what such a commit
//! inserted and is refused (`403`) on a version anyone else committed.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ferroehr::config::rest::RestConfig;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::{AuthSection, IssuerKey};

use crate::common::{self, BASE, EXT};

const AUDIENCE: &str = "ferroehr";

/// The professional's grant: every composition, template and query operation.
const PROFESSIONAL: &str = "user/composition-*.cruds user/template-*.cruds user/aql-*.s";

/// The person's grant: their own EHR's compositions, through the access service.
const PERSON: &str = "patient/composition-*.cruds";

/// An authorization server serving its RFC 8414 metadata and the committed test
/// key set.
async fn issuer() -> MockServer {
    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": uri,
            "jwks_uri": format!("{uri}/jwks"),
            "token_endpoint": format!("{uri}/token"),
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_string(include_str!(
            "../../../../docs/conformance/party/smart/jwks.json"
        )))
        .mount(&server)
        .await;
    server
}

/// A server trusting `issuer`, whose `patient/` grants count.
async fn app(issuer: &str) -> (testkit::TestDb, Router) {
    let mut cfg = AppConfig {
        server: common::server(),
        rest: RestConfig {
            base_path: BASE.to_owned(),
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            cors_permissive: false,
            ..Default::default()
        },
        auth: AuthSection {
            issuer: IssuerKey::Flat(issuer.to_owned()),
            audience: vec![AUDIENCE.to_owned()],
            ..AuthSection::default()
        },
        ..Default::default()
    };
    cfg.smart.patient_issuers = vec![issuer.to_owned()];
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(cfg, service).await)
}

/// A bearer token of `issuer` for `subject` granting `scope`, launched for
/// `ehr` when given.
fn token(issuer: &str, subject: &str, scope: &str, ehr: Option<&str>) -> String {
    let mut claims = json!({
        "sub": subject,
        "iss": issuer,
        "aud": AUDIENCE,
        "exp": jiff::Timestamp::now().as_second() + 3600,
        "scope": scope,
    });
    if let Some(ehr) = ehr {
        claims["ehrId"] = json!(ehr);
    }
    common::bearer(&claims)
}

/// A request with `credential`, carrying `body` as JSON when given.
fn call(
    verb: &str,
    uri: &str,
    credential: &str,
    if_match: Option<&str>,
    body: Option<&Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(verb)
        .uri(uri)
        .header(header::ACCEPT, "application/json")
        .header(header::AUTHORIZATION, credential);
    if let Some(version) = if_match {
        builder = builder.header(header::IF_MATCH, format!("\"{version}\""));
    }
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

/// The IPS composition with `composer` as its composer and no uid.
fn composition(composer: &Value) -> Value {
    let mut body = common::ips_canonical_composition();
    let object = body.as_object_mut().expect("a composition object");
    object.remove("uid");
    object.insert("composer".to_owned(), composer.clone());
    body
}

/// The composer naming the record's person.
fn party_self() -> Value {
    json!({ "_type": "PARTY_SELF" })
}

/// The composer naming a professional.
fn professional_composer() -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Bob" })
}

/// The versioned-object id of a version uid (`<vo>::<system>::<tree>`).
fn vo_of(version: &str) -> &str {
    version.split("::").next().expect("a version uid")
}

/// Commits `body` as a new composition of `ehr`, returning the version uid.
async fn create(app: &Router, ehr: &str, credential: &str, body: &Value) -> String {
    let (status, headers, text) = common::send(
        app,
        call(
            "POST",
            &format!("{BASE}/ehr/{ehr}/composition"),
            credential,
            None,
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "composition create: {text}");
    common::etag_uid(&headers)
}

/// Updates the composition `version` names to `body`.
async fn update(
    app: &Router,
    ehr: &str,
    credential: &str,
    version: &str,
    body: &Value,
) -> (StatusCode, header::HeaderMap, String) {
    common::send(
        app,
        call(
            "PUT",
            &format!("{BASE}/ehr/{ehr}/composition/{}", vo_of(version)),
            credential,
            Some(version),
            Some(body),
        ),
    )
    .await
}

/// Deletes the composition whose current version is `version`.
async fn delete(app: &Router, ehr: &str, credential: &str, version: &str) -> (StatusCode, String) {
    common::send_body(
        app,
        call(
            "DELETE",
            &format!("{BASE}/ehr/{ehr}/composition/{version}"),
            credential,
            None,
            None,
        ),
    )
    .await
}

/// The committer of the stored version `version`.
async fn committer(app: &Router, ehr: &str, credential: &str, version: &str) -> Value {
    let (status, text) = common::send_body(
        app,
        call(
            "GET",
            &format!(
                "{BASE}/ehr/{ehr}/versioned_composition/{}/version/{version}",
                vo_of(version)
            ),
            credential,
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "version read: {text}");
    let version: Value = serde_json::from_str(&text).unwrap();
    version["commit_audit"]["committer"].clone()
}

/// A professional's EHR with the IPS template, a professional's composition
/// and the person's credential, ready for the person to act.
struct Record {
    _pg: testkit::TestDb,
    _issuer: MockServer,
    app: Router,
    ehr: String,
    professional: String,
    person: String,
    professionals_entry: String,
}

async fn record() -> Record {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (pg, app) = app(&uri).await;
    let professional = token(&uri, "dr-bob", PROFESSIONAL, None);
    let (status, headers, text) = common::send(
        &app,
        call("POST", &format!("{BASE}/ehr"), &professional, None, None),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {text}");
    let ehr = common::etag_uid(&headers);
    let upload = common::req("POST", format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .auth(&professional)
        .body(common::ips_opt_xml())
        .build();
    let (status, text) = common::send_body(&app, upload).await;
    assert_eq!(status, StatusCode::CREATED, "IPS OPT upload: {text}");
    let professionals_entry = create(
        &app,
        &ehr,
        &professional,
        &composition(&professional_composer()),
    )
    .await;
    let person = token(&uri, "alice", PERSON, Some(&ehr));
    Record {
        _pg: pg,
        _issuer: issuer,
        app,
        ehr,
        professional,
        person,
        professionals_entry,
    }
}

/// A person's commit records `PARTY_SELF` with their login, whatever committer
/// the request names, and reads back so; a professional's records the
/// professional.
#[tokio::test]
async fn a_persons_commit_is_marked_as_theirs_and_reads_back_marked() {
    let r = record().await;
    let mut request = call(
        "POST",
        &format!("{BASE}/ehr/{}/composition", r.ehr),
        &r.person,
        None,
        Some(&composition(&party_self())),
    );
    request.headers_mut().insert(
        "openehr-audit-details",
        "committer.name=\"Dr Fake\"".parse().unwrap(),
    );
    let (status, headers, text) = common::send(&r.app, request).await;
    assert_eq!(status, StatusCode::CREATED, "the person's insert: {text}");
    let persons_entry = common::etag_uid(&headers);

    let theirs = committer(&r.app, &r.ehr, &r.professional, &persons_entry).await;
    assert_eq!(theirs["_type"], "PARTY_SELF", "{theirs}");
    assert_eq!(theirs["external_ref"]["id"]["value"], "alice", "{theirs}");
    assert_eq!(theirs["external_ref"]["type"], "PERSON", "{theirs}");
    let professionals = committer(&r.app, &r.ehr, &r.professional, &r.professionals_entry).await;
    assert_eq!(
        professionals["_type"], "PARTY_IDENTIFIED",
        "{professionals}"
    );
    assert_eq!(professionals["name"], "dr-bob", "{professionals}");

    // An AQL query reads each version's committer, which tells the person's
    // entries from the professional's.
    let query = json!({
        "q": "SELECT c/uid/value, v/commit_audit/committer \
              FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c"
    });
    let (status, text) = common::send_body(
        &r.app,
        call(
            "POST",
            &format!("{BASE}/query/aql"),
            &r.professional,
            None,
            Some(&query),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let result: Value = serde_json::from_str(&text).unwrap();
    let mut by_person: Vec<&str> = result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row[1]["_type"] == "PARTY_SELF")
        .filter_map(|row| row[0].as_str())
        .collect();
    by_person.sort_unstable();
    assert_eq!(by_person, vec![persons_entry.as_str()], "{text}");

    // The contribution list says which change set the person inserted.
    let (status, text) = common::send_body(
        &r.app,
        call(
            "GET",
            &format!("{EXT}/ehr/{}/contribution", r.ehr),
            &r.professional,
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let list: Value = serde_json::from_str(&text).unwrap();
    let flags: Vec<bool> = list["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["inserted_by_person"].as_bool().expect("a flag"))
        .collect();
    assert_eq!(flags.iter().filter(|person| **person).count(), 1, "{text}");
}

/// A person may amend and delete what they inserted.
#[tokio::test]
async fn a_person_amends_and_deletes_their_own_entry() {
    let r = record().await;
    let first = create(&r.app, &r.ehr, &r.person, &composition(&party_self())).await;
    let (status, headers, text) = update(
        &r.app,
        &r.ehr,
        &r.person,
        &first,
        &composition(&party_self()),
    )
    .await;
    assert!(
        status.is_success(),
        "the person's own amendment: {status} {text}"
    );
    let second = common::etag_uid(&headers);
    let amended = committer(&r.app, &r.ehr, &r.person, &second).await;
    assert_eq!(amended["_type"], "PARTY_SELF", "{amended}");
    let (status, text) = delete(&r.app, &r.ehr, &r.person, &second).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the person's own deletion: {text}"
    );
}

/// A person is refused (`403`) an update or a deletion of a professional's
/// version, and the version stays as the professional committed it.
#[tokio::test]
async fn a_person_may_not_alter_a_professionals_entry() {
    let r = record().await;
    let (status, _headers, text) = update(
        &r.app,
        &r.ehr,
        &r.person,
        &r.professionals_entry,
        &composition(&party_self()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        text.contains("EHDS Art. 5"),
        "the refusal names its reason: {text}"
    );
    let (status, text) = delete(&r.app, &r.ehr, &r.person, &r.professionals_entry).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        text.contains("EHDS Art. 5"),
        "the refusal names its reason: {text}"
    );
    let kept = committer(&r.app, &r.ehr, &r.professional, &r.professionals_entry).await;
    assert_eq!(kept["name"], "dr-bob", "{kept}");
}

/// A person may not present what they insert as another party's: a composer
/// other than `PARTY_SELF` or `PARTY_RELATED` is refused (`403`).
#[tokio::test]
async fn a_person_names_themselves_or_their_representative_as_composer() {
    let r = record().await;
    let (status, text) = common::send_body(
        &r.app,
        call(
            "POST",
            &format!("{BASE}/ehr/{}/composition", r.ehr),
            &r.person,
            None,
            Some(&composition(&professional_composer())),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(text.contains("EHDS Art. 5"), "{text}");
    let representative = json!({
        "_type": "PARTY_RELATED",
        "name": "Bea",
        "relationship": {
            "_type": "DV_CODED_TEXT",
            "value": "mother",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "10"
            }
        }
    });
    create(&r.app, &r.ehr, &r.person, &composition(&representative)).await;
}

/// A professional's writes are unchanged: they may update an entry the person
/// inserted, and the version they commit is then theirs, which the person may
/// no longer alter.
#[tokio::test]
async fn a_professional_updates_a_persons_entry_which_the_person_then_may_not_alter() {
    let r = record().await;
    let persons_entry = create(&r.app, &r.ehr, &r.person, &composition(&party_self())).await;
    let (status, headers, text) = update(
        &r.app,
        &r.ehr,
        &r.professional,
        &persons_entry,
        &composition(&party_self()),
    )
    .await;
    assert!(
        status.is_success(),
        "the professional's update: {status} {text}"
    );
    let reviewed = common::etag_uid(&headers);
    let audit = committer(&r.app, &r.ehr, &r.professional, &reviewed).await;
    assert_eq!(audit["name"], "dr-bob", "{audit}");
    let (status, _headers, text) = update(
        &r.app,
        &r.ehr,
        &r.person,
        &reviewed,
        &composition(&party_self()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
}
