// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end tests of the `spec_profile` compatibility stamp and its
//! read-time gate, against a real PostgreSQL 18 (shared testkit harness).
//!
//! No openEHR spec governs runtime specification-generation selection — our own
//! design/extension. The development-only construct these tests commit is
//! `GENERIC_ENTRY.data` holding a `CLUSTER`: RM 1.1.0 types that attribute
//! `ITEM_TREE`, and SPECRM-18 retyped it to the abstract `ITEM`
//! (= `CLUSTER` | `ELEMENT`) after that release —
//! `RM/docs/integration/master00-amendment_record.adoc`, issue 1.0, listed
//! above the `RM Release 1.1.0` marker. The two types are disjoint, so a
//! `CLUSTER` there is exactly a body the development generation reads and the
//! released one cannot.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    let_underscore_drop,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              RESULT_SET row indexing are the intended shape here (the Rust Book \
              ch11), and the archive temp-dir cleanup is best-effort — its failure \
              is not a test outcome"
)]

use serde_json::{Value, json};

use ferroehr::config::profile::SpecProfile;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::types::ExportSpec;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::status::{CallStatusType, SmError};

use crate::fixtures::uv;

/// The shared minimal valid RM COMPOSITION, carrying the given `content` list.
///
/// The generation delta these tests probe lives entirely in `content`, so the
/// suite adds it to the shared fixture rather than restating the mandatory
/// attributes.
fn composition(name: &str, content: &Value) -> Value {
    let mut composition = crate::fixtures::composition(name);
    composition["content"] = content.clone();
    composition
}

/// A COMPOSITION whose content both generation sets express.
fn stable_clean_composition() -> Value {
    composition(
        "released surface",
        &json!([ {
            "_type": "SECTION",
            "name": { "_type": "DV_TEXT", "value": "section" },
            "archetype_node_id": "openEHR-EHR-SECTION.adhoc.v1"
        } ]),
    )
}

/// A COMPOSITION whose content only the DEVELOPMENT generation set expresses
/// (see the module docs: `GENERIC_ENTRY.data` as a `CLUSTER`).
fn development_only_composition() -> Value {
    composition(
        "development surface",
        &json!([ {
            "_type": "GENERIC_ENTRY",
            "name": { "_type": "DV_TEXT", "value": "entry" },
            "archetype_node_id": "openEHR-EHR-GENERIC_ENTRY.msg.v1",
            "data": {
                "_type": "CLUSTER",
                "name": { "_type": "DV_TEXT", "value": "data" },
                "archetype_node_id": "at0000",
                "items": [ {
                    "_type": "ELEMENT",
                    "name": { "_type": "DV_TEXT", "value": "leaf" },
                    "archetype_node_id": "at0001",
                    "value": { "_type": "DV_TEXT", "value": "x" }
                } ]
            }
        } ]),
    )
}

/// The stored `version.stable_compatible` of an object's only version.
async fn stamp(pool: &sqlx::PgPool, vo_id: VoId) -> Option<bool> {
    sqlx::query_scalar::<_, Option<bool>>("SELECT stable_compatible FROM version WHERE vo_id = $1")
        .bind(vo_id)
        .fetch_one(pool)
        .await
        .expect("the stamp column is readable")
}

/// A `stable`-profile service over the same database.
fn stable_service(pool: &sqlx::PgPool) -> FerroEhrService {
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_spec_profile(SpecProfile::Stable)
}

/// Commit one COMPOSITION into a fresh EHR under the DEVELOPMENT profile.
async fn commit(svc: &FerroEhrService, body: &Value) -> (EhrId, VoId) {
    let ehr_id = svc.create_ehr(None).await.expect("ehr_create");
    let committed = svc
        .create_composition(ehr_id, uv(body, "249", None))
        .await
        .expect("composition create");
    (ehr_id, committed.vo_id)
}

/// A body the released generations express stamps `true` at commit and is
/// served under BOTH profiles.
#[tokio::test]
async fn a_released_surface_composition_stamps_true_and_reads_under_both_profiles() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr_id, vo_id) = commit(&svc, &stable_clean_composition()).await;

    assert_eq!(
        stamp(&db.pool(), vo_id).await,
        Some(true),
        "a released-surface body is stamped stable-compatible at commit"
    );

    let under_development = svc
        .get_composition_latest(ehr_id, vo_id)
        .await
        .expect("served under the development profile");
    assert_eq!(under_development["_type"], "COMPOSITION");

    let under_stable = stable_service(&db.pool())
        .get_composition_latest(ehr_id, vo_id)
        .await
        .expect("served under the stable profile");
    assert_eq!(under_stable, under_development);
}

/// A body only the development generations express stamps `false` at commit,
/// is served under `development`, and is a `409`-class conflict under
/// `stable` — naming the profile, the version and the remedy.
#[tokio::test]
async fn a_development_only_composition_stamps_false_and_is_refused_under_stable() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr_id, vo_id) = commit(&svc, &development_only_composition()).await;

    assert_eq!(
        stamp(&db.pool(), vo_id).await,
        Some(false),
        "a development-only body is stamped NOT stable-compatible at commit"
    );

    let served = svc
        .get_composition_latest(ehr_id, vo_id)
        .await
        .expect("served under the development profile that accepted it");
    assert_eq!(served["content"][0]["data"]["_type"], "CLUSTER");

    let refused = stable_service(&db.pool())
        .get_composition_latest(ehr_id, vo_id)
        .await;
    let Err(SmError {
        status: CallStatusType::Conflict,
        message,
        ..
    }) = refused
    else {
        panic!("the stable profile must refuse with a conflict, got {refused:?}");
    };
    assert!(message.contains("stable"), "{message}");
    assert!(message.contains("development"), "{message}");
    assert!(
        message.contains(&vo_id.to_string()),
        "the refusal names the version: {message}"
    );
}

/// An UNSTAMPED (`NULL`) row — one committed before the column existed, or
/// written by a verbatim-replay path — is assessed on the fly at read, in both
/// directions, and the read never writes the answer back.
#[tokio::test]
async fn an_unstamped_row_is_assessed_on_the_fly() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (clean_ehr, clean_vo) = commit(&svc, &stable_clean_composition()).await;
    let (dirty_ehr, dirty_vo) = commit(&svc, &development_only_composition()).await;

    sqlx::query("UPDATE version SET stable_compatible = NULL WHERE vo_id = ANY($1)")
        .bind(vec![clean_vo, dirty_vo])
        .execute(&db.pool())
        .await
        .expect("unstamp the two rows");

    let stable = stable_service(&db.pool());
    stable
        .get_composition_latest(clean_ehr, clean_vo)
        .await
        .expect("an unstamped released-surface body is assessed and served");
    let refused = stable.get_composition_latest(dirty_ehr, dirty_vo).await;
    assert!(
        matches!(
            refused,
            Err(SmError {
                status: CallStatusType::Conflict,
                ..
            })
        ),
        "an unstamped development-only body is assessed and refused, got {refused:?}"
    );

    assert_eq!(
        stamp(&db.pool(), dirty_vo).await,
        None,
        "a read never writes the assessment back — reads stay pure"
    );
}

/// The stamp travels with the rows through the cold archival tier: archiving
/// and restoring an EHR must not change a version's profile compatibility.
#[tokio::test]
async fn archive_and_restore_preserve_the_stamp() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr_id, vo_id) = commit(&svc, &development_only_composition()).await;
    assert_eq!(stamp(&db.pool(), vo_id).await, Some(false));

    svc.archive_ehrs(vec![ehr_id.to_string()])
        .await
        .expect("archive the EHR");
    let cold: Option<bool> = sqlx::query_scalar::<_, Option<bool>>(
        "SELECT stable_compatible FROM version_cold WHERE vo_id = $1",
    )
    .bind(vo_id)
    .fetch_one(&db.pool())
    .await
    .expect("the archived row carries the column");
    assert_eq!(cold, Some(false), "the cold mirror carries the stamp");

    // An archived object stays retrievable, and the gate still applies to it.
    let refused = stable_service(&db.pool())
        .get_composition_latest(ehr_id, vo_id)
        .await;
    assert!(
        matches!(
            refused,
            Err(SmError {
                status: CallStatusType::Conflict,
                ..
            })
        ),
        "the cold-tier read is gated exactly like the primary one, got {refused:?}"
    );

    svc.restore_archived_ehrs(vec![ehr_id.to_string()])
        .await
        .expect("restore the EHR");
    assert_eq!(
        stamp(&db.pool(), vo_id).await,
        Some(false),
        "the restored row carries the stamp it was archived with"
    );
    svc.get_composition_latest(ehr_id, vo_id)
        .await
        .expect("the restored object reads under the development profile");
}

/// A unique temporary directory for one dump archive.
fn archive_dir() -> String {
    std::env::temp_dir()
        .join(format!(
            "ferroehr-profile-dumpload-{}",
            uuid::Uuid::now_v7()
        ))
        .to_string_lossy()
        .into_owned()
}

/// The stamp survives the ADMIN dump/load round trip: a development-only body
/// exported from one repository and loaded into a fresh one is still refused
/// under `stable` there, and still served under `development`.
///
/// Dump/load is the one path that reconstructs a version in a database that
/// never saw the commit (SM `I_ADMIN_DUMP_LOAD.export_ehrs`/`load_ehrs`,
/// `docs/specs/openehr/SM/docs/UML/classes/i_admin_dump_load.adoc`), so it is
/// where an inherited `stable_compatible = true` would be a claim nothing made:
/// the loaded rows must never read as compatible with a generation set that
/// cannot express them.
#[tokio::test]
async fn dump_and_load_into_a_fresh_repository_preserve_the_stamp() {
    let source_db = testkit::db().await.expect("testkit database");
    let target_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let (ehr_id, vo_id) = commit(&source, &development_only_composition()).await;
    assert_eq!(stamp(&source_db.pool(), vo_id).await, Some(false));

    let dir = archive_dir();
    let export_reports = source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1024))
        .await
        .expect("export the EHR carrying the development-only body");
    assert!(
        export_reports.is_empty(),
        "a clean export reports no failures, got {export_reports:?}"
    );
    let load_reports = target
        .load_ehrs(dir.clone())
        .await
        .expect("load into the fresh repository");
    assert!(
        load_reports.is_empty(),
        "loading into an empty repository reports no failures, got {load_reports:?}"
    );

    // The loaded row never claims a compatibility the body does not have —
    // whether the archive carried the stamp or the read assesses it on the fly.
    assert_ne!(
        stamp(&target_db.pool(), vo_id).await,
        Some(true),
        "a loaded development-only body must not read as stable-compatible"
    );

    // The profile gate lands the same way in the repository that never saw the
    // commit: served under `development`, refused under `stable`.
    let served = target
        .get_composition_latest(ehr_id, vo_id)
        .await
        .expect("the loaded body is served under the development profile");
    assert_eq!(served["content"][0]["data"]["_type"], "CLUSTER");

    let refused = stable_service(&target_db.pool())
        .get_composition_latest(ehr_id, vo_id)
        .await;
    let Err(SmError {
        status: CallStatusType::Conflict,
        message,
        ..
    }) = refused
    else {
        panic!("the stable profile must refuse the loaded body, got {refused:?}");
    };
    assert!(message.contains("stable"), "{message}");
    assert!(
        message.contains(&vo_id.to_string()),
        "the refusal names the loaded version: {message}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The mirror case: a released-surface body dumped and loaded is still served
/// under BOTH profiles in the fresh repository — the gate above narrows nothing
/// it should not.
#[tokio::test]
async fn dump_and_load_keep_a_released_surface_body_readable_under_stable() {
    let source_db = testkit::db().await.expect("testkit database");
    let target_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ));

    let (ehr_id, vo_id) = commit(&source, &stable_clean_composition()).await;
    let dir = archive_dir();
    source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1024))
        .await
        .expect("export");
    target.load_ehrs(dir.clone()).await.expect("load");

    assert_ne!(
        stamp(&target_db.pool(), vo_id).await,
        Some(false),
        "a loaded released-surface body must not read as stable-INcompatible"
    );
    let under_development = target
        .get_composition_latest(ehr_id, vo_id)
        .await
        .expect("served under the development profile");
    let under_stable = stable_service(&target_db.pool())
        .get_composition_latest(ehr_id, vo_id)
        .await
        .expect("served under the stable profile");
    assert_eq!(under_stable, under_development);

    let _ = std::fs::remove_dir_all(&dir);
}

// ── AQL: whole-object projections take the same gate; leaf projections do not ─

/// Scope a query to one EHR (`i_query_service.adoc` `ehr_ids: List<UUID>`; the
/// single-EHR REST scope is the one-element case).
fn ehr_scope(ehr_id: EhrId) -> AqlQueryRequest {
    AqlQueryRequest {
        ehr_ids: vec![ehr_id.to_string()],
        ..AqlQueryRequest::default()
    }
}

/// The `RESULT_SET` rows of a successful query.
async fn query_rows(svc: &FerroEhrService, aql: &str, ehr_id: EhrId) -> Vec<Value> {
    let outcome = svc
        .execute_ad_hoc_query(aql.to_owned(), ehr_scope(ehr_id))
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"));
    outcome.result_set["rows"]
        .as_array()
        .expect("the RESULT_SET carries a rows array")
        .clone()
}

/// The refusal message of a query the active profile must not answer.
async fn query_conflict(svc: &FerroEhrService, aql: &str, ehr_id: EhrId) -> String {
    let refused = svc
        .execute_ad_hoc_query(aql.to_owned(), ehr_scope(ehr_id))
        .await;
    let Err(SmError {
        status: CallStatusType::Conflict,
        message,
        ..
    }) = refused
    else {
        panic!("the stable profile must refuse {aql:?} with a conflict, got {refused:?}");
    };
    message
}

const WHOLE_OBJECT_AQL: &str = "SELECT c FROM EHR e CONTAINS COMPOSITION c";

/// A whole-object projection serves a stored version BODY, so it takes the same
/// generation gate the resource reads take: under `stable` a development-only
/// stored body is the `409`-class conflict naming the version, while the
/// profile that accepted it still serves the row.
#[tokio::test]
async fn an_aql_whole_object_projection_is_gated_by_the_spec_profile() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr_id, vo_id) = commit(&svc, &development_only_composition()).await;

    let served = query_rows(&svc, WHOLE_OBJECT_AQL, ehr_id).await;
    assert_eq!(served.len(), 1, "the development profile serves the row");
    assert_eq!(served[0][0]["content"][0]["data"]["_type"], "CLUSTER");

    let message = query_conflict(&stable_service(&db.pool()), WHOLE_OBJECT_AQL, ehr_id).await;
    assert!(message.contains("stable"), "{message}");
    assert!(message.contains("development"), "{message}");
    assert!(
        message.contains(&vo_id.to_string()),
        "the refusal names the offending version: {message}"
    );
}

/// The gate narrows nothing it should not: the same whole-object projection
/// over a store of released-surface bodies serves identically under both
/// profiles.
#[tokio::test]
async fn an_aql_whole_object_projection_over_clean_rows_serves_under_both_profiles() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr_id, _) = commit(&svc, &stable_clean_composition()).await;

    let under_development = query_rows(&svc, WHOLE_OBJECT_AQL, ehr_id).await;
    let under_stable = query_rows(&stable_service(&db.pool()), WHOLE_OBJECT_AQL, ehr_id).await;
    assert_eq!(under_development.len(), 1);
    assert_eq!(under_stable, under_development);
}

/// The honest boundary: a LEAF projection over the very same store serves under
/// `stable`. It returns data values over paths the planning gate already bounded
/// to the released generation's declared surface — not a version body — so the
/// stamp does not apply to it.
#[tokio::test]
async fn an_aql_leaf_projection_is_not_gated_by_the_spec_profile() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr_id, _) = commit(&svc, &development_only_composition()).await;

    let stable = stable_service(&db.pool());
    let ehr_leaf = query_rows(&stable, "SELECT e/ehr_id/value FROM EHR e", ehr_id).await;
    assert_eq!(ehr_leaf.len(), 1, "the EHR leaf projection is served");
    assert_eq!(ehr_leaf[0][0], Value::String(ehr_id.to_string()));

    let name_leaf = query_rows(
        &stable,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c",
        ehr_id,
    )
    .await;
    assert_eq!(
        name_leaf.len(),
        1,
        "the COMPOSITION leaf projection is served"
    );
    assert_eq!(
        name_leaf[0][0],
        Value::String("development surface".to_owned())
    );
}

/// An UNSTAMPED (`NULL`) row reached by a whole-object projection is assessed on
/// the fly — batched with every other candidate of the page — exactly as the
/// resource read assesses it, and the read still writes nothing back.
#[tokio::test]
async fn the_query_gate_assesses_unstamped_rows_on_the_fly() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (clean_ehr, clean_vo) = commit(&svc, &stable_clean_composition()).await;
    let (dirty_ehr, dirty_vo) = commit(&svc, &development_only_composition()).await;

    sqlx::query("UPDATE version SET stable_compatible = NULL WHERE vo_id = ANY($1)")
        .bind(vec![clean_vo, dirty_vo])
        .execute(&db.pool())
        .await
        .expect("unstamp the two rows");

    let stable = stable_service(&db.pool());
    let served = query_rows(&stable, WHOLE_OBJECT_AQL, clean_ehr).await;
    assert_eq!(
        served.len(),
        1,
        "an unstamped released-surface body is assessed and served"
    );
    let message = query_conflict(&stable, WHOLE_OBJECT_AQL, dirty_ehr).await;
    assert!(
        message.contains(&dirty_vo.to_string()),
        "the on-the-fly assessment names the offending version: {message}"
    );
    assert_eq!(
        stamp(&db.pool(), dirty_vo).await,
        None,
        "a query never writes the assessment back — reads stay pure"
    );
}
