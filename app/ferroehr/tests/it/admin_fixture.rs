// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared fixtures for the service suites that drive a whole repository
//! (`service_admin`, `service_dump_load`, `service_sm3`,
//! `service_ehr_index_conflicts`, `service_message_audit`).
//!
//! They all start from the same shape — a fresh migrated database from the
//! shared `testkit` harness, an EHR carrying several versioned-object kinds,
//! and the SM `UPDATE_VERSION` commit envelope — so the seeds and probes live
//! here once rather than once per suite.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;

use crate::fixtures::{uid, uv, vo_of};

/// Returns a fresh migrated database, its pool, and a service over it.
///
/// Hold the returned [`testkit::TestDb`] guard for the test's lifetime —
/// dropping it releases the template clone.
pub(crate) async fn repository() -> (testkit::TestDb, PgPool, FerroEhrService) {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    (db, pool, service)
}

/// Returns the `uid.value` (`OBJECT_VERSION_ID`) of a versioned-object body.
/// Seeds an EHR carrying multiple versioned-object kinds and versions.
///
/// `EHR_STATUS` (create → update = two versions) plus the `EHR_ACCESS` minted at
/// creation, an item tag on the `EHR_STATUS`, and a directory `FOLDER` — enough
/// to populate every `version`/`node`/`contribution`/`audit`/`item_tag`
/// relation. COMPOSITION is deliberately absent: it would need a template the
/// shared fixtures do not supply, and adds no relation the kinds above miss.
///
/// The deactivating `EHR_STATUS` version lands LAST: content writes on an EHR
/// whose `EHR_STATUS.is_modifiable` is `false` are refused (RM ehr master04
/// §"EHR Active Status").
pub(crate) async fn seed_full_ehr(svc: &FerroEhrService) -> EhrId {
    let ehr_id = svc.create_ehr(None).await.expect("ehr");

    let mut updated = svc
        .get_ehr_status_at_time(ehr_id, None)
        .await
        .expect("status get");
    let status_ovid = uid(&updated).to_owned();
    let status_vo = vo_of(&status_ovid).to_owned();
    updated.as_object_mut().expect("status obj").remove("uid");
    svc.target_tags_replace(
        ehr_id,
        status_vo,
        "EHR_STATUS",
        vec![crate::item_tag_fixture::ehr_tag(
            "priority",
            Some("high"),
            None,
        )],
    )
    .await
    .expect("tag");

    svc.create_directory(
        ehr_id,
        uv(
            &json!({ "_type": "FOLDER", "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1", "name": { "_type": "DV_TEXT", "value": "root" } }),
            "249",
            None,
        ),
    )
    .await
    .expect("directory");

    updated["is_modifiable"] = json!(false);
    svc.replace_ehr_status(ehr_id, uv(&updated, "251", Some(&status_ovid)))
        .await
        .expect("status update");

    ehr_id
}

/// Builds a minimal valid demographic PERSON whose legal-name identity details
/// carry `identity_items` (PARTY invariant `Identities_valid`).
pub(crate) fn person(name: &str, identity_items: &Value) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": identity_items.clone()
            }
        }]
    })
}

/// Builds a `PARTY_RELATIONSHIP` from `source` to `target` (bare
/// versioned-object ids).
pub(crate) fn party_relationship(name: &str, source: &str, target: &str) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "name": { "_type": "DV_TEXT", "value": name },
        "source": {
            "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": source }
        },
        "target": {
            "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": target }
        }
    })
}

/// Runs one EHR-scoped `count(*)` query, binding `ehr_id` to its single
/// parameter.
pub(crate) async fn count_for_ehr(pool: &PgPool, sql: &'static str, ehr_id: Uuid) -> i64 {
    sqlx::query_scalar(sql)
        .bind(ehr_id)
        .fetch_one(pool)
        .await
        .expect("count")
}

/// Returns a unique temporary directory path for one archive.
///
/// Best-effort cleaned up by the OS temp directory; no round trip depends on
/// removal.
pub(crate) fn archive_dir() -> String {
    std::env::temp_dir()
        .join(format!("ferroehr-dumpload-{}", Uuid::now_v7()))
        .to_string_lossy()
        .into_owned()
}

/// Truncates `path` to its first half — readable bytes, unparseable content.
///
/// The remainder the division drops is irrelevant: any prefix shorter than the
/// whole document is corrupt, which is the fixture's only requirement.
#[expect(clippy::integer_division, reason = "a deliberate midpoint truncation")]
pub(crate) fn truncate_to_half(path: &std::path::Path) {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|e| panic!("read {} for truncation: {e}", path.display()));
    let half = bytes.len() / 2;
    std::fs::write(path, bytes.get(..half).expect("a prefix of the read bytes"))
        .unwrap_or_else(|e| panic!("truncate {}: {e}", path.display()));
}
