// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The sandbox seed's archetype-library walk against the upload it drives.
//!
//! `scripts/sandbox/reseed.sh` offers the whole vendored 2013 CKM ADL 2 export
//! to `POST /definition/template/adl2` and counts each answer as stored or
//! refused against the pin in `scripts/sandbox/seed/manifest.json`. The status
//! it counts as a refusal is that pin's `refused_status`, so this test holds
//! the pin to the status the upload answers for a file the corpus gate
//! adjudicates as refused (`crates/openehr-adl/tests/it/ckm_archetype_packs.rs`).
//!
//! The walk's later steps post the seed bodies under `scripts/sandbox/seed/`
//! through the real router here too, each the way `reseed.sh` renders it, so a
//! body the CDR refuses fails this suite before it stops a sandbox reseed.
use std::path::{Path, PathBuf};

use axum::Router;
use http::{StatusCode, header};
use serde_json::Value;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// A library file the AOM2 validator refuses on VCARM: it constrains
/// `property` on `DV_QUANTITY`, which RM 1.2.0 does not define.
const REFUSED: &str = "openEHR-EHR-CLUSTER.ambient_oxygen.v1.0.0.adls";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The sandbox seed manifest.
fn manifest() -> Value {
    serde_json::from_str(&seed_file("manifest.json")).expect("the sandbox seed manifest is JSON")
}

/// The manifest's archetype-library pin.
fn library_pin() -> Value {
    manifest()["adl2"]["archetype_library"].clone()
}

/// A file under `scripts/sandbox/seed/`, named as the manifest names it.
fn seed_file(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join("scripts/sandbox/seed").join(relative))
        .unwrap_or_else(|e| panic!("read the seed file {relative}: {e}"))
}

/// The seed body `relative` with each placeholder replaced, as `reseed.sh`
/// renders it.
fn render(relative: &str, substitutions: &[(&str, &str)]) -> String {
    substitutions
        .iter()
        .fold(seed_file(relative), |body, (from, to)| {
            body.replace(from, to)
        })
}

/// The manifest string at `pointer`.
fn manifest_str<'a>(manifest: &'a Value, pointer: &str) -> &'a str {
    manifest
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("the manifest names {pointer}"))
}

/// Sends a JSON `body` to `uri` and returns the status, the object id the
/// `ETag` names, and the response body.
async fn send_seed(
    app: &Router,
    method: &str,
    uri: String,
    body: String,
    if_match: Option<&str>,
) -> (StatusCode, Option<String>, String) {
    let mut request = common::req(method, uri).json(body);
    if let Some(version) = if_match {
        request = request.if_match(format!("\"{version}\""));
    }
    let (status, headers, body) = common::send(app, request.build()).await;
    let object_id = common::header(&headers, header::ETAG).map(|_| {
        let uid = common::etag_uid(&headers);
        uid.split("::").next().expect("an ETag uid").to_owned()
    });
    (status, object_id, body)
}

/// Creates the EHR of `subject` from the seed status body `status`, as step 5
/// of `reseed.sh` does, and returns its id.
async fn seeded_ehr(app: &Router, manifest: &Value, subject: &str, status: &str) -> String {
    let namespace = manifest_str(manifest, "/subject_namespace");
    let body = render(
        status,
        &[("__SUBJECT_ID__", subject), ("__SUBJECT_NS__", namespace)],
    );
    let (code, ehr, body) = send_seed(app, "POST", format!("{BASE}/ehr"), body, None).await;
    assert_eq!(code, StatusCode::CREATED, "ehr create {subject}: {body}");
    ehr.expect("the EHR create answers an ETag")
}

/// The upload declares only 201, 400 and 409
/// (`operations/definition_template_adl2_upload.yaml`), so a library archetype
/// AOM2 refuses is a `400`, and the seed counts exactly that status as a
/// refusal (#5044: it counted `422`, and the v5.6.3 reseed stopped at the
/// first refused file).
#[tokio::test]
async fn the_seed_counts_a_refused_library_archetype_on_the_status_the_upload_answers() {
    let pin = library_pin();
    let root = pin["root"]
        .as_str()
        .expect("the manifest names the library root");
    let source = std::fs::read_to_string(repo_root().join(root).join("cluster").join(REFUSED))
        .expect("read the refused library archetype");

    let (_pg, app) = common::app(common::api_config(false)).await;
    let req = common::req("POST", format!("{BASE}/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain")
        .prefer_representation()
        .body(source)
        .build();
    let (status, problem) = common::send_json(&app, req).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    let errors = problem
        .get("errors")
        .and_then(Value::as_array)
        .expect("the problem carries its violations in errors");
    assert!(
        errors.iter().any(|e| e["path"].as_str() == Some("VCARM")),
        "{REFUSED} is refused on VCARM: {errors:?}"
    );
    assert_eq!(
        pin["refused_status"].as_u64(),
        Some(u64::from(status.as_u16())),
        "the manifest's refused_status, which reseed.sh counts as a library refusal, \
         is the status the upload answers for a refused archetype: {problem}"
    );
}

/// Step 7 of the walk: the directory every seeded EHR receives, over the first
/// three compositions committed into it, is created (#5053: the v5.6.3 reseed
/// stopped here with a `422`, every FOLDER naming an archetype in
/// `archetype_node_id` and none carrying `archetype_details`, which
/// `locatable.adoc` `Archetyped_valid` makes non-Void at an archetype root).
#[tokio::test]
async fn the_seed_directory_is_created_in_a_seeded_ehr() {
    let manifest = manifest();
    let (_pg, app) = common::app(common::api_config(false)).await;
    let upload = common::req("POST", format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(common::ips_opt_xml())
        .build();
    let (status, body) = common::send_body(&app, upload).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");

    let ehr = seeded_ehr(
        &app,
        &manifest,
        manifest_str(&manifest, "/ehrs/0/subject"),
        manifest_str(&manifest, "/ehrs/0/status"),
    )
    .await;
    let mut compositions = Vec::new();
    for _ in 0..3 {
        let (status, id, body) = send_seed(
            &app,
            "POST",
            format!("{BASE}/ehr/{ehr}/composition"),
            common::ips_canonical_composition().to_string(),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "composition commit: {body}");
        compositions.push(id.expect("the commit answers an ETag"));
    }

    let folder = render(
        manifest_str(&manifest, "/directory/folder"),
        &[
            ("__COMPOSITION_1__", &compositions[0]),
            ("__COMPOSITION_2__", &compositions[1]),
            ("__COMPOSITION_3__", &compositions[2]),
        ],
    );
    let (status, _id, body) = send_seed(
        &app,
        "POST",
        format!("{BASE}/ehr/{ehr}/directory"),
        folder,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "directory create: {body}");
}

/// Step 8 of the walk: each EHR the manifest gives a `final_status` takes it
/// as the second `EHR_STATUS` version.
#[tokio::test]
async fn the_seed_final_ehr_statuses_are_accepted() {
    let manifest = manifest();
    let namespace = manifest_str(&manifest, "/subject_namespace");
    let (_pg, app) = common::app(common::api_config(false)).await;
    let ehrs = manifest["ehrs"]
        .as_array()
        .expect("the manifest lists EHRs");
    let finals: Vec<&Value> = ehrs
        .iter()
        .filter(|e| e.get("final_status").is_some())
        .collect();
    assert!(
        !finals.is_empty(),
        "the manifest gives some EHR a final status"
    );
    for entry in finals {
        let subject = manifest_str(entry, "/subject");
        let ehr = seeded_ehr(&app, &manifest, subject, manifest_str(entry, "/status")).await;
        let read = common::req("GET", format!("{BASE}/ehr/{ehr}/ehr_status")).build();
        let (status, headers, body) = common::send(&app, read).await;
        assert_eq!(status, StatusCode::OK, "ehr_status read {subject}: {body}");
        let current = common::etag_uid(&headers);
        let final_status = render(
            manifest_str(entry, "/final_status"),
            &[("__SUBJECT_ID__", subject), ("__SUBJECT_NS__", namespace)],
        );
        let (status, _id, body) = send_seed(
            &app,
            "PUT",
            format!("{BASE}/ehr/{ehr}/ehr_status"),
            final_status,
            Some(&current),
        )
        .await;
        assert!(
            matches!(status, StatusCode::OK | StatusCode::NO_CONTENT),
            "ehr_status update {subject}: {status} {body}"
        );
    }
}

/// Step 9 of the walk: every seeded person, role and party relationship is
/// created, the roles and relationships naming the persons created before
/// them.
#[tokio::test]
async fn the_seed_demographic_parties_are_created() {
    let manifest = manifest();
    let demographics = &manifest["demographics"];
    let (_pg, app) = common::app(common::api_config(false)).await;

    let mut persons = Vec::new();
    for file in demographics["persons"].as_array().expect("persons") {
        let file = file.as_str().expect("a person file");
        let (status, id, body) = send_seed(
            &app,
            "POST",
            format!("{BASE}/demographic/person"),
            seed_file(file),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "person {file}: {body}");
        persons.push(id.expect("the person create answers an ETag"));
    }
    let person = |entry: &Value, key: &str| -> String {
        let index = entry[key].as_u64().expect("a person index");
        persons[usize::try_from(index).expect("an index")].clone()
    };

    for entry in demographics["roles"].as_array().expect("roles") {
        let file = manifest_str(entry, "/file");
        let body = render(file, &[("__PERFORMER__", &person(entry, "performer"))]);
        let (status, _id, body) =
            send_seed(&app, "POST", format!("{BASE}/demographic/role"), body, None).await;
        assert_eq!(status, StatusCode::CREATED, "role {file}: {body}");
    }

    for entry in demographics["relationships"]
        .as_array()
        .expect("relationships")
    {
        let file = manifest_str(entry, "/file");
        let body = render(
            file,
            &[
                ("__SOURCE__", &person(entry, "source")),
                ("__TARGET__", &person(entry, "target")),
            ],
        );
        let (status, _id, body) = send_seed(
            &app,
            "POST",
            format!("{}/demographic/party_relationship", common::EXT),
            body,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "relationship {file}: {body}");
    }
}

/// Step 10 of the walk: every seeded AQL query is stored under its name and
/// version.
#[tokio::test]
async fn the_seed_stored_queries_are_stored() {
    let manifest = manifest();
    let (_pg, app) = common::app(common::api_config(false)).await;
    for entry in manifest["stored_queries"]
        .as_array()
        .expect("stored queries")
    {
        let name = manifest_str(entry, "/name");
        let version = manifest_str(entry, "/version");
        let store = common::req("PUT", format!("{BASE}/definition/query/{name}/{version}"))
            .header(header::CONTENT_TYPE, "text/plain")
            .body(seed_file(manifest_str(entry, "/file")))
            .build();
        let (status, body) = common::send_body(&app, store).await;
        assert!(
            matches!(status, StatusCode::OK | StatusCode::CREATED),
            "query {name}: {status} {body}"
        );
    }
}
