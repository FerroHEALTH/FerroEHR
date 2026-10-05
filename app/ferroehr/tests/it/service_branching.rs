// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Version-tree branching + merge provenance, end-to-end against a real
//! `PostgreSQL` 18 (shared testkit harness).
//!
//! Spec: RM common `master06-change_control_package.adoc` §The 'Virtual Version
//! Tree' and §Versioning Semantics → §Version Identification — its §Local
//! Versioning subsection ("To support branching, a further pair of numbers is
//! added") and its §Distributed Versioning subsection ("to require branching
//! version identifiers to be used when local modifications are made to versions
//! copied from elsewhere") — plus §Version Merging
//! (`ORIGINAL_VERSION.other_input_version_uids`). BASE `VERSION_TREE_ID`:
//! `trunk_version [ '.' branch_number '.' branch_version ]`.
//!
//! Covers:
//! 1. modifying an imported version created by ANOTHER system forks a branch
//!    (`t.1.1`, local `creating_system_id`) while the imported trunk version
//!    stays the container current;
//! 2. continuing one's own branch tip advances it (`t.1.2`) and supersedes
//!    the previous tip; every branch version stays addressable by its full
//!    `OBJECT_VERSION_ID`, with the true `preceding_version_uid` served;
//! 3. `other_input_version_uids` is preserved verbatim by the EHR-Extract
//!    import — the route that legitimately carries merge provenance — and
//!    served on the `ORIGINAL_VERSION` (the commit wire declares no merge
//!    shape, so a member carrying it is refused; that twin lives in
//!    `service_contribution`);
//! 4. an exported version tree containing branches re-imports whole
//!    (branch import is first-class).

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::fixtures::{change_type, committer, composition, uv};

/// The service's own system id (`DEFAULT_SYSTEM_ID`) — the local
/// `creating_system_id` every non-import write records.
const LOCAL: &str = "ferroehr.local";
/// The pretend foreign system a copied version tree originates from.
const FOREIGN: &str = "sysA.example.org";

/// A CONTRIBUTION body modifying one composition (`251|modification|`).
fn modify_contribution(data: &Value, preceding: &str) -> Value {
    json!({
        "versions": [{
            "data": data,
            "preceding_version_uid": { "value": preceding },
            // Required on every CONTRIBUTION member (SM master03 §Version
            // Update Semantics; ITS-REST UpdateVersion.yaml `required`).
            "lifecycle_state": { "terminology_id": "openehr", "code_string": "532" },
            "commit_audit": {
                "change_type": change_type("251", "modification"),
                "committer": committer("branching tester")
            }
        }],
        "audit": {
            "change_type": change_type("251", "modification"),
            "committer": committer("branching tester")
        }
    })
}

/// The `OBJECT_VERSION_ID` of the first version listed in a served CONTRIBUTION.
fn first_version_uid(contribution: &Value) -> String {
    contribution["versions"][0]["id"]["value"]
        .as_str()
        .expect("versions[0].id.value")
        .to_owned()
}

/// A whole-EHR, ALL-versions `EXTRACT_SPEC` (RM `ehr_extract` master04
/// `EXTRACT_VERSION_SPEC.include_all_versions`) — the full-tree copy the
/// latest-only `export_ehrs` deliberately is not (latest = the latest TRUNK
/// version, master06).
fn all_versions_spec(ehr: ferroehr::ids::EhrId) -> Value {
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
        "link_depth": 0,
        "criteria": []
    })
}

/// Seed an EHR holding one COMPOSITION at trunk v2, then export it and rewrite
/// every version identity to the FOREIGN system — producing the extract a copy
/// from another openEHR system would carry (RM common master06 §Copying: the
/// `creating_system_id` identifies the system of original creation).
async fn foreign_extract(svc: &FerroEhrService) -> (Value, String) {
    let source = svc.create_ehr(None).await.expect("create source ehr");
    let v1 = svc
        .create_composition(source, uv(&composition("v1"), "249", None))
        .await
        .expect("composition v1")
        .version_uid();
    let vo = v1.split("::").next().unwrap().to_owned();
    svc.update_composition(
        source,
        vo.parse().unwrap(),
        uv(&composition("v2"), "251", Some(&v1)),
    )
    .await
    .expect("composition v2");

    let mut extracts = svc.extract_ehrs(source).await.expect("export");
    let extract = extracts.remove(0);
    let rewritten: Value = serde_json::from_str(
        &serde_json::to_string(&extract)
            .unwrap()
            .replace(&format!("::{LOCAL}::"), &format!("::{FOREIGN}::")),
    )
    .unwrap();
    (rewritten, vo)
}

/// Import the foreign extract into a fresh EHR id on `svc`, returning
/// (target ehr id, composition vo id).
async fn import_foreign(
    svc: &FerroEhrService,
    extract: Value,
    vo: &str,
) -> (ferroehr::ids::EhrId, ferroehr::ids::VoId) {
    let target = ferroehr::ids::EhrId(Uuid::now_v7());
    svc.import_ehr(
        Some(target),
        openehr_its::json::from_canonical_value(&extract).expect("EXTRACT deserializes"),
    )
    .await
    .expect("import_ehr");
    (target, vo.parse().unwrap())
}

#[tokio::test]
async fn modifying_an_imported_foreign_version_forks_a_branch() {
    // The source and the importing target are separate repositories (a copied
    // versioned object keeps its vo_id, so importing into the SAME repository
    // that already owns it is — correctly — a conflict).
    let source_db = testkit::db().await.expect("testkit database");
    let source_svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (extract, vo) = foreign_extract(&source_svc).await;
    let (target, vo_id) = import_foreign(&svc, extract, &vo).await;

    // (1) Modify the imported trunk tip (created by FOREIGN) → the commit must
    // fork branch 2.1.1 with the LOCAL creating_system_id (master06: "branching
    // version identifiers [are required] when local modifications are made to
    // versions copied from elsewhere").
    let foreign_tip = format!("{vo_id}::{FOREIGN}::2");
    let contribution = svc
        .create_ehr_contribution(
            target,
            modify_contribution(&composition("local mod"), &foreign_tip),
        )
        .await
        .expect("branch-forking modification");
    let branch_uid = first_version_uid(&contribution.body);
    assert_eq!(
        branch_uid,
        format!("{vo_id}::{LOCAL}::2.1.1"),
        "a local modification of a foreign version forks branch t.1.1"
    );

    // (2) The container current (latest trunk, master06 latest_trunk_version)
    // is STILL the imported foreign trunk v2 — the branch coexists.
    let current = svc
        .get_composition_latest(target, vo_id)
        .await
        .expect("current composition");
    assert_eq!(
        current["uid"]["value"].as_str().unwrap(),
        foreign_tip,
        "the trunk tip stays current; a fork does not supersede it"
    );

    // (3) The branch version is addressable by its full OBJECT_VERSION_ID and
    // serves the TRUE preceding_version_uid (the foreign trunk version).
    let ov = svc
        .composition_version_envelope(target, branch_uid.parse().unwrap())
        .await
        .expect("branch ORIGINAL_VERSION");
    assert_eq!(ov["uid"]["value"], json!(branch_uid));
    assert_eq!(
        ov["preceding_version_uid"]["value"],
        json!(foreign_tip),
        "preceding_version_uid is stored, not synthesized — it names the \
         foreign version the branch forked from"
    );

    // (4) Continue OUR branch tip → 2.1.2, superseding 2.1.1; both remain
    // readable (versions are indelible).
    let contribution2 = svc
        .create_ehr_contribution(
            target,
            modify_contribution(&composition("local mod 2"), &branch_uid),
        )
        .await
        .expect("branch continuation");
    let branch_uid2 = first_version_uid(&contribution2.body);
    assert_eq!(branch_uid2, format!("{vo_id}::{LOCAL}::2.1.2"));
    let ov2 = svc
        .composition_version_envelope(target, branch_uid2.parse().unwrap())
        .await
        .expect("branch tip ORIGINAL_VERSION");
    assert_eq!(ov2["preceding_version_uid"]["value"], json!(branch_uid));
    svc.composition_version_envelope(target, branch_uid.parse().unwrap())
        .await
        .expect("superseded branch version stays readable");

    // (5) A second fork from the same foreign trunk version numbers the NEXT
    // branch (2.2.1) — branch numbers count per fork point (master06
    // §The 'Virtual Version Tree').
    let contribution3 = svc
        .create_ehr_contribution(
            target,
            modify_contribution(&composition("second fork"), &foreign_tip),
        )
        .await
        .expect("second fork");
    assert_eq!(
        first_version_uid(&contribution3.body),
        format!("{vo_id}::{LOCAL}::2.2.1")
    );
}

/// The served `IMPORTED_VERSION` is exactly `{_type, contribution,
/// commit_audit, item}` plus the optional `signature` — and carries NO
/// top-level `data`.
///
/// The RM gives `IMPORTED_VERSION` exactly one own attribute, `item:
/// ORIGINAL_VERSION 1..1`, and lists `uid`, `preceding_version_uid`,
/// `lifecycle_state` and `data` under §Functions as `1..1 (effected)`
/// computations over it — `data` is "Original content of this Version",
/// derived as `item.data`
/// (`UML/classes/org.openehr.rm.common.imported_version.adoc`). ITS-XML agrees
/// structurally: its abstract `VERSION` declares only `contribution`,
/// `commit_audit` and an optional `signature`, `data` is declared on
/// `ORIGINAL_VERSION`, and `IMPORTED_VERSION` extends `VERSION` with the single
/// `item` element (`components/RM/Release-1.1.0/Common.xsd`). Inherited
/// `commit_audit`/`contribution` are the LOCAL act of committal, "distinct from
/// those of the imported `ORIGINAL_VERSION`" (the class description), and the
/// wrapper's own `signature` "signifies the act of importing" (RM common
/// master06 §Digital Signature).
///
/// NOTE: the released OAS disagrees — its `Version` schema lists `data` under
/// `required` and `ImportedVersion` inherits it through `allOf`, so a
/// schema-validating client rejects this body. Emitting a top-level `data`
/// would duplicate the entire clinical payload with no consistency rule for
/// the two copies, and would contradict both the RM's `(effected)` typing and
/// the ITS-XML content model, so the wire follows the model; the omission is
/// deliberate, which is what this test pins.
#[tokio::test]
async fn the_served_imported_version_is_the_wrapper_shape_without_data() {
    let source_db = testkit::db().await.expect("testkit database");
    let source_svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (extract, vo) = foreign_extract(&source_svc).await;
    let (target, vo_id) = import_foreign(&svc, extract, &vo).await;

    let iv = svc
        .composition_version_envelope(target, format!("{vo_id}::{FOREIGN}::2").parse().unwrap())
        .await
        .expect("imported version");

    assert_eq!(iv["_type"], json!("IMPORTED_VERSION"));
    let keys: Vec<&str> = iv
        .as_object()
        .expect("the wrapper is an object")
        .keys()
        .map(String::as_str)
        .collect();
    // `VERSION.signature` is 0..1; the default deployment posture signs, so the
    // wrapper carries the signature that "signifies the act of importing"
    // (master06 §Digital Signature) — never one of the effected functions.
    assert_eq!(
        keys,
        ["_type", "contribution", "commit_audit", "item", "signature"],
        "the served IMPORTED_VERSION is the wrapper's serialized state only, \
         got {iv:?}"
    );
    // The four effected functions are NOT serialized attributes of the
    // wrapper: each is served on the wrapped ORIGINAL_VERSION instead.
    for effected in ["data", "uid", "preceding_version_uid", "lifecycle_state"] {
        assert!(
            iv.get(effected).is_none(),
            "{effected} is an effected function over item, never a top-level \
             attribute of IMPORTED_VERSION, got {iv:?}"
        );
        assert!(
            iv["item"].get(effected).is_some(),
            "{effected} is served on the wrapped ORIGINAL_VERSION, got {iv:?}"
        );
    }
    assert_eq!(
        iv["item"]["_type"],
        json!("ORIGINAL_VERSION"),
        "item is the foreign ORIGINAL_VERSION, reproduced verbatim"
    );
}

#[tokio::test]
async fn merge_provenance_is_preserved_by_the_route_that_carries_it() {
    // `ORIGINAL_VERSION.other_input_version_uids` (RM common master06 §Version
    // Merging) is PRODUCE-only on this server: the released commit wire declares
    // no merge shape at all, so a member carrying it is refused. What DOES carry
    // merge provenance is the route that reproduces a FOREIGN `ORIGINAL_VERSION`
    // verbatim — master06 §Copying: "the `ORIGINAL_VERSION` instance is never
    // modified". This test pins that direction end to end: an imported merged
    // version keeps its inputs and serves them, while a locally committed
    // version carries none (`Is_merged_validity`).
    let source_db = testkit::db().await.expect("testkit database");
    let source_svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (mut extract, vo) = foreign_extract(&source_svc).await;

    // Stamp merge provenance onto the LATEST foreign version, as a merging
    // source system would have recorded it before the copy.
    let merged_in = format!("{}::{FOREIGN}::3", Uuid::now_v7());
    let merged_uid = format!("{vo}::{FOREIGN}::2");
    let stamped = stamp_merge_provenance(&mut extract, &merged_uid, &merged_in);
    assert!(stamped, "the extract must carry the version to stamp");

    let (target, vo_id) = import_foreign(&svc, extract, &vo).await;
    // An imported version is served as the IMPORTED_VERSION wrapper whose
    // `item` IS the foreign ORIGINAL_VERSION, reproduced verbatim (RM common
    // master06 §Copying + §Version and its Subtypes).
    let iv = svc
        .composition_version_envelope(target, format!("{vo_id}::{FOREIGN}::2").parse().unwrap())
        .await
        .expect("imported merged version");
    assert_eq!(
        iv["item"]["other_input_version_uids"][0]["value"],
        json!(merged_in),
        "the imported version keeps its merge provenance verbatim (master06 §Copying)"
    );

    // The twin: an identical import whose version was never merged carries
    // none (`ORIGINAL_VERSION.Is_merged_validity` — merge provenance is
    // present exactly when the version really was merged).
    let (unstamped, plain_vo) = foreign_extract(&source_svc).await;
    let (plain_target, plain_vo_id) = import_foreign(&svc, unstamped, &plain_vo).await;
    let plain = svc
        .composition_version_envelope(
            plain_target,
            format!("{plain_vo_id}::{FOREIGN}::2").parse().unwrap(),
        )
        .await
        .expect("imported unmerged version");
    assert!(
        plain["item"].get("other_input_version_uids").is_none(),
        "a non-merged version carries no merge provenance (Is_merged_validity)"
    );
    let _ = target;
}

/// Add `other_input_version_uids` to the `ORIGINAL_VERSION` identified by
/// `version_uid` inside an EHR-Extract value, returning whether it was found.
/// Mirrors what a merging source system records before the version is copied
/// elsewhere (RM common master06 §Version Merging).
fn stamp_merge_provenance(extract: &mut Value, version_uid: &str, merged_in: &str) -> bool {
    fn walk(node: &mut Value, version_uid: &str, merged_in: &str, done: &mut bool) {
        match node {
            Value::Object(map) => {
                if map.get("_type").and_then(Value::as_str) == Some("ORIGINAL_VERSION")
                    && map
                        .get("uid")
                        .and_then(|u| u.get("value"))
                        .and_then(Value::as_str)
                        == Some(version_uid)
                {
                    map.insert(
                        "other_input_version_uids".to_owned(),
                        json!([{ "_type": "OBJECT_VERSION_ID", "value": merged_in }]),
                    );
                    *done = true;
                }
                for (_, child) in map.iter_mut() {
                    walk(child, version_uid, merged_in, done);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, version_uid, merged_in, done);
                }
            }
            _ => {}
        }
    }
    let mut done = false;
    walk(extract, version_uid, merged_in, &mut done);
    done
}

#[tokio::test]
async fn a_version_tree_with_branches_reexports_and_reimports_whole() {
    let source_db = testkit::db().await.expect("testkit database");
    let source_svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (extract, vo) = foreign_extract(&source_svc).await;
    let (target, vo_id) = import_foreign(&svc, extract, &vo).await;

    // Grow a local branch on the imported tree.
    let foreign_tip = format!("{vo_id}::{FOREIGN}::2");
    svc.create_ehr_contribution(
        target,
        modify_contribution(&composition("local branch"), &foreign_tip),
    )
    .await
    .expect("fork");

    // Export the whole tree (trunk from FOREIGN + branch from LOCAL, ALL
    // versions) and re-import into a THIRD repository: the mixed-system,
    // branched version tree is preserved verbatim (master06 §Copying — branch
    // import is first-class).
    let mut extracts = svc
        .export_ehr_extracts(
            openehr_its::json::from_canonical_value(&all_versions_spec(target))
                .expect("EXTRACT_SPEC"),
        )
        .await
        .expect("re-export (all versions)");
    let wire = serde_json::to_string(&extracts[0]).unwrap();
    assert!(
        wire.contains(&format!("::{LOCAL}::2.1.1")),
        "the exported version tree must include the branch version"
    );
    let third_db = testkit::db().await.expect("testkit database");
    let third_svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &third_db.pool(),
    ));
    let third = ferroehr::ids::EhrId(Uuid::now_v7());
    third_svc
        .import_ehr(
            Some(third),
            openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("EXTRACT"),
        )
        .await
        .expect("re-import of a branched, multi-system version tree");

    let branch_uid = format!("{vo_id}::{LOCAL}::2.1.1");
    let ov = third_svc
        .composition_version_envelope(third, branch_uid.parse().unwrap())
        .await
        .expect("re-imported branch version");
    // In the THIRD repository the version is a copy, so it is served as an
    // IMPORTED_VERSION wrapping the received original (master06 §Copying). An
    // IMPORTED_VERSION carries NO `uid` attribute of its own: `uid` is an
    // effected function, `Post: Result = item.uid` (`imported_version.adoc`
    // §Functions), and ITS-REST 1.1.0's `schemas/ehr/ImportedVersion.yaml`
    // likewise declares only `item` on top of `Version.yaml`. So the branch
    // identity is read through the wrapper.
    assert_eq!(ov["_type"], json!("IMPORTED_VERSION"));
    assert_eq!(ov["item"]["uid"]["value"], json!(branch_uid));
    assert_eq!(ov["item"]["_type"], json!("ORIGINAL_VERSION"));
    let trunk = third_svc
        .get_composition_latest(third, vo_id)
        .await
        .expect("re-imported trunk current");
    assert_eq!(trunk["uid"]["value"].as_str().unwrap(), foreign_tip);
}

/// `ALL_VERSIONS` returns the BRANCH row beside the trunk ones: the version
/// spine is the append-only `version` table unfiltered, so a fork is a row of
/// the result rather than a row the currency predicate hides (QUERY
/// `master03-syntax.adoc` §Containment over `VERSION`; the branch identity is
/// RM common master06 §Version Identification).
#[tokio::test]
async fn all_versions_returns_the_branch_row_beside_the_trunk() {
    let source_db = testkit::db().await.expect("testkit database");
    let source_svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (extract, vo) = foreign_extract(&source_svc).await;
    let (target, vo_id) = import_foreign(&svc, extract, &vo).await;

    // Fork branch 2.1.1 off the imported foreign trunk tip.
    svc.create_ehr_contribution(
        target,
        modify_contribution(&composition("local mod"), &format!("{vo_id}::{FOREIGN}::2")),
    )
    .await
    .expect("branch-forking modification");

    let request = AqlQueryRequest {
        ehr_ids: vec![target.0.to_string()],
        ..AqlQueryRequest::default()
    };
    let result = svc
        .execute_ad_hoc_query(
            "SELECT c/uid/value FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c"
                .to_owned(),
            request,
        )
        .await
        .expect("ALL_VERSIONS query")
        .result_set;
    let uids: Vec<String> = result["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|row| row[0].as_str().map(str::to_owned))
        .collect();
    assert!(
        uids.contains(&format!("{vo_id}::{LOCAL}::2.1.1")),
        "the branch version is in the ALL_VERSIONS result: {uids:?}"
    );
    assert!(
        uids.contains(&format!("{vo_id}::{FOREIGN}::2")),
        "the trunk tip is in it too: {uids:?}"
    );
    // …and the branch is NOT in the LATEST_VERSION result, which is the trunk
    // head alone (master06 §The 'Virtual Version Tree': a fork does not
    // supersede the trunk).
    let latest = svc
        .execute_ad_hoc_query(
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c".to_owned(),
            AqlQueryRequest {
                ehr_ids: vec![target.0.to_string()],
                ..AqlQueryRequest::default()
            },
        )
        .await
        .expect("LATEST_VERSION query")
        .result_set;
    let latest_uids: Vec<String> = latest["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|row| row[0].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        latest_uids,
        vec![format!("{vo_id}::{FOREIGN}::2")],
        "LATEST_VERSION is the trunk head alone"
    );
}
