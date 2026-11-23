// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared fixtures for the service suites that drive a whole repository
//! (`service_admin`, `service_dump_load`, `service_relationship_index`,
//! `service_ehr_index_conflicts`, `service_message_audit`).

use ferroehr::service::ehr::object::EhrObject;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;

use crate::fixtures::{uid, uv, vo_of};

/// Returns a fresh migrated database, its pool, and a service over it.
pub(crate) async fn repository() -> (testkit::TestDb, PgPool, FerroEhrService) {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_ehr_index(EhrIndexProvider::Local);
    (db, pool, service)
}

/// Returns the `uid.value` (`OBJECT_VERSION_ID`) of a versioned-object body.
/// Seeds an EHR carrying multiple versioned-object kinds and versions.
///
/// The deactivating `EHR_STATUS` version lands LAST: content writes on an EHR
/// whose `EHR_STATUS.is_modifiable` is `false` are refused (RM ehr master04
/// §"EHR Active Status").
pub(crate) async fn seed_full_ehr(svc: &FerroEhrService) -> EhrId {
    let ehr_id = crate::fixtures::create_ehr(svc).await;

    let mut updated = svc
        .ehr_object(ehr_id, EhrObject::Status, None)
        .await
        .map(|r| r.body)
        .expect("status get");
    let status_ovid = uid(&updated).to_owned();
    let status_vo = vo_of(&status_ovid).to_owned();
    updated.as_object_mut().expect("status obj").remove("uid");
    svc.target_tags_replace(
        ehr_id,
        crate::fixtures::at(status_vo),
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
            &json!({ "_type": "FOLDER", "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1", "archetype_details": { "_type": "ARCHETYPED", "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.generic.v1" }, "rm_version": "1.1.0" }, "name": { "_type": "DV_TEXT", "value": "root" } }),
            "249",
            None,
        ),
    )
    .await
    .expect("directory");

    updated["is_modifiable"] = json!(false);
    Box::pin(svc.replace_ehr_status(ehr_id, uv(&updated, "251", Some(&status_ovid))))
        .await
        .map(|m| m.uid)
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
pub(crate) fn archive_dir() -> String {
    std::env::temp_dir()
        .join(format!("ferroehr-dumpload-{}", Uuid::now_v7()))
        .to_string_lossy()
        .into_owned()
}

/// Truncates `path` to its first half — readable bytes, unparseable content.
#[expect(clippy::integer_division, reason = "a deliberate midpoint truncation")]
pub(crate) fn truncate_to_half(path: &std::path::Path) {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|e| panic!("read {} for truncation: {e}", path.display()));
    let half = bytes.len() / 2;
    std::fs::write(path, bytes.get(..half).expect("a prefix of the read bytes"))
        .unwrap_or_else(|e| panic!("truncate {}: {e}", path.display()));
}
