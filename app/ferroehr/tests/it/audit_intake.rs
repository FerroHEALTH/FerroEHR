// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit intake and the sealer (#5747, #5745, #6242): writers put
//! entries into the append-only log without waiting on any head, sealers on
//! several replicas seal them into one run of windows, and the windows they
//! leave verify, every derived column equal to its derivation and every
//! window signed.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ferroehr::access_log::frontier::Frontier;
use ferroehr::access_log::intake::{
    GroupWriter, IndexRow, IntakeEntry, IntakeToken, Named, take_in,
};
use sqlx::{Connection, PgPool};

use crate::access_log_checkpoint::{checkpoints, entry, generate_key, keyring};

/// The system the entries of these tests name.
const SYSTEM: &str = "cdr-intake.example.org";

/// The EHR the entries of these tests name.
const EHR: &str = "7d44b88c-4199-4bad-97dc-0000000000e1";

/// Writes the entries `numbers` into the intake, one transaction each.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
pub(crate) async fn write(pool: &PgPool, numbers: impl Iterator<Item = u32>) {
    for n in numbers {
        let mut conn = pool.acquire().await.expect("a connection");
        let mut tx = conn.begin().await.expect("begin");
        let intake = IntakeEntry::of(&mut entry(n, SYSTEM, EHR), None).expect("a stored form");
        take_in(
            &mut tx,
            &[intake],
            &[],
            None,
            &BTreeSet::new(),
            &Named::default(),
        )
        .await
        .expect("the intake write");
        tx.commit().await.expect("commit");
    }
}

/// Four writers and three sealers, each sealer as another replica would run
/// it, leave one run of windows: every entry in exactly one window, the
/// windows following one another from the first id with no id between, every
/// window and derived column checked by the verifier, and each window's seal
/// a checkpoint that verifies under the key.
#[tokio::test]
async fn sealers_on_several_replicas_keep_one_run_of_windows() {
    let db = testkit::db().await.expect("testkit database");
    sealers_keep_one_run(&db.pool()).await;
}

/// The run of [`sealers_on_several_replicas_keep_one_run_of_windows`] forty
/// times over, each on a fresh store, so a frontier that lets a window pass
/// a write still in flight is caught: such a frontier failed about one run in
/// eleven (#6242).
#[tokio::test]
async fn sealers_keep_one_run_of_windows_forty_times_over() {
    // NOTE: (#6242), a snapshot's xmax is one past the newest ended xid, so a write still
    // running can hold an xid at or above it; the frontier marks by an xid it takes itself.
    for _ in 0..40 {
        let db = testkit::db().await.expect("testkit database");
        sealers_keep_one_run(&db.pool()).await;
    }
}

/// Four writers and three sealers on `pool` leave one run of windows that
/// seals every entry once and verifies.
#[expect(
    clippy::disallowed_methods,
    reason = "a test moves the frontier on a raw pool connection, outside any request"
)]
async fn sealers_keep_one_run(pool: &PgPool) {
    let pool = pool.clone();
    let log = Arc::new(checkpoints(&pool, SYSTEM, keyring(&generate_key())));
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
                    frontier.round(&mut conn).await.expect("a frontier round")
                };
                log.store().seal_to(reached).await.expect("a sealing round");
                tokio::task::yield_now().await;
            }
        });
    }
    let mut writers = tokio::task::JoinSet::new();
    for writer in 0..4_u32 {
        let pool = pool.clone();
        writers.spawn(async move { write(&pool, (0..50).map(|i| writer * 1000 + i)).await });
    }
    while let Some(written) = writers.join_next().await {
        written.expect("a writer");
    }
    writing.store(false, Ordering::Release);
    while let Some(round) = sealers.join_next().await {
        round.expect("a sealer");
    }
    log.store()
        .seal_committed()
        .await
        .expect("every committed entry is sealed");

    let windows: Vec<(i64, i64, i32)> = sqlx::query_as(
        "SELECT first_id, last_id, entries FROM audit.access_log_seal ORDER BY window_no",
    )
    .fetch_all(&pool)
    .await
    .expect("the windows");
    let mut next = 1;
    for (first, last, _) in &windows {
        assert_eq!(*first, next, "the windows follow one another: {windows:?}");
        next = last + 1;
    }
    let sealed: i64 = windows
        .iter()
        .map(|(_, _, entries)| i64::from(*entries))
        .sum();
    assert_eq!(sealed, 200, "every entry sealed once");
    let uids: BTreeSet<String> =
        sqlx::query_scalar("SELECT uid::text FROM audit.access_log_record")
            .fetch_all(&pool)
            .await
            .expect("the uids")
            .into_iter()
            .collect();
    assert_eq!(uids.len(), 200, "every entry written once");
    let findings = log
        .store()
        .verify_windows(false)
        .await
        .expect("the verifier");
    assert!(findings.is_empty(), "the windows verify: {findings:?}");
    let verification = log.verify(None).await.expect("verify the checkpoints");
    assert!(verification.intact(), "{verification:?}");
    let signed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit.access_log_seal WHERE proof IS NOT NULL")
            .fetch_one(&pool)
            .await
            .expect("the seals");
    assert_eq!(
        usize::try_from(signed).expect("a count"),
        windows.len(),
        "every window is signed"
    );
}

/// The log is append-only: the runtime refuses an update, a delete and a
/// truncate of an entry, and of a system record waiting in its intake.
#[tokio::test]
async fn the_log_refuses_every_change_but_the_tombstone() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    write(&pool, std::iter::once(1)).await;
    let outside = ferroehr::system_log::store::AuditRecord {
        event: ferroehr::system_log::event::AuditEvent::new(
            ferroehr::system_log::event::EventActionCode::Execute,
            ferroehr::system_log::event::ObjectClass::Authentication,
            ferroehr::system_log::event::EventOutcome::Success,
        ),
        subject: None,
        entry: None,
    };
    ferroehr::system_log::store::AuditStore::new(pool.clone())
        .write(vec![outside], &[])
        .await
        .expect("a record of the system chain waits in its intake");
    for statement in [
        "UPDATE audit.access_log_entry SET content = '{}'",
        "DELETE FROM audit.access_log_entry",
        "TRUNCATE audit.access_log_entry",
        "UPDATE audit.audit_event_intake SET events = events",
        "DELETE FROM audit.audit_event_intake",
        "TRUNCATE audit.audit_event_intake",
    ] {
        let refused = sqlx::query(statement)
            .execute(&pool)
            .await
            .expect_err(statement);
        assert!(
            refused.to_string().contains("append-only"),
            "{statement}: {refused}"
        );
    }
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the log");
    assert_eq!(left, 1, "the entry is kept");
}

/// A seal that does not follow the seal state is refused by nothing but
/// found by the verifier: a seal row written past the sealer reads as a
/// removed or changed seal.
#[tokio::test]
async fn a_seal_written_past_the_sealer_is_found() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let log = checkpoints(&pool, SYSTEM, keyring(&generate_key()));
    write(&pool, 0..3).await;
    log.store().seal_committed().await.expect("sealed");
    sqlx::query(
        "INSERT INTO audit.access_log_seal (window_no, first_id, last_id, entries, \
         previous_digest, digest, sealed_at) \
         SELECT window_no + 1, last_id + 1, last_id + 5, 0, digest, digest, now() \
         FROM audit.access_log_seal ORDER BY window_no DESC LIMIT 1",
    )
    .execute(&pool)
    .await
    .expect("a seal past the sealer");
    let findings = log
        .store()
        .verify_windows(false)
        .await
        .expect("the verifier");
    assert!(!findings.is_empty(), "the stray seal is found");
}

/// Autovacuum never truncates the system intake (#5970): the truncate's
/// `ACCESS EXCLUSIVE` lock would hold every request's intake write.
#[tokio::test]
async fn autovacuum_never_truncates_an_intake() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let options: Vec<(String, Option<Vec<String>>)> = sqlx::query_as(
        "SELECT c.relname::text, c.reloptions::text[] FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'audit' AND c.relname IN ('audit_event_intake') \
         ORDER BY c.relname",
    )
    .fetch_all(&pool)
    .await
    .expect("the intakes' storage parameters");
    assert_eq!(options.len(), 1, "the system intake: {options:?}");
    for (table, reloptions) in options {
        assert!(
            reloptions
                .unwrap_or_default()
                .contains(&"vacuum_truncate=off".to_owned()),
            "{table} sets vacuum_truncate = off"
        );
    }
}

/// An entry `n` with an index row under the render policy `policy`, and its
/// one search token.
fn indexed(n: u32, policy: &[u8]) -> (IntakeEntry, Vec<IntakeToken>) {
    let index = IndexRow {
        action: "R",
        outcome: 0,
        principal: None,
        patient_id: None,
        resource_id: None,
        request_id: None,
        client_ip: None,
        ehr_id: None,
        search_tokens: String::new(),
        render_policy: policy.to_vec(),
    };
    let intake = IntakeEntry::of(&mut entry(n, SYSTEM, EHR), Some(index)).expect("a stored form");
    let token = IntakeToken {
        parameter: "purpose",
        system: Some("urn:example:generation".to_owned()),
        code: Some("TREAT".to_owned()),
        shares: false,
    };
    (intake, vec![token])
}

/// The search token ids the entry `n` names.
async fn named_tokens(pool: &PgPool, n: u32) -> Vec<i32> {
    sqlx::query_scalar(
        "SELECT search_tokens FROM audit.access_log_entry \
         WHERE uid = $1::uuid",
    )
    .bind(entry(n, SYSTEM, EHR).uid.value().to_owned())
    .fetch_one(pool)
    .await
    .expect("the entry")
}

/// The gathered writer names a known token by id with no lookup, and when
/// retention has moved the store's generation the plain write is refused
/// (`FE001`), the writer forgets every id, and the entry is written once with
/// the token the store holds (#6242).
#[tokio::test]
async fn a_write_naming_ids_of_an_old_generation_is_written_again() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let policy: Vec<u8> = sqlx::query_scalar("SELECT audit.register_render_policy('{}'::jsonb)")
        .fetch_one(&pool)
        .await
        .expect("a render policy");
    let writer = GroupWriter::spawn(&pool);
    for n in [1, 2] {
        let (intake, tokens) = indexed(n, &policy);
        writer
            .write(vec![intake], vec![tokens])
            .await
            .expect("the entry is written");
    }
    let first = named_tokens(&pool, 1).await;
    assert_eq!(first.len(), 1, "the entry names its one token");
    assert_eq!(
        named_tokens(&pool, 2).await,
        first,
        "the second names it by id"
    );

    let refused = sqlx::query("SELECT audit.access_log_token_generation_holds(-1)")
        .execute(&pool)
        .await
        .expect_err("an old generation is refused");
    assert_eq!(
        refused
            .as_database_error()
            .and_then(|error| error.code().map(std::borrow::Cow::into_owned)),
        Some("FE001".to_owned())
    );

    owner_moves_the_generation(&pool).await;
    let (intake, tokens) = indexed(3, &policy);
    writer
        .write(vec![intake], vec![tokens])
        .await
        .expect("the entry is written after the generation moved");
    assert_eq!(
        named_tokens(&pool, 3).await,
        first,
        "the token the store holds"
    );
    let entries: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the count");
    assert_eq!(entries, 3, "each entry once");
}

/// Moves the store's token generation, as retention does when it deletes a
/// token.
#[expect(
    clippy::disallowed_methods,
    reason = "a test moves the generation on a raw pool connection, as only the owner can"
)]
async fn owner_moves_the_generation(pool: &PgPool) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    for statement in [
        "ALTER TABLE audit.audit_search_token_generation DISABLE TRIGGER USER",
        "UPDATE audit.audit_search_token_generation SET generation = generation + 1",
        "ALTER TABLE audit.audit_search_token_generation ENABLE TRIGGER USER",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect("the generation moves");
    }
    tx.commit().await.expect("commit");
}
