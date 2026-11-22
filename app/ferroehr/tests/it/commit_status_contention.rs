// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `EHR_STATUS` commits that contend with each other or with COMPOSITION
//! commits on one EHR (#6257). A status commit judges its pre-checks on one
//! gate read and commits in one guarded statement that takes the EHR row
//! `FOR UPDATE` and writes its promoted columns beside the version, so a
//! COMPOSITION commit queued behind it reads the flipped `is_modifiable` and
//! answers `409` as it did when it waited for the status transaction, and of
//! two status commits naming one preceding version one commits and the other
//! answers `412`. No openEHR spec governs how a commit is run against the
//! database: our own design.

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

use crate::fixtures::{composition, uv};

/// A service over a fresh database and an EHR.
async fn ehr() -> (testkit::TestDb, Arc<FerroEhrService>, EhrId) {
    let (db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    (db, Arc::new(svc), ehr)
}

/// The current `EHR_STATUS` of `ehr` without its uid, with `is_modifiable`
/// set to `modifiable`, and the uid it had.
async fn status(svc: &FerroEhrService, ehr: EhrId, modifiable: bool) -> (Value, String) {
    let mut body = svc
        .ehr_object(ehr, EhrObject::Status, None)
        .await
        .expect("the EHR_STATUS")
        .body;
    let uid = body["uid"]["value"].as_str().expect("a uid").to_owned();
    let object = body.as_object_mut().expect("an object");
    object.shift_remove("uid");
    object.insert("is_modifiable".to_owned(), json!(modifiable));
    (body, uid)
}

/// A transaction that holds the row of `ehr`, so every write to it judges
/// its gate read and then waits at its commit statement until the
/// transaction ends, the writers queued in the order they reached it.
async fn hold(db: &testkit::TestDb, ehr: EhrId) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = db.pool().begin().await.expect("begin the hold");
    sqlx::query("SELECT 1 FROM ehr WHERE id = $1 FOR UPDATE")
        .bind(ehr)
        .execute(&mut *tx)
        .await
        .expect("hold the EHR row");
    tx
}

/// Lets the spawned writers reach their commit statement.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(400)).await;
}

/// A status commit that makes the EHR not modifiable, queued ahead of
/// `writers` COMPOSITION creates: it commits, and each create, judged on a
/// modifiable EHR, reads the flip, is judged again and answers `409`.
async fn creates_behind_a_status_flip_answer_409(writers: usize) {
    let (db, svc, ehr) = ehr().await;
    let (body, uid) = status(&svc, ehr, false).await;
    let held = hold(&db, ehr).await;
    let flip = {
        let svc = Arc::clone(&svc);
        tokio::spawn(async move {
            Box::pin(svc.replace_ehr_status(ehr, uv(&body, "251", Some(&uid)))).await
        })
    };
    settle().await;
    let creates: Vec<_> = (0..writers)
        .map(|n| {
            let svc = Arc::clone(&svc);
            let request = uv(&composition(&format!("writer {n}")), "249", None);
            tokio::spawn(async move { Box::pin(svc.create_composition(ehr, request)).await })
        })
        .collect();
    settle().await;
    held.commit().await.expect("release the hold");
    flip.await
        .expect("the status writer completes")
        .expect("the status commits");
    for create in creates {
        let outcome = create.await.expect("the create completes");
        assert!(
            matches!(outcome, Err(ServiceError::Conflict(_))),
            "a create behind the flip answers 409: {outcome:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn creates_behind_a_status_flip_answer_409_for_two_writers() {
    creates_behind_a_status_flip_answer_409(2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn creates_behind_a_status_flip_answer_409_for_eight_writers() {
    creates_behind_a_status_flip_answer_409(8).await;
}

/// `writers` status commits naming the same preceding version: one commits
/// and every other answers `412`, as when each waited for the object lock.
async fn one_status_commit_wins(writers: usize) {
    let (db, svc, ehr) = ehr().await;
    let (body, uid) = status(&svc, ehr, true).await;
    let held = hold(&db, ehr).await;
    let tasks: Vec<_> = (0..writers)
        .map(|_| {
            let svc = Arc::clone(&svc);
            let request = uv(&body, "251", Some(&uid));
            tokio::spawn(async move { Box::pin(svc.replace_ehr_status(ehr, request)).await })
        })
        .collect();
    settle().await;
    held.commit().await.expect("release the hold");
    let mut committed = 0;
    for task in tasks {
        match task.await.expect("the writer completes") {
            Ok(_) => committed += 1,
            Err(ServiceError::VersionConflict(_)) => {}
            Err(error) => panic!("a losing status commit answers 412: {error:?}"),
        }
    }
    assert_eq!(committed, 1, "exactly one status commit wins");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_two_status_commits_wins() {
    one_status_commit_wins(2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_eight_status_commits_wins() {
    one_status_commit_wins(8).await;
}
