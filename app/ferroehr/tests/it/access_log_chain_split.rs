// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The split of the shared audit chain (#5573,
//! `migrations/audit/0031_access_log_own_chain.sql`) against a real
//! `PostgreSQL` 18: a database migrated to the shared chain, holding
//! `audit_event` records and access-log entries interleaved, is migrated on,
//! and each chain verifies apart, the access log renumbered from 1 and the
//! `audit_event` records kept where they were, and both chains go on.
//!
//! No openEHR spec governs audit storage: our own design.

use jiff::Timestamp;
use serde_json::json;
use sqlx::migrate::Migrator;

use ferroehr::access_log::chain::link_digest;
use ferroehr::access_log::store::AccessLogStore;
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::store::AuditStore;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

static EXT: Migrator = sqlx::migrate!("migrations/ext");
static AUDIT: Migrator = sqlx::migrate!("migrations/audit");

/// The last audit migration of the shared chain.
const SHARED: i64 = 30;

/// The proposal's example entry with the uid and EHR of number `n`.
fn entry(n: u32) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["ehr_id"]["value"] = json!(format!("7d44b88c-4199-4bad-97dc-{n:012x}"));
    serde_json::from_value(value).expect("the example entry reads")
}

/// A record outside the access log: a read of a party.
fn outside() -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Demographic,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut event.user_id);
    event.timestamp = Timestamp::now();
    event
}

/// Every finding of the full verification of each chain, as text.
async fn findings(pool: &sqlx::PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT finding FROM audit.verify_audit_chain() \
         UNION ALL SELECT finding FROM audit.verify_access_log_chain()",
    )
    .fetch_all(pool)
    .await
    .expect("verify both chains")
}

/// The interleaved shared chain splits into two chains that verify, the
/// access log's export span no longer holds a record outside it, and both
/// chains go on from their heads.
#[tokio::test]
async fn the_shared_chain_splits_into_two_that_verify() {
    let db = testkit::empty_db().await.expect("an empty database");
    let pool = db.pool();
    let mut conn = pool.acquire().await.expect("a connection");
    let roles = testkit::roles::lock().await.expect("the roles lock");
    sqlx::raw_sql("CREATE SCHEMA ext; CREATE SCHEMA audit; SET search_path TO ext")
        .execute(&mut *conn)
        .await
        .expect("the schemas");
    EXT.run(&mut *conn).await.expect("the ext migrations");
    sqlx::query("SET search_path TO audit, ext")
        .execute(&mut *conn)
        .await
        .expect("the audit search path");
    AUDIT
        .run_to(SHARED, &mut *conn)
        .await
        .expect("the audit migrations of the shared chain");
    roles.release().await;

    // Shared positions: 1 outside, 2 and 3 entries, 4 outside, 5 an entry.
    let audit = AuditStore::new(pool.clone());
    let log = AccessLogStore::new(pool.clone());
    audit.insert(&outside(), None).await.expect("record 1");
    log.append(vec![entry(1), entry(2)])
        .await
        .expect("records 2 and 3");
    audit.insert(&outside(), None).await.expect("record 4");
    log.append(vec![entry(3)]).await.expect("record 5");
    sqlx::raw_sql(
        "INSERT INTO audit.fhir_feed_outbox (chain_seq) VALUES (3), (4); \
         INSERT INTO audit.access_log_checkpoint (sequence, system_id, digest, time_sealed, content) \
         SELECT chain_seq, system_id, digest, now(), '{}' FROM audit.access_log_record \
          WHERE chain_seq = 5",
    )
    .execute(&pool)
    .await
    .expect("an outbox and a checkpoint of the shared chain");
    let shared: Vec<String> = sqlx::query_scalar("SELECT finding FROM audit.verify_audit_chain()")
        .fetch_all(&pool)
        .await
        .expect("verify the shared chain");
    assert_eq!(shared, Vec::<String>::new(), "the shared chain verifies");

    sqlx::query("SET search_path TO audit, ext")
        .execute(&mut *conn)
        .await
        .expect("the audit search path");
    AUDIT.run(&mut *conn).await.expect("the split");
    drop(conn);

    assert_eq!(
        findings(&pool).await,
        Vec::<String>::new(),
        "both chains verify"
    );
    let system: Vec<i64> =
        sqlx::query_scalar("SELECT chain_seq FROM audit.audit_event ORDER BY chain_seq")
            .fetch_all(&pool)
            .await
            .expect("the system chain");
    assert_eq!(
        system,
        [1, 4],
        "the records outside the log keep their positions"
    );
    let links: Vec<(i64, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT chain_seq, previous_digest, content_digest, digest, signature \
         FROM audit.access_log_record ORDER BY chain_seq",
    )
    .fetch_all(&pool)
    .await
    .expect("the access log");
    let mut previous: String = sqlx::query_scalar("SELECT audit.access_log_genesis()")
        .fetch_one(&pool)
        .await
        .expect("the genesis");
    for ((sequence, before, content, digest, signature), expected) in links.iter().zip(1_i64..) {
        assert_eq!(*sequence, expected, "the access log is renumbered from 1");
        assert_eq!(*before, previous);
        assert_eq!(*digest, link_digest(before, *sequence, content));
        assert_eq!(
            *signature, None,
            "a signature of the shared chain is cleared"
        );
        previous.clone_from(digest);
    }
    assert_eq!(links.len(), 3);
    let span = log.chain(1, 3).await.expect("the span of every entry");
    assert_eq!(span.len(), 3, "the span holds the entries alone");

    let owed: Vec<(String, i64)> =
        sqlx::query_as("SELECT chain, chain_seq FROM audit.fhir_feed_outbox ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("the outbox");
    assert_eq!(
        owed,
        [("access_log".to_owned(), 2), ("system".to_owned(), 4)],
        "the outbox names each record by its chain and new position"
    );
    let checkpoints: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_checkpoint")
        .fetch_one(&pool)
        .await
        .expect("the checkpoints");
    assert_eq!(
        checkpoints, 0,
        "a checkpoint of the shared chain is removed"
    );

    audit
        .insert(&outside(), None)
        .await
        .expect("a record after");
    let after = log.append(vec![entry(4)]).await.expect("an entry after");
    assert_eq!(after.first().map(|entry| entry.link.sequence), Some(4));
    let newest: i64 = sqlx::query_scalar("SELECT max(chain_seq) FROM audit.audit_event")
        .fetch_one(&pool)
        .await
        .expect("the system head");
    assert_eq!(
        newest, 6,
        "the system chain goes on past the access log's last position"
    );
    assert_eq!(
        findings(&pool).await,
        Vec::<String>::new(),
        "both chains still verify"
    );
}
