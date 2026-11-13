// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-log store (#5125) against a real `PostgreSQL` 18: an entry is
//! kept as its canonical form and chained by the link the draft defines
//! (`docs/plans/access-log.md` §Integrity), the verifier finds a removed, an inserted and a reordered
//! record, a tombstone replaces an entry and leaves the chain intact, and the
//! records of `audit_event` and of the access log verify as one chain.
//!
//! No openEHR spec governs audit storage: our own design.

use jiff::Timestamp;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool, Row};

use ferroehr::access_log::store::{AccessLogStore, canonical_form};
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::store::{AuditStore, VerifyScope};
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

/// The example entry of the access-log proposal, the fixture the draft's own
/// tests in `openehr-rm` read.
pub(crate) fn example() -> Value {
    serde_json::from_str(include_str!(
        "../../../../crates/openehr-rm/tests/fixtures/draft/access_log_entry.json"
    ))
    .expect("the example entry fixture is JSON")
}

/// The proposal's example entry with the uid and EHR of number `n`.
fn entry(n: u32) -> AccessLogEntry {
    let mut value = example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["ehr_id"]["value"] = json!(format!("7d44b88c-4199-4bad-97dc-{n:012x}"));
    serde_json::from_value(value).expect("the example entry reads")
}

/// `bytes` in lower-case hexadecimal.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// The SHA-256 of `text`'s UTF-8 octets in lower-case hexadecimal.
pub(crate) fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// The digest the first record of the chain links back to
/// (`audit.audit_chain_genesis()`).
fn genesis() -> String {
    sha256_hex("ferroehr.audit_chain.v1")
}

/// Append entries `numbers` through the store, in their order.
async fn append(
    store: &AccessLogStore,
    numbers: std::ops::RangeInclusive<u32>,
) -> Vec<AccessLogEntry> {
    store
        .append(numbers.map(entry).collect())
        .await
        .expect("the entries append")
}

/// The findings of a full verification, as (position, text).
async fn findings(pool: &PgPool) -> Vec<(Option<i64>, String)> {
    crate::fixtures::verify_chain(pool)
        .await
        .expect("verify")
        .into_iter()
        .map(|finding| (finding.chain_seq, finding.finding))
        .collect()
}

/// Run `statements` with the table's triggers off, as only its owner can: the
/// attacker the chain detects.
async fn tamper(pool: &PgPool, statements: &[&str]) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    sqlx::query("ALTER TABLE audit.access_log_record DISABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("disable the triggers");
    for statement in statements {
        sqlx::query(AssertSqlSafe((*statement).to_owned()))
            .execute(&mut *tx)
            .await
            .expect("the tampering statement itself must succeed");
    }
    sqlx::query("ALTER TABLE audit.access_log_record ENABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("enable the triggers");
    tx.commit().await.expect("commit");
}

/// The grounds of a removal at the end of its retention period.
fn expiry() -> NonEmptyVec<DvCodedText> {
    let ground: DvCodedText = serde_json::from_value(json!({
        "_type": "DV_CODED_TEXT", "value": "retention period ended",
        "defining_code": { "terminology_id": { "value": "local" }, "code_string": "expired" }
    }))
    .expect("the ground reads");
    NonEmptyVec::new(vec![ground]).expect("one ground")
}

/// A date and time as the RM writes it.
fn at(value: &str) -> DvDateTime {
    serde_json::from_value(json!({ "_type": "DV_DATE_TIME", "value": value }))
        .expect("the time reads")
}

/// An entry is stored as its canonical form, read back with its link, and
/// the link is the draft's: the content digest over the canonical form and
/// the digest over the previous digest, the sequence and the content digest.
#[tokio::test]
async fn entries_are_stored_as_their_canonical_form_and_chained_by_their_links() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let written = append(&store, 1..=3).await;

    let mut previous = genesis();
    for (expected, entry) in (1_i64..).zip(&written) {
        let link = &entry.link;
        assert_eq!(link.sequence, expected);
        assert_eq!(link.algorithm, "SHA-256");
        assert_eq!(link.previous_digest, previous);
        let canonical = canonical_form(entry).expect("canonical form");
        assert!(
            !canonical.contains("\"link\""),
            "the canonical form leaves the link out"
        );
        assert_eq!(link.content_digest, sha256_hex(&canonical));
        assert_eq!(
            link.digest,
            sha256_hex(&format!(
                "{}\n{}\n{}",
                link.previous_digest, link.sequence, link.content_digest
            ))
        );
        previous.clone_from(&link.digest);

        let read = store.read(entry.uid.value()).await.expect("read");
        assert_eq!(read, Some(AccessLogRecord::AccessLogEntry(entry.clone())));
        let stored: String =
            sqlx::query_scalar("SELECT content FROM audit.access_log_record WHERE chain_seq = $1")
                .bind(expected)
                .fetch_one(&pool)
                .await
                .expect("content");
        assert_eq!(stored, canonical, "the record is its canonical form");
    }

    let row = sqlx::query(
        "SELECT ehr_id, action, result, party_id, request_id, retain_until::text, \
                time_recorded = '2026-11-07T09:14:03.120Z'::timestamptz AS at \
           FROM audit.access_log_record WHERE chain_seq = 2",
    )
    .fetch_one(&pool)
    .await
    .expect("the search columns");
    assert_eq!(
        row.get::<String, _>("ehr_id"),
        "7d44b88c-4199-4bad-97dc-000000000002"
    );
    assert_eq!(row.get::<String, _>("action"), "read");
    assert_eq!(row.get::<String, _>("result"), "granted");
    assert_eq!(
        row.get::<String, _>("party_id"),
        "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5"
    );
    assert_eq!(row.get::<String, _>("request_id"), "01JC2Q7N3X");
    assert_eq!(row.get::<String, _>("retain_until"), "2031-11-07");
    assert!(row.get::<bool, _>("at"));

    assert_eq!(
        findings(&pool).await,
        Vec::new(),
        "a fresh log verifies clean"
    );
}

/// An entry that breaks an invariant of the draft leaves no record.
#[tokio::test]
async fn an_entry_that_breaks_an_invariant_is_not_written() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let mut broken = entry(1);
    broken.system_id = String::new();
    assert!(store.append(vec![entry(2), broken]).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_record")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 0, "the batch leaves nothing behind");
}

/// An openEHR-coded term of the draft's groups.
fn openehr(rubric: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": rubric,
            "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": rubric } })
}

/// An entry carries the grounds of its access decision and its emergency
/// mark through the store unchanged, and a refused access that reached no
/// EHR is stored with no EHR: the two shapes the request path (#5126) and
/// the decision grounds (#5514) write.
#[tokio::test]
async fn an_entry_keeps_its_decision_grounds_and_a_refusal_needs_no_ehr() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());

    let mut grounded = serde_json::to_value(entry(1)).expect("the entry writes");
    grounded["is_emergency"] = json!(true);
    grounded["authorisations"] = json!([
        { "_type": "ACCESS_AUTHORISATION", "kind": openehr("role") },
        { "_type": "ACCESS_AUTHORISATION", "kind": openehr("treatment relationship") },
        { "_type": "ACCESS_AUTHORISATION", "kind": openehr("consent"),
          "reference": { "_type": "DV_IDENTIFIER", "issuer": "cdr.example.org",
              "id": "8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::2",
              "type": "EHR_ACCESS" } },
        { "_type": "ACCESS_AUTHORISATION", "kind": openehr("agreement"),
          "policy": { "_type": "DV_URI", "value": "ehr://cdr.example.org/access_policy/1" },
          "name": "access policy", "version": "3",
          "legal_grounds": [openehr("legal obligation")] },
        { "_type": "ACCESS_AUTHORISATION", "kind": openehr("emergency") }
    ]);
    let grounded: AccessLogEntry =
        serde_json::from_value(grounded).expect("the grounded entry reads");

    let mut refused = serde_json::to_value(entry(2)).expect("the entry writes");
    refused["outcome"] = json!({ "_type": "ACCESS_OUTCOME", "result": openehr("refused"),
        "reason": { "_type": "DV_CODED_TEXT", "value": "no grant",
            "defining_code": { "terminology_id": { "value": "local" }, "code_string": "no-grant" } },
        "delivered": 0 });
    let members = refused.as_object_mut().expect("an object");
    members.remove("ehr_id");
    members.remove("items");
    let refused: AccessLogEntry = serde_json::from_value(refused).expect("the refusal reads");

    let written = store
        .append(vec![grounded, refused])
        .await
        .expect("both append");
    for entry in &written {
        let read = store.read(entry.uid.value()).await.expect("read");
        assert_eq!(read, Some(AccessLogRecord::AccessLogEntry(entry.clone())));
    }
    let grounds = written[0].authorisations.as_deref().unwrap_or_default();
    assert_eq!(grounds.len(), 5, "every ground is kept");
    assert!(written[0].is_emergency);

    let row = sqlx::query("SELECT ehr_id, result FROM audit.access_log_record WHERE chain_seq = 2")
        .fetch_one(&pool)
        .await
        .expect("the refusal's search columns");
    assert_eq!(row.get::<Option<String>, _>("ehr_id"), None);
    assert_eq!(row.get::<String, _>("result"), "refused");
    assert_eq!(findings(&pool).await, Vec::new(), "both verify");
}

/// The statements that insert a forged record at position 2 and move every
/// later record one on, its link correct for the record before it.
const INSERTED: &[&str] = &[
    "UPDATE audit.access_log_record SET chain_seq = -chain_seq WHERE chain_seq >= 2",
    "UPDATE audit.access_log_record SET chain_seq = -chain_seq + 1 WHERE chain_seq < 0",
    "INSERT INTO audit.access_log_record \
     SELECT (audit.access_log_record_derive(forged)).* FROM ( \
       SELECT jsonb_populate_record(NULL::audit.access_log_record, jsonb_build_object( \
         'chain_seq', 2, 'content', c.content, 'algorithm', 'SHA-256', \
         'previous_digest', p.digest, 'content_digest', audit.access_log_content_digest(c.content), \
         'digest', audit.access_log_link_digest(p.digest, 2, audit.access_log_content_digest(c.content)), \
         'stored_at', now())) AS forged \
         FROM audit.access_log_record p, \
              (SELECT replace(content, '000000000003', '0000000000ff') AS content \
                 FROM audit.access_log_record WHERE chain_seq = 4) c \
        WHERE p.chain_seq = 1) AS f",
];

/// A record removed, inserted or reordered breaks the chain where it happened,
/// and the verifier names the position.
#[tokio::test]
async fn the_verifier_finds_a_removed_an_inserted_and_a_reordered_record() {
    let cases: [(&str, &[&str], i64); 3] = [
        (
            "removed",
            &["DELETE FROM audit.access_log_record WHERE chain_seq = 2"],
            3,
        ),
        ("inserted", INSERTED, 3),
        (
            "reordered",
            &[
                "UPDATE audit.access_log_record SET chain_seq = -chain_seq WHERE chain_seq IN (2, 3)",
                "UPDATE audit.access_log_record SET chain_seq = 5 + chain_seq WHERE chain_seq < 0",
            ],
            2,
        ),
    ];
    for (case, statements, position) in cases {
        let testdb = testkit::db().await.expect("testkit database");
        let pool = testdb.pool();
        append(&AccessLogStore::new(pool.clone()), 1..=4).await;
        tamper(&pool, statements).await;
        let found = findings(&pool).await;
        assert!(
            found.iter().any(|(at, _)| *at == Some(position)),
            "the {case} record must be found at position {position}: {found:?}"
        );
    }
}

/// A modified entry is found, and so is a modified search column.
#[tokio::test]
async fn the_verifier_finds_a_modified_entry_and_a_modified_search_column() {
    for statement in [
        "UPDATE audit.access_log_record SET content = replace(content, '\"read\"', '\"update\"') \
         WHERE chain_seq = 2",
        "UPDATE audit.access_log_record SET ehr_id = 'someone-else' WHERE chain_seq = 2",
    ] {
        let testdb = testkit::db().await.expect("testkit database");
        let pool = testdb.pool();
        append(&AccessLogStore::new(pool.clone()), 1..=3).await;
        tamper(&pool, &[statement]).await;
        let found = findings(&pool).await;
        assert_eq!(
            found,
            vec![(
                Some(2),
                "record content was modified after it was written".to_owned()
            )],
            "`{statement}` must be found"
        );
    }
}

/// A tombstone replaces an entry in the middle of the log in place: it keeps
/// the uid, the system, the time and the link, and the chain verifies.
#[tokio::test]
async fn a_tombstone_replaces_an_entry_and_the_chain_stays_intact() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let written = append(&store, 1..=3).await;
    let middle = written.get(1).expect("the second entry");

    let tombstone = store
        .tombstone(middle.uid.value(), at("2031-11-08T00:00:00Z"), expiry())
        .await
        .expect("the tombstone is written")
        .expect("the entry is there");
    assert_eq!(tombstone.uid, middle.uid);
    assert_eq!(tombstone.system_id, middle.system_id);
    assert_eq!(tombstone.time_recorded, middle.time_recorded);
    assert_eq!(tombstone.link, middle.link);
    assert_eq!(
        store.read(middle.uid.value()).await.expect("read"),
        Some(AccessLogRecord::AccessLogTombstone(tombstone))
    );

    let row = sqlx::query(
        "SELECT record_type, ehr_id, retain_until, time_removed IS NOT NULL AS removed \
           FROM audit.access_log_record WHERE chain_seq = 2",
    )
    .fetch_one(&pool)
    .await
    .expect("the tombstone row");
    assert_eq!(row.get::<String, _>("record_type"), "ACCESS_LOG_TOMBSTONE");
    assert_eq!(
        row.get::<Option<String>, _>("ehr_id"),
        None,
        "the content is gone"
    );
    assert!(row.get::<bool, _>("removed"));

    assert_eq!(
        findings(&pool).await,
        Vec::new(),
        "the chain verifies over the tombstone"
    );
    assert_eq!(
        store
            .tombstone(middle.uid.value(), at("2031-11-09T00:00:00Z"), expiry())
            .await
            .expect("a second removal answers"),
        None,
        "a tombstone is not replaced again"
    );
}

/// The canonical form of `record` as a SQL string literal.
fn literal(record: &impl serde::Serialize) -> String {
    format!(
        "'{}'",
        canonical_form(record)
            .expect("canonical form")
            .replace('\'', "''")
    )
}

/// The table admits an entry's replacement by its own tombstone and refuses
/// every other change, whoever asks.
#[tokio::test]
async fn the_table_admits_the_tombstone_replacement_and_nothing_else() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let written = append(&store, 1..=2).await;
    let first = written.first().expect("the first entry");

    let mut changed = first.clone();
    changed.is_emergency = true;
    let foreign: Value = json!({
        "_type": "ACCESS_LOG_TOMBSTONE",
        "uid": { "_type": "HIER_OBJECT_ID", "value": "0193f1a2-7c1e-7b4a-9f00-0000000000aa" },
        "system_id": first.system_id,
        "time_recorded": { "_type": "DV_DATE_TIME", "value": "2026-11-07T09:14:03.120Z" },
        "link": example()["link"],
        "time_removed": { "_type": "DV_DATE_TIME", "value": "2031-11-08T00:00:00Z" },
        "grounds": [{ "_type": "DV_CODED_TEXT", "value": "retention period ended",
            "defining_code": { "terminology_id": { "value": "local" }, "code_string": "expired" } }]
    });
    let foreign: AccessLogRecord = serde_json::from_value(foreign).expect("the tombstone reads");
    let with_link = serde_json::to_string(&AccessLogRecord::AccessLogEntry(entry(9)))
        .expect("the entry writes");

    for statement in [
        format!(
            "UPDATE audit.access_log_record SET content = {} WHERE chain_seq = 1",
            literal(&changed)
        ),
        format!(
            "UPDATE audit.access_log_record SET content = {} WHERE chain_seq = 1",
            literal(&foreign)
        ),
        "UPDATE audit.access_log_record SET digest = repeat('0', 64) WHERE chain_seq = 1"
            .to_owned(),
        "UPDATE audit.access_log_record SET ehr_id = 'someone-else' WHERE chain_seq = 1".to_owned(),
        "DELETE FROM audit.access_log_record WHERE chain_seq = 2".to_owned(),
        "TRUNCATE audit.access_log_record".to_owned(),
        format!(
            "INSERT INTO audit.access_log_record (content) VALUES ('{}')",
            with_link.replace('\'', "''")
        ),
        format!(
            "INSERT INTO audit.access_log_record (content) VALUES ({})",
            literal(&foreign)
        ),
    ] {
        let outcome = sqlx::query(AssertSqlSafe(statement.clone()))
            .execute(&pool)
            .await;
        assert!(outcome.is_err(), "the table must refuse `{statement}`");
    }

    store
        .tombstone(first.uid.value(), at("2031-11-08T00:00:00Z"), expiry())
        .await
        .expect("the tombstone replacement is admitted")
        .expect("the entry is there");
    let again = sqlx::query(AssertSqlSafe(format!(
        "UPDATE audit.access_log_record SET content = {} WHERE chain_seq = 1",
        literal(first)
    )))
    .execute(&pool)
    .await;
    assert!(again.is_err(), "a tombstone never turns back into an entry");
    assert_eq!(findings(&pool).await, Vec::new());
}

/// A read event of the `audit_event` form, written at `at`.
fn legacy_event(at: Timestamp) -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut event.user_id);
    event.timestamp = at;
    event
}

/// The `audit_event` records and the access-log records are one chain: each
/// links to the record before it whatever its form, a full and an incremental
/// verification walk both, and the incremental one finds a change to an
/// access-log record written after its checkpoint.
#[tokio::test]
async fn both_forms_verify_in_one_chain_from_the_checkpoint_on() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let audit = AuditStore::new(pool.clone());
    let log = AccessLogStore::new(pool.clone());

    audit
        .insert(&legacy_event(Timestamp::now()), Some("patient-42"))
        .await
        .expect("an audit_event record");
    let first = append(&log, 1..=2).await;
    audit
        .insert(&legacy_event(Timestamp::now()), Some("patient-42"))
        .await
        .expect("an audit_event record after the entries");

    let legacy_link: Vec<u8> =
        sqlx::query_scalar("SELECT row_hash FROM audit.audit_event WHERE chain_seq = 1")
            .fetch_one(&pool)
            .await
            .expect("the first record's link");
    let legacy_hex = hex(&legacy_link);
    let opening = first.first().expect("the first entry");
    assert_eq!(opening.link.sequence, 2, "the entry continues the chain");
    assert_eq!(opening.link.previous_digest, legacy_hex);
    let after: Vec<u8> =
        sqlx::query_scalar("SELECT prev_hash FROM audit.audit_event WHERE chain_seq = 4")
            .fetch_one(&pool)
            .await
            .expect("the last record's predecessor link");
    let closing = first.last().expect("the second entry");
    assert_eq!(
        hex(&after),
        closing.link.digest,
        "an audit_event record links to the entry before it"
    );

    assert_eq!(findings(&pool).await, Vec::new());
    assert!(
        audit
            .verify_chain_advancing(VerifyScope::SinceCheckpoint)
            .await
            .expect("verify")
            .is_empty()
    );
    let checkpoint: i64 =
        sqlx::query_scalar("SELECT through_seq FROM audit.audit_chain_checkpoint")
            .fetch_one(&pool)
            .await
            .expect("the checkpoint");
    assert_eq!(checkpoint, 4);

    append(&log, 3..=4).await;
    assert!(
        audit
            .verify_chain_advancing(VerifyScope::SinceCheckpoint)
            .await
            .expect("verify")
            .is_empty()
    );
    let checkpoint: i64 =
        sqlx::query_scalar("SELECT through_seq FROM audit.audit_chain_checkpoint")
            .fetch_one(&pool)
            .await
            .expect("the checkpoint");
    assert_eq!(
        checkpoint, 6,
        "the checkpoint moves to the newest access-log record"
    );

    append(&log, 5..=5).await;
    tamper(
        &pool,
        &["UPDATE audit.access_log_record SET content = replace(content, '\"read\"', '\"delete\"') \
           WHERE chain_seq = 7"],
    )
    .await;
    let found = audit
        .verify_chain_advancing(VerifyScope::SinceCheckpoint)
        .await
        .expect("verify");
    assert!(
        found.iter().any(|finding| finding.chain_seq == Some(7)),
        "the incremental run finds the record changed after its checkpoint: {found:?}"
    );
}

/// The application role appends and replaces by a tombstone through the
/// store, and can change nothing else.
#[tokio::test]
async fn the_application_role_appends_and_replaces_by_a_tombstone_only() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let role = format!("{}_logrole", testdb.name());
    sqlx::query(AssertSqlSafe(format!(
        "CREATE ROLE {role} LOGIN PASSWORD 'testpw' IN ROLE ferroehr_clinical"
    )))
    .execute(&pool)
    .await
    .expect("create the app-privilege role");
    let (scheme, rest) = testdb.url().split_once("://").expect("dsn scheme");
    let tail = rest.split_once('@').map_or(rest, |(_, t)| t);
    let url = format!("{scheme}://{role}:testpw@{tail}");
    let app = PgPool::connect(&url)
        .await
        .expect("connect as the app role");

    let store = AccessLogStore::new(app.clone());
    let written = append(&store, 1..=2).await;
    let first = written.first().expect("the first entry");
    store
        .tombstone(first.uid.value(), at("2031-11-08T00:00:00Z"), expiry())
        .await
        .expect("the app role replaces an entry by its tombstone")
        .expect("the entry is there");

    let mut conn = PgConnection::connect(&url).await.expect("connect");
    for statement in [
        "UPDATE audit.access_log_record SET digest = repeat('0', 64)",
        "UPDATE audit.access_log_record SET ehr_id = 'someone-else'",
        "INSERT INTO audit.access_log_record (content, chain_seq) VALUES ('{}', 99)",
        "DELETE FROM audit.access_log_record",
        "TRUNCATE audit.access_log_record",
        "ALTER TABLE audit.access_log_record DISABLE TRIGGER USER",
    ] {
        let outcome = sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&mut conn)
            .await;
        assert!(
            outcome.is_err(),
            "the application role must not run `{statement}`"
        );
    }
    let clean: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.verify_audit_chain()")
        .fetch_one(&mut conn)
        .await
        .expect("the application role verifies the chain");
    assert_eq!(clean, 0);
}
