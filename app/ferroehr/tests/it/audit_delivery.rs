// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The delivery of both audit logs from the store to its sinks (#6242): a
//! cursor per sink and log, moved only after the repository accepted a
//! record, so a crash repeats at most the record in flight and loses and
//! skips none; a record a sink cannot deliver, or retention removes or
//! replaces first, is parked and kept until it is delivered on a retry or an
//! operator dismisses it; retention waits a day for a sink that has not
//! delivered a record; and a sink taken out of the configuration keeps its
//! cursor until an operator forgets it. No openEHR spec governs audit
//! delivery: our own design.

use std::collections::BTreeMap;

use jiff::Timestamp;
use sqlx::PgPool;

use ferroehr::access_log::frontier::Frontier;
use ferroehr::system_log::config::Retention;
use ferroehr::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::store::chains::AuditChain;
use ferroehr::system_log::store::delivery::{Due, Parked, ParkedAct, Sink};
use ferroehr::system_log::store::{AuditRecord, AuditStore, RecordEnd};

/// A template read of subject `n`, a system record.
fn record(n: u32) -> AuditRecord {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Template,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut event.user_id);
    event.object_id = Some(format!("delivery.template.v{n}"));
    event.event_type = Some(EventType::rest("template_get"));
    AuditRecord {
        event,
        subject: None,
        entry: None,
    }
}

/// A store over `pool` delivering to the FHIR Feed, with its cursors started.
async fn feeding(pool: &PgPool) -> AuditStore {
    let store = AuditStore::new(pool.clone()).with_fhir_feed(true);
    store.start_cursors().await.expect("the cursors start");
    store
}

/// The highest position drawn.
async fn last_seq(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT coalesce(max(seq), 0) FROM audit.audit_event")
        .fetch_one(pool)
        .await
        .expect("the positions")
}

/// Frontier rounds of `store` until the delivery frontier of the system
/// records reaches `target`, which another test's transaction anywhere in
/// the cluster may delay for a while.
async fn delivering_up_to(pool: &PgPool, store: &AuditStore, target: i64) {
    let mut frontier = Frontier::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        store
            .frontier_round(&mut frontier, false)
            .await
            .expect("a frontier round");
        let reached: i64 = sqlx::query_scalar(
            "SELECT through_id FROM audit.delivery_frontier WHERE chain = 'system'",
        )
        .fetch_one(pool)
        .await
        .expect("the delivery frontier");
        if reached >= target {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the delivery frontier never reached {target}: {reached}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The records `sink` parked, by log and id, with their reasons.
async fn parked(pool: &PgPool, sink: Sink) -> BTreeMap<(String, i64), String> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT chain, record_id, reason FROM audit.delivery_parked WHERE sink = $1",
    )
    .bind(sink.as_str())
    .fetch_all(pool)
    .await
    .expect("the parked records");
    rows.into_iter()
        .map(|(chain, id, reason)| ((chain, id), reason))
        .collect()
}

/// A sink that crashes after every fifth send, before the repository's
/// acceptance moved its cursor, repeats that one record and no other, and
/// in the end has sent every record and skipped none.
#[tokio::test]
async fn a_crash_repeats_at_most_the_record_in_flight_and_skips_none() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = feeding(&pool).await;
    store
        .write((1..=20).map(record).collect(), &[])
        .await
        .expect("twenty records");
    let last = last_seq(&pool).await;
    delivering_up_to(&pool, &store, last).await;

    let mut sent = Vec::new();
    let mut crashes = 0;
    loop {
        let (next, _) = store.next_owed(Sink::FhirFeed).await.expect("the next");
        let Some((due, rendered)) = next else {
            break;
        };
        rendered.expect("the record renders");
        sent.push(due.id);
        // NOTE: a crash between the send and the acceptance leaves the cursor where it was.
        if sent.len() % 5 == 0 && sent.iter().filter(|id| **id == due.id).count() == 1 {
            crashes += 1;
            continue;
        }
        store
            .mark_delivered(Sink::FhirFeed, due)
            .await
            .expect("the acceptance");
    }
    let mut once: Vec<i64> = sent.clone();
    once.dedup();
    let expected: Vec<i64> = (last - 19..=last).collect();
    assert_eq!(once, expected, "every record sent, in order, none skipped");
    assert_eq!(
        sent.len(),
        expected.len() + crashes,
        "each crash repeats its one record"
    );
}

/// A record its repository refuses is parked, counted, and the sink moves on
/// to the next; on an operator's retry it is offered again before the next
/// record and, once accepted, leaves the parked set.
#[tokio::test]
async fn a_refused_record_is_parked_and_delivered_after_a_retry() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = feeding(&pool).await;
    store
        .write((1..=3).map(record).collect(), &[])
        .await
        .expect("three records");
    let last = last_seq(&pool).await;
    delivering_up_to(&pool, &store, last).await;

    let (next, _) = store.next_owed(Sink::FhirFeed).await.expect("the next");
    let (first, _) = next.expect("a record");
    store
        .park(Sink::FhirFeed, first, Parked::Refused)
        .await
        .expect("parked");
    let (next, waiting) = store.next_owed(Sink::FhirFeed).await.expect("the next");
    let (second, _) = next.expect("the sink moves on");
    assert_eq!(second.id, first.id + 1);
    assert_eq!(waiting.parked, 1, "the parked record is counted");
    assert_eq!(
        parked(&pool, Sink::FhirFeed).await,
        BTreeMap::from([(("system".to_owned(), first.id), "refused".to_owned())])
    );
    let view = store.delivery_view().await.expect("the view");
    assert_eq!(view.parked.len(), 1, "{view:?}");

    assert!(
        store
            .act_on_parked(
                ParkedAct::Retry,
                Sink::FhirFeed,
                AuditChain::System,
                first.id
            )
            .await
            .expect("a retry")
    );
    let (next, _) = store.next_owed(Sink::FhirFeed).await.expect("the next");
    let (retried, _) = next.expect("the retried record");
    assert_eq!(
        (retried.id, retried.retry),
        (first.id, true),
        "a retry comes before the next record"
    );
    store
        .mark_delivered(Sink::FhirFeed, retried)
        .await
        .expect("accepted");
    assert_eq!(
        parked(&pool, Sink::FhirFeed).await,
        BTreeMap::new(),
        "the accepted record leaves the parked set"
    );
    let (next, _) = store.next_owed(Sink::FhirFeed).await.expect("the next");
    assert_eq!(
        next.map(|(due, _)| due.id),
        Some(second.id),
        "the cursor stayed where the park left it"
    );
}

/// A dismissed parked record leaves the parked set and is never offered.
#[tokio::test]
async fn a_dismissed_record_is_never_offered() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = feeding(&pool).await;
    store.write(vec![record(1)], &[]).await.expect("a record");
    let last = last_seq(&pool).await;
    delivering_up_to(&pool, &store, last).await;
    let (next, _) = store.next_owed(Sink::FhirFeed).await.expect("the next");
    let (due, _) = next.expect("a record");
    store
        .park(Sink::FhirFeed, due, Parked::Unrenderable)
        .await
        .expect("parked");
    assert!(
        store
            .act_on_parked(
                ParkedAct::Dismiss,
                Sink::FhirFeed,
                AuditChain::System,
                due.id
            )
            .await
            .expect("a dismissal")
    );
    assert!(
        !store
            .act_on_parked(ParkedAct::Retry, Sink::FhirFeed, AuditChain::System, due.id)
            .await
            .expect("a retry of nothing"),
        "a dismissed record is parked no more"
    );
    let (next, waiting) = store.next_owed(Sink::FhirFeed).await.expect("the next");
    assert!(next.is_none(), "nothing is offered");
    assert_eq!(waiting.parked, 0);
}

/// Retention waits for a sink that has not delivered a record until a day
/// past its end, then removes it and parks it for that sink; a delivered
/// record is removed at its end and parked for none.
#[tokio::test]
async fn retention_waits_a_day_for_delivery_then_parks_the_record() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = feeding(&pool).await;
    let today = Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
    let long_ago = today
        .checked_sub(jiff::Span::new().days(10))
        .expect("a date");
    store
        .write(
            vec![record(1), record(2), record(3)],
            &[
                RecordEnd::On(today),
                RecordEnd::On(long_ago),
                RecordEnd::On(today),
            ],
        )
        .await
        .expect("three records");
    let last = last_seq(&pool).await;
    let (first, second, third) = (last - 2, last - 1, last);
    delivering_up_to(&pool, &store, last).await;

    assert_eq!(
        store
            .reap_due(today, Retention::Forever)
            .await
            .expect("reap"),
        1,
        "only the record a day past its end goes undelivered"
    );
    assert_eq!(
        parked(&pool, Sink::FhirFeed).await,
        BTreeMap::from([(("system".to_owned(), second), "reaped".to_owned())]),
        "the undelivered record removed is parked, never skipped"
    );

    let (next, _) = store.next_owed(Sink::FhirFeed).await.expect("the next");
    let (due, _) = next.expect("the first record");
    assert_eq!(due.id, first);
    store
        .mark_delivered(Sink::FhirFeed, due)
        .await
        .expect("accepted");
    assert_eq!(
        store
            .reap_due(today, Retention::Forever)
            .await
            .expect("reap"),
        1,
        "the delivered record goes at its end"
    );
    let remaining: Vec<i64> = sqlx::query_scalar("SELECT seq FROM audit.audit_event ORDER BY seq")
        .fetch_all(&pool)
        .await
        .expect("the records");
    assert_eq!(remaining, [third], "the undelivered one due today waits");
    assert_eq!(parked(&pool, Sink::FhirFeed).await.len(), 1);
}

/// An indexed access-log entry its sink has not delivered, replaced by its
/// tombstone, is parked for that sink, never skipped.
#[tokio::test]
async fn a_tombstoned_entry_is_parked_never_skipped() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = feeding(&pool).await;
    let mut access = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut access.user_id);
    access.object_id = Some("8fa1::ferroehr::1".to_owned());
    access.event_type = Some(EventType::rest("composition_get"));
    store.insert(&access, None).await.expect("the access");
    let (id, uid): (i64, String) = sqlx::query_as(
        "SELECT id, uid::text FROM audit.access_log_entry WHERE render_policy_id IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("the indexed entry");
    let ground: openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText =
        serde_json::from_value(serde_json::json!({
            "_type": "DV_CODED_TEXT", "value": "erasure ordered",
            "defining_code": { "terminology_id": { "value": "local" }, "code_string": "erased" }
        }))
        .expect("the ground");
    let removed: openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime =
        serde_json::from_value(serde_json::json!({
            "_type": "DV_DATE_TIME", "value": "2026-11-22T00:00:00Z"
        }))
        .expect("the time");
    ferroehr::access_log::store::AccessLogStore::new(pool.clone())
        .tombstone(
            &uid,
            removed,
            openehr_base::containers::NonEmptyVec::new(vec![ground]).expect("a ground"),
        )
        .await
        .expect("the tombstone")
        .expect("the entry");
    assert_eq!(
        parked(&pool, Sink::FhirFeed).await,
        BTreeMap::from([(("access_log".to_owned(), id), "tombstoned".to_owned())])
    );
}

/// A sink the configuration no longer names keeps its cursor and is listed
/// with how many records it has not received, until an operator forgets it;
/// configured again, it starts at the last record drawn then.
#[tokio::test]
async fn an_unconfigured_sink_keeps_its_cursor_until_it_is_forgotten() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let configured = feeding(&pool).await;
    configured
        .write(vec![record(1), record(2)], &[])
        .await
        .expect("two records");
    let unconfigured = AuditStore::new(pool.clone());
    unconfigured
        .start_cursors()
        .await
        .expect("no cursor to start");
    assert_eq!(
        unconfigured.unconfigured_sinks().await.expect("the sinks"),
        vec![(Sink::FhirFeed, 2)],
        "the feed missed both records"
    );
    let view = unconfigured.delivery_view().await.expect("the view");
    assert!(
        view.cursors.iter().all(|cursor| !cursor.configured),
        "{view:?}"
    );
    assert_eq!(
        unconfigured
            .forget_sink(Sink::FhirFeed)
            .await
            .expect("forgotten"),
        (2, 0),
        "both of its cursors went"
    );
    assert_eq!(
        unconfigured.unconfigured_sinks().await.expect("the sinks"),
        Vec::new(),
        "the forgotten sink is listed no more"
    );
    let again = feeding(&pool).await;
    let cursor: i64 = sqlx::query_scalar(
        "SELECT through_id FROM audit.delivery_cursor WHERE sink = 'fhir_feed' AND chain = 'system'",
    )
    .fetch_one(&pool)
    .await
    .expect("the new cursor");
    assert_eq!(
        cursor,
        last_seq(&pool).await,
        "a new sink starts at the last record drawn"
    );
    drop(again);
}

/// A delivery sequence names its record, the log by its parity.
#[test]
fn a_delivery_sequence_names_its_record() {
    for due in [
        Due {
            chain: AuditChain::System,
            id: 41,
            retry: false,
        },
        Due {
            chain: AuditChain::AccessLog,
            id: 41,
            retry: false,
        },
    ] {
        let sequence = due.sequence().expect("a sequence");
        assert_eq!(Due::of_sequence(sequence), Some(due));
    }
}
