// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The node side of the openEHR Federation Tier with AQL 1.0.0 that this
//! repository can prove on its own: a record moved between two nodes keeps its
//! identity, so a gateway can route a follow-up read or write by the
//! `creating_system_id` inside each `OBJECT_VERSION_ID` (Federation Tier N21,
//! N22, N36).
//!
//! RM common `docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`
//! §The Copy Operation: the copied `ORIGINAL_VERSION` "remains a faithful copy of
//! its original, no matter how many systems it may be copied through"; RM ehr
//! master04 §EHR Identifier Allocation for the receiving EHR's own id.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;

use crate::fixtures::{composition, folder, uv};

/// The `system_id` of the node a record is created on.
const NODE_A: &str = "node-a.federation.example";
/// The `system_id` of the node it moves to.
const NODE_B: &str = "node-b.federation.example";

/// A node: a service over its own testkit database, under its own `system_id`.
async fn node(system_id: &str) -> (testkit::TestDb, FerroEhrService) {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_system_id(system_id);
    (db, svc)
}

/// An `EXTRACT_SPEC` asking for every version of every versioned object of
/// `ehr` (RM `ehr_extract` `extract_version_spec.adoc`).
fn all_versions(ehr: EhrId) -> Value {
    json!({
        "_type": "EXTRACT_SPEC",
        "version_spec": {
            "_type": "EXTRACT_VERSION_SPEC",
            "include_all_versions": true,
            "include_revision_history": false,
            "include_data": true
        },
        "manifest": {
            "_type": "EXTRACT_MANIFEST",
            "entities": [ {
                "_type": "EXTRACT_ENTITY_MANIFEST",
                "extract_id_key": ehr.to_string(),
                "ehr_id": ehr.to_string(),
                "other_ids": [],
                "item_list": []
            } ]
        },
        "extract_type": {
            "_type": "DV_CODED_TEXT",
            "value": "openehr-ehr",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "openehr-ehr"
            }
        },
        "include_multimedia": true,
        "priority": 0,
        "link_depth": 0
    })
}

/// Export every version of `ehr` from `node`.
async fn export_all(node: &FerroEhrService, ehr: EhrId) -> Value {
    node.export_ehr_extracts(
        openehr_its::json::from_canonical_value(&all_versions(ehr)).expect("EXTRACT_SPEC"),
    )
    .await
    .expect("export every version")
    .into_iter()
    .next()
    .expect("one extract")
}

/// Every version uid an export carries, across all its versioned objects, as
/// the `object_id::creating_system_id::version_tree_id` strings: an
/// `ORIGINAL_VERSION`'s own uid, and for an `IMPORTED_VERSION` the uid of the
/// original it wraps (RM common master06 §The Copy Operation).
fn version_uids(extract: &Value) -> BTreeSet<String> {
    extract["chapters"]
        .as_array()
        .expect("chapters")
        .iter()
        .flat_map(|chapter| chapter["items"].as_array().expect("chapter items").iter())
        .flat_map(|item| {
            item["item"]["versions"]
                .as_array()
                .expect("versions")
                .iter()
        })
        .map(|version| {
            let original = match version["_type"].as_str() {
                Some("ORIGINAL_VERSION") => version,
                Some("IMPORTED_VERSION") => &version["item"],
                other => panic!("an extract carries versions, not {other:?}"),
            };
            original["uid"]["value"].as_str().expect("uid").to_owned()
        })
        .collect()
}

/// The `creating_system_id` part of an `OBJECT_VERSION_ID` string.
fn creating_system_id(uid: &str) -> &str {
    uid.split("::").nth(1).expect("creating_system_id")
}

/// An EHR built on node A (its status, a directory, a COMPOSITION in
/// two versions), exported, imported on node B under an id node B chose, and
/// exported again from B: every `OBJECT_VERSION_ID` and its
/// `creating_system_id` arrive byte for byte, and none names node B.
#[tokio::test]
async fn a_moved_ehr_keeps_every_object_version_id_and_creating_system_id() {
    let (_db_a, node_a) = node(NODE_A).await;
    let (_db_b, node_b) = node(NODE_B).await;

    let ehr = node_a.create_ehr(None).await.expect("create the EHR on A");
    node_a
        .create_directory(ehr, uv(&folder("root"), "249", None))
        .await
        .expect("directory on A");
    let created = node_a
        .create_composition(ehr, uv(&composition("Encounter"), "249", None))
        .await
        .expect("composition on A");
    let first = format!(
        "{}::{}::{}",
        created.vo_id, created.creating_system_id, created.tree
    );
    let amended = node_a
        .update_composition(
            ehr,
            created.vo_id,
            uv(&composition("Encounter, amended"), "251", Some(&first)),
        )
        .await
        .expect("composition update on A");
    let second = format!(
        "{}::{}::{}",
        amended.vo_id, amended.creating_system_id, amended.tree
    );

    let on_a = export_all(&node_a, ehr).await;
    let sent = version_uids(&on_a);
    assert!(
        sent.contains(&first) && sent.contains(&second) && sent.len() >= 4,
        "the export carries the status, the directory and both composition versions: {sent:?}"
    );
    assert!(
        sent.iter().all(|uid| creating_system_id(uid) == NODE_A),
        "every version was created on A: {sent:?}"
    );

    // Federation Tier N42a: a node that ingests an EHR issues its own ehr_id
    // instead of adopting the foreign one.
    let local = EhrId(uuid::Uuid::now_v7());
    let extract = openehr_its::json::from_canonical_value(&on_a)
        .expect("the export deserializes into the typed RM model");
    let landed = node_b
        .import_ehr(Some(local), extract)
        .await
        .expect("import on B");
    assert_eq!(landed, local, "B holds the record under the id B chose");

    let on_b = export_all(&node_b, local).await;
    let received = version_uids(&on_b);
    assert_eq!(
        received, sent,
        "every OBJECT_VERSION_ID crosses the node boundary byte for byte"
    );
    assert!(
        received.iter().all(|uid| creating_system_id(uid) != NODE_B),
        "no version claims B as its creating system: {received:?}"
    );
}
