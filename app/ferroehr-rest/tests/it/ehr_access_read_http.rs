// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR_ACCESS` reads at the wire (#5517): FerroEHR's own extension,
//! shaped on the ITS-REST `EHR_STATUS` and `VERSIONED_EHR_STATUS` reads,
//! since ITS-REST 1.1.0 publishes no `EHR_ACCESS` operation.
//!
//! A CONTRIBUTION writes an `EHR_ACCESS` version (RM ehr
//! `master04-ehr_package.adoc` §EHR Access) and the reads return it
//! unchanged, under the access evaluator, each read recorded in the access
//! log. A CONTRIBUTION whose directive names a party the demographic store
//! does not hold is refused with `422`, one case per kind of reference. The
//! scheme and the check are our own design.

use axum::Router;
use axum::body::Body;
use ferroehr::service::{DEFAULT_SYSTEM_ID, FerroEhrService};
use http::{HeaderMap, Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::access_evaluator_http::{app, credential, directive, naming, refusal, send};
use crate::common;
use crate::common::{BASE, EXT};

/// A UUID no party of the store carries.
const NOBODY: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f99";

/// An EHR created by the administrator.
async fn new_ehr(app: &Router) -> String {
    let (status, body) = send(app, "POST", &format!("{BASE}/ehr"), "root", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned()
}

/// Settings, open where no directive applies, whose gate-keepers are the
/// `PERSON`s of the Basic users `keepers` and which hold `directives`.
async fn settings_of(pool: &PgPool, keepers: &[&str], directives: &[Value]) -> Value {
    let mut persons = Vec::new();
    for user in keepers {
        persons.push(common::basic_person(pool, user).await);
    }
    let mut settings = common::access_settings("open", &persons);
    if !directives.is_empty() {
        settings["directives"] = Value::Array(directives.to_vec());
    }
    settings
}

/// Commits, through the controller's service call, settings naming the Basic
/// users `keepers` the gate-keepers of `ehr`, as its second `EHR_ACCESS`
/// version: only a gate-keeper or an administrator commits the next one, and
/// reads it.
async fn keep(
    svc: &FerroEhrService,
    pool: &PgPool,
    ehr: &str,
    keepers: &[&str],
    directives: &[Value],
) {
    common::commit_access_settings(
        svc,
        ehr.parse().expect("an EHR id"),
        settings_of(pool, keepers, directives).await,
    )
    .await;
}

/// `root`'s CONTRIBUTION of `settings` as the third `EHR_ACCESS` version of
/// `ehr`, over the version [`keep`] committed.
async fn contribute(app: &Router, ehr: &str, settings: &Value) -> (StatusCode, String) {
    let (status, body) = send(app, "GET", &format!("{BASE}/ehr/{ehr}"), "root", None, None).await;
    assert_eq!(status, StatusCode::OK, "EHR read: {body}");
    let ehr_object: Value = serde_json::from_str(&body).expect("EHR json");
    let contribution =
        crate::ehr_access_gate::access_contribution_over(&ehr_object, settings, 2, "root");
    send(
        app,
        "POST",
        &format!("{BASE}/ehr/{ehr}/contribution"),
        "root",
        None,
        Some(("application/json", contribution.to_string())),
    )
    .await
}

/// `GET uri` as `user` asking for `accept`: the status, headers and body.
async fn read(
    app: &Router,
    uri: &str,
    user: &str,
    accept: &str,
) -> (StatusCode, HeaderMap, String) {
    common::send(
        app,
        Request::get(uri)
            .header(header::AUTHORIZATION, credential(user))
            .header(header::ACCEPT, accept)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

/// [`read`] of canonical JSON, the body parsed.
async fn read_json(app: &Router, uri: &str, user: &str) -> (StatusCode, HeaderMap, Value) {
    let (status, headers, body) = read(app, uri, user, "application/json").await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    (status, headers, serde_json::from_str(&body).expect("JSON"))
}

/// The weak `ETag` of `headers`, unquoted.
fn etag(headers: &HeaderMap) -> String {
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("an ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

/// A `PARTY_REF` of `rm_type` naming `id` as a `HIER_OBJECT_ID`.
fn party_ref(rm_type: &str, id: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": rm_type,
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// A consent permit directive with `extra` members.
fn permit(extra: &Value) -> Value {
    directive("consent", "permit", extra)
}

/// A CONTRIBUTION writes directives naming parties of the store, and every
/// `EHR_ACCESS` read returns them unchanged, with the version headers of the
/// `EHR_STATUS` reads. A read that carries the settings is canonical JSON
/// only, since no canonical-XML class maps them.
#[tokio::test]
async fn the_directives_round_trip_through_the_reads() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let ehr = new_ehr(&app).await;
    keep(&svc, &pool, &ehr, &["root"], &[]).await;
    let nurse = common::basic_person(&pool, "nurse").await;
    let psychiatrist = common::basic_person(&pool, "psychiatrist").await;
    let settings = settings_of(
        &pool,
        &["root"],
        &[permit(&json!({
            "applies_to": naming(nurse),
            "instructed_by": { "_type": "PARTY_IDENTIFIED",
                               "external_ref": party_ref("PERSON", &psychiatrist.to_string()) },
        }))],
    )
    .await;
    let (status, body) = contribute(&app, &ehr, &settings).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let current = format!("{EXT}/ehr/{ehr}/ehr_access");
    let (_, headers, access) = read_json(&app, &current, "root").await;
    assert_eq!(access["_type"], json!("EHR_ACCESS"), "{access}");
    assert_eq!(
        access["settings"], settings,
        "the settings come back unchanged"
    );
    let uid = access["uid"]["value"].as_str().expect("a uid").to_owned();
    assert!(uid.ends_with("::3"), "{uid}");
    assert_eq!(etag(&headers), uid);
    assert!(headers.contains_key(header::LAST_MODIFIED));
    assert!(!headers.contains_key(header::LOCATION));

    let (_, _, by_version) = read_json(&app, &format!("{current}/{uid}"), "root").await;
    assert_eq!(by_version, access);
    let vo = uid.split("::").next().expect("object id").to_owned();
    let (_, _, first) = read_json(
        &app,
        &format!("{current}/{vo}::{DEFAULT_SYSTEM_ID}::1"),
        "root",
    )
    .await;
    assert!(
        first.get("settings").is_none(),
        "the default carries none: {first}"
    );

    let versioned = format!("{EXT}/ehr/{ehr}/versioned_ehr_access");
    let (_, headers, object) = read_json(&app, &versioned, "root").await;
    assert_eq!(object["_type"], json!("VERSIONED_EHR_ACCESS"), "{object}");
    assert_eq!(object["uid"]["value"], json!(vo));
    assert_eq!(etag(&headers), vo);
    let (_, _, history) = read_json(&app, &format!("{versioned}/revision_history"), "root").await;
    assert_eq!(
        history["items"].as_array().map(Vec::len),
        Some(3),
        "{history}"
    );
    let (_, headers, version) = read_json(&app, &format!("{versioned}/version"), "root").await;
    assert_eq!(version["_type"], json!("ORIGINAL_VERSION"), "{version}");
    assert_eq!(version["data"]["settings"], settings);
    assert_eq!(etag(&headers), uid);
    let (_, _, by_id) = read_json(&app, &format!("{versioned}/version/{uid}"), "root").await;
    assert_eq!(by_id, version);

    for uri in [current.clone(), format!("{versioned}/version")] {
        let (status, _, body) = read(&app, &uri, "root", "application/xml").await;
        assert_eq!(
            status,
            StatusCode::NOT_ACCEPTABLE,
            "no XML class maps the settings: {body}"
        );
    }
    let (status, headers, xml) = read(&app, &versioned, "root", "application/xml").await;
    assert_eq!(status, StatusCode::OK, "{xml}");
    assert_eq!(etag(&headers), vo);
    let (status, _, body) = read(&app, &current, "root", "application/openehr.wt.flat+json").await;
    assert_eq!(status, StatusCode::NOT_ACCEPTABLE, "{body}");
}

/// An EHR with no such version or no such EHR answers `404`.
#[tokio::test]
async fn an_absent_target_is_404() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let ehr = new_ehr(&app).await;
    keep(&svc, &pool, &ehr, &["root"], &[]).await;
    for uri in [
        format!("{EXT}/ehr/{NOBODY}/ehr_access"),
        format!("{EXT}/ehr/{NOBODY}/versioned_ehr_access"),
        format!("{EXT}/ehr/{ehr}/ehr_access?version_at_time=2000-01-01T00:00:00Z"),
        format!("{EXT}/ehr/{ehr}/ehr_access/{NOBODY}::{DEFAULT_SYSTEM_ID}::1"),
    ] {
        let (status, _, body) = read(&app, &uri, "root", "application/json").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }
}

/// The access evaluator guards the reads: a consent deny naming Dr B refuses
/// him the directives with the consent code. Of the callers it admits, the
/// gate-keeper reads them and the psychiatrist, who may not read the
/// settings, finds none, as on the CONTRIBUTION and extract reads.
#[tokio::test]
async fn a_caller_the_evaluator_refuses_reads_no_directives() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let ehr = new_ehr(&app).await;
    let drb = common::issuer_person(&pool, "https://medics.example", "drb").await;
    let deny = directive("consent", "deny", &json!({ "applies_to": naming(drb) }));
    keep(&svc, &pool, &ehr, &["nurse"], &[deny]).await;
    for path in [
        "ehr_access",
        "versioned_ehr_access",
        "versioned_ehr_access/revision_history",
        "versioned_ehr_access/version",
    ] {
        let uri = format!("{EXT}/ehr/{ehr}/{path}");
        let (status, body) = send(&app, "GET", &uri, "drb", Some("TREAT"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
        assert_eq!(refusal(&body).0, json!("refused-consent-denied"), "{body}");
        let (status, body) = send(&app, "GET", &uri, "nurse", None, None).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        let (status, body) = send(&app, "GET", &uri, "psychiatrist", None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert!(!body.contains("refused-"), "{body}");
    }
}

/// Each read of the directives writes its access-log entry, naming the
/// operation and the EHR.
#[tokio::test]
async fn a_read_of_the_directives_is_recorded() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let ehr = new_ehr(&app).await;
    keep(&svc, &pool, &ehr, &["nurse"], &[]).await;
    let (status, _, body) = read(
        &app,
        &format!("{EXT}/ehr/{ehr}/ehr_access"),
        "nurse",
        "application/json",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = crate::access_log_reads_http::stored(&pool, 1, |entry| {
        entry["operation"]["value"] == json!("ehr_access_get_at_time")
    })
    .await;
    assert_eq!(entries[0]["ehr_id"]["value"], json!(ehr), "{}", entries[0]);
    assert_eq!(
        entries[0]["action"]["value"],
        json!("read"),
        "{}",
        entries[0]
    );
}

/// A directive naming a party the demographic store does not hold is refused
/// with `422` naming the directive and the party, for each kind of
/// reference: an actor scope, the instructing party as `PARTY_IDENTIFIED` and
/// as `PARTY_RELATED`, a reference in an exception, a reference whose type
/// does not match the party, and an id that is no UUID. Nothing is
/// committed.
#[tokio::test]
async fn a_directive_naming_an_unknown_party_is_refused() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let ehr = new_ehr(&app).await;
    keep(&svc, &pool, &ehr, &["root"], &[]).await;
    let nurse = common::basic_person(&pool, "nurse").await.to_string();
    let guardian = json!({ "_type": "DV_CODED_TEXT", "value": "guardian",
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": "10" } });
    let cases = [
        (
            "applies_to[0].party",
            NOBODY.to_owned(),
            permit(&json!({ "applies_to": [{ "party": party_ref("PERSON", NOBODY) }] })),
        ),
        (
            "instructed_by.external_ref",
            NOBODY.to_owned(),
            permit(&json!({ "instructed_by": { "_type": "PARTY_IDENTIFIED",
                "external_ref": party_ref("PERSON", NOBODY) } })),
        ),
        (
            "instructed_by.external_ref",
            NOBODY.to_owned(),
            permit(&json!({ "instructed_by": { "_type": "PARTY_RELATED",
                "relationship": guardian, "external_ref": party_ref("PERSON", NOBODY) } })),
        ),
        (
            "directives[0].exceptions[0]",
            NOBODY.to_owned(),
            directive(
                "consent",
                "deny",
                &json!({ "exceptions": [permit(&json!({
                    "applies_to": [{ "party": party_ref("PERSON", NOBODY) }] }))] }),
            ),
        ),
        (
            "applies_to[0].party",
            nurse.clone(),
            permit(&json!({ "applies_to": [{ "party": party_ref("ORGANISATION", &nurse) }] })),
        ),
        (
            "applies_to[0].party",
            "nurse-17".to_owned(),
            permit(&json!({ "applies_to": [{ "party": { "_type": "PARTY_REF",
                "namespace": "demographic", "type": "PERSON",
                "id": { "_type": "GENERIC_ID", "value": "nurse-17", "scheme": "local" } } }] })),
        ),
    ];
    for (place, party, refused) in cases {
        let settings = settings_of(&pool, &["root"], &[refused]).await;
        let (status, body) = contribute(&app, &ehr, &settings).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{place}: {body}");
        assert!(body.contains(place), "{place} named: {body}");
        assert!(body.contains(&party), "{party} named: {body}");
    }
    let (_, _, history) = read_json(
        &app,
        &format!("{EXT}/ehr/{ehr}/versioned_ehr_access/revision_history"),
        "root",
    )
    .await;
    assert_eq!(
        history["items"].as_array().map(Vec::len),
        Some(2),
        "{history}"
    );

    let by_self = permit(
        &json!({ "applies_to": naming(nurse.parse().expect("a UUID")),
        "instructed_by": { "_type": "PARTY_SELF" } }),
    );
    let settings = settings_of(&pool, &["root"], &[by_self]).await;
    let (status, body) = contribute(&app, &ehr, &settings).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the subject needs no lookup: {body}"
    );
}
