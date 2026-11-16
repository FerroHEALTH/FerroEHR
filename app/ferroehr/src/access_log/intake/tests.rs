// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The gathered writer answers each request by its own outcome (#5881): an
//! entry the database refuses refuses only the request that brought it, and
//! the others of its statement are written once each.

use super::{IntakeEntry, Job, write_jobs};
use crate::access_log::entry::entry_of;
use crate::access_log::retention::stated;
use crate::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use crate::system_log::retention::LogRetention;

/// The intake form of a granted read of the EHR `ehr`.
fn intake(ehr: &str) -> IntakeEntry {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    event.user_id = "alice".to_owned();
    event.ehr_id = Some(ehr.to_owned());
    event.object_id = Some(ehr.to_owned());
    let retention = stated(
        Some(jiff::civil::date(2031, 11, 8)),
        &LogRetention::default(),
    );
    let mut entry =
        entry_of(&event, None, "cdr-intake-writer.example.org", retention).expect("an entry");
    IntakeEntry::of(&mut entry, None).expect("the intake form")
}

/// A request of `entries`, with the receiver of its answer.
fn job(
    entries: Vec<IntakeEntry>,
) -> (
    Job,
    tokio::sync::oneshot::Receiver<Result<(), std::sync::Arc<super::AccessLogError>>>,
) {
    let (done, answer) = tokio::sync::oneshot::channel();
    let tokens = vec![Vec::new(); entries.len()];
    (
        Job {
            entries,
            tokens,
            done,
        },
        answer,
    )
}

/// Of three requests gathered into one statement, the one whose entry the
/// database refuses is refused, and the other two are written.
#[tokio::test]
async fn a_refused_entry_refuses_only_its_own_request() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let mut refused = intake("7d44b88c-4199-4bad-97dc-0000000000f2");
    refused.derived.uid = Some("not-a-uuid".to_owned());
    let (first, first_answer) = job(vec![intake("7d44b88c-4199-4bad-97dc-0000000000f1")]);
    let (bad, bad_answer) = job(vec![refused]);
    let (last, last_answer) = job(vec![
        intake("7d44b88c-4199-4bad-97dc-0000000000f3"),
        intake("7d44b88c-4199-4bad-97dc-0000000000f4"),
    ]);
    write_jobs(&pool, vec![first, bad, last], 4).await;
    assert!(
        first_answer.await.expect("an answer").is_ok(),
        "the first request is written"
    );
    assert!(
        bad_answer.await.expect("an answer").is_err(),
        "the refused entry's request is refused"
    );
    assert!(
        last_answer.await.expect("an answer").is_ok(),
        "the last request is written"
    );
    let ehrs: Vec<String> =
        sqlx::query_scalar("SELECT ehr_id FROM audit.access_log_intake ORDER BY ehr_id")
            .fetch_all(&pool)
            .await
            .expect("the intake");
    assert_eq!(
        ehrs,
        [
            "7d44b88c-4199-4bad-97dc-0000000000f1",
            "7d44b88c-4199-4bad-97dc-0000000000f3",
            "7d44b88c-4199-4bad-97dc-0000000000f4",
        ],
        "each written entry once, the refused one not at all"
    );
}

/// A statement that fails before the database answers, here because no
/// connection can be had, refuses every request it gathered: none is
/// answered as written.
#[tokio::test]
async fn a_failure_with_no_answer_refuses_every_request() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    pool.close().await;
    let (first, first_answer) = job(vec![intake("7d44b88c-4199-4bad-97dc-0000000000f5")]);
    let (second, second_answer) = job(vec![intake("7d44b88c-4199-4bad-97dc-0000000000f6")]);
    write_jobs(&pool, vec![first, second], 2).await;
    assert!(first_answer.await.expect("an answer").is_err());
    assert!(second_answer.await.expect("an answer").is_err());
}
