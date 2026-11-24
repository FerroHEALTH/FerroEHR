// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Directory (FOLDER) commits that contend on one EHR (#6257). A directory
//! write judges its pre-checks on one gate read and commits in one guarded
//! statement, so of several creates of the directory one commits and every
//! other answers `409`, of several updates or deletes naming one preceding
//! version one commits and every other answers `412`, and a write queued
//! behind a status flip answers `409`. No openEHR spec governs how a commit
//! is run against the database: our own design.

#![expect(
    clippy::disallowed_methods,
    reason = "a test holds rows on raw pool connections, outside any request"
)]

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr::object::EhrObject;

use crate::fixtures::{folder, uv};

/// A service over a fresh database and an EHR.
async fn ehr() -> (testkit::TestDb, Arc<FerroEhrService>, EhrId) {
    let (db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    (db, Arc::new(svc), ehr)
}

/// A transaction that holds the row of `ehr`, so every write to it judges
/// its gate read and then waits at its commit statement until the
/// transaction ends.
async fn hold(db: &testkit::TestDb, ehr: EhrId) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = db.pool().begin().await.expect("begin the hold");
    sqlx::query("SELECT 1 FROM ehr WHERE id = $1 FOR UPDATE")
        .bind(ehr)
        .execute(&mut *tx)
        .await
        .expect("hold the EHR row");
    tx
}

/// Waits until `writers` sessions of the database of `db` wait on a lock,
/// so the writers spawned so far stand queued in their order.
async fn queued(db: &testkit::TestDb, writers: usize) {
    let writers = i64::try_from(writers).expect("a writer count");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(&db.pool())
        .await
        .expect("read the waiting sessions");
        if waiting >= writers {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{waiting} of {writers} writers queued on the hold"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Lets the spawned writers reach their commit statement.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(400)).await;
}

/// The directory of `ehr`, created, and its version uid.
async fn directory(svc: &FerroEhrService, ehr: EhrId) -> String {
    svc.create_directory(ehr, uv(&folder("root"), "249", None))
        .await
        .expect("the directory")
        .uid
}

/// `writers` creates of the directory of one EHR at once: one commits and
/// every other answers `409`, as a create after it does.
async fn one_directory_create_wins(writers: usize) {
    let (db, svc, ehr) = ehr().await;
    let held = hold(&db, ehr).await;
    let tasks: Vec<_> = (0..writers)
        .map(|n| {
            let svc = Arc::clone(&svc);
            let request = uv(&folder(&format!("root {n}")), "249", None);
            tokio::spawn(async move { Box::pin(svc.create_directory(ehr, request)).await })
        })
        .collect();
    settle().await;
    held.commit().await.expect("release the hold");
    let mut committed = 0;
    for task in tasks {
        match task.await.expect("the writer completes") {
            Ok(_) => committed += 1,
            Err(ServiceError::Conflict(_)) => {}
            Err(error) => panic!("a losing directory create answers 409: {error:?}"),
        }
    }
    assert_eq!(committed, 1, "exactly one directory create wins");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_two_directory_creates_wins() {
    one_directory_create_wins(2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_eight_directory_creates_wins() {
    one_directory_create_wins(8).await;
}

/// `writers` updates, or deletes when `delete`, of one directory naming the
/// same preceding version: one commits and every other answers `412`.
async fn one_directory_write_wins(writers: usize, delete: bool) {
    let (db, svc, ehr) = ehr().await;
    let uid = directory(&svc, ehr).await;
    let held = hold(&db, ehr).await;
    let tasks: Vec<_> = (0..writers)
        .map(|n| {
            let svc = Arc::clone(&svc);
            let uid = uid.clone();
            tokio::spawn(async move {
                if delete {
                    let request = crate::fixtures::deleting(Some(uid.parse().expect("a uid")));
                    Box::pin(svc.delete_directory(ehr, request))
                        .await
                        .map(|_| ())
                } else {
                    let request = uv(&folder(&format!("renamed {n}")), "251", Some(&uid));
                    Box::pin(svc.update_directory(ehr, request))
                        .await
                        .map(|_| ())
                }
            })
        })
        .collect();
    settle().await;
    held.commit().await.expect("release the hold");
    let mut committed = 0;
    for task in tasks {
        match task.await.expect("the writer completes") {
            Ok(()) => committed += 1,
            Err(ServiceError::VersionConflict(_)) => {}
            Err(error) => panic!("a losing directory write answers 412: {error:?}"),
        }
    }
    assert_eq!(committed, 1, "exactly one directory write wins");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_two_directory_updates_wins() {
    one_directory_write_wins(2, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_eight_directory_updates_wins() {
    one_directory_write_wins(8, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_two_directory_deletes_wins() {
    one_directory_write_wins(2, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_eight_directory_deletes_wins() {
    one_directory_write_wins(8, true).await;
}

/// The current `EHR_STATUS` of `ehr` without its uid, made not modifiable,
/// and the uid it had.
async fn unmodifiable_status(svc: &FerroEhrService, ehr: EhrId) -> (Value, String) {
    let mut body = svc
        .ehr_object(ehr, EhrObject::Status, None)
        .await
        .expect("the EHR_STATUS")
        .body;
    let uid = body["uid"]["value"].as_str().expect("a uid").to_owned();
    let object = body.as_object_mut().expect("an object");
    object.shift_remove("uid");
    object.insert("is_modifiable".to_owned(), json!(false));
    (body, uid)
}

/// A status commit that makes the EHR not modifiable, seen queued on the
/// hold before `writers` directory updates queue behind it: it commits, and
/// each update, judged on a modifiable EHR, reads the flip, is judged again
/// and answers `409`.
async fn directory_writes_behind_a_status_flip_answer_409(writers: usize) {
    let (db, svc, ehr) = ehr().await;
    let uid = directory(&svc, ehr).await;
    let (body, status_uid) = unmodifiable_status(&svc, ehr).await;
    let held = hold(&db, ehr).await;
    let flip = {
        let svc = Arc::clone(&svc);
        tokio::spawn(async move {
            Box::pin(svc.replace_ehr_status(ehr, uv(&body, "251", Some(&status_uid)))).await
        })
    };
    queued(&db, 1).await;
    let updates: Vec<_> = (0..writers)
        .map(|n| {
            let svc = Arc::clone(&svc);
            let request = uv(&folder(&format!("renamed {n}")), "251", Some(&uid));
            tokio::spawn(async move { Box::pin(svc.update_directory(ehr, request)).await })
        })
        .collect();
    queued(&db, 1 + writers).await;
    held.commit().await.expect("release the hold");
    flip.await
        .expect("the status writer completes")
        .expect("the status commits");
    for update in updates {
        let outcome = update.await.expect("the update completes");
        assert!(
            matches!(outcome, Err(ServiceError::Conflict(_))),
            "a directory update behind the flip answers 409: {outcome:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn directory_writes_behind_a_status_flip_answer_409_for_two_writers() {
    directory_writes_behind_a_status_flip_answer_409(2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn directory_writes_behind_a_status_flip_answer_409_for_eight_writers() {
    directory_writes_behind_a_status_flip_answer_409(8).await;
}
