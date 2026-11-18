// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-log store (#5125) against a real `PostgreSQL` 18: an entry is
//! kept as its canonical form and chained by the link the draft defines
//! (`docs/plans/access-log.md` §Integrity), the verifier finds a removed, an inserted and a reordered
//! record, a tombstone replaces an entry and leaves the chain intact, and the
//! access log is a chain of its own, apart from the records of `audit_event`
//! (#5573).
//!
//! No openEHR spec governs audit storage: our own design.

use jiff::Timestamp;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool, Row};

use ferroehr::access_log::store::{AccessLogStore, canonical_form};
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::store::AuditStore;
use ferroehr::system_log::store::chains::{AuditChain, VerifyScope};
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

/// The digest the first record of the access log links back to
/// (`audit.access_log_genesis()`).
fn genesis() -> String {
    sha256_hex("ferroehr.access_log.v1")
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
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn tamper(pool: &PgPool, statements: &[&str]) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    for table in ["access_log_entry", "access_log_link"] {
        sqlx::query(AssertSqlSafe(format!(
            "ALTER TABLE audit.{table} DISABLE TRIGGER USER"
        )))
        .execute(&mut *tx)
        .await
        .expect("disable the triggers");
    }
    for statement in statements {
        sqlx::query(AssertSqlSafe((*statement).to_owned()))
            .execute(&mut *tx)
            .await
            .expect("the tampering statement itself must succeed");
    }
    for table in ["access_log_entry", "access_log_link"] {
        sqlx::query(AssertSqlSafe(format!(
            "ALTER TABLE audit.{table} ENABLE TRIGGER USER"
        )))
        .execute(&mut *tx)
        .await
        .expect("enable the triggers");
    }
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

/// An entry is stored as its row part and the shared part its caller's
/// entries name once, which join back into its canonical form; it is read
/// back with its link, and the link is the draft's: the content digest over
/// the whole canonical form and the digest over the previous digest, the
/// sequence and the content digest (#5703).
#[tokio::test]
async fn entries_are_stored_in_two_parts_that_join_into_their_canonical_form() {
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
        let (own, document): (String, String) = sqlx::query_as(
            "SELECT r.content, audit.access_log_document(r.content, sh.content, r.record_type, \
             r.uid, r.time_recorded, r.ehr_id, r.request_id, r.resource_id)::text FROM audit.access_log_record r \
             LEFT JOIN audit.access_log_shared sh ON sh.digest = r.shared \
             WHERE r.chain_seq = $1",
        )
        .bind(expected)
        .fetch_one(&pool)
        .await
        .expect("content");
        assert!(
            !own.contains("\"accessor\""),
            "the row keeps no accessor: {own}"
        );
        for column in ["\"_type\":\"ACCESS_LOG_ENTRY\"", "\"uid\"", "\"ehr_id\""] {
            assert!(
                !own.contains(column),
                "the row leaves {column} to its column: {own}"
            );
        }
        assert!(
            own.contains("\"time_recorded\""),
            "a time its column would not write back as it was written stays on the row: {own}"
        );
        let read: Value = serde_json::from_str(&document).expect("the document");
        assert_eq!(
            canonical_json(&read).expect("canonical"),
            canonical,
            "the row part, the shared part and the columns read back as the canonical form"
        );
    }
    let shared: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_shared")
        .fetch_one(&pool)
        .await
        .expect("the shared parts");
    assert_eq!(
        shared, 1,
        "the caller's part is stored once for its three entries"
    );

    let row = sqlx::query(
        "SELECT ehr_id::text AS ehr_id, action_terminology, action_code, result_terminology, \
                result_code, party_id, retain_until::text, \
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
    assert_eq!(
        row.get::<String, _>("action_terminology"),
        "audit-event-action"
    );
    assert_eq!(row.get::<String, _>("action_code"), "R");
    assert_eq!(row.get::<String, _>("result_terminology"), "openehr_access");
    assert_eq!(row.get::<String, _>("result_code"), "granted");
    assert_eq!(
        row.get::<String, _>("party_id"),
        "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5"
    );
    assert_eq!(row.get::<String, _>("retain_until"), "2031-11-07");
    assert!(row.get::<bool, _>("at"));

    assert_eq!(
        findings(&pool).await,
        Vec::new(),
        "a fresh log verifies clean"
    );
}

/// The links of the same entries are the same bytes whatever form the log
/// stores them in (#5784): the digests below were written by the log that
/// kept each entry's index in a second table, and the log that keeps it on
/// the entry's row writes them again, and reads them back from its rows.
#[tokio::test]
async fn the_links_of_the_same_entries_are_unchanged_by_the_stored_form() {
    const LINKS: [(&str, &str); 3] = [
        (
            "9b343e891a7013a6bd67fee757f68efb56dea2fb1051f69f1e5da72476c7f111",
            "cd88160055c4d6ac897274f9dfe41afbe4542124275bdbabbda53fc5cf313c0c",
        ),
        (
            "9959bcaad431d4851812de2fa9706c3c8ec9bc33a091d186cafaa5b322a472b4",
            "54259816c69edd83a2461961d177d33bc14a244c9a1f20fd3f7037e7d240daf4",
        ),
        (
            "3b2b1125847ca121a5f2b79ffb3d1700ea213eaf44533ff5a6ecbf25dddea108",
            "760f494dcf6b375c226d9af65df07c99c71c080a4a071d4383e305d87f92a0b1",
        ),
    ];
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let written = append(&store, 1..=3).await;
    for ((entry, (content_digest, digest)), sequence) in written.iter().zip(LINKS).zip(1_i64..) {
        assert_eq!(entry.link.content_digest, content_digest);
        assert_eq!(entry.link.digest, digest);
        let stored: (String, String) = sqlx::query_as(
            "SELECT encode(content_digest, 'hex'), encode(digest, 'hex') \
               FROM audit.access_log_record WHERE chain_seq = $1",
        )
        .bind(sequence)
        .fetch_one(&pool)
        .await
        .expect("the stored link");
        assert_eq!(stored, (content_digest.to_owned(), digest.to_owned()));
    }
    assert_eq!(findings(&pool).await, Vec::new(), "the log verifies clean");
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

/// A term of the draft `openehr_access` groups, its code the rubric.
fn access(rubric: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": rubric,
            "defining_code": { "terminology_id": { "value": "openehr_access" },
                "code_string": rubric } })
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
        { "_type": "ACCESS_AUTHORISATION", "kind": access("role") },
        { "_type": "ACCESS_AUTHORISATION", "kind": access("treatment relationship") },
        { "_type": "ACCESS_AUTHORISATION", "kind": access("consent"),
          "reference": { "_type": "DV_IDENTIFIER", "issuer": "cdr.example.org",
              "id": "8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::2",
              "type": "EHR_ACCESS" } },
        { "_type": "ACCESS_AUTHORISATION", "kind": access("agreement"),
          "policy": { "_type": "DV_URI", "value": "ehr://cdr.example.org/access_policy/1" },
          "name": "access policy", "version": "3",
          "legal_grounds": [access("legal obligation")] },
        { "_type": "ACCESS_AUTHORISATION", "kind": access("emergency") }
    ]);
    let grounded: AccessLogEntry =
        serde_json::from_value(grounded).expect("the grounded entry reads");

    let mut refused = serde_json::to_value(entry(2)).expect("the entry writes");
    refused["outcome"] = json!({ "_type": "ACCESS_OUTCOME", "result": access("refused"),
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

    let row = sqlx::query(
        "SELECT ehr_id::text AS ehr_id, result_terminology, result_code FROM audit.access_log_record \
         WHERE chain_seq = 2",
    )
    .fetch_one(&pool)
    .await
    .expect("the refusal's search columns");
    assert_eq!(row.get::<Option<String>, _>("ehr_id"), None);
    assert_eq!(row.get::<String, _>("result_terminology"), "openehr_access");
    assert_eq!(row.get::<String, _>("result_code"), "refused");
    assert_eq!(findings(&pool).await, Vec::new(), "both verify");
}

/// The statements that insert a forged record at position 2 and move every
/// later record one on, its link correct for the record before it.
const INSERTED: &[&str] = &[
    "UPDATE audit.access_log_link SET chain_seq = -chain_seq WHERE chain_seq >= 2",
    "UPDATE audit.access_log_link SET chain_seq = -chain_seq + 1 WHERE chain_seq < 0",
    "INSERT INTO audit.access_log_entry OVERRIDING SYSTEM VALUE \
     SELECT (audit.access_log_entry_derive(forged)).* FROM ( \
       SELECT jsonb_populate_record(NULL::audit.access_log_entry, jsonb_build_object( \
         'id', 1000, 'content', c.content, \
         'record_type', c.record_type, 'uid', c.uid, 'time_recorded', c.time_recorded, \
         'ehr_id', c.ehr_id, \
         'content_digest', '\\x' || audit.access_log_content_digest(c.content), \
         'shared', '\\x' || encode(c.shared, 'hex'))) AS forged \
         FROM (SELECT replace(content, '000000000003', '0000000000ff') AS content, shared, \
                      record_type, replace(uid::text, '000000000003', '0000000000ff') AS uid, \
                      time_recorded, replace(ehr_id::text, '000000000003', '0000000000ff') AS ehr_id \
                 FROM audit.access_log_record WHERE chain_seq = 4) c) AS f",
    "INSERT INTO audit.access_log_link (chain_seq, entry_id, algorithm, previous_digest, digest) \
     SELECT 2, 1000, 'SHA-256', p.digest, decode(audit.access_log_link_digest( \
              encode(p.digest, 'hex'), 2, audit.access_log_content_digest(e.content)), 'hex') \
       FROM audit.access_log_record p, audit.access_log_entry e \
      WHERE p.chain_seq = 1 AND e.id = 1000",
];

/// A record removed, inserted or reordered breaks the chain where it happened,
/// and the verifier names the position.
#[tokio::test]
async fn the_verifier_finds_a_removed_an_inserted_and_a_reordered_record() {
    let cases: [(&str, &[&str], i64); 3] = [
        (
            "removed",
            &["DELETE FROM audit.access_log_link WHERE chain_seq = 2"],
            3,
        ),
        ("inserted", INSERTED, 3),
        (
            "reordered",
            &[
                "UPDATE audit.access_log_link SET chain_seq = -chain_seq WHERE chain_seq IN (2, 3)",
                "UPDATE audit.access_log_link SET chain_seq = 5 + chain_seq WHERE chain_seq < 0",
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
        "UPDATE audit.access_log_entry \
         SET content = replace(content, '\"was_delivered\":true', '\"was_delivered\":false') \
         WHERE id = (SELECT entry_id FROM audit.access_log_link WHERE chain_seq = 2)",
        "UPDATE audit.access_log_entry SET ehr_id = '00000000-0000-4000-8000-000000000000' \
         WHERE id = (SELECT entry_id FROM audit.access_log_link WHERE chain_seq = 2)",
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

/// A changed shared part is found at every entry that names it, by the
/// server's content check, even when its key check is dropped (#5703).
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn a_changed_shared_part_is_found_at_every_entry_naming_it() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    append(&AccessLogStore::new(pool.clone()), 1..=3).await;
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    for statement in [
        "ALTER TABLE audit.access_log_shared DROP CONSTRAINT ck_access_log_shared_digest",
        "ALTER TABLE audit.access_log_shared DISABLE TRIGGER USER",
        "UPDATE audit.access_log_shared \
         SET content = replace(content, '\"is_emergency\":false', '\"is_emergency\":true')",
        "ALTER TABLE audit.access_log_shared ENABLE TRIGGER USER",
    ] {
        sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&mut *tx)
            .await
            .expect("the tampering statement itself must succeed");
    }
    tx.commit().await.expect("commit");
    let found = findings(&pool).await;
    for position in 1..=3 {
        assert!(
            found.contains(&(
                Some(position),
                "record content was modified after it was written".to_owned()
            )),
            "the entry at {position} names the changed part: {found:?}"
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
            .tombstone(middle.uid.value(), at("2031-11-09T00:00:00Z"), expiry(),)
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
            "UPDATE audit.access_log_entry SET content = {} WHERE id = \
             (SELECT entry_id FROM audit.access_log_link WHERE chain_seq = 1)",
            literal(&changed)
        ),
        format!(
            "UPDATE audit.access_log_entry SET content = {} WHERE id = \
             (SELECT entry_id FROM audit.access_log_link WHERE chain_seq = 1)",
            literal(&foreign)
        ),
        "UPDATE audit.access_log_link SET digest = decode(repeat('0', 64), 'hex') WHERE chain_seq = 1"
            .to_owned(),
        "UPDATE audit.access_log_entry SET ehr_id = '00000000-0000-4000-8000-000000000000' \
         WHERE id = (SELECT entry_id FROM audit.access_log_link WHERE chain_seq = 1)"
            .to_owned(),
        "DELETE FROM audit.access_log_link WHERE chain_seq = 2".to_owned(),
        "DELETE FROM audit.access_log_entry \
         WHERE id = (SELECT entry_id FROM audit.access_log_link WHERE chain_seq = 2)"
            .to_owned(),
        "TRUNCATE audit.access_log_link".to_owned(),
        "TRUNCATE audit.access_log_entry".to_owned(),
        format!(
            "INSERT INTO audit.access_log_entry (content) VALUES ('{}')",
            with_link.replace('\'', "''")
        ),
        format!(
            "INSERT INTO audit.access_log_entry (content) VALUES ({})",
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
        "UPDATE audit.access_log_entry SET content = {} WHERE id = \
             (SELECT entry_id FROM audit.access_log_link WHERE chain_seq = 1)",
        literal(first)
    )))
    .execute(&pool)
    .await;
    assert!(again.is_err(), "a tombstone never turns back into an entry");
    assert_eq!(findings(&pool).await, Vec::new());
}

/// A party read written at `at`: an access outside the access log, which the
/// store keeps in the `audit_event` form.
fn legacy_event(at: Timestamp) -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Demographic,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut event.user_id);
    event.timestamp = at;
    event
}

/// The access log and the `audit_event` records are two chains: an entry
/// written between two `audit_event` records takes the next position of the
/// access log and links to the entry before it, the `audit_event` records
/// stay adjacent and link to each other, and each chain verifies from its own
/// cursor, the access log's incremental run finding a change to an entry
/// written after its cursor.
#[tokio::test]
async fn each_chain_verifies_apart_from_its_own_cursor_on() {
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

    let opening = first.first().expect("the first entry");
    assert_eq!(
        opening.link.sequence, 1,
        "the access log starts its own chain"
    );
    assert_eq!(opening.link.previous_digest, genesis());
    let system: Vec<(i64, Vec<u8>, Vec<u8>)> = sqlx::query_as(
        "SELECT chain_seq, prev_hash, row_hash FROM audit.audit_event ORDER BY chain_seq",
    )
    .fetch_all(&pool)
    .await
    .expect("the audit_event records");
    let positions: Vec<i64> = system.iter().map(|(seq, _, _)| *seq).collect();
    assert_eq!(
        positions,
        [1, 2],
        "the system chain holds no access-log position"
    );
    let (_, second_previous, _) = system.get(1).expect("the second record");
    let (_, _, first_link) = system.first().expect("the first record");
    assert_eq!(
        second_previous, first_link,
        "an audit_event record links to the audit_event record before it"
    );
    let system_links: Vec<String> = system.iter().map(|(_, _, link)| hex(link)).collect();
    assert!(
        first
            .iter()
            .all(|entry| !system_links.contains(&entry.link.previous_digest)),
        "no entry links to an audit_event record"
    );

    assert_eq!(findings(&pool).await, Vec::new());
    for chain in AuditChain::ALL {
        assert!(
            audit
                .verify_chain_advancing(chain, VerifyScope::SinceCheckpoint)
                .await
                .expect("verify")
                .is_empty()
        );
    }
    let cursor = |sql: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&pool)
                .await
                .expect("the cursor")
        }
    };
    assert_eq!(
        cursor("SELECT through_seq FROM audit.audit_chain_checkpoint").await,
        2
    );
    assert_eq!(
        cursor("SELECT through_seq FROM audit.access_log_chain_verified").await,
        2
    );

    append(&log, 3..=4).await;
    assert!(
        audit
            .verify_chain_advancing(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
            .await
            .expect("verify")
            .is_empty()
    );
    assert_eq!(
        cursor("SELECT through_seq FROM audit.access_log_chain_verified").await,
        4,
        "the access log's cursor moves to its newest record"
    );
    assert_eq!(
        cursor("SELECT through_seq FROM audit.audit_chain_checkpoint").await,
        2,
        "the system chain's cursor stays where it was"
    );

    append(&log, 5..=5).await;
    tamper(
        &pool,
        &["UPDATE audit.access_log_entry \
           SET content = replace(content, '\"was_delivered\":true', '\"was_delivered\":false') \
           WHERE id = (SELECT entry_id FROM audit.access_log_link WHERE chain_seq = 5)"],
    )
    .await;
    let found = audit
        .verify_chains_advancing(VerifyScope::SinceCheckpoint)
        .await
        .expect("verify");
    assert!(
        found
            .iter()
            .any(|finding| finding.chain == AuditChain::AccessLog && finding.chain_seq == Some(5)),
        "the incremental run finds the entry changed after its cursor: {found:?}"
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
        "UPDATE audit.access_log_link SET digest = decode(repeat('0', 64), 'hex')",
        "UPDATE audit.access_log_entry SET ehr_id = '00000000-0000-4000-8000-000000000000'",
        "INSERT INTO audit.access_log_link (chain_seq, entry_id, algorithm, previous_digest, digest) \
         SELECT 99, id, 'SHA-256', content_digest, content_digest FROM audit.access_log_entry LIMIT 1",
        "DELETE FROM audit.access_log_link",
        "DELETE FROM audit.access_log_entry",
        "TRUNCATE audit.access_log_link",
        "TRUNCATE audit.access_log_entry",
        "ALTER TABLE audit.access_log_entry DISABLE TRIGGER USER",
        "ALTER TABLE audit.access_log_link DISABLE TRIGGER USER",
    ] {
        let outcome = sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&mut conn)
            .await;
        assert!(
            outcome.is_err(),
            "the application role must not run `{statement}`"
        );
    }
    let clean: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.verify_access_log_chain()")
        .fetch_one(&mut conn)
        .await
        .expect("the application role verifies the access log");
    assert_eq!(clean, 0);
}
