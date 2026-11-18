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
use std::time::Duration;

use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log::search::LogQuery;
use ferroehr::db::domain::DomainPools;
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::sender::{
    AuditHandle, AuditSender, ResolvedSubject, SubjectLookup, SubjectResolver, start,
};
use ferroehr::system_log::store::{AuditCriterion, AuditSearchFilter};
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
const CEILING: i64 = 1_840;

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

/// The relations an entry takes room in: its row, its link, its shared part,
/// its search tokens and their sets, and the checkpoints that seal it.
const RELATIONS: [&str; 6] = [
    "access_log_entry",
    "access_log_link",
    "access_log_shared",
    "audit_search_token",
    "audit_search_set",
    "access_log_checkpoint",
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
    _drain: AuditHandle,
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
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
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
        _drain: drain,
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

/// The ITI-81 total of the access log, the whole log and a date window, is
/// counted from the time index alone, which carries whether an entry is
/// indexed, so it reads no entry's row and no link (#5784, #5968).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_iti81_total_is_counted_from_the_time_index() {
    let seeded = seed(ENTRIES).await;
    let pool = seeded.db.pool();
    vacuum(&pool).await;
    for count in [
        "SELECT count(*) FROM audit.access_log_search",
        "SELECT count(*) FROM audit.access_log_search \
         WHERE recorded_at >= now() - interval '1 hour' AND recorded_at < now()",
    ] {
        let plan = plan(&pool, count).await;
        assert!(
            plan.contains("\"Index Only Scan\"")
                && plan.contains("\"ix_access_log_entry_time\"")
                && !plan.contains("\"access_log_link\"")
                && !plan.contains("\"Seq Scan\""),
            "`{count}` is counted from the time index alone: {plan}"
        );
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

/// The entries of the ITI-81 total an intake row still names, which no page
/// returns, are found by the entries' key and never by a read of the whole
/// log or of the links (#6000).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_iti81_total_finds_the_unchained_entries_by_their_key() {
    let seeded = seed(ENTRIES).await;
    let pool = seeded.db.pool();
    vacuum(&pool).await;
    for unchained in [
        "SELECT count(*) FROM audit.access_log_search \
         WHERE entry_id = ANY (ARRAY(SELECT unnest(i.entries) FROM audit.access_log_intake i))",
        "SELECT count(*) FROM audit.access_log_search \
         WHERE recorded_at >= now() - interval '1 hour' AND recorded_at < now() \
         AND entry_id = ANY (ARRAY(SELECT unnest(i.entries) FROM audit.access_log_intake i))",
    ] {
        let plan = plan(&pool, unchained).await;
        let nodes = plan_nodes(&serde_json::from_str(&plan).expect("a plan"));
        let reads = |relation: &str, node: &str| {
            nodes
                .iter()
                .any(|(kind, read, _)| kind == node && read == relation)
        };
        assert!(
            nodes
                .iter()
                .any(|(_, _, index)| index == "pk_access_log_entry")
                && !reads("access_log_entry", "Seq Scan")
                && !nodes.iter().any(|(_, read, _)| read == "access_log_link"),
            "`{unchained}` finds the entries by their key: {plan}"
        );
    }
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
    seeded
        .sender
        .store()
        .search(&AuditSearchFilter {
            criteria,
            count: 1,
            total: true,
            ..AuditSearchFilter::default()
        })
        .await
        .expect("the ITI-81 search answers")
        .total
        .expect("the search counts")
}

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
