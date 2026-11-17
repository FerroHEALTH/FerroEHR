// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The gathered writer answers each request by its own outcome (#5881): an
//! entry the database refuses refuses only the request that brought it, and
//! the others of its statement are written once each.

use super::{Derived, IntakeEntry, Job, write_jobs};
use crate::access_log::entry::entry_of;
use crate::access_log::retention::stated;
use crate::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use crate::system_log::retention::LogRetention;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

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

/// The intake form as the `Value` path wrote it before #5849: the entry's
/// JSON value without its link, canonicalised whole, split, its row without
/// the members its columns give back (#5784), and read at the derive's
/// paths.
#[expect(
    clippy::disallowed_types,
    reason = "the reference is computed over the entry's ITS-JSON value (#1694 boundary class: \
              canonical form)"
)]
fn through_value(entry: &AccessLogEntry) -> (String, String, Option<(String, String)>, Derived) {
    use openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json;
    use serde_json::Value;
    let Value::Object(mut members) = serde_json::to_value(entry).expect("the entry serialises")
    else {
        panic!("an entry is an object");
    };
    members.remove("link");
    let document = Value::Object(members.clone());
    let whole = canonical_json(&document).expect("the whole form");
    let text = |path: &str| {
        document
            .pointer(path)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let derived = Derived {
        record_type: text("/_type"),
        uid: text("/uid/value"),
        time_recorded: text("/time_recorded/value"),
        ehr_id: text("/ehr_id/value"),
        action_terminology: text("/action/defining_code/terminology_id/value"),
        action_code: text("/action/defining_code/code_string"),
        result_terminology: text("/outcome/result/defining_code/terminology_id/value"),
        result_code: text("/outcome/result/defining_code/code_string"),
        party_id: text("/accessor/party/external_ref/id/value"),
        retain_until: text("/retention/retain_until/value"),
    };
    let mut form = crate::access_log::stored_form::split(members);
    if let Value::Object(row) = &mut form.row {
        row.remove("_type");
        let value_of = |member: Option<&Value>, kind: &str| {
            member
                .and_then(Value::as_object)
                .filter(|object| {
                    object.len() == 2 && object.get("_type").and_then(Value::as_str) == Some(kind)
                })
                .and_then(|object| object.get("value"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        for id in ["uid", "ehr_id"] {
            if value_of(row.get(id), "HIER_OBJECT_ID")
                .is_some_and(|text| crate::access_log::stored_form::column_uuid(&text).is_some())
            {
                row.remove(id);
            }
        }
        if value_of(row.get("time_recorded"), "DV_DATE_TIME")
            .is_some_and(|text| crate::access_log::stored_form::column_instant(&text))
        {
            row.remove("time_recorded");
        }
    }
    let row = canonical_json(&form.row).expect("the row");
    let shared = form.shared.map(|shared| {
        let text = canonical_json(&shared).expect("the shared part");
        (crate::digest::sha256_hex(&text), text)
    });
    (crate::digest::sha256_hex(&whole), row, shared, derived)
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(4096))]

    /// The intake form written straight from an entry is the one its JSON
    /// value gave: the content digest, the row, the shared part and every
    /// derived column (#5849).
    #[test]
    fn the_intake_form_is_the_one_the_value_path_wrote(
        event in crate::access_log::stored_form::parts::tests::event(),
    ) {
        let retention = stated(Some(jiff::civil::date(2031, 11, 8)), &LogRetention::default());
        if let Some(entry) = entry_of(&event, None, "cdr.example.org", retention) {
            let (digest, row, shared, derived) = through_value(&entry);
            let mut linked = entry;
            let intake = IntakeEntry::of(&mut linked, None).expect("the intake form");
            proptest::prop_assert_eq!(intake.content_digest, digest);
            proptest::prop_assert_eq!(intake.row, row);
            proptest::prop_assert_eq!(intake.shared, shared);
            proptest::prop_assert_eq!(intake.derived, derived);
        }
    }
}
