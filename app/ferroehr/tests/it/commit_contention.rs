// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! COMPOSITION writes that contend for one object or one EHR (#6256). Each
//! write judges its pre-checks on one gate read and commits in one guarded
//! statement that writes nothing when what the gate read judged moved; a
//! moved head or EHR is judged again from a new gate read, so every write
//! answers as it did when it waited for the per-object lock: a conditioned
//! update that lost answers `412`, an unconditioned one commits on the new
//! tip, a delete of a version that is no longer the latest answers `409`, and
//! a version committed later never carries an earlier commit instant. The
//! writers here are held at their commit statement by a transaction that
//! holds the EHR row, which the statement reads `FOR SHARE`. No openEHR spec
//! governs how a commit is run against the database: our own design.

#![expect(
    clippy::disallowed_methods,
    reason = "a test holds and reads rows on raw pool connections, outside any request"
)]

use std::sync::Arc;
use std::time::Duration;

use ferroehr::error::ServiceError;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::change::Committed;
use openehr_base::prelude::ObjectVersionId;

use crate::fixtures::{composition, uv};

/// A service over a fresh database, an EHR and one COMPOSITION in it.
async fn object() -> (testkit::TestDb, Arc<FerroEhrService>, EhrId, Committed) {
    let (db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let created = Box::pin(svc.create_composition(ehr, uv(&composition("v1"), "249", None)))
        .await
        .expect("the first version commits");
    (db, Arc::new(svc), ehr, created)
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

/// Lets the spawned writers reach their commit statement.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(400)).await;
}

/// `writers` updates of `vo`, each naming `preceding` (or none), held until
/// all have judged their gate read; their outcomes.
async fn updates(
    db: &testkit::TestDb,
    svc: &Arc<FerroEhrService>,
    (ehr, vo): (EhrId, VoId),
    preceding: Option<&str>,
    writers: usize,
) -> Vec<Result<Committed, ServiceError>> {
    let held = hold(db, ehr).await;
    let tasks: Vec<_> = (0..writers)
        .map(|n| {
            let svc = Arc::clone(svc);
            let request: WriteRequest<_> =
                uv(&composition(&format!("writer {n}")), "251", preceding);
            tokio::spawn(async move { Box::pin(svc.update_composition(ehr, vo, request)).await })
        })
        .collect();
    settle().await;
    held.commit().await.expect("release the hold");
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.expect("the writer completes"));
    }
    outcomes
}

/// The commit instants of `vo`'s versions in ordinal order.
async fn instants(db: &testkit::TestDb, vo: VoId) -> Vec<jiff_sqlx::Timestamp> {
    sqlx::query_scalar("SELECT committed_at FROM version WHERE vo_id = $1 ORDER BY sys_version")
        .bind(vo)
        .fetch_all(&db.pool())
        .await
        .expect("the versions")
}

/// Unconditioned updates that all judged version 1 commit, one after the
/// other, each on the tip the one before it left, as they did when each
/// waited for the lock; no version carries an earlier instant than the one
/// before it.
async fn unconditioned_updates_all_commit(writers: usize) {
    let (db, svc, ehr, created) = object().await;
    let outcomes = updates(&db, &svc, (ehr, created.vo_id), None, writers).await;
    let mut trees: Vec<String> = outcomes
        .into_iter()
        .map(|outcome| {
            outcome
                .expect("every unconditioned update commits")
                .tree
                .to_string()
        })
        .collect();
    trees.sort_unstable();
    trees.dedup();
    assert_eq!(
        trees.len(),
        writers,
        "each writer committed a version of its own"
    );
    let instants = instants(&db, created.vo_id).await;
    assert_eq!(instants.len(), writers + 1, "every version is stored");
    assert!(
        instants
            .windows(2)
            .all(|pair| pair[0].to_jiff() <= pair[1].to_jiff()),
        "a later version never carries an earlier commit instant: {instants:?}"
    );
}

#[tokio::test]
async fn two_unconditioned_updates_of_one_object_both_commit() {
    unconditioned_updates_all_commit(2).await;
}

#[tokio::test]
async fn eight_unconditioned_updates_of_one_object_all_commit() {
    unconditioned_updates_all_commit(8).await;
}

/// Conditioned updates that name the same version: one commits and every
/// other answers `412`, the version it named no longer being the latest.
async fn conditioned_updates_one_wins(writers: usize) {
    let (db, svc, ehr, created) = object().await;
    let preceding = created.version_uid();
    let outcomes = updates(&db, &svc, (ehr, created.vo_id), Some(&preceding), writers).await;
    let won = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
    assert_eq!(won, 1, "exactly one update commits: {outcomes:?}");
    for outcome in outcomes.into_iter().filter_map(Result::err) {
        assert!(
            matches!(outcome, ServiceError::VersionConflict(_)),
            "a losing conditioned update answers 412: {outcome:?}"
        );
    }
    assert_eq!(
        instants(&db, created.vo_id).await.len(),
        2,
        "one version landed"
    );
}

#[tokio::test]
async fn two_conditioned_updates_of_one_version_one_wins_the_other_is_412() {
    conditioned_updates_one_wins(2).await;
}

#[tokio::test]
async fn eight_conditioned_updates_of_one_version_one_wins_the_rest_are_412() {
    conditioned_updates_one_wins(8).await;
}

/// A delete of a version an update superseded while the delete waited answers
/// `409`, the stale-preceding answer; whichever of the two commits first, the
/// other answers as it did behind the lock.
#[tokio::test]
async fn a_delete_of_a_version_an_update_superseded_is_409() {
    let (db, svc, ehr, created) = object().await;
    let latest: ObjectVersionId = created.version_uid().parse().expect("OBJECT_VERSION_ID");
    let held = hold(&db, ehr).await;
    let updating = {
        let svc = Arc::clone(&svc);
        let request: WriteRequest<_> = uv(&composition("v2"), "251", None);
        let vo = created.vo_id;
        tokio::spawn(async move { Box::pin(svc.update_composition(ehr, vo, request)).await })
    };
    let deleting = {
        let svc = Arc::clone(&svc);
        tokio::spawn(async move {
            Box::pin(svc.delete_composition(ehr, &latest, WriteRequest::new(()))).await
        })
    };
    settle().await;
    held.commit().await.expect("release the hold");
    let updated = updating.await.expect("the update completes");
    let deleted = deleting.await.expect("the delete completes");
    // When the delete committed first, the unconditioned update finds a deleted
    // tip, which it answers 404 (RM common master06 §Logical Deletion).
    assert!(
        matches!(
            (&updated, &deleted),
            (Ok(_), Err(ServiceError::Conflict(_))) | (Err(ServiceError::NotFound(_)), Ok(_))
        ),
        "the losing write answers as behind the lock: {:?}",
        (&updated, &deleted)
    );
}

/// Two deletes of the latest version: one commits and the other finds the
/// object already deleted, which it answers `400` as it did behind the lock.
#[tokio::test]
async fn two_deletes_of_one_version_one_wins_the_other_finds_it_deleted() {
    let (db, svc, ehr, created) = object().await;
    let latest: ObjectVersionId = created.version_uid().parse().expect("OBJECT_VERSION_ID");
    let held = hold(&db, ehr).await;
    let tasks: Vec<_> = (0..2)
        .map(|_| {
            let svc = Arc::clone(&svc);
            let latest = latest.clone();
            tokio::spawn(async move {
                Box::pin(svc.delete_composition(ehr, &latest, WriteRequest::new(()))).await
            })
        })
        .collect();
    settle().await;
    held.commit().await.expect("release the hold");
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.expect("the delete completes"));
    }
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
        1,
        "exactly one delete commits: {outcomes:?}"
    );
    for outcome in outcomes.into_iter().filter_map(Result::err) {
        assert!(
            matches!(outcome, ServiceError::BadRequest(_)),
            "the other finds the object already deleted: {outcome:?}"
        );
    }
}

/// An EHR deactivated while a create waits at its commit statement refuses
/// the create with `409`: the statement reads the EHR row `FOR SHARE`, sees
/// the deactivation once it committed, and the write judged again refuses.
#[tokio::test]
async fn a_create_into_an_ehr_deactivated_while_it_waits_is_409() {
    let (db, svc, ehr, _) = object().await;
    let mut held = hold(&db, ehr).await;
    sqlx::query("UPDATE ehr SET is_modifiable = false WHERE id = $1")
        .bind(ehr)
        .execute(&mut *held)
        .await
        .expect("an uncommitted deactivation");
    let creating = {
        let svc = Arc::clone(&svc);
        let request: WriteRequest<_> = uv(&composition("raced"), "249", None);
        tokio::spawn(async move { Box::pin(svc.create_composition(ehr, request)).await })
    };
    settle().await;
    held.commit().await.expect("commit the deactivation");
    let refused = creating
        .await
        .expect("the create completes")
        .expect_err("the create sees the deactivation");
    assert!(
        refused.message().contains("not modifiable"),
        "the refusal names the deactivation: {refused:?}"
    );
}

/// An `ORIGINAL_VERSION` member of a CONTRIBUTION modifying the version
/// `preceding` with `data`.
fn modification(data: &serde_json::Value, preceding: &str) -> serde_json::Value {
    serde_json::json!({
        "_type": "ORIGINAL_VERSION",
        "commit_audit": {
            "change_type": crate::fixtures::change_type("251", "modification"),
            "committer": crate::fixtures::committer("author")
        },
        "lifecycle_state": crate::fixtures::change_type("532", "complete"),
        "preceding_version_uid": { "value": preceding },
        "data": data.clone()
    })
}

/// A CONTRIBUTION that writes the `EHR_STATUS` and modifies a COMPOSITION,
/// racing a PUT of that COMPOSITION. With the PUT's object lock taken before
/// the EHR row and the CONTRIBUTION's EHR row taken before its object lock,
/// the two could deadlock; with no object lock on the PUT, which waits for the
/// EHR row alone and has its version's keys arbitrate, both complete. The one
/// that commits second named a superseded version and answers `412`, as it
/// did behind the lock.
#[tokio::test]
async fn a_status_writing_contribution_and_a_put_of_its_composition_both_complete() {
    let (db, svc, ehr, created) = object().await;
    let status = svc
        .ehr_object(ehr, ferroehr::service::ehr::object::EhrObject::Status, None)
        .await
        .expect("the current EHR_STATUS")
        .body;
    let status_uid = status["uid"]["value"]
        .as_str()
        .expect("the status uid")
        .to_owned();
    let mut status_body = status;
    if let Some(object) = status_body.as_object_mut() {
        object.shift_remove("uid");
    }
    let preceding = created.version_uid();
    let held = hold(&db, ehr).await;
    let putting = {
        let svc = Arc::clone(&svc);
        let request: WriteRequest<_> = uv(&composition("put"), "251", Some(&preceding));
        let vo = created.vo_id;
        tokio::spawn(async move { Box::pin(svc.update_composition(ehr, vo, request)).await })
    };
    let contributing = {
        let svc = Arc::clone(&svc);
        let contribution = serde_json::json!({
            "versions": [
                modification(&status_body, &status_uid),
                modification(&composition("contributed"), &preceding),
            ],
            "audit": crate::fixtures::commit_audit("251", "modification", "author")
        });
        tokio::spawn(async move { Box::pin(svc.create_ehr_contribution(ehr, contribution)).await })
    };
    settle().await;
    held.commit().await.expect("release the hold");
    let bound = Duration::from_secs(60);
    let put = tokio::time::timeout(bound, putting)
        .await
        .expect("the PUT completes")
        .expect("the PUT task");
    let contributed = tokio::time::timeout(bound, contributing)
        .await
        .expect("the CONTRIBUTION completes")
        .expect("the CONTRIBUTION task");
    assert!(
        matches!(
            (&put, &contributed),
            (Ok(_), Err(ServiceError::VersionConflict(_)))
                | (Err(ServiceError::VersionConflict(_)), Ok(_))
        ),
        "one commits and the other answers 412: {:?}",
        (
            put.as_ref().map(Committed::version_uid),
            contributed.as_ref().map(|_| ())
        )
    );
}
