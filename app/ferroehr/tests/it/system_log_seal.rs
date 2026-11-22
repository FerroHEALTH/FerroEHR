// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The system records' frontier and seal windows (#6242): a window never
//! covers a record a transaction still writes or a write still holds, every
//! record lands in a window once committed and released, a record inserted
//! inside a sealed window is reported, the sealers of several replicas leave
//! one run of windows, a keyring without a key signs no seal, a stored seal
//! never changes, and the fold removes the reaped positions of whole sealed
//! windows only. No openEHR spec governs audit storage: our own design.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jiff::Timestamp;
use sqlx::{AssertSqlSafe, Connection, PgPool};
use uuid::Uuid;

use ferroehr::access_log::frontier::Frontier;
use ferroehr::access_log::seal::SYSTEM;
use ferroehr::system_log::config::Retention;
use ferroehr::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::seal::{SealError, SealFinding};
use ferroehr::system_log::store::{AuditRecord, AuditStore};
use ferroehr::versioning::signature::signer::{AttestError, Signer};

use crate::access_log_checkpoint::{checkpoints, generate_key, keyring};

/// The system the seals name.
const SYSTEM_ID: &str = "cdr-system-seal.example.org";

/// A party read at `at`, a record outside the access log, of subject `n`;
/// every such record names the same search tokens.
fn event(n: u32, at: Timestamp) -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Demographic,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut event.user_id);
    event.client_ip = Some("10.0.0.9".to_owned());
    event.object_id = Some(format!("8fa1::ferroehr::{n}"));
    event.event_type = Some(EventType::rest("person_get"));
    event.timestamp = at;
    event
}

/// The record of [`event`] `n`, written now.
fn record(n: u32) -> AuditRecord {
    AuditRecord {
        event: event(n, Timestamp::now()),
        subject: None,
        entry: None,
    }
}

/// The position of the record of subject `n`.
async fn seq_of(pool: &PgPool, n: u32) -> i64 {
    sqlx::query_scalar("SELECT seq FROM audit.audit_event WHERE resource_id = $1")
        .bind(format!("8fa1::ferroehr::{n}"))
        .fetch_one(pool)
        .await
        .expect("the record")
}

/// The last position sealed.
async fn sealed_through(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT last_id FROM audit.audit_event_seal_state WHERE singleton")
        .fetch_one(pool)
        .await
        .expect("the seal state")
}

/// The system records' frontier, moved by rounds until it stops moving.
#[expect(
    clippy::disallowed_methods,
    reason = "a test moves the frontier on a raw pool connection, outside any request"
)]
async fn settled_frontier(pool: &PgPool, frontier: &mut Frontier) -> i64 {
    let mut conn = pool.acquire().await.expect("connection");
    let mut reached = frontier.round(&mut conn).await.expect("a round").system;
    for _ in 0..5 {
        let next = frontier.round(&mut conn).await.expect("a round").system;
        if next == reached {
            break;
        }
        reached = next;
    }
    reached
}

/// The system records' frontier, moved by rounds until it reaches `target`,
/// which another test's transaction anywhere in the cluster may delay.
#[expect(
    clippy::disallowed_methods,
    reason = "a test moves the frontier on a raw pool connection, outside any request"
)]
async fn frontier_reaching(pool: &PgPool, frontier: &mut Frontier, target: i64) -> i64 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut conn = pool.acquire().await.expect("connection");
    loop {
        let reached = frontier.round(&mut conn).await.expect("a round").system;
        if reached >= target || std::time::Instant::now() > deadline {
            return reached;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The findings of a full verification of both logs.
async fn findings(pool: &PgPool) -> Vec<String> {
    crate::fixtures::verify_chain(pool)
        .await
        .expect("the verification runs")
        .into_iter()
        .map(|finding| finding.finding)
        .collect()
}

/// A record written in a transaction held open stays out of every window
/// until it commits, and a committed record a write holds stays out until
/// its release; then it lands in a later window, never lost and never
/// sealed early.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::disallowed_methods,
    reason = "a test holds a write open on a raw pool connection, outside any request"
)]
async fn a_window_never_covers_a_record_a_transaction_still_writes_or_holds() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let log = checkpoints(&pool, SYSTEM_ID, Arc::new(Signer::digest_default()));
    let store = AuditStore::new(pool.clone());
    // NOTE: our own design (#6242), the tokens are stored first, as a running writer knows them.
    store
        .insert(&event(0, Timestamp::now()), None)
        .await
        .expect("the first record");

    let reservation = Uuid::now_v7();
    let mut conn = pool.acquire().await.expect("connection");
    let mut open = conn.begin().await.expect("begin");
    store
        .write_held_on(&mut open, vec![record(1)], &[], reservation)
        .await
        .expect("the open write");
    store
        .write(vec![record(2)], &[])
        .await
        .expect("a later record commits");
    let later = seq_of(&pool, 2).await;

    let mut frontier = Frontier::default();
    let reached = settled_frontier(&pool, &mut frontier).await;
    assert!(
        reached < later,
        "the frontier stops below the open write's position while it runs: {reached} against {later}"
    );
    log.store().seal_to(&SYSTEM, reached).await.expect("a seal");
    assert!(sealed_through(&pool).await < later);

    open.commit().await.expect("the open write commits");
    let held = seq_of(&pool, 1).await;
    let reached = frontier_reaching(&pool, &mut frontier, later).await;
    assert!(reached >= later, "the frontier passes both once both ended");
    log.store().seal_to(&SYSTEM, reached).await.expect("a seal");
    assert!(
        sealed_through(&pool).await < held,
        "a held record is in no window until its release"
    );

    let mut release = pool.acquire().await.expect("connection");
    assert_eq!(
        AuditStore::release(&mut release, &[reservation])
            .await
            .expect("the release"),
        1
    );
    while matches!(
        log.store().seal_to(&SYSTEM, reached).await.expect("a seal"),
        ferroehr::access_log::seal::SealRound::Sealed { .. }
    ) {}
    let window: i64 = sqlx::query_scalar(
        "SELECT window_no FROM audit.audit_event_seal WHERE $1 BETWEEN first_id AND last_id",
    )
    .bind(held)
    .fetch_one(&pool)
    .await
    .expect("the held record is sealed once released");
    assert!(window >= 1);
    assert_eq!(
        findings(&pool).await,
        Vec::<String>::new(),
        "every window verifies"
    );
}

/// The identity sequence of the system records keeps one value per draw,
/// which the frontier's proof rests on.
#[tokio::test]
async fn the_system_position_sequence_draws_one_value_at_a_time() {
    let testdb = testkit::db().await.expect("testkit database");
    let cache: i64 = sqlx::query_scalar(
        "SELECT cache_size FROM pg_sequences \
         WHERE schemaname = 'audit' AND sequencename = 'audit_event_seq_seq'",
    )
    .fetch_one(&testdb.pool())
    .await
    .expect("the sequence");
    assert_eq!(cache, 1);
}

/// Run `statement` with the record table's triggers off, as only its owner
/// can: the change the seals detect.
#[expect(
    clippy::disallowed_methods,
    reason = "a test changes the records on a raw pool connection, outside any request"
)]
async fn tamper(pool: &PgPool, statement: &str) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    for statement in [
        "ALTER TABLE audit.audit_event DISABLE TRIGGER USER",
        statement,
        "ALTER TABLE audit.audit_event ENABLE TRIGGER USER",
    ] {
        sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&mut *tx)
            .await
            .expect("the change");
    }
    tx.commit().await.expect("commit");
}

/// Three records sealed in one window, on a fresh store.
async fn sealed_three(pool: &PgPool) -> AuditStore {
    let store = AuditStore::new(pool.clone());
    store
        .insert_batch(&[record(1), record(2), record(3)], &[])
        .await
        .expect("the records");
    assert_eq!(
        findings(pool).await,
        Vec::<String>::new(),
        "the window verifies before the change"
    );
    store
}

/// A record whose content digest is changed inside a sealed window is
/// reported, and so is one whose column changed under its digest.
#[tokio::test]
async fn a_changed_record_in_a_sealed_window_is_reported() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    sealed_three(&pool).await;
    let seq = seq_of(&pool, 2).await;
    tamper(
        &pool,
        &format!("UPDATE audit.audit_event SET content_digest = sha256('x') WHERE seq = {seq}"),
    )
    .await;
    let found = findings(&pool).await;
    assert!(
        found.iter().any(|f| f.contains("does not match its seal")),
        "{found:?}"
    );
    assert!(found.iter().any(|f| f.contains("modified")), "{found:?}");
}

/// A record removed from a sealed window is reported.
#[tokio::test]
async fn a_removed_record_in_a_sealed_window_is_reported() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    sealed_three(&pool).await;
    let seq = seq_of(&pool, 2).await;
    tamper(
        &pool,
        &format!("DELETE FROM audit.audit_event WHERE seq = {seq}"),
    )
    .await;
    let found = findings(&pool).await;
    assert!(
        found.iter().any(|f| f.contains("does not match its seal")),
        "{found:?}"
    );
}

/// A record inserted into a sealed window, at a position a rolled-back write
/// left unused, is reported.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test rolls a write back on a raw pool connection, outside any request"
)]
async fn an_inserted_record_in_a_sealed_window_is_reported() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    store
        .write(vec![record(1)], &[])
        .await
        .expect("the first record");
    {
        let mut conn = pool.acquire().await.expect("connection");
        let mut rolled = conn.begin().await.expect("begin");
        store
            .write_held_on(&mut rolled, vec![record(9)], &[], Uuid::now_v7())
            .await
            .expect("a write that rolls back");
        rolled.rollback().await.expect("rollback");
    }
    store
        .insert(&event(3, Timestamp::now()), None)
        .await
        .expect("the last record");
    assert_eq!(findings(&pool).await, Vec::<String>::new());
    let (first, last) = (seq_of(&pool, 1).await, seq_of(&pool, 3).await);
    assert!(
        last > first.saturating_add(1),
        "the rolled-back write used a position between"
    );
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT quote_ident(column_name) FROM information_schema.columns \
         WHERE table_schema = 'audit' AND table_name = 'audit_event' \
           AND column_name NOT IN ('seq', 'id') ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .expect("the columns");
    let columns = columns.join(", ");
    tamper(
        &pool,
        &format!(
            "INSERT INTO audit.audit_event (seq, id, {columns}) OVERRIDING SYSTEM VALUE \
             SELECT {first} + 1, gen_random_uuid(), {columns} \
             FROM audit.audit_event WHERE seq = {first}"
        ),
    )
    .await;
    let found = findings(&pool).await;
    assert!(
        found.iter().any(|f| f.contains("does not match its seal")),
        "{found:?}"
    );
}

/// The sealers of several replicas sealing while writers write leave one run
/// of windows: no position between two windows and none twice, every record
/// sealed once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sealers_on_several_replicas_keep_one_run_of_system_windows() {
    let db = testkit::db().await.expect("testkit database");
    sealers_keep_one_run(&db.pool()).await;
}

/// The run of [`sealers_on_several_replicas_keep_one_run_of_system_windows`]
/// forty times over, each on a fresh store, so a frontier that lets a window
/// pass a write still in flight is caught (#6242).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sealers_keep_one_run_of_system_windows_forty_times_over() {
    for _ in 0..40 {
        let db = testkit::db().await.expect("testkit database");
        sealers_keep_one_run(&db.pool()).await;
    }
}

/// Four writers and three sealers on `pool` leave one run of windows that
/// seals every system record once and verifies.
#[expect(
    clippy::disallowed_methods,
    reason = "a test moves the frontier on raw pool connections, outside any request"
)]
async fn sealers_keep_one_run(pool: &PgPool) {
    let pool = pool.clone();
    let log = Arc::new(checkpoints(&pool, SYSTEM_ID, keyring(&generate_key())));
    let store = AuditStore::new(pool.clone());
    store
        .insert(&event(0, Timestamp::now()), None)
        .await
        .expect("the tokens are stored");
    let writing = Arc::new(AtomicBool::new(true));
    let mut sealers = tokio::task::JoinSet::new();
    for _ in 0..3 {
        let log = Arc::clone(&log);
        let writing = Arc::clone(&writing);
        let pool = pool.clone();
        sealers.spawn(async move {
            let mut frontier = Frontier::default();
            while writing.load(Ordering::Acquire) {
                let reached = {
                    let mut conn = pool.acquire().await.expect("a connection");
                    frontier
                        .round(&mut conn)
                        .await
                        .expect("a frontier round")
                        .system
                };
                log.store()
                    .seal_to(&SYSTEM, reached)
                    .await
                    .expect("a sealing round");
                tokio::task::yield_now().await;
            }
        });
    }
    let mut writers = tokio::task::JoinSet::new();
    for writer in 0..4_u32 {
        let store = store.clone();
        writers.spawn(async move {
            for i in 0..25 {
                store
                    .write(vec![record(writer * 1000 + i + 1)], &[])
                    .await
                    .expect("a write");
            }
        });
    }
    while let Some(written) = writers.join_next().await {
        written.expect("a writer");
    }
    writing.store(false, Ordering::Release);
    while let Some(round) = sealers.join_next().await {
        round.expect("a sealer");
    }
    log.store()
        .seal_committed(&SYSTEM)
        .await
        .expect("every committed record is sealed");
    let windows: Vec<(i64, i64, i32)> = sqlx::query_as(
        "SELECT first_id, last_id, entries FROM audit.audit_event_seal ORDER BY window_no",
    )
    .fetch_all(&pool)
    .await
    .expect("the windows");
    let mut next = 1;
    for (first, last, _) in &windows {
        assert_eq!(*first, next, "the windows follow one another: {windows:?}");
        next = last + 1;
    }
    let sealed: i64 = windows.iter().map(|(_, _, n)| i64::from(*n)).sum();
    assert_eq!(sealed, 101, "every record sealed once");
    assert_eq!(
        findings(&pool).await,
        Vec::<String>::new(),
        "every window verifies"
    );
}

/// A keyring with no `OpenPGP` key seals windows unsigned and gives no
/// signed seal; asking for one while a record waits is refused.
#[tokio::test]
async fn a_keyring_without_a_key_signs_no_system_seal() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let log = checkpoints(&pool, SYSTEM_ID, Arc::new(Signer::digest_default()));
    AuditStore::new(pool.clone())
        .write(vec![record(1)], &[])
        .await
        .expect("a record");
    let refused = log
        .system_seals()
        .seal()
        .await
        .expect_err("no key, no seal");
    assert!(
        matches!(refused, SealError::Sign(AttestError::NoKey)),
        "{refused:?}"
    );
    log.store()
        .seal_committed(&SYSTEM)
        .await
        .expect("sealed unsigned");
    let (unsigned, signed): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE proof IS NULL), count(*) FILTER (WHERE proof IS NOT NULL) \
         FROM audit.audit_event_seal",
    )
    .fetch_one(&pool)
    .await
    .expect("the seals");
    assert!(unsigned >= 1);
    assert_eq!(signed, 0);
    assert!(
        log.system_seals()
            .stored(None)
            .await
            .expect("read")
            .is_none()
    );
}

/// A signed seal verifies the records, is never changed or removed, and a
/// seal whose digest was replaced or that another key signed does not verify.
#[tokio::test]
async fn a_stored_system_seal_is_never_changed_and_a_forged_one_fails() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let log = checkpoints(&pool, SYSTEM_ID, keyring(&generate_key()));
    AuditStore::new(pool.clone())
        .write(vec![record(1), record(2)], &[])
        .await
        .expect("the records");
    let seals = log.system_seals();
    let seal = seals.seal().await.expect("seal").expect("a signed seal");
    assert_eq!(seal.sequence, seq_of(&pool, 2).await);
    let verified = seals.verify(Some(seal.clone())).await.expect("verify");
    assert!(verified.intact(), "{verified:?}");
    for statement in [
        "UPDATE audit.audit_event_seal SET proof = 'x'",
        "UPDATE audit.audit_event_seal SET digest = sha256('x')",
        "DELETE FROM audit.audit_event_seal",
        "TRUNCATE audit.audit_event_seal",
    ] {
        let refused = sqlx::query(statement)
            .execute(&pool)
            .await
            .expect_err(statement);
        assert!(refused.to_string().contains("append-only"), "{refused}");
    }
    let mut replaced = seal.clone();
    replaced.digest = "0".repeat(64);
    let verified = seals.verify(Some(replaced)).await.expect("verify");
    assert!(
        verified
            .findings
            .iter()
            .any(|f| matches!(f, SealFinding::Proof { .. }))
            && verified
                .findings
                .iter()
                .any(|f| matches!(f, SealFinding::Digest { .. })),
        "{:?}",
        verified.findings
    );
    let foreign = checkpoints(&pool, SYSTEM_ID, keyring(&generate_key())).system_seals();
    let verified = foreign.verify(Some(seal)).await.expect("verify");
    assert!(
        verified
            .findings
            .iter()
            .any(|f| matches!(f, SealFinding::Proof { .. })),
        "a seal another key signed does not verify: {:?}",
        verified.findings
    );
}

/// Retention removes each record past its end and keeps its position and
/// content digest; once every record of a sealed window is reaped, the fold
/// removes the window's reaped positions and records it, and only then.
#[tokio::test]
async fn a_window_folds_only_once_every_record_of_it_is_reaped() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    let now = Timestamp::now();
    let old = now - jiff::SignedDuration::from_hours(60 * 24);
    let records = |ats: &[Timestamp]| {
        ats.iter()
            .enumerate()
            .map(|(n, at)| AuditRecord {
                event: event(u32::try_from(n).expect("few"), *at),
                subject: None,
                entry: None,
            })
            .collect::<Vec<_>>()
    };
    store
        .insert_batch(&records(&[old, old, now]), &[])
        .await
        .expect("one window of an old, an old and a new record");
    let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
    assert_eq!(
        store
            .reap_due(today, Retention::Days(30))
            .await
            .expect("reap"),
        2
    );
    let (reaped, folded): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM audit.audit_event_reaped), \
                (SELECT count(*) FROM audit.audit_event_seal_fold)",
    )
    .fetch_one(&pool)
    .await
    .expect("counts");
    assert_eq!(
        (reaped, folded),
        (2, 0),
        "the window keeps a record: no fold"
    );
    assert_eq!(findings(&pool).await, Vec::<String>::new());

    store
        .insert_batch(&records(&[old, old]), &[])
        .await
        .expect("a window of old records");
    assert_eq!(
        store
            .reap_due(today, Retention::Days(30))
            .await
            .expect("reap"),
        2
    );
    let (reaped, folded): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM audit.audit_event_reaped), \
                (SELECT count(*) FROM audit.audit_event_seal_fold)",
    )
    .fetch_one(&pool)
    .await
    .expect("counts");
    assert_eq!(
        (reaped, folded),
        (2, 1),
        "the second window folds whole and the first keeps its reaped positions"
    );
    assert_eq!(findings(&pool).await, Vec::<String>::new());
    let refused = sqlx::query("DELETE FROM audit.audit_event_reaped")
        .execute(&pool)
        .await
        .expect_err("a reaped position leaves only by the fold");
    assert!(refused.to_string().contains("append-only"), "{refused}");
}

/// Adds, as the owner can, a statement trigger that refuses a statement
/// whose transaction already holds an xid, and a row trigger that refuses a
/// record whose transaction holds none once it drew its position: a record
/// that passes both proves its own statement took the xid before the
/// position was drawn.
#[expect(
    clippy::disallowed_methods,
    reason = "a test adds triggers on a raw pool connection, as only the owner can"
)]
async fn refuse_a_position_drawn_before_the_xid(pool: &PgPool) {
    let mut conn = pool.acquire().await.expect("connection");
    for statement in [
        "CREATE FUNCTION audit.test_no_xid_yet() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF pg_catalog.pg_current_xact_id_if_assigned() IS NOT NULL THEN \
             RAISE EXCEPTION 'the transaction held an xid before the inserting statement'; \
           END IF; \
           RETURN NULL; \
         END $$",
        "CREATE TRIGGER test_no_xid_yet BEFORE INSERT ON audit.audit_event \
         FOR EACH STATEMENT EXECUTE FUNCTION audit.test_no_xid_yet()",
        "CREATE FUNCTION audit.test_xid_before_seq() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF pg_catalog.pg_current_xact_id_if_assigned() IS NULL THEN \
             RAISE EXCEPTION 'the record % drew its position before its transaction held an xid', NEW.seq; \
           END IF; \
           RETURN NEW; \
         END $$",
        "CREATE TRIGGER test_xid_before_seq BEFORE INSERT ON audit.audit_event \
         FOR EACH ROW EXECUTE FUNCTION audit.test_xid_before_seq()",
    ] {
        sqlx::query(statement)
            .execute(&mut *conn)
            .await
            .expect("the check is added");
    }
}

/// The statement that writes system records takes its transaction's xid
/// before the first row draws its position, held or not (#6242).
#[tokio::test]
async fn every_system_insert_takes_its_xid_before_its_first_position() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    store
        .write(vec![record(1)], &[])
        .await
        .expect("the tokens and the policy are stored");
    refuse_a_position_drawn_before_the_xid(&pool).await;
    store
        .write(vec![record(2), record(3)], &[])
        .await
        .expect("the write takes its xid first");
    store
        .write_held(vec![record(4)], &[], Uuid::now_v7())
        .await
        .expect("the held write takes its xid first");
}

/// A frontier round never passes a system record whose transaction holds the
/// newest xid, which no other transaction completed after: a snapshot's
/// `xmax` stays at that xid, so a frontier marked by it would pass the record
/// while its write runs (#6242).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::disallowed_methods,
    reason = "a test holds a write open on a raw pool connection, outside any request"
)]
async fn a_round_never_passes_a_system_write_holding_the_newest_xid() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    // NOTE: our own design (#6242), the tokens are stored first, as a running writer knows them.
    store
        .write(vec![record(0)], &[])
        .await
        .expect("the tokens are stored");

    let mut conn = pool.acquire().await.expect("connection");
    let mut open = conn.begin().await.expect("begin");
    store
        .write_held_on(&mut open, vec![record(1)], &[], Uuid::now_v7())
        .await
        .expect("the open write");
    let drawn: i64 = sqlx::query_scalar(
        "SELECT pg_sequence_last_value(pg_get_serial_sequence('audit.audit_event', 'seq')::regclass)",
    )
    .fetch_one(&pool)
    .await
    .expect("the position the open write drew");

    let mut frontier = Frontier::default();
    let reached = settled_frontier(&pool, &mut frontier).await;
    assert!(
        reached < drawn,
        "the frontier stops below the open write's position: {reached} against {drawn}"
    );
    open.rollback().await.expect("the open write ends");
}
