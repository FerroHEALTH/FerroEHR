// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log's frontier and seal windows (#6242): a window never covers
//! an entry a transaction still writes, every entry lands in a window once
//! committed, a changed, removed or inserted record inside a sealed window is
//! reported, and the fold removes whole sealed windows only. No openEHR spec
//! governs audit storage: our own design.

use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use serde_json::json;
use sqlx::{AssertSqlSafe, Connection, PgPool};

use ferroehr::access_log::frontier::Frontier;
use ferroehr::access_log::seal::SealRound;
use ferroehr::access_log::store::AccessLogStore;

/// The example entry with the uid and EHR of number `n`.
fn entry(n: u32) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["ehr_id"]["value"] = json!(format!("7d44b88c-4199-4bad-97dc-{n:012x}"));
    serde_json::from_value(value).expect("the example entry reads")
}

/// The id of the entry whose uid is that of number `n`.
async fn id_of(pool: &PgPool, n: u32) -> i64 {
    sqlx::query_scalar("SELECT id FROM audit.access_log_entry WHERE uid = $1::uuid")
        .bind(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"))
        .fetch_one(pool)
        .await
        .expect("the entry")
}

/// The frontier, moved by rounds until it stops moving.
#[expect(
    clippy::disallowed_methods,
    reason = "a test moves the frontier on a raw pool connection, outside any request"
)]
async fn settled_frontier(pool: &PgPool, frontier: &mut Frontier) -> i64 {
    let mut conn = pool.acquire().await.expect("connection");
    let mut reached = frontier.round(&mut conn).await.expect("a round");
    for _ in 0..5 {
        let next = frontier.round(&mut conn).await.expect("a round");
        if next == reached {
            break;
        }
        reached = next;
    }
    reached
}

/// The frontier, moved by rounds until it reaches `target`, which another
/// test's transaction anywhere in the cluster may delay for a while.
#[expect(
    clippy::disallowed_methods,
    reason = "a test moves the frontier on a raw pool connection, outside any request"
)]
async fn frontier_reaching(pool: &PgPool, frontier: &mut Frontier, target: i64) -> i64 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut conn = pool.acquire().await.expect("connection");
    loop {
        let reached = frontier.round(&mut conn).await.expect("a round");
        if reached >= target || std::time::Instant::now() > deadline {
            return reached;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The findings of a full verification of the windows.
async fn findings(store: &AccessLogStore) -> Vec<String> {
    store
        .verify_windows(false)
        .await
        .expect("the verification runs")
        .into_iter()
        .map(|finding| finding.finding)
        .collect()
}

/// Every entry the plain insert of a write in flight holds stays out of
/// every window until its transaction ends, and lands in a later window once
/// it commits: never lost and never sealed early.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_window_never_covers_an_entry_a_transaction_still_writes() {
    open_write_stays_unsealed(Vec::new()).await;
}

/// The same holds for an entry the intake statement writes, which resolves
/// its search tokens (#6242).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_window_never_covers_an_entry_the_intake_statement_still_writes() {
    open_write_stays_unsealed(vec![ferroehr::access_log::intake::IntakeToken {
        parameter: "purpose",
        system: Some("urn:example:frontier".to_owned()),
        code: Some("TREAT".to_owned()),
        shares: false,
    }])
    .await;
}

/// An entry written with `tokens` in a transaction held open stays out of
/// every window until it commits, and then lands in one.
#[expect(
    clippy::disallowed_methods,
    reason = "a test holds a write open on a raw pool connection, outside any request"
)]
async fn open_write_stays_unsealed(tokens: Vec<ferroehr::access_log::intake::IntakeToken>) {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    // NOTE: our own design (#6242), the part is stored first, as a running writer knows it.
    store
        .append(vec![entry(9)])
        .await
        .expect("the shared part is stored");
    let part: String =
        sqlx::query_scalar("SELECT encode(digest, 'hex') FROM audit.access_log_shared")
            .fetch_one(&pool)
            .await
            .expect("the one shared part");

    let mut conn = pool.acquire().await.expect("connection");
    let mut open = conn.begin().await.expect("begin");
    let mut early = entry(1);
    let intake = ferroehr::access_log::intake::IntakeEntry::of(&mut early, None).expect("intake");
    ferroehr::access_log::intake::take_in(
        &mut open,
        &[intake],
        &[tokens],
        None,
        &std::collections::BTreeSet::from([part.as_str()]),
        &ferroehr::access_log::intake::Named::default(),
    )
    .await
    .expect("the open write");
    store
        .append(vec![entry(2)])
        .await
        .expect("a later entry commits");
    let later = id_of(&pool, 2).await;

    let mut frontier = Frontier::default();
    let reached = settled_frontier(&pool, &mut frontier).await;
    assert!(
        reached < later,
        "the frontier stops below the open write's id while it runs: {reached} against {later}"
    );
    let first = store.seal_to(reached).await.expect("a seal");
    let sealed_through: i64 =
        sqlx::query_scalar("SELECT last_id FROM audit.access_log_seal_state WHERE singleton")
            .fetch_one(&pool)
            .await
            .expect("the seal state");
    assert!(sealed_through < later, "{first:?} sealed {sealed_through}");

    open.commit().await.expect("the open write commits");
    let open_id = id_of(&pool, 1).await;
    let reached = frontier_reaching(&pool, &mut frontier, later).await;
    assert!(reached >= later, "the frontier passes both once both ended");
    assert!(
        matches!(
            store.seal_to(reached).await.expect("a seal"),
            SealRound::Sealed { .. }
        ),
        "the entries below the frontier are sealed"
    );
    let window: i64 = sqlx::query_scalar(
        "SELECT window_no FROM audit.access_log_seal WHERE $1 BETWEEN first_id AND last_id",
    )
    .bind(open_id)
    .fetch_one(&pool)
    .await
    .expect("the open write's entry is sealed");
    assert!(window >= 1, "the open write's entry lies in a window");
    assert_eq!(
        findings(&store).await,
        Vec::<String>::new(),
        "every window verifies"
    );
}

/// The identity sequence of the entries keeps one value per draw, which the
/// frontier's proof rests on.
#[tokio::test]
async fn the_entry_id_sequence_draws_one_value_at_a_time() {
    let testdb = testkit::db().await.expect("testkit database");
    let cache: i64 = sqlx::query_scalar(
        "SELECT cache_size FROM pg_sequences \
         WHERE schemaname = 'audit' AND sequencename = 'access_log_entry_id_seq'",
    )
    .fetch_one(&testdb.pool())
    .await
    .expect("the sequence");
    assert_eq!(cache, 1);
}

/// Run `statement` with the entry table's triggers off, as only its owner
/// can: the change the seals detect.
#[expect(
    clippy::disallowed_methods,
    reason = "a test changes the log on a raw pool connection, outside any request"
)]
async fn tamper(pool: &PgPool, statement: &str) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    for statement in [
        "ALTER TABLE audit.access_log_entry DISABLE TRIGGER USER",
        statement,
        "ALTER TABLE audit.access_log_entry ENABLE TRIGGER USER",
    ] {
        sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&mut *tx)
            .await
            .expect("the change");
    }
    tx.commit().await.expect("commit");
}

/// Three entries sealed in one window, on a fresh store.
async fn sealed_three(pool: &PgPool) -> AccessLogStore {
    let store = AccessLogStore::new(pool.clone());
    store
        .append(vec![entry(1), entry(2), entry(3)])
        .await
        .expect("the entries");
    store.seal_committed().await.expect("sealed");
    assert_eq!(
        findings(&store).await,
        Vec::<String>::new(),
        "the window verifies before the change"
    );
    store
}

/// A record changed inside a sealed window is reported.
#[tokio::test]
async fn a_changed_record_in_a_sealed_window_is_reported() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = sealed_three(&pool).await;
    let id = id_of(&pool, 2).await;
    tamper(
        &pool,
        &format!("UPDATE audit.access_log_entry SET content_digest = sha256('x') WHERE id = {id}"),
    )
    .await;
    assert_ne!(
        findings(&store).await,
        Vec::<String>::new(),
        "the changed record is reported"
    );
}

/// A record removed from a sealed window is reported.
#[tokio::test]
async fn a_removed_record_in_a_sealed_window_is_reported() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = sealed_three(&pool).await;
    let id = id_of(&pool, 2).await;
    tamper(
        &pool,
        &format!("DELETE FROM audit.access_log_entry WHERE id = {id}"),
    )
    .await;
    let found = findings(&store).await;
    assert!(
        found.iter().any(|f| f.contains("does not match its seal")),
        "{found:?}"
    );
}

/// A record inserted into a sealed window, at an id a rolled-back write
/// left unused, is reported.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test rolls a write back on a raw pool connection, outside any request"
)]
async fn an_inserted_record_in_a_sealed_window_is_reported() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    store.append(vec![entry(1)]).await.expect("the first entry");
    {
        let mut conn = pool.acquire().await.expect("connection");
        let mut rolled = conn.begin().await.expect("begin");
        let mut gone = entry(9);
        let intake =
            ferroehr::access_log::intake::IntakeEntry::of(&mut gone, None).expect("intake");
        ferroehr::access_log::intake::take_in(
            &mut rolled,
            &[intake],
            &[Vec::new()],
            None,
            &std::collections::BTreeSet::new(),
            &ferroehr::access_log::intake::Named::default(),
        )
        .await
        .expect("a write that rolls back");
        rolled.rollback().await.expect("rollback");
    }
    store.append(vec![entry(3)]).await.expect("the last entry");
    store.seal_committed().await.expect("sealed");
    assert_eq!(findings(&store).await, Vec::<String>::new());
    let (first, last) = (id_of(&pool, 1).await, id_of(&pool, 3).await);
    assert!(
        last > first.saturating_add(1),
        "the rolled-back write used an id between"
    );
    tamper(
        &pool,
        &format!(
            "INSERT INTO audit.access_log_entry (id, record_type, uid, time_recorded, content, \
             content_digest) OVERRIDING SYSTEM VALUE \
             SELECT {first} + 1, record_type, gen_random_uuid(), time_recorded, content, \
             content_digest FROM audit.access_log_entry WHERE id = {first}"
        ),
    )
    .await;
    let found = findings(&store).await;
    assert!(
        found.iter().any(|f| f.contains("does not match its seal")),
        "{found:?}"
    );
}

/// The fold removes the entries of a sealed window only when every record
/// in it is a tombstone past its end, and records the window as folded.
#[tokio::test]
async fn the_fold_removes_whole_sealed_windows_only() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = sealed_three(&pool).await;
    let ground: openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText =
        serde_json::from_value(json!({
            "_type": "DV_CODED_TEXT", "value": "retention period ended",
            "defining_code": { "terminology_id": { "value": "local" }, "code_string": "expired" }
        }))
        .expect("the ground");
    let grounds = openehr_base::containers::NonEmptyVec::new(vec![ground]).expect("one ground");
    let removed: openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime =
        serde_json::from_value(json!({ "_type": "DV_DATE_TIME", "value": "2026-01-01T00:00:00Z" }))
            .expect("the time");
    for n in [1, 2] {
        store
            .tombstone(
                &format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"),
                removed.clone(),
                grounds.clone(),
            )
            .await
            .expect("the tombstone")
            .expect("the entry");
    }
    let far = jiff::Timestamp::from_second(4_102_444_800).expect("2100");
    let folded = store
        .fold_expired(ferroehr::access_log::expiry::Log::Local, far, 10)
        .await
        .expect("the fold");
    assert_eq!(
        folded.tombstones, 0,
        "one record of the window is still an entry"
    );
    store
        .tombstone(
            "0193f1a2-7c1e-7b4a-9f00-000000000003",
            removed.clone(),
            grounds.clone(),
        )
        .await
        .expect("the tombstone")
        .expect("the entry");
    let folded = store
        .fold_expired(ferroehr::access_log::expiry::Log::Local, far, 10)
        .await
        .expect("the fold");
    assert_eq!(folded.tombstones, 3);
    assert_eq!(folded.gaps, 1, "the window is recorded as folded");
    assert_eq!(findings(&store).await, Vec::<String>::new());
}

/// Adds, as the table's owner can, a row trigger that refuses an entry whose
/// transaction holds no xid when the row has already drawn its id: no write
/// between the draw and a row trigger takes an xid, so a row that passes
/// proves its statement took the xid before it drew the id.
#[expect(
    clippy::disallowed_methods,
    reason = "a test adds a trigger on a raw pool connection, as only the owner can"
)]
async fn refuse_an_id_drawn_before_the_xid(pool: &PgPool) {
    let mut conn = pool.acquire().await.expect("connection");
    for statement in [
        "CREATE FUNCTION audit.test_xid_before_id() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF pg_catalog.pg_current_xact_id_if_assigned() IS NULL THEN \
             RAISE EXCEPTION 'the entry % drew its id before its transaction held an xid', NEW.id; \
           END IF; \
           RETURN NEW; \
         END $$",
        "CREATE TRIGGER test_xid_before_id BEFORE INSERT ON audit.access_log_entry \
         FOR EACH ROW EXECUTE FUNCTION audit.test_xid_before_id()",
    ] {
        sqlx::query(statement)
            .execute(&mut *conn)
            .await
            .expect("the check is added");
    }
}

/// Each statement that inserts an entry takes its transaction's xid before
/// the first row draws its id, the order the frontier's proof rests on: the
/// plain insert with and without a part it carries, the intake statement when
/// it stores nothing before the entries, and the intake statement when it
/// stores a part first (#6242).
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test writes on raw pool connections, each statement its transaction's first"
)]
async fn every_inserting_statement_takes_its_xid_before_its_first_id() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    store
        .append(vec![entry(1)])
        .await
        .expect("the part and a token are stored");
    let part: String =
        sqlx::query_scalar("SELECT encode(digest, 'hex') FROM audit.access_log_shared")
            .fetch_one(&pool)
            .await
            .expect("the one shared part");
    let token = || ferroehr::access_log::intake::IntakeToken {
        parameter: "purpose",
        system: Some("urn:example:xid".to_owned()),
        code: Some("TREAT".to_owned()),
        shares: false,
    };
    let write = |n: u32, tokens: Vec<ferroehr::access_log::intake::IntakeToken>, known: bool| {
        let pool = pool.clone();
        let part = part.clone();
        async move {
            let mut written = entry(n);
            let intake =
                ferroehr::access_log::intake::IntakeEntry::of(&mut written, None).expect("intake");
            let stored = if known {
                std::collections::BTreeSet::from([part.as_str()])
            } else {
                std::collections::BTreeSet::new()
            };
            let mut conn = pool.acquire().await.expect("connection");
            ferroehr::access_log::intake::take_in(
                &mut conn,
                &[intake],
                &[tokens],
                None,
                &stored,
                &ferroehr::access_log::intake::Named::default(),
            )
            .await
        }
    };
    write(2, vec![token()], true)
        .await
        .expect("the token is stored once");

    refuse_an_id_drawn_before_the_xid(&pool).await;
    write(3, Vec::new(), true)
        .await
        .expect("the plain insert takes its xid first");
    write(4, vec![token()], true)
        .await
        .expect("the intake statement, storing nothing first, takes its xid first");
    write(5, vec![token()], false)
        .await
        .expect("the intake statement, storing the part first, takes its xid first");
    write(6, Vec::new(), false)
        .await
        .expect("the plain insert of a part it carries takes its xid first");
}

/// Every statement in the server's code and migrations that inserts into
/// `audit.access_log_entry` reads `pg_current_xact_id()` in the CTE its rows
/// join, so a new inserting path that leaves it out fails here (#6242).
#[test]
fn no_statement_inserts_an_entry_without_taking_its_xid_first() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    let mut dirs = vec![
        root.join("src"),
        root.join("migrations"),
        root.join("../ferroehr-rest/src"),
        root.join("../ferroehr-server/src"),
    ];
    while let Some(dir) = dirs.pop() {
        for item in std::fs::read_dir(&dir).expect("a source directory") {
            let path = item.expect("an entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "rs" || ext == "sql")
            {
                files.push(path);
            }
        }
    }
    let mut inserts = 0;
    for file in files {
        let text = std::fs::read_to_string(&file).expect("a source file");
        let code: String = if file.extension().is_some_and(|ext| ext == "sql") {
            text.lines()
                .map(|line| line.split_once("--").map_or(line, |(code, _)| code))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            text
        };
        let flat: String = code.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut rest = flat.as_str();
        while let Some(at) = rest.find("INSERT INTO audit.access_log_entry ") {
            let before = &rest[..at];
            let statement_start = before
                .rfind(';')
                .into_iter()
                .chain(before.rfind("const "))
                .max()
                .unwrap_or(0);
            let statement = &rest[statement_start..];
            let end = statement[at - statement_start..]
                .find(';')
                .map_or(statement.len(), |end| at - statement_start + end);
            let statement = &statement[..end];
            inserts += 1;
            assert!(
                statement.contains(
                    "own_xid AS MATERIALIZED (SELECT pg_catalog.pg_current_xact_id() AS taken)"
                ) && statement.contains("FROM own_xid CROSS JOIN"),
                "{}: an insert into access_log_entry joins no own_xid CTE: {statement}",
                file.display()
            );
            rest = &rest[at + 1..];
        }
    }
    assert!(
        inserts >= 2,
        "the plain insert and the intake statement are found"
    );
}
