// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The writes of the `/directory` endpoint serialise on the EHR's directory
//! slot lock (#6354). RM ehr master04 §Folders lets an EHR hold any number of
//! live hierarchies, so no key holds the slot `EHR.directory`; two directory
//! writes whose statements run at once would otherwise both pass a guard read
//! at their own snapshots. A CONTRIBUTION that adds a hierarchy takes no such
//! lock. No openEHR spec governs how a commit runs against the database: our
//! own design.

#![expect(
    clippy::disallowed_methods,
    reason = "a test holds rows and locks on raw pool connections, outside any request"
)]

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::storage::version_repo::guarded::DIRECTORY_SLOT_LOCK;
use sqlx::PgPool;

use crate::fixtures::{commit_audit, folder, lifecycle, uv};

/// Waits until `backends` sessions of this database wait on a lock, each one
/// a write whose statement is in flight and not committed.
async fn waiting_on_locks(pool: &PgPool, backends: i64) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock'",
            )
            .fetch_one(pool)
            .await
            .expect("the lock waits");
            if waiting >= backends {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the writes reach their statements");
}

/// How many of the EHR's hierarchies have a live head.
async fn live_hierarchies(pool: &PgPool, ehr: EhrId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM clinical.ehr_folder f \
         JOIN clinical.vo_head h ON h.vo_id = f.vo_id \
         WHERE f.ehr_id = $1 AND h.lifecycle_state <> '523'",
    )
    .bind(ehr)
    .fetch_one(pool)
    .await
    .expect("the live hierarchies")
}

/// A directory create whose gate read found the directory deleted and an
/// update that brings it back, both statements in flight at once behind a
/// transaction holding the EHR row: one commits and the other is judged
/// again, a create answering `409` and an update `412`, so the EHR keeps one
/// live hierarchy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overlapping_directory_create_and_revival_leave_one_live_hierarchy() {
    let (db, svc) = crate::fixtures::service().await;
    let svc = Arc::new(svc);
    let pool = db.pool();
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let created = Box::pin(svc.create_directory(ehr, uv(&folder("old"), "249", None)))
        .await
        .expect("the directory is created");
    let deleted = Box::pin(svc.delete_directory(
        ehr,
        crate::fixtures::deleting(Some(created.uid.parse().expect("ovid"))),
    ))
    .await
    .expect("the directory is deleted")
    .meta
    .expect("the delete answers its version")
    .uid;
    let mut barrier = pool.begin().await.expect("begin the barrier");
    sqlx::query("SELECT 1 FROM ehr WHERE id = $1 FOR UPDATE")
        .bind(ehr)
        .execute(&mut *barrier)
        .await
        .expect("hold the EHR row");
    let create = {
        let svc = Arc::clone(&svc);
        let request = uv(&folder("new"), "249", None);
        tokio::spawn(async move { Box::pin(svc.create_directory(ehr, request)).await })
    };
    let revival = {
        let svc = Arc::clone(&svc);
        let request = uv(&folder("revived"), "251", Some(&deleted));
        tokio::spawn(async move { Box::pin(svc.update_directory(ehr, request)).await })
    };
    waiting_on_locks(&pool, 2).await;
    barrier.commit().await.expect("release the barrier");
    let create = create.await.expect("the create completes");
    let revival = revival.await.expect("the update completes");
    match (&create, &revival) {
        (Ok(_), Err(ServiceError::VersionConflict(_)))
        | (Err(ServiceError::Conflict(_)), Ok(_)) => {}
        outcomes => panic!("one directory write commits and the other is refused: {outcomes:?}"),
    }
    assert_eq!(live_hierarchies(&pool, ehr).await, 1, "one live hierarchy");
}

/// While a `/directory` write holds the EHR's directory slot lock, a
/// CONTRIBUTION that adds a second hierarchy commits without waiting on it,
/// and a directory delete waits until the lock is released.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_contribution_adding_a_hierarchy_takes_no_directory_slot_lock() {
    let (db, svc) = crate::fixtures::service().await;
    let svc = Arc::new(svc);
    let pool = db.pool();
    let ehr = crate::fixtures::create_ehr(&svc).await;
    Box::pin(svc.create_directory(ehr, uv(&folder("primary"), "249", None)))
        .await
        .expect("the directory is created");
    let mut slot = pool.begin().await.expect("begin the slot hold");
    sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2::text))")
        .bind(DIRECTORY_SLOT_LOCK)
        .bind(ehr)
        .execute(&mut *slot)
        .await
        .expect("hold the directory slot lock");
    let mut secondary = folder("secondary");
    secondary["archetype_node_id"] = json!("openEHR-EHR-FOLDER.episodes.v1");
    secondary["archetype_details"]["archetype_id"]["value"] =
        json!("openEHR-EHR-FOLDER.episodes.v1");
    let contribution = json!({
        "audit": commit_audit("251", "modification", "tester"),
        "versions": [{
            "lifecycle_state": lifecycle("532"),
            "data": secondary,
            "commit_audit": commit_audit("249", "creation", "tester")
        }]
    });
    tokio::time::timeout(
        Duration::from_secs(10),
        Box::pin(svc.create_ehr_contribution(ehr, contribution)),
    )
    .await
    .expect("the CONTRIBUTION does not wait on the directory slot lock")
    .expect("the second hierarchy commits");
    assert_eq!(
        live_hierarchies(&pool, ehr).await,
        2,
        "two live hierarchies"
    );
    let deleted = {
        let svc = Arc::clone(&svc);
        tokio::spawn(async move {
            let uid = Box::pin(svc.get_directory_at_time(ehr, None, None))
                .await
                .expect("the directory")
                .meta
                .expect("its version")
                .uid;
            let request = crate::fixtures::deleting(Some(uid.parse().expect("ovid")));
            Box::pin(svc.delete_directory(ehr, request)).await
        })
    };
    waiting_on_locks(&pool, 1).await;
    assert!(
        !deleted.is_finished(),
        "the directory delete waits on the lock"
    );
    slot.commit().await.expect("release the slot lock");
    deleted
        .await
        .expect("the delete completes")
        .expect("the directory delete commits once the lock is released");
}

/// The slot read refuses to run at any isolation other than READ COMMITTED,
/// where a waiting write would read the rows of its own older snapshot.
#[tokio::test]
async fn the_directory_slot_read_refuses_any_isolation_but_read_committed() {
    let (db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let pool = db.pool();
    for level in ["REPEATABLE READ", "SERIALIZABLE"] {
        let mut tx = pool.begin().await.expect("begin");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SET TRANSACTION ISOLATION LEVEL {level}"
        )))
        .execute(&mut *tx)
        .await
        .expect("set the isolation");
        let refused = sqlx::query_scalar::<_, bool>(
            "SELECT clinical.directory_slot_held(true, $1, 0, NULL, NULL)",
        )
        .bind(ehr)
        .fetch_one(&mut *tx)
        .await
        .expect_err("the slot read refuses");
        assert!(
            refused.to_string().contains("READ COMMITTED"),
            "{level} is refused by name: {refused}"
        );
    }
    let held: bool =
        sqlx::query_scalar("SELECT clinical.directory_slot_held(true, $1, 0, NULL, NULL)")
            .bind(ehr)
            .fetch_one(&pool)
            .await
            .expect("READ COMMITTED reads the slot");
    assert!(held, "an EHR with no hierarchy has highest rank 0");
}
