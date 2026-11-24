// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The bytes an access-log entry takes on disk, measured on a seeded log
//! against a real `PostgreSQL` 18 (shared testkit harness, #5784).
//!
//! A deployment writes one entry per access, so the size of an entry decides
//! how far the log grows. The seed is EHR reads through the audit drain, as
//! the REST layer records them: many EHRs, a few users, varied addresses,
//! a few accesses per request. No openEHR spec governs audit storage: our
//! own design.

use std::sync::Arc;

use sqlx::Connection as _;
use std::time::Duration;

use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log::search::LogQuery;
use ferroehr::db::domain::DomainPools;
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::sender::{
    AuditHandle, AuditSender, ResolvedSubject, SubjectLookup, SubjectResolver, start,
};
use ferroehr::system_log::store::{AuditCriterion, AuditSearchFilter, AuditStore};
use ferroehr::versioning::signature::trust::TrustedCertificates;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

/// The system the log records as.
const SYSTEM: &str = "cdr.example.org";

/// The entries the seed writes.
const ENTRIES: u32 = 10_000;

/// The EHRs the seed reads.
const EHRS: u32 = 500;

/// The users who read them.
const USERS: u32 = 50;

/// The accesses one request records.
const PER_REQUEST: u32 = 8;

/// The ceiling on the bytes an entry takes in every relation of the access
/// log, heap and indexes: this seed measured 1 844 to 1 893 with the index
/// in a table of its own, and 1 781 to 1 806 with one row per entry (#5784).
/// Written once with no intake row or link (#6242) it measured 1 154 to
/// 1 193 against 1 327 to 1 391 beside the merge base, and the ceiling fell
/// by the drop of the median, 187.
const CEILING: i64 = 1_653;

/// The EHR id of EHR `n`.
fn ehr(n: u32) -> String {
    format!("7d44b88c-4199-4bad-97dc-{n:012x}")
}

/// A resolver that names the subject of every EHR.
fn subjects() -> SubjectResolver {
    Arc::new(|ehr_id: String| {
        Box::pin(async move {
            SubjectLookup::Found(ResolvedSubject {
                id: format!("patient-{}", ehr_id.get(24..).unwrap_or_default()),
                namespace: Some("nl.example.patients".to_owned()),
            })
        })
    })
}

/// Read `n` of the seed.
fn read(n: u32) -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    event.user_id = format!("user-{}", n % USERS);
    event.client_ip = Some(format!("10.0.{}.{}", n % 7, n % 251));
    event.ehr_id = Some(ehr((n * 7919) % EHRS));
    event
}

/// The relations an entry takes room in: its row, its search index row, its
/// shared part, its search tokens and their sets, and the seals of its
/// windows.
const RELATIONS: [&str; 6] = [
    "access_log_entry",
    "access_log_index",
    "access_log_shared",
    "audit_search_token",
    "audit_search_set",
    "access_log_seal",
];

/// Each relation of [`RELATIONS`] with the bytes its heap and its indexes
/// take.
async fn sizes(pool: &sqlx::PgPool) -> Vec<(String, i64, i64)> {
    sqlx::query_as(
        "SELECT c.relname::text, pg_table_size(c.oid), pg_indexes_size(c.oid) FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'audit' AND c.relkind = 'r' AND c.relname = ANY ($1) \
         ORDER BY c.relname",
    )
    .bind(RELATIONS)
    .fetch_all(pool)
    .await
    .expect("the sizes are read")
}

/// A log seeded with `entries` reads, its drain, its checkpoints and its
/// pool, every entry chained.
struct Seeded {
    db: testkit::TestDb,
    drain: Option<AuditHandle>,
    sender: AuditSender,
    checkpoints: Checkpoints,
}

/// Seed a fresh log with the reads `0..entries` and wait for the chainer to
/// link every one.
async fn seed(entries: u32) -> Seeded {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let pools = DomainPools::from_shared(&pool);
    let config = ferroehr::system_log::config::AuditConfig {
        enabled: true,
        ..ferroehr::system_log::config::AuditConfig::default()
    };
    let (sender, drain) = start(config, Some(subjects()), pools.audit.clone())
        .await
        .expect("the audit drain starts");
    let signer =
        crate::access_log_checkpoint::keyring(&crate::access_log_checkpoint::generate_key());
    let checkpoints = Checkpoints::new(
        pools.audit.clone(),
        SYSTEM.to_owned(),
        signer,
        Arc::new(TrustedCertificates::default()),
    );
    assert!(
        sender.store().use_access_log(checkpoints.store().clone()),
        "the drain appends through the signing store"
    );
    for first in (0..entries).step_by(usize::try_from(PER_REQUEST).expect("a step")) {
        let events = (first..first + PER_REQUEST).map(read).collect();
        sender
            .record(events, None)
            .await
            .expect("the reads are recorded");
    }
    let expected = i64::from(entries);
    let deadline =
        std::time::Instant::now() + Duration::from_secs(120 * u64::from(entries.div_ceil(ENTRIES)));
    loop {
        let chained: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_record")
            .fetch_one(&pool)
            .await
            .expect("the access log is counted");
        if chained == expected {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "every read is one entry: {chained} of {expected}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Seeded {
        db,
        drain: Some(drain),
        sender,
        checkpoints,
    }
}

/// Vacuum and analyse every relation of [`RELATIONS`], as autovacuum leaves
/// them between its runs.
async fn vacuum(pool: &sqlx::PgPool) {
    for table in RELATIONS {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "VACUUM (ANALYZE) audit.{table}"
        )))
        .execute(pool)
        .await
        .expect("the relation is vacuumed");
    }
}

/// The plan of `count`, a statement counting the ITI-81 index, as JSON text.
async fn plan(pool: &sqlx::PgPool, count: &str) -> String {
    let plan: serde_json::Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN (FORMAT JSON) {count}"
    )))
    .fetch_one(pool)
    .await
    .expect("the count is planned");
    plan.to_string()
}

/// The ITI-81 total of the access log, the whole log and a date window,
/// reads no indexed entry's row: each is counted from the search index's time
/// index alone, which carries whether an entry is indexed, or from the
/// index's own narrow rows. The entries
/// past the index's cursor are read by a key range of their rows, and the
/// entries a write holds are left out by their key (#5784, #5968, #6242,
/// #6319).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_iti81_total_is_counted_from_the_time_index() {
    let seeded = seed(ENTRIES).await;
    let pool = seeded.db.pool();
    let last: i64 = sqlx::query_scalar("SELECT max(id) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the last entry id reads");
    let cursor = seeded
        .sender
        .store()
        .index_access_log(last)
        .await
        .expect("the index moves");
    vacuum(&pool).await;
    // NOTE: our own design (#6319), where the count takes most of the index the planner may read
    // its narrow rows in place of its time index; neither reads an entry's row.
    for count in [
        format!("SELECT count(*) FROM audit.access_log_search({cursor})"),
        format!(
            "SELECT count(*) FROM audit.access_log_search({cursor}) \
             WHERE recorded_at >= now() - interval '1 hour' AND recorded_at < now()"
        ),
    ] {
        let plan = plan(&pool, &count).await;
        let nodes = plan_nodes(&serde_json::from_str(&plan).expect("a plan"));
        let indexed = |kind: &str, index: &str| {
            (kind == "Index Only Scan" && index == "ix_access_log_index_time") || kind == "Seq Scan"
        };
        assert!(
            nodes
                .iter()
                .any(|(kind, read, index)| read == "access_log_index" && indexed(kind, index))
                && nodes
                    .iter()
                    .all(|(kind, read, index)| read != "access_log_index" || indexed(kind, index))
                && nodes
                    .iter()
                    .all(|(kind, read, index)| read != "access_log_entry"
                        || (kind.starts_with("Index") && index == "pk_access_log_entry")),
            "`{count}` reads no indexed entry's row, and past the cursor reads by key: {plan}"
        );
    }
}

/// The newest entries of the ITI-81 search, the person's trail and a page of
/// the log are read through the search index, merged with the few past its
/// cursor, never by a sort of the whole index (#6319).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_newest_page_reads_the_index_in_its_order() {
    let seeded = seed(ENTRIES).await;
    let pool = seeded.db.pool();
    let last: i64 = sqlx::query_scalar("SELECT max(id) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the last entry id reads");
    let cursor = seeded
        .sender
        .store()
        .index_access_log(last)
        .await
        .expect("the index moves");
    vacuum(&pool).await;
    for (page, index) in [
        (
            format!(
                "SELECT chain_seq FROM audit.access_log_search({cursor}) \
                 ORDER BY recorded_at DESC LIMIT 21"
            ),
            "ix_access_log_index_time",
        ),
        (
            "SELECT chain_seq, content FROM audit.access_log_record \
             ORDER BY time_recorded DESC LIMIT 21"
                .to_owned(),
            "ix_access_log_index_time",
        ),
        (
            format!(
                "SELECT chain_seq FROM audit.access_log_search({cursor}) \
                 WHERE patient_id = 'patient-000000000007' ORDER BY recorded_at DESC LIMIT 21"
            ),
            "ix_access_log_index_patient",
        ),
    ] {
        let plan = plan(&pool, &page).await;
        let nodes = plan_nodes(&serde_json::from_str(&plan).expect("a plan"));
        // NOTE: our own design (#6373), a filter the planner knows to match a few rows may read
        // them through its index as a bitmap and sort those few.
        assert!(
            nodes.iter().any(|(kind, read, used)| used == index
                && (kind == "Bitmap Index Scan"
                    || (read == "access_log_index" && kind.starts_with("Index"))))
                && nodes
                    .iter()
                    .all(|(kind, read, _)| read != "access_log_index" || kind != "Seq Scan"),
            "`{page}` reads {index}, never the whole index: {plan}"
        );
    }
}

/// An ITI-81 page reads each access-log record it serves by one primary-key
/// read of its row, with entries past the search index's cursor (#6320).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_iti81_page_reads_each_record_it_serves_by_its_key() {
    let seeded = seed(ENTRIES).await;
    let pool = seeded.db.pool();
    let last: i64 = sqlx::query_scalar("SELECT max(id) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the last entry id reads");
    // The newest entries stay past the cursor, as between two frontier rounds.
    let cursor = seeded
        .sender
        .store()
        .index_access_log(last - i64::from(PER_REQUEST) * 4)
        .await
        .expect("the index moves");
    vacuum(&pool).await;
    let filter = AuditSearchFilter {
        count: 20,
        ..AuditSearchFilter::default()
    };
    let (sql, values) =
        ferroehr::system_log::store::search_page::page_statement(&pool, &filter, cursor)
            .await
            .expect("the criteria resolve")
            .expect("a page");
    let plan: serde_json::Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("EXPLAIN (ANALYZE, FORMAT JSON) {sql}")),
        values,
    )
    .fetch_one(&pool)
    .await
    .expect("the page is planned and run");
    let mut loops = Vec::new();
    executed(&plan, &mut loops);
    assert!(
        loops
            .iter()
            .filter(|(_, read, ..)| {
                ["access_log_index", "access_log_index_state"].contains(&read.as_str())
            })
            .all(|(.., n)| *n <= 1),
        "the search index and its cursor are read for the page's keys, never once a record: \
         {plan}"
    );
    assert!(
        loops
            .iter()
            .filter(|(_, read, ..)| read == "access_log_entry")
            .all(|(kind, _, index, _)| kind.starts_with("Index") && index == "pk_access_log_entry"),
        "an entry's row is read by its key: {plan}"
    );
    assert!(
        loops
            .iter()
            .any(|(_, read, index, n)| read == "access_log_entry"
                && index == "pk_access_log_entry"
                && *n == 20),
        "each of the 20 records served is read by its key once: {plan}"
    );
}

/// Every executed node of `plan` as (type, relation, index, loops).
fn executed(plan: &serde_json::Value, nodes: &mut Vec<(String, String, String, u64)>) {
    match plan {
        serde_json::Value::Array(items) => items.iter().for_each(|item| executed(item, nodes)),
        serde_json::Value::Object(members) => {
            if let Some(kind) = members.get("Node Type").and_then(serde_json::Value::as_str) {
                let text = |name: &str| {
                    members
                        .get(name)
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                let loops = members
                    .get("Actual Loops")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                nodes.push((
                    kind.to_owned(),
                    text("Relation Name"),
                    text("Index Name"),
                    loops,
                ));
            }
            members.values().for_each(|value| executed(value, nodes));
        }
        _ => {}
    }
}

/// Every node of the plan `plan`: its type, the relation it reads and the
/// index it reads it by, empty when it names none.
fn plan_nodes(plan: &serde_json::Value) -> Vec<(String, String, String)> {
    let mut nodes = Vec::new();
    let mut pending = vec![plan];
    while let Some(value) = pending.pop() {
        match value {
            serde_json::Value::Array(items) => pending.extend(items),
            serde_json::Value::Object(members) => {
                if let Some(kind) = members.get("Node Type").and_then(serde_json::Value::as_str) {
                    let text = |name: &str| {
                        members
                            .get(name)
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    nodes.push((kind.to_owned(), text("Relation Name"), text("Index Name")));
                }
                pending.extend(members.values());
            }
            _ => {}
        }
    }
    nodes
}

/// The seeded log takes no more than [`CEILING`] bytes per entry, every
/// relation of the access log counted, after a vacuum, as the log stands
/// between the autovacuum runs of a deployment.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_entry_takes_no_more_than_its_ceiling_on_disk() {
    let seeded = seed(ENTRIES).await;
    let pool = seeded.db.pool();
    let expected = i64::from(ENTRIES);
    vacuum(&pool).await;
    let sizes = sizes(&pool).await;
    let total: i64 = sizes.iter().map(|(_, heap, indexes)| heap + indexes).sum();
    let per_entry = total.div_euclid(expected);
    let table = sizes
        .iter()
        .map(|(name, heap, indexes)| {
            format!(
                "{name}: {} + {} B/entry (heap + indexes)",
                heap.div_euclid(expected),
                indexes.div_euclid(expected)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    println!("access-log storage over {ENTRIES} entries: {per_entry} B/entry ({table})");
    assert!(
        per_entry <= CEILING,
        "an entry takes {per_entry} bytes, above {CEILING}: {table}"
    );
}

/// The ITI-81 search of `filter`, counted.
async fn found(seeded: &Seeded, criteria: Vec<AuditCriterion>) -> i64 {
    found_in(seeded.sender.store(), criteria).await
}

/// The ITI-81 total of the records `criteria` match in `store`, asked again
/// while it is refused as behind until [`SETTLED`] passes.
///
/// The frontier follows the `xmin` of the whole server, so a transaction
/// another test holds open can keep the index behind for longer than one
/// search waits (#6475).
async fn found_in(store: &AuditStore, criteria: Vec<AuditCriterion>) -> i64 {
    let filter = AuditSearchFilter {
        criteria,
        count: 1,
        total: true,
        ..AuditSearchFilter::default()
    };
    let deadline = std::time::Instant::now() + SETTLED;
    let answered = loop {
        match store.search(&filter).await {
            Err(ferroehr::system_log::AuditError::IndexBehind(_))
                if std::time::Instant::now() < deadline => {}
            answered => break answered,
        }
    };
    answered
        .expect("the ITI-81 search answers")
        .total
        .expect("the search counts")
}

/// How long [`found_in`] asks again while other tests' transactions hold
/// the server's `xmin` back.
const SETTLED: Duration = Duration::from_secs(120);

/// The reads of the seed `0..entries` that `keep` keeps, counted.
fn reads(entries: u32, keep: impl Fn(&AuditEvent) -> bool) -> i64 {
    i64::try_from((0..entries).map(read).filter(|event| keep(event)).count()).expect("a count")
}

/// On the seeded log the verifier, the ITI-81 search, the reads of one
/// EHR's log and the erasure of an entry answer as the definitions of each
/// say, whatever form the log stores an entry in (#5784).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_seeded_log_verifies_searches_reads_and_erases() {
    const SEEDED: u32 = 400;
    let seeded = seed(SEEDED).await;
    let pool = seeded.db.pool();
    let findings = crate::fixtures::verify_chain(&pool)
        .await
        .expect("the chains are verified");
    assert!(findings.is_empty(), "the seeded log verifies: {findings:?}");

    let patient = "patient-000000000007";
    let of_patient = |event: &AuditEvent| event.ehr_id.as_deref() == Some(&ehr(7));
    assert_eq!(
        found(
            &seeded,
            vec![AuditCriterion::Patient(vec![patient.to_owned()])]
        )
        .await,
        reads(SEEDED, of_patient)
    );
    let of_user = |event: &AuditEvent| event.user_id == "user-3";
    assert_eq!(
        found(
            &seeded,
            vec![AuditCriterion::Agent(vec!["user-3".to_owned()])]
        )
        .await,
        reads(SEEDED, of_user)
    );
    let from_address = |event: &AuditEvent| {
        event
            .client_ip
            .as_deref()
            .is_some_and(|address| address.contains("10.0.2.9"))
    };
    assert_eq!(
        found(
            &seeded,
            vec![AuditCriterion::Address(vec!["10.0.2.9".to_owned()])]
        )
        .await,
        reads(SEEDED, from_address)
    );
    assert_eq!(found(&seeded, Vec::new()).await, i64::from(SEEDED));

    let log = seeded.checkpoints.store();
    let own = log
        .search(&LogQuery {
            ehr_id: Some(ehr(7)),
            ..LogQuery::default()
        })
        .await
        .expect("the EHR's log is read");
    assert_eq!(
        i64::try_from(own.len()).expect("a count"),
        reads(SEEDED, of_patient)
    );
    let Some(AccessLogRecord::AccessLogEntry(erased)) = own.first() else {
        panic!("the EHR's log holds entries: {own:?}");
    };
    let time_removed: DvDateTime = serde_json::from_value(serde_json::json!({
        "_type": "DV_DATE_TIME", "value": "2031-11-08T00:00:00Z"
    }))
    .expect("the time reads");
    let ground: DvCodedText = serde_json::from_value(serde_json::json!({
        "_type": "DV_CODED_TEXT", "value": "erasure",
        "defining_code": { "terminology_id": { "value": "local" }, "code_string": "erased" }
    }))
    .expect("the ground reads");
    log.tombstone(
        erased.uid.value(),
        time_removed,
        NonEmptyVec::new(vec![ground]).expect("one ground"),
    )
    .await
    .expect("the tombstone is written")
    .expect("the entry is there");

    let own_after = log
        .search(&LogQuery {
            ehr_id: Some(ehr(7)),
            ..LogQuery::default()
        })
        .await
        .expect("the EHR's log is read");
    assert_eq!(
        own_after.len() + 1,
        own.len(),
        "the erased entry leaves the EHR's log"
    );
    assert_eq!(
        found(
            &seeded,
            vec![AuditCriterion::Patient(vec![patient.to_owned()])]
        )
        .await,
        reads(SEEDED, of_patient) - 1,
        "the erased entry leaves the ITI-81 search"
    );
    assert_eq!(found(&seeded, Vec::new()).await, i64::from(SEEDED) - 1);
    assert!(
        matches!(
            log.read(erased.uid.value()).await.expect("read"),
            Some(AccessLogRecord::AccessLogTombstone(_))
        ),
        "the entry reads as its tombstone"
    );
    let findings = crate::fixtures::verify_chain(&pool)
        .await
        .expect("the chains are verified");
    assert!(
        findings.is_empty(),
        "the log verifies over the tombstone: {findings:?}"
    );
}

/// What the readers of the seeded log answer: [`column_answers`] and
/// [`token_answer`] of its store.
async fn answers(seeded: &Seeded) -> String {
    let store = seeded.sender.store();
    format!(
        "{}\n{}",
        column_answers(seeded, store).await,
        token_answer(store).await
    )
}

/// The ITI-81 totals of a patient, an agent and the whole log and its newest
/// page from `store`, one EHR's log and one entry read by its uid.
async fn column_answers(seeded: &Seeded, store: &AuditStore) -> String {
    let mut answers = Vec::new();
    for criteria in [
        vec![AuditCriterion::Patient(vec![
            "patient-000000000007".to_owned(),
        ])],
        vec![AuditCriterion::Agent(vec!["user-3".to_owned()])],
        Vec::new(),
    ] {
        answers.push(found_in(store, criteria).await.to_string());
    }
    let page = store
        .search(&AuditSearchFilter {
            count: 50,
            total: true,
            ..AuditSearchFilter::default()
        })
        .await
        .expect("the ITI-81 page answers");
    answers.push(format!(
        "{:?} {} {:?}",
        page.total, page.more, page.documents
    ));
    let log = seeded.checkpoints.store();
    let own = log
        .search(&LogQuery {
            ehr_id: Some(ehr(7)),
            ..LogQuery::default()
        })
        .await
        .expect("the EHR's log is read");
    answers.push(format!("{own:?}"));
    let Some(AccessLogRecord::AccessLogEntry(first)) = own.first() else {
        panic!("the EHR's log holds entries: {own:?}");
    };
    let read = log.read(first.uid.value()).await.expect("the entry reads");
    answers.push(format!("{read:?}"));
    answers.join("\n")
}

/// The ITI-81 total of an address from `store`, which moves the index past
/// every entry first (#6322).
async fn token_answer(store: &AuditStore) -> String {
    found_in(
        store,
        vec![AuditCriterion::Address(vec!["10.0.2.9".to_owned()])],
    )
    .await
    .to_string()
}

/// The rows of the search index, as text, in entry order.
async fn index_rows(pool: &sqlx::PgPool) -> String {
    sqlx::query_scalar(
        "SELECT coalesce(string_agg(i::text, E'\\n' ORDER BY i.entry_id), '') \
         FROM audit.access_log_index i",
    )
    .fetch_one(pool)
    .await
    .expect("the index is read")
}

/// The search index lags the entries by up to a frontier round (#6319): with
/// rounds held while entries stand past the cursor, with the cursor an ITI-81
/// page binds behind the stored one (#6373), and with the cursor behind after
/// a crash, every reader answers as with every entry indexed. A token reader
/// moves the index past every entry first (#6322), and the next round renders
/// and copies the same rows again, search tokens included, even after the
/// store's token generation moved on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[expect(
    clippy::disallowed_methods,
    reason = "a test moves the token generation on a raw pool connection, as only the owner can"
)]
async fn every_entry_is_found_wherever_the_index_cursor_stands() {
    const SEEDED: u32 = 400;
    const PAST: u32 = PER_REQUEST * 2;
    let mut seeded = seed(SEEDED).await;
    let pool = seeded.db.pool();
    let store = seeded.sender.store().clone();

    // No round commits while the lock is held, as when each one fails before its commit.
    let mut held = pool.acquire().await.expect("a connection");
    sqlx::query("SELECT pg_advisory_lock(hashtext('audit.access_log_index'), 0)")
        .execute(&mut *held)
        .await
        .expect("the indexing lock is held");
    for first in (SEEDED..SEEDED + PAST).step_by(usize::try_from(PER_REQUEST).expect("a step")) {
        let events = (first..first + PER_REQUEST).map(read).collect();
        seeded
            .sender
            .record(events, None)
            .await
            .expect("the reads are recorded");
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    while sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit.access_log_record")
        .fetch_one(&pool)
        .await
        .expect("the access log is counted")
        < i64::from(SEEDED + PAST)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the reads are chained"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let last: i64 = sqlx::query_scalar("SELECT max(id) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the last entry id reads");
    let stored: i64 =
        sqlx::query_scalar("SELECT last_id FROM audit.access_log_index_state WHERE singleton")
            .fetch_one(&pool)
            .await
            .expect("the cursor reads");
    assert!(
        stored < last,
        "entries stand past the cursor: {stored} of {last}"
    );
    // The token reader would wait on the held rounds, so it is left out here.
    let behind = column_answers(&seeded, &store).await;
    sqlx::query("SELECT pg_advisory_unlock(hashtext('audit.access_log_index'), 0)")
        .execute(&mut *held)
        .await
        .expect("the indexing lock is released");
    drop(held);

    assert_eq!(
        store.index_access_log(last).await.expect("the index moves"),
        last,
        "every entry is indexed"
    );
    let rows = index_rows(&pool).await;
    assert_eq!(
        i64::try_from(rows.lines().count()).expect("a count"),
        i64::from(SEEDED + PAST),
        "one index row per entry"
    );
    // The store's first page after the round binds the cursor it saw before the round.
    assert_eq!(
        column_answers(&seeded, &store).await,
        behind,
        "the readers answer the same with the page's cursor behind the stored one"
    );
    assert_eq!(
        column_answers(&seeded, &store).await,
        behind,
        "and with every entry indexed"
    );
    let indexed = answers(&seeded).await;
    let tokenless: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit.access_log_index \
          WHERE render_policy_id IS NOT NULL AND coalesce(cardinality(search_tokens), 0) = 0",
    )
    .fetch_one(&pool)
    .await
    .expect("the index is read");
    assert_eq!(
        tokenless, 0,
        "every indexed entry carries its search tokens"
    );

    // The frontier round stops, as by a crash, while the cursor stands behind; the
    // readers are those of a restarted store, which has seen no cursor.
    seeded
        .drain
        .take()
        .expect("the drain runs")
        .shutdown(Duration::from_secs(10))
        .await;
    for cursor in [last.div_euclid(2), 0] {
        cursor_at(&pool, cursor).await;
        let restarted = AuditStore::new(pool.clone());
        assert_eq!(
            format!(
                "{}\n{}",
                column_answers(&seeded, &restarted).await,
                token_answer(&restarted).await
            ),
            indexed,
            "the readers answer the same with the cursor at {cursor}"
        );
    }
    cursor_at(&pool, 0).await;
    let mut tx = pool.begin().await.expect("begin");
    for statement in [
        "ALTER TABLE audit.audit_search_token_generation DISABLE TRIGGER USER",
        "UPDATE audit.audit_search_token_generation SET generation = generation + 1",
        "ALTER TABLE audit.audit_search_token_generation ENABLE TRIGGER USER",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect("the generation moves on");
    }
    tx.commit().await.expect("commit");
    assert_eq!(
        store.index_access_log(last).await.expect("the index moves"),
        last
    );
    assert_eq!(
        index_rows(&pool).await,
        rows,
        "the next round renders and copies the same rows"
    );
    assert_eq!(answers(&seeded).await, indexed);
}

/// The entries the ITI-81 page's scale test seeds (#6373).
const AT_SCALE: u32 = 200_000;

/// On a log of [`AT_SCALE`] entries an ITI-81 page, unfiltered and by a
/// patient, a date window and an address, starts no parallel worker and reads
/// fewer than 1,000 buffers, with the index cursor caught up and with entries
/// past it (#6373).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_iti81_page_at_scale_starts_no_worker_and_reads_few_buffers() {
    let seeded = seed(AT_SCALE).await;
    let pool = seeded.db.pool();
    let last: i64 = sqlx::query_scalar("SELECT max(id) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the last entry id reads");
    let now = jiff::Timestamp::now();
    let filters = [
        ("unfiltered", Vec::new(), None),
        (
            "patient",
            vec![AuditCriterion::Patient(vec![
                "patient-000000000007".to_owned(),
            ])],
            None,
        ),
        (
            "date",
            Vec::new(),
            Some(now - jiff::SignedDuration::from_secs(3600)),
        ),
        (
            "address",
            vec![AuditCriterion::Address(vec!["10.0.2.9".to_owned()])],
            None,
        ),
    ];
    for behind in [2_000, i64::from(PER_REQUEST) * 4, 0] {
        let cursor = seeded
            .sender
            .store()
            .index_access_log(last - behind)
            .await
            .expect("the index moves");
        vacuum(&pool).await;
        for (name, criteria, from) in &filters {
            let filter = AuditSearchFilter {
                count: 20,
                criteria: criteria.clone(),
                from: *from,
                ..AuditSearchFilter::default()
            };
            let (sql, values) =
                ferroehr::system_log::store::search_page::page_statement(&pool, &filter, cursor)
                    .await
                    .expect("the criteria resolve")
                    .expect("a page");
            let mut conn = ferroehr::db::acquire::acquire(&pool)
                .await
                .expect("a connection");
            sqlx::query("SET max_parallel_workers_per_gather = 2")
                .execute(&mut *conn)
                .await
                .expect("parallel plans are allowed");
            let mut plan = serde_json::Value::Null;
            for _ in 0..3 {
                plan = sqlx::query_scalar_with(
                    sqlx::AssertSqlSafe(format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}")),
                    values.clone(),
                )
                .fetch_one(&mut *conn)
                .await
                .expect("the page is planned and run");
            }
            let mut nodes = Vec::new();
            executed(&plan, &mut nodes);
            let buffers = ["Shared Hit Blocks", "Shared Read Blocks"]
                .iter()
                .map(|field| plan[0]["Plan"][field].as_u64().unwrap_or_default())
                .sum::<u64>();
            eprintln!(
                "ITI-81 page at {AT_SCALE}, {behind} behind, {name}: {buffers} buffers, {} ms",
                plan[0]["Execution Time"]
            );
            assert!(
                nodes.iter().all(|(kind, ..)| !kind.starts_with("Gather")),
                "{behind} behind, {name}: the page starts no worker: {plan}"
            );
            assert!(
                buffers < 1_000,
                "{behind} behind, {name}: the page reads {buffers} buffers: {plan}"
            );
        }
    }
}

/// An ITI-81 page bound to a cursor behind the stored one serves the same
/// records as one bound to the stored cursor: the entries the index holds
/// past the bound are read from the index alone (#6373).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_page_bound_behind_the_cursor_serves_the_same_records() {
    let seeded = seed(ENTRIES).await;
    let pool = seeded.db.pool();
    let last: i64 = sqlx::query_scalar("SELECT max(id) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the last entry id reads");
    let stored = seeded
        .sender
        .store()
        .index_access_log(last - i64::from(PER_REQUEST) * 4)
        .await
        .expect("the index moves");
    let filter = AuditSearchFilter {
        count: 20,
        ..AuditSearchFilter::default()
    };
    let page = |cursor: i64| {
        let pool = pool.clone();
        let filter = filter.clone();
        async move {
            let (sql, values) =
                ferroehr::system_log::store::search_page::page_statement(&pool, &filter, cursor)
                    .await
                    .expect("the criteria resolve")
                    .expect("a page");
            let rows: Vec<(String, i64)> = sqlx::query_as_with(
                sqlx::AssertSqlSafe(format!("SELECT p.chain, p.chain_seq FROM ({sql}) p")),
                values,
            )
            .fetch_all(&pool)
            .await
            .expect("the page reads");
            rows
        }
    };
    let exact = page(stored).await;
    assert_eq!(exact.len(), 21, "a page and the record past it: {exact:?}");
    for behind in [1, 500, stored] {
        assert_eq!(
            page(stored - behind).await,
            exact,
            "a cursor {behind} behind the stored one"
        );
    }
}

/// Stands the search index's cursor at `cursor`, its rows past it gone, as a
/// round a crash lost leaves them.
async fn cursor_at(pool: &sqlx::PgPool, cursor: i64) {
    sqlx::query("DELETE FROM audit.access_log_index WHERE entry_id > $1")
        .bind(cursor)
        .execute(pool)
        .await
        .expect("the lost round's rows are gone");
    sqlx::query("UPDATE audit.access_log_index_state SET last_id = $1")
        .bind(cursor)
        .execute(pool)
        .await
        .expect("the cursor stands behind");
}

/// The ITI-81 address search of `address`, counted ([`found_in`]).
async fn by_address(store: &AuditStore, address: &str) -> i64 {
    found_in(
        store,
        vec![AuditCriterion::Address(vec![address.to_owned()])],
    )
    .await
}

/// A read from `address`, recorded through the sender as a request records
/// it.
async fn recorded_from(seeded: &Seeded, n: u32, address: &str) {
    let mut event = read(n);
    event.client_ip = Some(address.to_owned());
    seeded
        .sender
        .record(vec![event], None)
        .await
        .expect("the read is recorded");
}

/// A search on a token parameter finds an entry written a moment before it,
/// though no frontier round indexed it: the search moves the frontier and
/// the index past it first (#6322).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_token_search_finds_an_entry_written_a_moment_before() {
    let mut seeded = seed(16).await;
    let pool = seeded.db.pool();
    let store = seeded.sender.store().clone();
    seeded
        .drain
        .take()
        .expect("the drain runs")
        .shutdown(Duration::from_secs(10))
        .await;
    recorded_from(&seeded, 7, "10.77.0.1").await;
    let written: i64 = sqlx::query_scalar("SELECT max(id) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the newest entry");
    let cursor: i64 =
        sqlx::query_scalar("SELECT last_id FROM audit.access_log_index_state WHERE singleton")
            .fetch_one(&pool)
            .await
            .expect("the cursor");
    assert!(
        cursor < written,
        "no round indexed the new entry: {cursor} against {written}"
    );
    assert_eq!(
        by_address(&store, "10.77.0.1").await,
        1,
        "the new entry is found"
    );
}

/// A token search whose entries an open transaction keeps behind the
/// frontier is refused within its wait, never left hanging, and answers
/// exactly once the transaction ends (#6322).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[expect(
    clippy::disallowed_methods,
    reason = "a test holds a transaction open on a raw pool connection, outside any request"
)]
async fn a_token_search_held_back_by_an_open_transaction_is_refused_in_time() {
    let seeded = seed(16).await;
    let pool = seeded.db.pool();
    let store = seeded.sender.store().clone();
    let mut conn = pool.acquire().await.expect("a connection");
    let mut open = conn.begin().await.expect("begin");
    sqlx::query("SELECT pg_current_xact_id()")
        .execute(&mut *open)
        .await
        .expect("the open transaction takes an xid");
    recorded_from(&seeded, 9, "10.77.0.2").await;
    let started = std::time::Instant::now();
    let refused = store
        .search(&AuditSearchFilter {
            criteria: vec![AuditCriterion::Address(vec!["10.77.0.2".to_owned()])],
            count: 1,
            total: true,
            ..AuditSearchFilter::default()
        })
        .await;
    let waited = started.elapsed();
    assert!(
        matches!(
            refused,
            Err(ferroehr::system_log::AuditError::IndexBehind(_))
        ),
        "the search is refused while the index cannot catch up: {refused:?}"
    );
    assert!(
        waited < ferroehr::system_log::store::access_index::CATCH_UP_WAIT + Duration::from_secs(1),
        "the refusal comes within the wait: {waited:?}"
    );
    open.rollback().await.expect("the open transaction ends");
    assert_eq!(
        by_address(&store, "10.77.0.2").await,
        1,
        "the search then answers exactly"
    );
}

/// Token searches that start together run at most one catch-up round
/// between them: one moves the index past every entry, and each other finds
/// it there and only reads (#6322).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_token_searches_run_one_catch_up_round() {
    const SEARCHES: usize = 8;
    let mut seeded = seed(16).await;
    let store = seeded.sender.store().clone();
    seeded
        .drain
        .take()
        .expect("the drain runs")
        .shutdown(Duration::from_secs(10))
        .await;
    recorded_from(&seeded, 11, "10.77.0.3").await;
    let before = store.catch_up_rounds();
    let mut searches = tokio::task::JoinSet::new();
    for _ in 0..SEARCHES {
        let store = store.clone();
        searches.spawn(async move { by_address(&store, "10.77.0.3").await });
    }
    while let Some(found) = searches.join_next().await {
        assert_eq!(found.expect("a search"), 1, "each search finds the entry");
    }
    assert!(
        store.catch_up_rounds() - before <= 1,
        "{} rounds for {SEARCHES} searches",
        store.catch_up_rounds() - before
    );
}
