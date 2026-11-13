// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR access-log entry of every management route kind that touches
//! an EHR (#5564), end to end over the assembled router, the audit sender and
//! the local store.
//!
//! The kinds are the physical EHR delete, one EHR and in bulk; the EHR mark
//! registers (restriction, access restriction, research objection, and the
//! opt-out in `crate::access_opt_out_http`, which grants the access service);
//! the archival tier; the retention register; the repository dump and load;
//! and the EHR Extract export and import. Each writes one entry per EHR it
//! touched through the one recording path. SCIM touches party data only and
//! writes none. No openEHR spec governs the access log: our own design on the
//! draft `org.openehr.rm.access_log` 0.1.0.

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::json;
use sqlx::PgPool;

use crate::access_log_http::{SUBJECT, Stored, as_root, commit, create_ehr, entries};
use crate::common;

/// Send `method` to the route `path` (relative to its namespace) as the ADMIN
/// `root`, with `body` as JSON when there is one; returns the answer.
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    body: Option<String>,
) -> (StatusCode, String) {
    let request = as_root(Request::builder().method(method).uri(common::full(path)));
    let request = match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body)),
        None => request.body(Body::empty()),
    };
    common::send_body(app, request.expect("the request")).await
}

/// The entries for `ehr` whose action is `action`, polled until there is one.
async fn of(pool: &PgPool, ehr: &str, action: &str) -> Vec<Stored> {
    entries(pool, 1, |s| {
        s.ehr_id.as_deref() == Some(ehr) && s.action.as_deref() == Some(action)
    })
    .await
}

/// The entry for `ehr` whose item names the object `object`, polled until it
/// lands; asserts its action and that it was granted.
async fn names(pool: &PgPool, ehr: &str, object: &str, action: &str) {
    let quoted = format!("\"{object}\"");
    let found = entries(pool, 1, |s| {
        s.ehr_id.as_deref() == Some(ehr) && s.content.contains(&quoted)
    })
    .await;
    assert_eq!(
        found[0].action.as_deref(),
        Some(action),
        "{object}: {found:?}"
    );
    assert_eq!(
        found[0].result.as_deref(),
        Some("granted"),
        "{object}: {found:?}"
    );
}

/// The physical delete of one EHR writes a `delete` entry for it, and the
/// bulk delete one for each EHR it removed.
#[tokio::test]
async fn an_ehr_delete_writes_a_delete_entry_per_ehr() {
    let (_db, pool, app, ehr) = common::audited_app(SUBJECT, "root").await;
    let (status, body) = send(&app, "DELETE", &format!("/admin/ehr/{ehr}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete: {body}");
    assert_eq!(
        of(&pool, &ehr, "delete").await[0].result.as_deref(),
        Some("granted")
    );

    let bulk = [create_ehr(&app).await, create_ehr(&app).await];
    let (status, body) = send(
        &app,
        "DELETE",
        &format!("/admin/ehr/all?ehr_id={},{}", bulk[0], bulk[1]),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "bulk delete: {body}");
    for removed in &bulk {
        let deleted = of(&pool, removed, "delete").await;
        assert_eq!(deleted.len(), 1, "one entry per removed EHR: {deleted:?}");
    }
}

/// Each EHR mark register writes an entry for the EHR it marks, naming the
/// register.
#[tokio::test]
async fn each_mark_register_writes_an_entry_for_its_ehr() {
    let (_db, pool, app, ehr) = common::audited_app(SUBJECT, "root").await;
    let marks = [
        (
            "/admin/restriction",
            json!({ "ehr_id": ehr, "ground": "national" }),
            "restriction",
        ),
        (
            "/admin/access-restriction",
            json!({ "ehr_id": ehr }),
            "access-restriction",
        ),
        (
            "/admin/research-objection",
            json!({ "ehr_id": ehr, "objected": true }),
            "research-objection",
        ),
    ];
    for (path, body, register) in marks {
        let (status, answer) = send(&app, "POST", path, Some(body.to_string())).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{path}: {answer}");
        names(&pool, &ehr, &format!("{register}:ehr-wide"), "create").await;
    }
}

/// Moving an EHR to the archival tier and back writes an `update` entry for
/// each move.
#[tokio::test]
async fn archiving_and_restoring_an_ehr_writes_an_entry_for_each_move() {
    let (_db, pool, app, ehr) = common::audited_app(SUBJECT, "root").await;
    commit(&app, &ehr).await;
    for (path, register) in [
        ("/admin/archive/ehrs", "archive"),
        ("/admin/archive/ehrs/restore", "archive-restore"),
    ] {
        let body = json!({ "ehr_ids": [ehr] }).to_string();
        let (status, answer) = send(&app, "POST", path, Some(body)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{path}: {answer}");
        names(&pool, &ehr, &format!("{register}:ehr-wide"), "update").await;
    }
}

/// The retention anchor of an EHR and the hold on one of its objects each
/// write an entry for that EHR.
#[tokio::test]
async fn the_retention_register_writes_an_entry_for_the_ehr() {
    let (_db, pool, app, ehr) = common::audited_app(SUBJECT, "root").await;
    let policy = json!({ "kind": "EHR", "jurisdiction": "CH", "period": "20 years",
        "anchor": "last_commit", "source": "EPDV Art. 10 Abs. 1 lit. d" });
    let (status, answer) = send(
        &app,
        "PUT",
        "/admin/retention/policy",
        Some(policy.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "policy: {answer}");
    let anchor = json!({ "ehr_id": ehr, "jurisdiction": "CH" });
    let (status, answer) = send(
        &app,
        "PUT",
        "/admin/retention/anchor",
        Some(anchor.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "anchor: {answer}");
    names(&pool, &ehr, "retention-anchor:ehr-wide", "update").await;

    let uid = commit(&app, &ehr).await;
    let vo = uid.split("::").next().expect("a versioned object id");
    let hold = json!({ "vo_id": vo, "held": true });
    let (status, answer) = send(
        &app,
        "POST",
        "/admin/retention/hold",
        Some(hold.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "hold: {answer}");
    names(&pool, &ehr, &format!("retention-hold:{vo}"), "create").await;
}

/// The repository dump writes an `export` entry for each EHR it wrote, and the
/// load an `import` entry for each EHR it loaded.
#[tokio::test]
async fn a_dump_and_a_load_write_an_entry_per_ehr() {
    let (_db, pool, app, ehr) = common::audited_app(SUBJECT, "root").await;
    let dir = std::env::temp_dir()
        .join(format!("ferroehr-access-log-dump-{}", uuid::Uuid::now_v7()))
        .to_string_lossy()
        .into_owned();
    let spec = json!({ "file_sys_loc": dir, "segment_split_size": 1024 });
    let (status, answer) = send(&app, "POST", "/admin/dump", Some(spec.to_string())).await;
    assert_eq!(status, StatusCode::OK, "dump: {answer}");
    assert_eq!(of(&pool, &ehr, "export").await.len(), 1);

    // The EHR leaves the repository and the load brings it back.
    let (status, answer) = send(&app, "DELETE", &format!("/admin/ehr/{ehr}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete: {answer}");
    let load = json!({ "file_sys_loc": dir });
    let (status, answer) = send(&app, "POST", "/admin/load", Some(load.to_string())).await;
    assert_eq!(status, StatusCode::OK, "load: {answer}");
    assert!(!answer.contains(&ehr), "the dumped EHR loads: {answer}");
    assert_eq!(of(&pool, &ehr, "import").await.len(), 1);
}

/// The EHR Extract import writes an `import` entry for the EHR it created.
#[tokio::test]
async fn an_extract_import_writes_an_import_entry() {
    let (_db, pool, app, _ehr) = common::audited_app(SUBJECT, "root").await;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/messaging/ehr_extract.v1.json");
    let extract = std::fs::read_to_string(&path).expect("the extract fixture");
    let (status, answer) = send(&app, "POST", "/message/import", Some(extract)).await;
    assert_eq!(status, StatusCode::CREATED, "import: {answer}");
    let imported = of(&pool, "9d1e6a7c-2f04-4c1b-9a41-0d5c7f2b8e10", "import").await;
    assert_eq!(imported[0].result.as_deref(), Some("granted"));
}
