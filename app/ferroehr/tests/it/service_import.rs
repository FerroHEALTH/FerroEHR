// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end service tests for the EHR Extract **import** path (SM
//! `I_EHR_EXTRACT_SERVICE.import_ehr` / `import_ehr_extract`) against a real
//! `PostgreSQL` 18 (shared testkit harness).
//!
//! Spec: SM `docs/specs/openehr/SM/docs/UML/classes/i_ehr_extract_service.adoc`;
//! RM EHR Extract IM master05 (`X_VERSIONED_*`) + RM common
//! `docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`
//! §Copying (`IMPORTED_VERSION` semantics, Cases 1/2/3) + §Committal and
//! Audits. The acceptance properties:
//!
//! 1. **Whole-EHR clone** (`import_ehr`) replays a whole-EHR export into an
//!    empty target, reusing the source EHR id when none is given (master06
//!    §Copying Case 1: "the newly created EHR should re-use the EHR identifier
//!    from the source system"), or a caller-provided fixed id (the SM's
//!    "same patient in other EHR services" case). Each imported version keeps
//!    the wrapped `ORIGINAL_VERSION`'s identity (`OBJECT_VERSION_ID`) and data
//!    verbatim — so reading the clone's `EHR_STATUS` returns byte-identical
//!    canonical JSON to the source's.
//! 2. **A duplicate target id** is `ehr_create_fail_duplicate_id` (`import_ehr`
//!    imports into an *empty* target).
//! 3. **Extract import into an existing EHR** (`import_ehr_extract`) lands a new
//!    versioned object (master06 §Copying Case 2: first receipt of an item
//!    clones its `VERSIONED_OBJECT` with the received `uid.object_id()`);
//!    re-importing the same trunk version is a conflict.
//! 4. **The clone is a complete, first-class local EHR**: `EHR.ehr_access` is
//!    1..1 (RM ehr `ehr.adoc` invariant `Ehr_access_valid`; master04 §EHR
//!    Creation — "a root EHR object, an EHR Status object, and an EHR Access
//!    object … created and committed in a Contribution"), so an extract that
//!    carries no `EHR_ACCESS` is completed with the local default; and the
//!    imported `EHR_STATUS.subject` is promoted, so the clone is found by the
//!    subject lookup (SM `I_EHR_SERVICE.get_ehrs_for_subject`,
//!    `operations/ehr_get_by_subject.yaml`) and bound by the
//!    one-EHR-per-subject rule (RM ehr master04 §EHR Status) like any other EHR.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use serde_json::{Value, json};

use openehr_rm::v1_2::ehr_extract::common::extract::Extract;
use openehr_rm::v1_2::ehr_extract::common::extract_spec::ExtractSpec;

use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::ehr_index::types::SubjectRef;
use ferroehr::service::status::CallStatusType;

use crate::fixtures::{composition, uv};
use crate::typed_body::typed;
use ferroehr::versioning::signature::config::{Mode, SigningConfig, VerifyOnRead};
use ferroehr::versioning::signature::signer::Signer;

/// The database's own current instant, for a probe an assertion compares
/// against a stored one.
///
/// Committal instants are stamped by PostgreSQL, not by this process:
/// `audit.time_committed` defaults to `now()`, because RM common
/// `master06-change_control_package.adoc` §Committal and Audits requires the
/// committal instant to be computed on the server. Taking the probe from
/// `jiff::Timestamp::now()` would therefore compare a host clock against a
/// container clock, and the two are free to disagree (#3138). Every database
/// here comes from the one testkit server, so a probe read from any pool shares
/// its clock with every instant under test.
async fn db_now(pool: &sqlx::PgPool) -> jiff::Timestamp {
    let now: jiff_sqlx::Timestamp = sqlx::query_scalar("SELECT now()")
        .fetch_one(pool)
        .await
        .expect("the database's current instant");
    now.to_jiff()
}

/// Seed an EHR with an `EHR_STATUS` (create → update = two versions), a directory
/// `FOLDER`, and the auto-created `EHR_ACCESS` — the same shape the export tests
/// seed. Returns the EHR id.
async fn seed_ehr(svc: &FerroEhrService) -> ferroehr::ids::EhrId {
    let ehr_uuid = svc.create_ehr(None).await.expect("ehr");

    let mut status = svc
        .get_ehr_status_at_time(ehr_uuid, None)
        .await
        .expect("status get");
    let status_ovid = status["uid"]["value"].as_str().expect("uid").to_owned();
    status.as_object_mut().expect("status obj").remove("uid");

    svc.create_directory(
        ehr_uuid,
        uv(
            &json!({
                "_type": "FOLDER",
                "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                "name": { "_type": "DV_TEXT", "value": "root" }
            }),
            "249",
            None,
        ),
    )
    .await
    .expect("directory");

    status["is_modifiable"] = json!(false);
    svc.replace_ehr_status(ehr_uuid, uv(&status, "251", Some(&status_ovid)))
        .await
        .expect("status update");

    ehr_uuid
}

/// The single-`EXTRACT` result of an `export_ehrs`, as a typed [`Extract`]
/// (the wire shape the import calls receive).
async fn export_one(svc: &FerroEhrService, ehr: ferroehr::ids::EhrId) -> Extract {
    let mut extracts = svc.extract_ehrs(ehr).await.expect("export_ehrs");
    assert_eq!(extracts.len(), 1, "one EHR id → one EXTRACT");
    openehr_its::json::from_canonical_value(&extracts.remove(0))
        .expect("EXTRACT deserializes into the typed RM model")
}

/// The content item of an extract whose wrapped object has the given
/// `X_VERSIONED_*` `_type`.
fn find_by_xtype<'a>(extract: &'a Value, xtype: &str) -> Option<&'a Value> {
    extract["chapters"][0]["items"]
        .as_array()
        .expect("chapter items")
        .iter()
        .find(|it| it["item"]["_type"] == json!(xtype))
}

/// How many content items of an extract wrap the given `X_VERSIONED_*` `_type`.
fn count_by_xtype(extract: &Value, xtype: &str) -> usize {
    extract["chapters"][0]["items"]
        .as_array()
        .expect("chapter items")
        .iter()
        .filter(|it| it["item"]["_type"] == json!(xtype))
        .count()
}

/// The same extract with every `X_VERSIONED_EHR_ACCESS` content item removed —
/// the export of a source that holds no `EHR_ACCESS`, or one filtered out en
/// route.
fn without_ehr_access(extract: &Value) -> Extract {
    let mut raw = extract.clone();
    let items: Vec<Value> = raw["chapters"][0]["items"]
        .as_array()
        .expect("chapter items")
        .iter()
        .filter(|it| it["item"]["_type"] != json!("X_VERSIONED_EHR_ACCESS"))
        .cloned()
        .collect();
    raw["chapters"][0]["items"] = Value::Array(items);
    openehr_its::json::from_canonical_value(&raw).expect("EXTRACT without EHR_ACCESS")
}

/// [`status_for_subject`] with an explicit namespace (the extract rewrite
/// normalizes carried refs to `"local"` — EHR Extract master09 §Creation
/// Semantics — so conflict fixtures must name it).
fn status_for_subject_in(subject_id: &str, namespace: &str) -> Value {
    let mut status = status_for_subject(subject_id);
    status["subject"]["external_ref"]["namespace"] = json!(namespace);
    status
}

/// An `EHR_STATUS` whose `PARTY_SELF` subject carries an `external_ref` —
/// RM ehr master04 §EHR Status (the subject 0..1 identifies the EHR).
fn status_for_subject(subject_id: &str) -> Value {
    json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": "patients",
                "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": subject_id }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    })
}

#[tokio::test]
async fn import_ehr_clone_into_fresh_target_reuses_source_id() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let source_status = source
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("source status");

    // No fixed id → the clone reuses the source EHR id (master06 §Copying Case 1).
    let extract = export_one(&source, ehr).await;
    target.import_ehr(None, extract).await.expect("import_ehr");

    // The clone exists under the *same* EHR id in the target, which holds no
    // other EHR.
    let target_status = target
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("cloned status readable under the reused ehr_id");

    // The wrapped ORIGINAL_VERSION was preserved verbatim: identical uid
    // (OBJECT_VERSION_ID) and data ("the ORIGINAL_VERSION instance is never
    // modified", master06 §Copying).
    assert_eq!(
        target_status, source_status,
        "cloned EHR_STATUS must be byte-identical to the source (uid + data preserved)"
    );
    assert_eq!(
        target_status["is_modifiable"],
        json!(false),
        "the latest (v2) EHR_STATUS was imported, is_modifiable = false"
    );

    // The clone also carries the source's EHR_ACCESS and directory FOLDER.
    let re_export = target.extract_ehrs(ehr).await.expect("re-export clone");
    for xtype in [
        "X_VERSIONED_EHR_STATUS",
        "X_VERSIONED_EHR_ACCESS",
        "X_VERSIONED_FOLDER",
    ] {
        assert!(
            find_by_xtype(&re_export[0], xtype).is_some(),
            "clone must carry {xtype}"
        );
    }
    // Exactly one: an extract that already carries an EHR_ACCESS is never given
    // a second, locally bootstrapped one (EHR.ehr_access is 1..1 — RM ehr
    // `ehr.adoc` invariant `Ehr_access_valid`).
    assert_eq!(
        count_by_xtype(&re_export[0], "X_VERSIONED_EHR_ACCESS"),
        1,
        "the carried EHR_ACCESS is the only one"
    );
}

#[tokio::test]
async fn import_ehr_without_ehr_access_bootstraps_the_mandatory_default() {
    // RM ehr `ehr.adoc` invariant `Ehr_access_valid` makes `EHR.ehr_access`
    // 1..1, and master04 §EHR Creation requires that creating an EHR yields "a
    // root EHR object, an EHR Status object, and an EHR Access object …
    // created and committed in a Contribution". An extract that carries no
    // EHR_ACCESS must therefore be completed locally — otherwise the clone
    // violates the invariant permanently and its served EHR body omits the
    // mandatory reference.
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    assert_eq!(
        count_by_xtype(&exported[0], "X_VERSIONED_EHR_ACCESS"),
        1,
        "fixture: the source export carries an EHR_ACCESS to strip"
    );
    target
        .import_ehr(None, without_ehr_access(&exported[0]))
        .await
        .expect("import an extract with no EHR_ACCESS");

    // The served EHR body carries the mandatory ehr_access reference.
    let body = target.ehr_object(ehr).await.expect("EHR body");
    assert_eq!(body["ehr_access"]["_type"], json!("OBJECT_REF"));
    assert_eq!(body["ehr_access"]["type"], json!("VERSIONED_EHR_ACCESS"));
    assert!(
        body["ehr_access"]["id"]["value"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "the ehr_access ref names the VERSIONED_EHR_ACCESS, got {body:#}"
    );

    // And the container really exists — one locally created first version.
    let re_export = target.extract_ehrs(ehr).await.expect("re-export clone");
    assert_eq!(
        count_by_xtype(&re_export[0], "X_VERSIONED_EHR_ACCESS"),
        1,
        "exactly one bootstrapped EHR_ACCESS"
    );
    let access =
        find_by_xtype(&re_export[0], "X_VERSIONED_EHR_ACCESS").expect("bootstrapped EHR_ACCESS");
    let versions = access["item"]["versions"]
        .as_array()
        .expect("EHR_ACCESS versions");
    assert_eq!(versions.len(), 1, "the bootstrap commits one version");
    let version_uid = versions[0]["uid"]["value"]
        .as_str()
        .expect("version uid.value");
    assert!(
        version_uid.ends_with("::1"),
        "the bootstrapped EHR_ACCESS is a first version, got {version_uid}"
    );
    assert_eq!(versions[0]["data"]["_type"], json!("EHR_ACCESS"));
    // The default is an archetype root (RM ehr `ehr_access.adoc`
    // `Is_archetype_root`; RM common `locatable.adoc` `Archetyped_valid`).
    assert!(
        versions[0]["data"]["archetype_details"].is_object(),
        "the default EHR_ACCESS carries its ARCHETYPED block, got {:#}",
        versions[0]["data"]
    );
}

#[tokio::test]
async fn import_ehr_promotes_the_subject_for_lookup_and_uniqueness() {
    // An imported EHR is a full local EHR: its EHR_STATUS.subject is promoted,
    // so the clone is found by the subject lookup (SM
    // `I_EHR_SERVICE.get_ehrs_for_subject`;
    // `operations/ehr_get_by_subject.yaml`) and holds the subject against a
    // later create (one EHR per subject — RM ehr master04 §EHR Status).
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = source
        .create_ehr(Some(typed(&status_for_subject("patient-import-1"))))
        .await
        .expect("source EHR for the subject");
    let extract = export_one(&source, ehr).await;
    target.import_ehr(None, extract).await.expect("import_ehr");

    // The extract normalizes every OBJECT_REF-family namespace to "local"
    // (EHR Extract master09 §Creation Semantics: "rewriting its
    // OBJECT_REFs so that namespace = \"local\""), so the imported
    // subject is patient-import-1@local — the promoted columns follow
    // the STORED status, exactly as the fix requires.
    let subject = SubjectRef::person("patient-import-1", "local");
    assert!(
        target
            .has_ehr_for_subject(subject.clone())
            .await
            .expect("has_ehr_for_subject"),
        "the imported clone must be visible to the subject lookup"
    );
    let found = target
        .get_ehrs_for_subject(subject)
        .await
        .expect("get_ehrs_for_subject");
    assert_eq!(found.len(), 1, "one EHR per subject");
    assert_eq!(found[0].ehr_id, ehr.to_string());

    // The subject is now taken in the target — under the "local" namespace
    // the extract normalized it to — so a later create naming that same
    // (id, namespace) pair is a 409.
    let dup = target
        .create_ehr(Some(typed(&status_for_subject_in(
            "patient-import-1",
            "local",
        ))))
        .await;
    assert!(
        matches!(
            dup,
            Err(ferroehr::service::status::SmError {
                status: CallStatusType::EhrForSubjectAlreadyExists,
                ..
            })
        ),
        "an EHR for a subject an imported clone already holds must 409, got {dup:?}"
    );
}

#[tokio::test]
async fn import_ehr_conflicting_subject_is_rejected_and_rolled_back() {
    // Importing a clone of a subject this repository already holds under
    // another EHR is a conflict, not a silent duplicate (one EHR per subject —
    // RM ehr master04 §EHR Status); the whole import transaction rolls back.
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    // The import will carry the subject under namespace "local" (the
    // extract's master09 §Creation Semantics OBJECT_REF rewrite), so the
    // pre-existing holder must own it under that namespace to collide.
    let owner = target
        .create_ehr(Some(typed(&status_for_subject_in(
            "patient-import-2",
            "local",
        ))))
        .await
        .expect("the target already holds the subject");
    let ehr = source
        .create_ehr(Some(typed(&status_for_subject("patient-import-2"))))
        .await
        .expect("source EHR for the same subject");
    let extract = export_one(&source, ehr).await;

    let err = target
        .import_ehr(None, extract)
        .await
        .expect_err("importing a subject the target already holds must be rejected");
    assert_eq!(err.status, CallStatusType::EhrForSubjectAlreadyExists);
    assert!(
        err.message.contains("patient-import-2") && err.message.contains(&owner.to_string()),
        "the error must name the subject and the EHR that holds it, got: {}",
        err.message
    );

    // Rolled back: the clone left nothing behind, and the owner is untouched.
    assert!(
        !target.has_ehr(ehr).await.expect("has_ehr"),
        "a rejected import must not leave a partial EHR"
    );
    let found = target
        .get_ehrs_for_subject(SubjectRef::person("patient-import-2", "local"))
        .await
        .expect("get_ehrs_for_subject");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].ehr_id, owner.to_string());
}

#[tokio::test]
async fn import_ehr_into_fixed_fresh_id() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let source_status = source
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("source status");

    // A caller-provided fixed id (the SM's "same patient in other EHR services"
    // case): the clone lands under `fixed`, not the source id.
    let fixed = ferroehr::ids::EhrId(uuid::Uuid::now_v7());
    let extract = export_one(&source, ehr).await;
    target
        .import_ehr(Some(fixed), extract)
        .await
        .expect("import_ehr with a fixed id");

    // The source id does not exist in the target; the fixed id does.
    assert_eq!(
        target
            .extract_ehrs(ehr)
            .await
            .expect_err("source id absent in target")
            .status,
        CallStatusType::EhrIdDoesNotExist
    );
    let mut target_status = target
        .get_ehr_status_at_time(fixed, None)
        .await
        .expect("status under the fixed id");

    // The versioned-object identity (uid) is preserved regardless of the target
    // EHR id (the OBJECT_VERSION_ID's object_id is the EHR_STATUS container, not
    // the EHR): data + uid match the source exactly.
    assert_eq!(target_status, source_status);
    target_status.as_object_mut().unwrap().remove("uid");
    assert_eq!(target_status["is_modifiable"], json!(false));
}

#[tokio::test]
async fn import_ehr_duplicate_target_is_rejected() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = seed_ehr(&svc).await;

    // import_ehr imports into an *empty* target; a fixed id that already exists
    // is `ehr_create_fail_duplicate_id`.
    let extract = export_one(&svc, ehr).await;
    let err = svc
        .import_ehr(Some(ehr), extract)
        .await
        .expect_err("duplicate target EHR id must be rejected");
    assert_eq!(err.status, CallStatusType::EhrCreateFailDuplicateId);
}

#[tokio::test]
async fn import_ehr_extract_adds_a_versioned_object_and_rejects_re_import() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let src_ehr = seed_ehr(&source).await;

    // A FOLDER-only extract (item_list restricts to the directory container),
    // so importing it into an existing EHR that has no directory is a clean
    // Case-2 create (no EHR_STATUS/EHR_ACCESS singleton clash).
    let whole = source.extract_ehrs(src_ehr).await.expect("whole export");
    let folder_vo = find_by_xtype(&whole[0], "X_VERSIONED_FOLDER").expect("folder in export")
        ["item"]["uid"]["value"]
        .as_str()
        .expect("folder vo uid")
        .to_owned();
    let source_folder_data =
        find_by_xtype(&whole[0], "X_VERSIONED_FOLDER").unwrap()["item"]["versions"][0]["data"]
            .clone();

    let spec: ExtractSpec = openehr_its::json::from_canonical_value(&json!({
        "_type": "EXTRACT_SPEC",
        "manifest": {
            "_type": "EXTRACT_MANIFEST",
            "entities": [ {
                "_type": "EXTRACT_ENTITY_MANIFEST",
                "extract_id_key": src_ehr.to_string(),
                "ehr_id": src_ehr.to_string(),
                "other_ids": [],
                "item_list": [ {
                    "_type": "OBJECT_REF",
                    "namespace": "local",
                    "type": "VERSIONED_FOLDER",
                    "id": { "_type": "HIER_OBJECT_ID", "value": folder_vo }
                } ]
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
    }))
    .expect("EXTRACT_SPEC");

    let mut folder_extracts = source
        .export_ehr_extracts(spec)
        .await
        .expect("folder-only export");
    let folder_extract: Extract =
        openehr_its::json::from_canonical_value(&folder_extracts.remove(0))
            .expect("folder EXTRACT");

    // A fresh target EHR (its own EHR_STATUS/EHR_ACCESS, no directory yet).
    let tgt_ehr = target.create_ehr(None).await.expect("target ehr");
    target
        .import_ehr_extract(tgt_ehr, folder_extract.clone())
        .await
        .expect("import the FOLDER into the existing EHR");

    // The target now carries the imported directory FOLDER with the source's data.
    let re_export = target
        .extract_ehrs(tgt_ehr)
        .await
        .expect("re-export target");
    let imported_folder =
        find_by_xtype(&re_export[0], "X_VERSIONED_FOLDER").expect("folder now present in target");
    assert_eq!(
        imported_folder["item"]["versions"][0]["data"], source_folder_data,
        "imported FOLDER data must match the source"
    );

    // Re-importing the same trunk version of the same container is a conflict
    // (trunk-only; the container already has that version).
    let err = target
        .import_ehr_extract(tgt_ehr, folder_extract)
        .await
        .expect_err("re-import of the same version must be rejected");
    assert_eq!(err.status, CallStatusType::Conflict);
}

/// The `IMPORTED_VERSION` semantics of RM common master06 §Committal and Audits:
/// "Both the contribution and `commit_audit` of the latter object correspond to
/// the local act of committal, while the knowledge of the original Contribution
/// and committal are retained inside the wrapped `ORIGINAL_VERSION` instance."
///
/// So the VERSION resource of an imported version serves an `IMPORTED_VERSION`
/// whose own contribution/audit are LOCAL and whose `item` is the received
/// `ORIGINAL_VERSION`, foreign contribution and commit audit intact. Regression
/// for #1679, where the wrapper never materialised: the foreign audit was
/// written as the row's own and the foreign contribution reference was dropped.
#[tokio::test]
async fn an_imported_version_serves_the_wrapper_and_wrapped_split() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    let source_version = find_by_xtype(&exported[0], "X_VERSIONED_EHR_STATUS")
        .expect("exported EHR_STATUS")["item"]["versions"][0]
        .clone();
    assert_eq!(
        source_version["_type"],
        json!("ORIGINAL_VERSION"),
        "fixture: a locally created source version is an ORIGINAL_VERSION"
    );
    let source_uid = source_version["uid"]["value"]
        .as_str()
        .expect("source version uid")
        .to_owned();
    let (vo_id, version) = {
        let mut parts = source_uid.splitn(3, "::");
        let vo: &str = parts.next().expect("object_id");
        let _system = parts.next().expect("creating_system_id");
        (
            ferroehr::ids::VoId(vo.parse().expect("vo uuid")),
            parts.next().expect("version_tree_id").to_owned(),
        )
    };

    target
        .import_ehr(None, export_one(&source, ehr).await)
        .await
        .expect("import_ehr");

    let served = target
        .ehr_status_version_envelope(ehr, vo_id, &version)
        .await
        .expect("the imported version reads back");

    // The wrapper.
    assert_eq!(
        served["_type"],
        json!("IMPORTED_VERSION"),
        "a copied version is committed as an IMPORTED_VERSION (master06 §Copying), got {served:#}"
    );
    assert_eq!(
        served["commit_audit"]["change_type"]["defining_code"]["code_string"],
        json!("249"),
        "master06 §Contributions, import of item: the wrapper's change_type is 249|creation|"
    );
    assert_eq!(
        served["contribution"]["type"],
        json!("CONTRIBUTION"),
        "the wrapper names the LOCAL import CONTRIBUTION"
    );

    // The wrapped original — never modified (master06 §Copying).
    let item = &served["item"];
    assert_eq!(item["_type"], json!("ORIGINAL_VERSION"));
    assert_eq!(item["uid"]["value"], json!(source_uid.clone()));
    assert_eq!(
        item["contribution"], source_version["contribution"],
        "the wrapped original keeps the SOURCE contribution reference"
    );
    assert_eq!(
        item["commit_audit"], source_version["commit_audit"],
        "the wrapped original keeps the SOURCE commit audit verbatim"
    );
    assert_eq!(item["data"], source_version["data"]);

    // The two acts are genuinely distinct.
    assert_ne!(
        served["contribution"]["id"]["value"], item["contribution"]["id"]["value"],
        "the local import CONTRIBUTION is not the source's"
    );
    assert_ne!(
        served["commit_audit"]["time_committed"]["value"],
        item["commit_audit"]["time_committed"]["value"],
        "the local committal instant is not the source's"
    );
}

/// RM common master06 §Copying: "the commit times always reflect the local
/// (more recent) act of committal, not the original committal … rather than
/// giving the illusion that recently copied Versions were there earlier than
/// the time of local committal." So an imported container's
/// `VERSIONED_OBJECT.time_created` and its revision history's commit audit are
/// the LOCAL import act, never the source's clock. Regression for #1679.
#[tokio::test]
async fn an_imported_container_reports_the_local_chronology() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    let source_version = find_by_xtype(&exported[0], "X_VERSIONED_EHR_STATUS")
        .expect("exported EHR_STATUS")["item"]["versions"][0]
        .clone();
    let source_instant = source_version["commit_audit"]["time_committed"]["value"]
        .as_str()
        .expect("source commit instant")
        .parse::<jiff::Timestamp>()
        .expect("ISO 8601 instant");

    let before_import = db_now(&target_db.pool()).await;
    target
        .import_ehr(None, export_one(&source, ehr).await)
        .await
        .expect("import_ehr");

    let container = target
        .versioned_ehr_status_response(ehr)
        .await
        .expect("VERSIONED_EHR_STATUS");
    let time_created = container.body["time_created"]["value"]
        .as_str()
        .expect("VERSIONED_OBJECT.time_created")
        .parse::<jiff::Timestamp>()
        .expect("ISO 8601 instant");
    assert!(
        time_created >= before_import,
        "time_created must be the local import instant ({before_import}), got {time_created}"
    );
    assert!(
        time_created > source_instant,
        "time_created must not report the source committal ({source_instant}), got {time_created}"
    );

    // REVISION_HISTORY_ITEM.audits: "there will always be at least one commit
    // audit" (RM common `revision_history_item.adoc` §Attributes) — the commit
    // audit of the version IN THIS CONTAINER, which for an imported version is
    // the IMPORTED_VERSION's own, i.e. the local import act.
    let history = target
        .ehr_status_revision_history(ehr)
        .await
        .expect("REVISION_HISTORY");
    let audit = &history["items"][0]["audits"][0];
    let logged = audit["time_committed"]["value"]
        .as_str()
        .expect("commit instant")
        .parse::<jiff::Timestamp>()
        .expect("ISO 8601 instant");
    assert!(
        logged >= before_import,
        "the revision history logs the local act ({before_import}), got {logged}"
    );
    assert_eq!(
        audit["change_type"]["defining_code"]["code_string"],
        json!("249"),
        "master06 §Contributions, import of item: 249|creation|"
    );
}

/// RM `ehr_extract` `x_versioned_object.adoc` §Attributes types
/// `X_VERSIONED_OBJECT.versions` as `List<ORIGINAL_VERSION<T>>`, and master06
/// §Copying keeps the received original "a faithful copy of its original, no
/// matter how many systems it may be copied through" — so re-exporting an
/// imported EHR reproduces the source's `ORIGINAL_VERSION` exactly, wrapper and
/// all local bookkeeping stripped. Regression for #1679, where the re-export
/// carried the LOCAL contribution reference under the source's identity.
#[tokio::test]
async fn a_re_export_reproduces_the_wrapped_original_verbatim() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    target
        .import_ehr(None, export_one(&source, ehr).await)
        .await
        .expect("import_ehr");
    let re_exported = target.extract_ehrs(ehr).await.expect("re-export");

    for xtype in ["X_VERSIONED_EHR_STATUS", "X_VERSIONED_FOLDER"] {
        let before =
            find_by_xtype(&exported[0], xtype).expect("source item")["item"]["versions"].clone();
        let after =
            find_by_xtype(&re_exported[0], xtype).expect("re-exported item")["item"]["versions"]
                .clone();
        assert_eq!(
            after, before,
            "{xtype}: the re-exported ORIGINAL_VERSION must equal the received one"
        );
    }
}

/// RM common master06 §Digital Signature: "If the object to be serialised is an
/// `IMPORTED_VERSION`, the process is the same — all attributes of the object
/// are serialised and then used to generate a signature. The result will be
/// that the `IMPORTED_VERSION` instance will carry its own signature which
/// signifies the act of importing and making available locally an
/// `ORIGINAL_VERSION` from another system."
///
/// So a deployment with signing on signs the wrapper it creates, and the
/// signature must verify against the wrapper the READ path rebuilds — under
/// `verify_on_read = strict` a byte drift between the two is a 5xx, which is
/// exactly what makes this test the proof. The wrapped original's own foreign
/// signature is stored verbatim and never re-verified.
#[tokio::test]
async fn a_signed_import_signs_the_wrapper_and_the_read_verifies_it() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Digest,
        key_path: None,
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_paths: Vec::new(),
        verify_on_read: Some(VerifyOnRead::Strict),
    };
    let signer = Signer::from_config(&config).expect("digest signer");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ))
    .with_signer(Arc::new(signer));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    let source_version = find_by_xtype(&exported[0], "X_VERSIONED_EHR_STATUS")
        .expect("exported EHR_STATUS")["item"]["versions"][0]
        .clone();
    let source_uid = source_version["uid"]["value"]
        .as_str()
        .expect("source version uid")
        .to_owned();
    let (vo_id, version) = {
        let mut parts = source_uid.splitn(3, "::");
        let vo: &str = parts.next().expect("object_id");
        let _system = parts.next().expect("creating_system_id");
        (
            ferroehr::ids::VoId(vo.parse().expect("vo uuid")),
            parts.next().expect("version_tree_id").to_owned(),
        )
    };

    target
        .import_ehr(None, export_one(&source, ehr).await)
        .await
        .expect("import_ehr under a signing deployment");

    // The read is the assertion: under `strict`, a wrapper signature that did
    // not recompute over the served bytes would be an Exception, not a body.
    let served = target
        .ehr_status_version_envelope(ehr, vo_id, &version)
        .await
        .expect("the signed IMPORTED_VERSION verifies on read");
    assert_eq!(served["_type"], json!("IMPORTED_VERSION"));
    assert!(
        served["signature"].as_str().is_some_and(|s| !s.is_empty()),
        "the import act signs the wrapper it creates, got {served:#}"
    );
    // The wrapped original keeps whatever signature it arrived with — here the
    // source deployment signed nothing, so it carries none, and it is certainly
    // not this server's wrapper signature.
    assert_ne!(
        served["item"].get("signature"),
        served.get("signature"),
        "the wrapper signature must not be copied onto the wrapped original"
    );
}

/// A cloned EHR keeps the SOURCE `ehr_id` but takes the RECEIVING system's
/// `system_id` — on both routes that can produce a clone.
///
/// RM ehr `master04-ehr_package.adoc` §EHR Identifier Allocation states both
/// halves in one paragraph: "the EHR id should be a copy of the EHR id from the
/// other institution. In all cases, the `EHR._system_id_` value should be set
/// to the value that would normally be used for locally created EHRs. In the
/// case of creating a cloned EHR, the `_system_id_` is from the receiving
/// (cloning) system." RM common `master06-change_control_package.adoc` §Copying
/// says the same of the id half — "the newly created EHR should re-use the EHR
/// identifier from the source system. This establishes the new EHR as an
/// intentional clone."
///
/// The two routes are the EHR-Extract import and the released
/// client-supplied-id creation (the only released affordance for the clone),
/// and the source here runs under a DIFFERENT system id, so a regression that
/// copied the source's value would be visible rather than accidentally equal.
#[tokio::test]
async fn a_cloned_ehr_keeps_the_source_ehr_id_but_takes_the_local_system_id() {
    const SOURCE_SYSTEM: &str = "sysA.example.org";
    const TARGET_SYSTEM: &str = "sysB.example.org";

    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ))
    .with_system_id(SOURCE_SYSTEM);
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ))
    .with_system_id(TARGET_SYSTEM);

    let ehr = seed_ehr(&source).await;
    let source_summary = source.get_ehr(ehr).await.expect("source summary");
    assert_eq!(
        source_summary.system_id, SOURCE_SYSTEM,
        "fixture: the source EHR really was created by the other system"
    );

    // Route 1 — the EHR-Extract import (master06 §Copying's first situation).
    target
        .import_ehr(None, export_one(&source, ehr).await)
        .await
        .expect("import_ehr");
    let clone = target.get_ehr(ehr).await.expect("clone summary");
    assert_eq!(
        clone.ehr_id, source_summary.ehr_id,
        "the clone re-uses the source EHR identifier"
    );
    assert_eq!(
        clone.system_id, TARGET_SYSTEM,
        "EHR.system_id is the receiving (cloning) system's, never the source's"
    );

    // Route 2 — the released client-supplied-id creation, into another
    // repository: the same id, and again the local system id.
    let third_db = testkit::db().await.expect("testkit database");
    let third = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &third_db.pool(),
    ))
    .with_system_id(TARGET_SYSTEM);
    third
        .create_ehr_with_id(ehr, None)
        .await
        .expect("create with a client-supplied id");
    let by_id = third.get_ehr(ehr).await.expect("summary");
    assert_eq!(by_id.ehr_id, source_summary.ehr_id);
    assert_eq!(
        by_id.system_id, TARGET_SYSTEM,
        "a client-supplied EHR id never carries a foreign system_id with it"
    );
}

/// The as-of illusion RM common `master06-change_control_package.adoc` §Copying
/// argues against at length: "a query for the state of a Version container at
/// earlier commit times correctly returns what information existed at that time
/// in that container, rather than giving the illusion that recently copied
/// Versions were there earlier than the time of local committal (as would occur
/// if the original commit time of the `ORIGINAL_VERSION` object was used for
/// comparison purposes in such queries)."
///
/// So an as-of read at an instant BEFORE the local import must not see the
/// imported version, even though the wrapped original's own commit audit
/// records an earlier instant — which it still does, and this test asserts
/// both halves so a server that simply discarded the source chronology would
/// fail it too.
#[tokio::test]
async fn an_as_of_read_before_the_import_does_not_see_the_imported_version() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    // A plain modifiable source EHR (the seeded fixture ends non-modifiable).
    let ehr = source.create_ehr(None).await.expect("source ehr");
    let composition_uid = source
        .create_composition(ehr, uv(&composition("as-of"), "249", None))
        .await
        .expect("source composition")
        .version_uid();
    let vo_id: ferroehr::ids::VoId = composition_uid
        .split("::")
        .next()
        .expect("object_id")
        .parse()
        .expect("vo uuid");
    let source_instant = source
        .composition_version_envelope(ehr, composition_uid.parse().expect("ovid"))
        .await
        .expect("source version")["commit_audit"]["time_committed"]["value"]
        .as_str()
        .expect("source commit instant")
        .to_owned();

    // Everything the source did is now in the past; the import has not run.
    let before_import = db_now(&target_db.pool()).await.to_string();

    target
        .import_ehr(None, export_one(&source, ehr).await)
        .await
        .expect("import_ehr");

    // At the pre-import instant the container held nothing locally.
    let illusion = target
        .get_composition_at_time(ehr, vo_id, Some(before_import.clone()))
        .await;
    assert!(
        illusion.is_err(),
        "an as-of read before the local committal must not see the copied version, got {illusion:?}"
    );
    // …while an as-of read now does see it.
    target
        .get_composition_at_time(ehr, vo_id, None)
        .await
        .expect("the imported version is current now");
    // The control: the source's own committal instant IS retained, inside the
    // wrapped original (master06 §Committal and Audits), and it precedes the
    // instant the as-of read just refused — so the refusal is chronology, not
    // discarded provenance.
    let served = target
        .composition_version_envelope(ehr, composition_uid.parse().expect("ovid"))
        .await
        .expect("imported version");
    assert_eq!(
        served["item"]["commit_audit"]["time_committed"]["value"],
        json!(source_instant),
        "the wrapped original keeps its own (earlier) committal instant"
    );
    assert!(
        source_instant.as_str() < before_import.as_str(),
        "fixture: the source committed before the instant the as-of read refused"
    );
}

/// RM common `master06-change_control_package.adoc` §Committal and Audits:
/// "the `ORIGINAL_VERSION` instance is never modified - it remains a faithful
/// copy of its original, no matter how many systems it may be copied through",
/// and each receiving system commits its own `IMPORTED_VERSION` wrapper whose
/// `commit_audit`/`contribution` "record the local act of committal".
///
/// Two hops therefore produce TWO distinct local wrappers over ONE unchanged
/// original — never a wrapper wrapping a wrapper, and never a third rewriting
/// of the original.
#[tokio::test]
async fn a_two_hop_copy_wraps_at_each_system_over_one_unchanged_original() {
    let db_a = testkit::db().await.expect("testkit database");
    let system_a = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &db_a.pool(),
    ))
    .with_system_id("sysA.example.org");
    let db_b = testkit::db().await.expect("testkit database");
    let system_b = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &db_b.pool(),
    ))
    .with_system_id("sysB.example.org");
    let db_c = testkit::db().await.expect("testkit database");
    let system_c = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &db_c.pool(),
    ))
    .with_system_id("sysC.example.org");

    let ehr = seed_ehr(&system_a).await;
    let original = find_by_xtype(
        &system_a.extract_ehrs(ehr).await.expect("export A")[0],
        "X_VERSIONED_EHR_STATUS",
    )
    .expect("exported EHR_STATUS")["item"]["versions"][0]
        .clone();
    let uid = original["uid"]["value"].as_str().expect("uid").to_owned();
    let (vo_id, version) = {
        let mut parts = uid.splitn(3, "::");
        let vo: &str = parts.next().expect("object_id");
        let _system = parts.next().expect("creating_system_id");
        (
            ferroehr::ids::VoId(vo.parse().expect("vo uuid")),
            parts.next().expect("version_tree_id").to_owned(),
        )
    };

    // Hop 1: A → B.
    system_b
        .import_ehr(None, export_one(&system_a, ehr).await)
        .await
        .expect("import into B");
    let at_b = system_b
        .ehr_status_version_envelope(ehr, vo_id, &version)
        .await
        .expect("the version at B");

    // Hop 2: B → C, from B's own re-export.
    system_c
        .import_ehr(None, export_one(&system_b, ehr).await)
        .await
        .expect("import into C");
    let at_c = system_c
        .ehr_status_version_envelope(ehr, vo_id, &version)
        .await
        .expect("the version at C");

    for (system, served) in [("B", &at_b), ("C", &at_c)] {
        assert_eq!(
            served["_type"],
            json!("IMPORTED_VERSION"),
            "system {system} wraps the received version in its own IMPORTED_VERSION"
        );
        assert_eq!(
            served["item"]["_type"],
            json!("ORIGINAL_VERSION"),
            "system {system} wraps the ORIGINAL, never another wrapper"
        );
        assert_eq!(
            served["item"], original,
            "system {system} must carry the original unchanged, however many hops it took"
        );
    }
    // Each hop's wrapper records its OWN act of committal.
    assert_ne!(
        at_b["contribution"]["id"]["value"], at_c["contribution"]["id"]["value"],
        "the two wrappers name two different local CONTRIBUTIONs"
    );
    assert_ne!(
        at_b["commit_audit"]["time_committed"]["value"],
        at_c["commit_audit"]["time_committed"]["value"],
        "the two wrappers record two different local committal instants"
    );
}

/// The `is_original_version(a_ver_id)` half of the
/// `VERSIONED_OBJECT.commit_attestation` precondition
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.versioned_object.adoc`
/// §Functions: "Attestations can only be added to Original versions"): a
/// `666|attestation|` CONTRIBUTION member naming an `IMPORTED_VERSION` target is
/// refused, with the violation carried as data. The accepting twin — a 666
/// member naming an `ORIGINAL_VERSION` — is
/// `service_contribution::accompanying_attestation_then_standalone_666_attestation`.
#[tokio::test]
async fn attesting_an_imported_version_is_refused() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    let imported_uid = find_by_xtype(&exported[0], "X_VERSIONED_EHR_STATUS")
        .expect("exported EHR_STATUS")["item"]["versions"][0]["uid"]["value"]
        .as_str()
        .expect("source version uid")
        .to_owned();

    target
        .import_ehr(None, export_one(&source, ehr).await)
        .await
        .expect("import_ehr");

    let attest_contribution = json!({
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } }, "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" } }, "versions": [{
            "preceding_version_uid": { "value": imported_uid },
            "lifecycle_state": {
                "_type": "DV_CODED_TEXT",
                "value": "complete",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "532"
                }
            },
            "commit_audit": {
                "change_type": {
                    "_type": "DV_CODED_TEXT",
                    "value": "attestation",
                    "defining_code": {
                        "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                        "code_string": "666"
                    }
                },
                "committer": {
                    "_type": "PARTY_IDENTIFIED",
                    "name": "senior reviewer"
                },
                "reason": { "_type": "DV_TEXT", "value": "authorised" },
                "is_pending": false
            }
        }]
    });
    let err = target
        .create_ehr_contribution(ehr, attest_contribution)
        .await
        .expect_err("attesting an IMPORTED_VERSION must be refused");
    // The SM seam carries status + message (the documented lossy seam,
    // #1729); the status class and both violation facts must survive.
    assert_eq!(
        err.status,
        CallStatusType::ContentInvalid,
        "the refusal is the semantic 422 class, got {err:?}"
    );
    assert!(
        err.message.contains("preceding_version_uid"),
        "the refusal names the target-bearing member, got: {}",
        err.message
    );
    assert!(
        err.message.contains("VERSIONED_OBJECT.commit_attestation"),
        "the refusal names the precondition, got: {}",
        err.message
    );
}

/// Rewrite the `version_tree_id` of the `X_VERSIONED_EHR_STATUS` version whose
/// tree is `from` to `to`, returning the re-decoded extract. The uid keeps its
/// `object_id`/`creating_system_id`; only the tree segment moves — the fixture
/// tool for the copy-closure twins below.
fn retree_status_version(extract: &Value, from: &str, to: &str) -> Extract {
    let mut raw = extract.clone();
    let items = raw["chapters"][0]["items"]
        .as_array_mut()
        .expect("chapter items");
    items.retain(|it| it["item"]["_type"] == json!("X_VERSIONED_EHR_STATUS"));
    let mut hit = false;
    for it in items {
        for v in it["item"]["versions"].as_array_mut().expect("versions") {
            let uid = v["uid"]["value"].as_str().expect("uid").to_owned();
            if let Some(prefix) = uid.strip_suffix(&format!("::{from}")) {
                v["uid"]["value"] = json!(format!("{prefix}::{to}"));
                hit = true;
            }
        }
    }
    assert!(hit, "fixture: no status version with tree '{from}' found");
    openehr_its::json::from_canonical_value(&raw).expect("re-treed EXTRACT decodes")
}

/// The copy closure, refusal twin (a): RM common master06 §Copying — branch
/// versions "cannot be copied without their corresponding preceding versions
/// on the same branch (if any) and trunk versions also being copied". An
/// extract carrying branch version `2.1.1` without trunk version `2` (neither
/// in the extract nor stored) is refused as a precondition violation.
#[tokio::test]
async fn import_refuses_a_branch_version_without_its_fork_point_trunk() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    // Status versions 1 and 2 exist; re-tree 2 → 2.1.1: its fork-point trunk
    // (version 2 itself) is now absent from the set and nothing is stored.
    let mutated = retree_status_version(&exported[0], "2", "2.1.1");

    let err = target
        .import_ehr(None, mutated)
        .await
        .expect_err("a branch version without its fork-point trunk must be refused");
    assert_eq!(err.status, CallStatusType::PreconditionViolation);
    assert!(
        err.message.contains("fork-point trunk"),
        "the refusal names the missing trunk, got: {}",
        err.message
    );
}

/// The copy closure, refusal twin (b): branch version `2.1.2` without its
/// same-branch predecessor `2.1.1` (master06 §Copying) is refused even though
/// its fork-point trunk (version 2) is already stored from the first receipt
/// (Case-3 append).
#[tokio::test]
async fn import_refuses_a_branch_version_without_its_branch_predecessor() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    let ehr_id = target
        .import_ehr(
            None,
            openehr_its::json::from_canonical_value(&exported[0]).expect("EXTRACT decodes"),
        )
        .await
        .expect("first receipt lands the trunk");

    let err = target
        .import_ehr_extract(ehr_id, retree_status_version(&exported[0], "2", "2.1.2"))
        .await
        .expect_err("a branch version without its same-branch predecessor must be refused");
    assert_eq!(err.status, CallStatusType::PreconditionViolation);
    assert!(
        err.message.contains("same-branch"),
        "the refusal names the missing predecessor, got: {}",
        err.message
    );
}

/// The copy closure, accepting twin: branch version `2.1.1` whose fork-point
/// trunk (version 2) is already stored from the first receipt satisfies
/// master06 §Copying — the Case-3 append lands and the branch version reads
/// back as an `IMPORTED_VERSION`.
#[tokio::test]
async fn import_accepts_a_branch_version_whose_closure_is_already_stored() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    let status_uid = find_by_xtype(&exported[0], "X_VERSIONED_EHR_STATUS")
        .expect("exported EHR_STATUS")["item"]["versions"][0]["uid"]["value"]
        .as_str()
        .expect("uid")
        .to_owned();
    let vo_id = ferroehr::ids::VoId(
        status_uid
            .split("::")
            .next()
            .expect("object_id")
            .parse()
            .expect("vo uuid"),
    );
    let ehr_id = target
        .import_ehr(
            None,
            openehr_its::json::from_canonical_value(&exported[0]).expect("EXTRACT decodes"),
        )
        .await
        .expect("first receipt lands the trunk");

    target
        .import_ehr_extract(ehr_id, retree_status_version(&exported[0], "2", "2.1.1"))
        .await
        .expect("a closure-satisfied branch append lands");
    let served = target
        .ehr_status_version_envelope(ehr_id, vo_id, "2.1.1")
        .await
        .expect("the imported branch version reads back");
    assert_eq!(served["_type"], json!("IMPORTED_VERSION"));
}

/// A later receipt may also ADVANCE an already-held branch lineage: with
/// `2.1.1` stored and open, importing `2.1.2` closes the stored tip and lands
/// as the lineage's one open row (RM common master06 §Copying, Case-3 append;
/// §Semantics in Distributed Systems keeps lineages coexisting). A stale
/// re-receipt of `2.1.1` is then a conflict, mirroring the trunk
/// strictly-newer rule. Regression: the branch append never closed the open
/// stored tip, so the insert died on the one-open-row-per-lineage index and
/// surfaced as a bare storage conflict.
#[tokio::test]
async fn import_advances_an_open_branch_lineage_and_rejects_a_stale_branch_version() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let ehr = seed_ehr(&source).await;
    let exported = source.extract_ehrs(ehr).await.expect("export");
    let status_uid = find_by_xtype(&exported[0], "X_VERSIONED_EHR_STATUS")
        .expect("exported EHR_STATUS")["item"]["versions"][0]["uid"]["value"]
        .as_str()
        .expect("uid")
        .to_owned();
    let vo_id = ferroehr::ids::VoId(
        status_uid
            .split("::")
            .next()
            .expect("object_id")
            .parse()
            .expect("vo uuid"),
    );
    let ehr_id = target
        .import_ehr(
            None,
            openehr_its::json::from_canonical_value(&exported[0]).expect("EXTRACT decodes"),
        )
        .await
        .expect("first receipt lands the trunk");
    target
        .import_ehr_extract(ehr_id, retree_status_version(&exported[0], "2", "2.1.1"))
        .await
        .expect("the branch opens");

    // The branch advance: `2.1.2` supersedes the open stored `2.1.1`.
    target
        .import_ehr_extract(ehr_id, retree_status_version(&exported[0], "2", "2.1.2"))
        .await
        .expect("a branch advance over an open stored tip lands");
    let served = target
        .ehr_status_version_envelope(ehr_id, vo_id, "2.1.2")
        .await
        .expect("the advanced branch version reads back");
    assert_eq!(served["_type"], json!("IMPORTED_VERSION"));

    // The stale tip is refused by name, exactly like a stale trunk re-import.
    let err = target
        .import_ehr_extract(ehr_id, retree_status_version(&exported[0], "2", "2.1.1"))
        .await
        .expect_err("a stale branch version must be refused");
    assert_eq!(err.status, CallStatusType::Conflict);
    assert!(
        err.message.contains("already has branch version"),
        "the refusal names the stored tip, got: {}",
        err.message
    );
}

/// A locally-held PARTY referenced from exported content rides the extract's
/// demographics chapter and lands in the receiver's demographic domain.
///
/// RM EHR Extract `master09-semantics.adoc` §Creation Semantics: "Create a
/// demographics `EXTRACT_CHAPTER` and write the `PARTYs` in", and on receipt
/// the parties are committed like any other received container. The two
/// pseudonymisation domains are separate schemas reached by separate pools, so
/// this is also what proves the extract path did not lose the parties when they
/// left `ehr`: the party has to be READ out of the source's demographic domain
/// and WRITTEN into the target's, and a routing miss fails loudly at the
/// boundary CHECK rather than quietly.
#[tokio::test]
async fn an_extract_carries_its_parties_into_the_receivers_demographic_domain() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let created = source
        .party_create(PartyKind::Person, typed(&extract_person()), None)
        .await
        .expect("create the referenced person");
    let party: uuid::Uuid = created.body["uid"]["value"]
        .as_str()
        .expect("uid.value")
        .split("::")
        .next()
        .expect("the versioned-object uuid")
        .parse()
        .expect("a uuid");

    // An EHR whose EHR_STATUS names that party as its subject, in the
    // `demographic` namespace the extract builder follows.
    let ehr = source.create_ehr(None).await.expect("ehr");
    let mut status = source
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("status get");
    let status_ovid = status["uid"]["value"].as_str().expect("uid").to_owned();
    status.as_object_mut().expect("status obj").remove("uid");
    status["subject"] = json!({
        "_type": "PARTY_SELF",
        "external_ref": {
            "_type": "PARTY_REF",
            "namespace": "demographic",
            "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": party.to_string() }
        }
    });
    source
        .replace_ehr_status(ehr, uv(&status, "251", Some(&status_ovid)))
        .await
        .expect("status update");

    let exported = source.extract_ehrs(ehr).await.expect("export_ehrs");
    assert_eq!(exported.len(), 1, "one EHR id → one EXTRACT");
    let carried = exported[0]["chapters"]
        .as_array()
        .expect("chapters")
        .iter()
        .filter_map(|c| c["items"].as_array())
        .flatten()
        .filter(|it| it["item"]["_type"] == json!("X_VERSIONED_PARTY"))
        .count();
    assert_eq!(
        carried, 1,
        "the demographics chapter carries the referenced party: {}",
        exported[0]
    );

    let extract: Extract = openehr_its::json::from_canonical_value(&exported[0])
        .expect("EXTRACT deserializes into the typed RM model");
    target.import_ehr(None, extract).await.expect("import_ehr");

    let in_demographic: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.version WHERE vo_id = $1 AND kind = 'PERSON'",
    )
    .bind(party)
    .fetch_one(&target_db.pool())
    .await
    .expect("count the received party");
    assert_eq!(
        in_demographic, 1,
        "the received party is in the target's demographic domain"
    );
    assert_eq!(
        target
            .party_get(PartyKind::Person, party.to_string(), None)
            .await
            .expect("the received party reads back")
            .body["_type"],
        json!("PERSON"),
        "and it serves through the demographic surface"
    );
}

/// The PERSON the extract test references, authored as canonical JSON exactly
/// as a client would post it (`.claude/rules/testing.md` §Test-fixture
/// construction, class 2).
fn extract_person() -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0"
        },
        "name": { "_type": "DV_TEXT", "value": "Grace Hopper" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": "Hopper" }
                }]
            }
        }]
    })
}
