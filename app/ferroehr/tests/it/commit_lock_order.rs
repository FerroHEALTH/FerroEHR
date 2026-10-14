// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The order a write takes its locks in.
//!
//! Two writers that take the same locks in different orders can deadlock, so
//! every multi-object writer takes the EHR row lock first and then the
//! per-object advisory locks in ascending `vo_id` order. Each test parks a
//! writer on the first lock it should take, while a blocker holds that lock,
//! and asserts the writer holds none of the locks it should take after it. No
//! openEHR spec governs the locking — our own storage design; the advisory-lock
//! semantics are PostgreSQL docs §13.3.5 Advisory Locks.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers; a failing fixture or wait panics there (the Rust \
              Book ch11)"
)]

use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::db::domain::DomainPools;
use ferroehr::service::FerroEhrService;

use crate::fixtures::{change_type, committer, composition, uv, vo_of};

/// The per-object lock the commit path takes, by the same key expression.
const ADVISORY_LOCK: &str = "SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))";

/// Whether another session can take the per-object lock of `vo_id` right now.
async fn advisory_lock_is_free(pool: &PgPool, vo_id: Uuid) -> bool {
    let mut probe = pool.begin().await.expect("probe transaction");
    let free: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(vo_id)
            .fetch_one(&mut *probe)
            .await
            .expect("probe the lock");
    probe.rollback().await.expect("release the probe");
    free
}

/// Waits until some session of this database waits on a lock.
async fn wait_for_a_lock_waiter(pool: &PgPool) {
    for _ in 0..200 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await
        .expect("read pg_stat_activity");
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the writer never waited on the held lock");
}

/// A COMPOSITION modification member naming `preceding` as its target.
fn modification(preceding: &str, name: &str) -> Value {
    json!({
        "preceding_version_uid": { "value": preceding },
        "lifecycle_state": change_type("532", "complete"),
        "commit_audit": {
            "change_type": change_type("251", "modification"),
            "committer": committer("lock order")
        },
        "data": composition(name)
    })
}

/// A CONTRIBUTION modifying two objects listed in DESCENDING `vo_id` order
/// waits on the lower one before it holds the higher one, so it takes its
/// locks in the order an import takes them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_contribution_takes_its_object_locks_in_vo_id_order() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&DomainPools::from_shared(&pool));
    let ehr = svc.create_ehr(None).await.expect("ehr");

    let creation = |name: &str| {
        json!({
            "lifecycle_state": change_type("532", "complete"),
            "commit_audit": {
                "change_type": change_type("249", "creation"),
                "committer": committer("lock order")
            },
            "data": composition(name)
        })
    };
    let created = svc
        .create_ehr_contribution(
            ehr,
            json!({
                "versions": [creation("first"), creation("second")],
                "audit": {
                    "change_type": change_type("249", "creation"),
                    "committer": committer("lock order")
                }
            }),
        )
        .await
        .expect("two creations");
    let mut ovids: Vec<String> = created.body["versions"]
        .as_array()
        .expect("versions")
        .iter()
        .map(|v| v["id"]["value"].as_str().expect("ovid").to_owned())
        .collect();
    let vo = |ovid: &str| Uuid::parse_str(vo_of(ovid)).expect("vo uuid");
    ovids.sort_by_key(|ovid| vo(ovid));
    let (low, high) = (ovids[0].clone(), ovids[1].clone());

    let mut blocker = pool.begin().await.expect("blocker transaction");
    sqlx::query(ADVISORY_LOCK)
        .bind(vo(&low))
        .execute(&mut *blocker)
        .await
        .expect("hold the lower object's lock");

    let writer = {
        let svc = svc.clone();
        let body = json!({
            "versions": [modification(&high, "high"), modification(&low, "low")],
            "audit": {
                "change_type": change_type("251", "modification"),
                "committer": committer("lock order")
            }
        });
        tokio::spawn(async move { svc.create_ehr_contribution(ehr, body).await })
    };
    wait_for_a_lock_waiter(&pool).await;
    assert!(
        advisory_lock_is_free(&pool, vo(&high)).await,
        "the CONTRIBUTION took the higher object's lock before the lower one's"
    );

    blocker.rollback().await.expect("release the blocker");
    writer
        .await
        .expect("the writer task")
        .expect("the modification lands once the lock is free");
}

/// An import into an existing EHR waits on the EHR row before it holds any
/// per-object lock, the order the CONTRIBUTION path takes them in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_import_takes_the_ehr_row_lock_before_its_object_locks() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&DomainPools::from_shared(&source_db.pool()));
    let target_db = testkit::db().await.expect("testkit database");
    let pool = target_db.pool();
    let target = FerroEhrService::new(&DomainPools::from_shared(&pool));

    let source_ehr = source.create_ehr(None).await.expect("source ehr");
    source
        .create_directory(
            source_ehr,
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
    let mut extract = source
        .extract_ehrs(source_ehr)
        .await
        .expect("export")
        .remove(0);
    let items = extract["chapters"][0]["items"]
        .as_array_mut()
        .expect("chapter items");
    items.retain(|it| it["item"]["_type"] == json!("X_VERSIONED_FOLDER"));
    let folder_vo = Uuid::parse_str(
        items[0]["item"]["uid"]["value"]
            .as_str()
            .expect("folder vo uid"),
    )
    .expect("folder vo uuid");
    let extract = openehr_its::json::from_canonical_value(&extract).expect("folder EXTRACT");

    let ehr = target.create_ehr(None).await.expect("target ehr");
    let mut blocker = pool.begin().await.expect("blocker transaction");
    sqlx::query("SELECT 1 FROM ehr WHERE id = $1 FOR NO KEY UPDATE")
        .bind(ehr)
        .execute(&mut *blocker)
        .await
        .expect("hold the EHR row");

    let importer = {
        let target = target.clone();
        tokio::spawn(async move { target.import_ehr_extract(ehr, extract).await })
    };
    wait_for_a_lock_waiter(&pool).await;
    assert!(
        advisory_lock_is_free(&pool, folder_vo).await,
        "the import took the folder's lock before the EHR row lock"
    );

    blocker.rollback().await.expect("release the blocker");
    importer
        .await
        .expect("the import task")
        .expect("the folder lands once the EHR row is free");
}
