// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The retention of the access logs imported from another system (#5664)
//! against a real `PostgreSQL` 18: an imported entry is replaced by its
//! tombstone at the `retain_until` it states, its tombstone is kept as long
//! again, and then the positions before the first record still kept in each
//! span between two imported checkpoints fold into imported gaps. The stored
//! imported log verifies against its checkpoints after every step, with or
//! without the exporter's link signatures, across a later import from the
//! same system, and with two replicas folding at once.
//!
//! No openEHR spec governs audit retention: our own design.

use jiff::Timestamp;
use serde_json::json;
use sqlx::PgPool;

use ferroehr::access_log::chain::ChainFinding;
use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log::expiry::{expire_imported, fold_imported};
use ferroehr::access_log::gap::Folded;
use ferroehr::access_log::imported::ImportedFinding;
use ferroehr::access_log::store::AccessLogStore;
use ferroehr::versioning::signature::trust::TrustedCertificates;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

use crate::access_log_checkpoint::{generate_key, import_record, keyring, tamper, trusting};

/// The system the log is exported from.
const SYSTEM: &str = "cdr-a.example.org";
/// The system that imports it.
const RECEIVER: &str = "cdr-b.example.org";
/// The EHR whose log moves first.
const MOVED: &str = "7d44b88c-4199-4bad-97dc-000000000001";
/// An EHR whose log moves later, or stays.
const OTHER: &str = "7d44b88c-4199-4bad-97dc-000000000002";

/// The instant `text` names.
fn at(text: &str) -> Timestamp {
    text.parse().expect("an instant")
}

/// A run past the retention of every entry kept to 2029.
fn expiry_run() -> Timestamp {
    at("2030-06-01T12:00:00Z")
}

/// A run past the end of every tombstone [`expiry_run`] made.
fn fold_run() -> Timestamp {
    at("2035-01-01T00:00:00Z")
}

/// Entry `n` of [`SYSTEM`] about `ehr_id`, recorded on 1 January 2026 and
/// kept to the end of `retain_until`.
fn entry(n: u32, ehr_id: &str, retain_until: &str) -> AccessLogEntry {
    entry_recorded(n, ehr_id, "2026-01-01T00:00:00Z", retain_until)
}

/// Entry `n` of [`SYSTEM`] about `ehr_id`, recorded at `recorded` and kept
/// to the end of `retain_until`.
fn entry_recorded(n: u32, ehr_id: &str, recorded: &str, retain_until: &str) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["system_id"] = json!(SYSTEM);
    value["ehr_id"]["value"] = json!(ehr_id);
    value["time_recorded"]["value"] = json!(recorded);
    value["retention"]["retain_until"]["value"] = json!(retain_until);
    serde_json::from_value(value).expect("the example entry reads")
}

/// The exporting system's log in `pool`, signing with `key`.
fn exporter(pool: &PgPool, key: &pgp::composed::SignedSecretKey) -> Checkpoints {
    Checkpoints::new(
        pool.clone(),
        SYSTEM.to_owned(),
        keyring(key),
        std::sync::Arc::new(TrustedCertificates::default()),
    )
}

/// The importing system's log in `pool`, trusting `key`.
fn importer(pool: &PgPool, key: &pgp::composed::SignedSecretKey) -> Checkpoints {
    Checkpoints::new(
        pool.clone(),
        RECEIVER.to_owned(),
        keyring(&generate_key()),
        trusting(key),
    )
}

/// The imported gaps, as (first, last).
async fn gaps(pool: &PgPool) -> Vec<(i64, i64)> {
    sqlx::query_as(
        "SELECT first_seq, last_seq FROM audit.access_log_imported_gap \
         ORDER BY first_seq, last_seq",
    )
    .fetch_all(pool)
    .await
    .expect("read the imported gaps")
}

/// The imported records, as (sequence, record type).
async fn records(pool: &PgPool) -> Vec<(i64, String)> {
    sqlx::query_as(
        "SELECT sequence, record_type FROM audit.access_log_imported_record ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .expect("read the imported records")
}

/// The imported links' positions.
async fn links(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT sequence FROM audit.access_log_imported_link ORDER BY sequence")
        .fetch_all(pool)
        .await
        .expect("read the imported links")
}

/// A tombstone and an entry, as their record types.
fn typed(rows: &[(i64, &str)]) -> Vec<(i64, String)> {
    rows.iter()
        .map(|(sequence, kind)| (*sequence, (*kind).to_owned()))
        .collect()
}

const ENTRY: &str = "ACCESS_LOG_ENTRY";
const TOMBSTONE: &str = "ACCESS_LOG_TOMBSTONE";

/// An imported span ages out: its entries are replaced by their tombstones at
/// the `retain_until` they state, the tombstones are kept as long again, and
/// then everything before the first record still kept, other EHRs' links
/// included, folds into one imported gap that keeps the exporter's link. The
/// imported log verifies after each step, and through `verify`, which
/// reports an imported record changed afterwards.
#[tokio::test]
async fn an_imported_span_ages_out_and_still_verifies() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    let written = source
        .store()
        .append(vec![
            entry(1, MOVED, "2029-01-01"),
            entry(2, OTHER, "2029-01-01"),
            entry(3, MOVED, "2029-01-01"),
            entry(4, OTHER, "2040-01-01"),
            entry(5, MOVED, "2040-01-01"),
            entry(6, OTHER, "2040-01-01"),
        ])
        .await
        .expect("the entries append");
    let export = source.export(MOVED).await.expect("the export verifies");
    assert_eq!(export.checkpoint.sequence, 6);

    let pool = target_db.pool();
    let target = importer(&pool, &key);
    target
        .import(&export, import_record())
        .await
        .expect("the import verifies");
    assert_eq!(target.verify_imported().await.expect("verify"), []);

    let store = target.store();
    assert_eq!(
        expire_imported(store, at("2028-06-01T00:00:00Z"))
            .await
            .expect("expire"),
        0,
        "an imported entry is kept to its own retain_until"
    );
    assert_eq!(
        expire_imported(store, expiry_run()).await.expect("expire"),
        2
    );
    assert_eq!(
        records(&pool).await,
        typed(&[(1, TOMBSTONE), (3, TOMBSTONE), (5, ENTRY)])
    );
    let content: String = sqlx::query_scalar(
        "SELECT content FROM audit.access_log_imported_record WHERE sequence = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("read the tombstone");
    assert!(
        !content.contains(MOVED),
        "the tombstone keeps no EHR: {content}"
    );
    assert_eq!(target.verify_imported().await.expect("verify"), []);

    assert_eq!(
        fold_imported(store, expiry_run()).await.expect("fold"),
        Folded::default(),
        "an imported tombstone is kept past its removal"
    );
    assert_eq!(
        fold_imported(store, fold_run()).await.expect("fold"),
        Folded {
            tombstones: 2,
            gaps: 1
        }
    );
    assert_eq!(
        gaps(&pool).await,
        [(1, 4)],
        "the other EHR's links fold too"
    );
    assert_eq!(links(&pool).await, [5, 6]);
    assert_eq!(records(&pool).await, typed(&[(5, ENTRY)]));
    let link: String = sqlx::query_scalar("SELECT link FROM audit.access_log_imported_gap")
        .fetch_one(&pool)
        .await
        .expect("read the gap");
    let fourth = source.store().links(4, 4).await.expect("read the link");
    assert_eq!(link, fourth[0].digest, "the gap keeps the exporter's link");
    assert_eq!(
        target.verify_imported().await.expect("verify"),
        [],
        "the imported log verifies across the gap"
    );
    assert_eq!(
        fold_imported(store, fold_run()).await.expect("fold again"),
        Folded::default()
    );
    assert_eq!(written.len(), 6);

    tamper(
        &pool,
        "audit.access_log_imported_record",
        &["UPDATE audit.access_log_imported_record \
           SET content = replace(content, '2040-01-01', '2041-01-01') WHERE sequence = 5"],
    )
    .await;
    let verification = target.verify(None).await.expect("verify");
    assert!(!verification.intact());
    assert_eq!(
        verification.imported,
        [ImportedFinding {
            system_id: SYSTEM.to_owned(),
            checkpoint: 6,
            finding: ChainFinding::ContentDigest { sequence: 5 }
        }]
    );
}

/// An exporter that signs no link: a record kept in the middle stays
/// hash-chained to the checkpoint, so the tombstones after it wait, and a
/// rewritten link of the kept record is caught by the tombstone after it.
/// Once the kept record ends too, everything folds, and the log verifies
/// against the checkpoint through the gaps alone.
#[tokio::test]
async fn a_kept_record_holds_the_tombstones_after_it_without_signatures() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    AccessLogStore::new(source_db.pool())
        .append(vec![
            entry(1, MOVED, "2029-01-01"),
            entry(2, MOVED, "2029-01-01"),
            entry(3, MOVED, "2040-01-01"),
            entry(4, MOVED, "2029-01-01"),
            entry(5, MOVED, "2029-01-01"),
        ])
        .await
        .expect("the unsigned entries append");
    let export = source.export(MOVED).await.expect("the export verifies");
    assert!(
        export.links.iter().all(|link| link.signature.is_none()),
        "the exporter signs no link"
    );

    let pool = target_db.pool();
    let target = importer(&pool, &key);
    target
        .import(&export, import_record())
        .await
        .expect("the import verifies");
    let store = target.store();
    assert_eq!(
        expire_imported(store, expiry_run()).await.expect("expire"),
        4
    );
    assert_eq!(
        fold_imported(store, fold_run()).await.expect("fold"),
        Folded {
            tombstones: 2,
            gaps: 1
        },
        "only the positions before the kept record fold"
    );
    assert_eq!(gaps(&pool).await, [(1, 2)]);
    assert_eq!(
        records(&pool).await,
        typed(&[(3, ENTRY), (4, TOMBSTONE), (5, TOMBSTONE)])
    );
    assert_eq!(target.verify_imported().await.expect("verify"), []);

    let later = at("2041-01-01T00:00:00Z");
    assert_eq!(expire_imported(store, later).await.expect("expire"), 1);
    assert_eq!(
        fold_imported(store, at("2057-01-01T00:00:00Z"))
            .await
            .expect("fold"),
        Folded {
            tombstones: 3,
            gaps: 1
        }
    );
    assert_eq!(gaps(&pool).await, [(1, 2), (3, 5)]);
    assert_eq!(records(&pool).await, typed(&[]));
    assert_eq!(links(&pool).await, Vec::<i64>::new());
    assert_eq!(
        target.verify_imported().await.expect("verify"),
        [],
        "the checkpoint seals the last gap's link"
    );
}

/// The kept record's own link, rewritten consistently, no longer links to
/// the tombstone after it, which the fold keeps for that reason: without
/// link signatures the chain to the checkpoint is the proof.
#[tokio::test]
async fn a_rewritten_kept_link_is_caught_by_the_tombstone_after_it() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    AccessLogStore::new(source_db.pool())
        .append(vec![
            entry(1, MOVED, "2029-01-01"),
            entry(2, MOVED, "2040-01-01"),
            entry(3, MOVED, "2029-01-01"),
        ])
        .await
        .expect("the unsigned entries append");
    let export = source.export(MOVED).await.expect("the export verifies");
    let pool = target_db.pool();
    let target = importer(&pool, &key);
    target
        .import(&export, import_record())
        .await
        .expect("the import verifies");
    expire_imported(target.store(), expiry_run())
        .await
        .expect("expire");
    fold_imported(target.store(), fold_run())
        .await
        .expect("fold");
    assert_eq!(gaps(&pool).await, [(1, 1)]);
    assert_eq!(links(&pool).await, [2, 3], "the tombstone after it stays");

    tamper(
        &pool,
        "audit.access_log_imported_link",
        &["UPDATE audit.access_log_imported_link \
           SET content_digest = repeat('a', 64), \
               digest = encode(sha256(convert_to(previous_digest || E'\\n' || sequence \
                        || E'\\n' || repeat('a', 64), 'UTF8')), 'hex') \
           WHERE sequence = 2"],
    )
    .await;
    let found: Vec<ChainFinding> = target
        .verify_imported()
        .await
        .expect("verify")
        .into_iter()
        .map(|finding| finding.finding)
        .collect();
    assert!(
        found.contains(&ChainFinding::PreviousDigest { sequence: 3 }),
        "the tombstone's link no longer follows: {found:?}"
    );
}

/// A later import from the same system after a fold brings another EHR's
/// records at positions a gap already holds: they are kept and verify
/// through their own links, a repeated import of the first EHR does not
/// bring back the entries whose retention ended here, and once the later
/// records end, the fold writes no second gap over the same span.
#[tokio::test]
async fn a_later_import_after_a_fold_still_verifies() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    let recorded = "2020-01-01T00:00:00Z";
    source
        .store()
        .append(vec![
            entry_recorded(1, MOVED, recorded, "2023-01-01"),
            entry(2, OTHER, "2040-01-01"),
            entry_recorded(3, MOVED, recorded, "2023-01-01"),
            entry(4, OTHER, "2040-01-01"),
        ])
        .await
        .expect("the entries append");
    let first = source.export(MOVED).await.expect("the export verifies");

    let pool = target_db.pool();
    let target = importer(&pool, &key);
    target
        .import(&first, import_record())
        .await
        .expect("the first import");
    assert_eq!(
        expire_imported(target.store(), expiry_run())
            .await
            .expect("expire"),
        2
    );
    fold_imported(target.store(), at("2045-01-01T00:00:00Z"))
        .await
        .expect("fold");
    assert_eq!(
        gaps(&pool).await,
        [(1, 4)],
        "nothing is kept: all of it folds"
    );
    assert_eq!(records(&pool).await, typed(&[]));

    let second = source.export(OTHER).await.expect("the export verifies");
    assert_eq!(second.checkpoint.sequence, 4);
    target
        .import(&second, import_record())
        .await
        .expect("the later import is kept beside the gap");
    assert_eq!(records(&pool).await, typed(&[(2, ENTRY), (4, ENTRY)]));
    assert_eq!(links(&pool).await, [2, 3, 4]);
    assert_eq!(
        target.verify_imported().await.expect("verify"),
        [],
        "the later records verify through their own links"
    );

    target
        .import(&first, import_record())
        .await
        .expect("the repeated import");
    assert_eq!(
        records(&pool).await,
        typed(&[(2, ENTRY), (4, ENTRY)]),
        "an entry whose retention ended here is not stored again"
    );
    assert_eq!(target.verify_imported().await.expect("verify"), []);

    let later = at("2041-01-01T00:00:00Z");
    assert_eq!(
        expire_imported(target.store(), later)
            .await
            .expect("expire"),
        2
    );
    let folded = fold_imported(target.store(), at("2057-01-01T00:00:00Z"))
        .await
        .expect("fold");
    assert_eq!(folded.tombstones, 2);
    assert_eq!(gaps(&pool).await, [(1, 4)], "the same span is held once");
    assert_eq!(links(&pool).await, Vec::<i64>::new());
    assert_eq!(target.verify_imported().await.expect("verify"), []);
}

/// Two replicas folding at once never fold a position twice: the logs of
/// twenty EHRs, each imported with its own checkpoint, fold into one gap per
/// span between two checkpoints, without overlap or hole, and verify.
#[tokio::test]
async fn two_replicas_fold_imported_logs_at_once() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    let pool = target_db.pool();
    let target = importer(&pool, &key);
    let per_ehr = 30_u32;
    let ehrs = 20_u32;
    for ehr in 0..ehrs {
        let ehr_id = format!("7d44b88c-4199-4bad-97dc-{:012}", 100 + ehr);
        let batch = (1..=per_ehr)
            .map(|n| entry(ehr * per_ehr + n, &ehr_id, "2029-01-01"))
            .collect();
        source
            .store()
            .append(batch)
            .await
            .expect("the entries append");
        source.seal().await.expect("seal").expect("a checkpoint");
        let export = source.export(&ehr_id).await.expect("the export verifies");
        target
            .import(&export, import_record())
            .await
            .expect("the import verifies");
    }
    let count = u64::from(per_ehr * ehrs);
    assert_eq!(
        expire_imported(target.store(), expiry_run())
            .await
            .expect("expire"),
        count
    );

    let replica = |store: AccessLogStore| async move {
        let mut total = 0_u64;
        loop {
            let batch = store
                .fold_expired_imported(fold_run(), 25)
                .await
                .expect("a replica folds");
            total += batch.tombstones;
            if batch.gaps == 0 {
                return total;
            }
        }
    };
    let (one, two) = tokio::join!(
        tokio::spawn(replica(AccessLogStore::new(pool.clone()))),
        tokio::spawn(replica(AccessLogStore::new(pool.clone())))
    );
    let one = one.expect("the first replica runs");
    let two = two.expect("the second replica runs");
    assert_eq!(one + two, count, "every tombstone folds once");

    let held = gaps(&pool).await;
    let expected: Vec<(i64, i64)> = (0..i64::from(ehrs))
        .map(|ehr| {
            let size = i64::from(per_ehr);
            (ehr * size + 1, (ehr + 1) * size)
        })
        .collect();
    assert_eq!(held, expected, "one gap per span, cut at each checkpoint");
    assert_eq!(records(&pool).await, typed(&[]));
    assert_eq!(links(&pool).await, Vec::<i64>::new());
    assert_eq!(target.verify_imported().await.expect("verify"), []);
}

/// The fold is the one way out: an imported tombstone or link deleted
/// directly is refused.
#[tokio::test]
async fn an_imported_tombstone_leaves_only_through_the_fold() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    source
        .store()
        .append(vec![entry(1, MOVED, "2029-01-01")])
        .await
        .expect("the entry appends");
    let export = source.export(MOVED).await.expect("the export verifies");
    let pool = target_db.pool();
    let target = importer(&pool, &key);
    target
        .import(&export, import_record())
        .await
        .expect("the import verifies");
    expire_imported(target.store(), expiry_run())
        .await
        .expect("expire");
    for statement in [
        "DELETE FROM audit.access_log_imported_record",
        "DELETE FROM audit.access_log_imported_link",
        "UPDATE audit.access_log_imported_link SET digest = repeat('0', 64)",
    ] {
        assert!(
            sqlx::query(statement).execute(&pool).await.is_err(),
            "{statement} is refused"
        );
    }
    fold_imported(target.store(), fold_run())
        .await
        .expect("fold");
    for statement in [
        "UPDATE audit.access_log_imported_gap SET link = repeat('0', 64)",
        "DELETE FROM audit.access_log_imported_gap",
    ] {
        assert!(
            sqlx::query(statement).execute(&pool).await.is_err(),
            "{statement} is refused"
        );
    }
    assert_eq!(gaps(&pool).await, [(1, 1)]);
}
