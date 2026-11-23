// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-log store (#5125) against a real `PostgreSQL` 19: an entry is
//! kept as its canonical form with its content digest and sealed in windows
//! (#6242), the verifier finds a removed, an inserted and a reordered record
//! inside a sealed window, a tombstone replaces an entry and leaves its window
//! intact, and the access log is kept apart from the records of `audit_event`
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

/// Seal every committed entry, then run `statements` with the table's
/// triggers off, as only its owner can: the attacker the seals detect.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn tamper(pool: &PgPool, statements: &[&str]) {
    AccessLogStore::new(pool.clone())
        .seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
        .await
        .expect("sealed before the change");
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    for table in ["access_log_entry"] {
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
    for table in ["access_log_entry"] {
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
/// back as it was written, and its content digest is the SHA-256 of its
/// stored facts, never of the canonical form a reader renders (#5703, #6322).
#[tokio::test]
async fn entries_are_stored_in_two_parts_that_join_into_their_canonical_form() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let written = append(&store, 1..=3).await;

    for (expected, entry) in (1_i64..).zip(&written) {
        let canonical = canonical_form(entry).expect("canonical form");
        let content_digest: String = sqlx::query_scalar(
            "SELECT encode(content_digest, 'hex') FROM audit.access_log_entry WHERE id = $1",
        )
        .bind(expected)
        .fetch_one(&pool)
        .await
        .expect("the content digest");
        assert_eq!(content_digest, facts_digest(&pool, expected).await);
        assert_ne!(
            content_digest,
            sha256_hex(&canonical),
            "the seal covers the stored facts, never the rendering"
        );

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
        "HL7_AuditEventAction"
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

/// The content digests of the same entries are the same bytes under one
/// stored form (#5784, #6322): the digests below are the facts digests of
/// the example entries with their action coded under `HL7_AuditEventAction`
/// (#6070), stored in form 1, and the log reads them back from its rows.
#[tokio::test]
async fn the_content_digests_of_the_same_entries_are_unchanged_by_the_stored_form() {
    const DIGESTS: [&str; 3] = [
        "01798f8f00165796e0b3834b28bbc0a36ece0283566b9618909a9989079ee0d5",
        "77cb7f85cce51dd543e42ec0adf93c97efc1dbd414980b2be4f0d4118be5d7dc",
        "af8c09a6c0415f623b691dfdf9f4ace04f6adaf8386a92f13fc82bbac2920189",
    ];
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    append(&store, 1..=3).await;
    for (content_digest, id) in DIGESTS.iter().zip(1_i64..) {
        let stored: String = sqlx::query_scalar(
            "SELECT encode(content_digest, 'hex') FROM audit.access_log_record WHERE chain_seq = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("the stored digest");
        assert_eq!(stored, *content_digest);
    }
    assert_eq!(findings(&pool).await, Vec::new(), "the log verifies clean");
}

/// The facts digest of the entry `id`, computed from its row as the verifier
/// computes it.
async fn facts_digest(pool: &PgPool, id: i64) -> String {
    let row = sqlx::query(
        "SELECT content, encode(shared, 'hex') AS shared, record_type, uid, time_recorded, \
                ehr_id, request_id, resource_id, form FROM audit.access_log_entry WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("the entry's row");
    let columns = ferroehr::access_log::stored_form::parts::assemble::RowColumns {
        record_type: row.get("record_type"),
        uid: row.get("uid"),
        time_recorded: row
            .get::<jiff_sqlx::Timestamp, _>("time_recorded")
            .to_jiff(),
        ehr_id: row.get("ehr_id"),
        index: ferroehr::access_log::stored_form::parts::index::IndexColumns {
            request_id: row.get("request_id"),
            resource_id: row.get("resource_id"),
        },
    };
    hex(
        &ferroehr::access_log::stored_form::parts::facts::facts_digest(
            row.get("form"),
            row.get("content"),
            row.get("shared"),
            &columns,
        )
        .expect("the facts digest"),
    )
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

/// The statements that insert a forged record at id 2 of a sealed window and
/// move every later entry one on, as the table's owner can once the id
/// column takes a value given.
const INSERTED: &[&str] = &[
    "ALTER TABLE audit.access_log_entry ALTER COLUMN id SET GENERATED BY DEFAULT",
    "UPDATE audit.access_log_entry SET id = id + 100 WHERE id >= 2",
    "UPDATE audit.access_log_entry SET id = id - 99 WHERE id >= 102",
    "INSERT INTO audit.access_log_entry OVERRIDING SYSTEM VALUE \
     SELECT (audit.access_log_entry_derive(forged)).* FROM ( \
       SELECT jsonb_populate_record(NULL::audit.access_log_entry, jsonb_build_object( \
         'id', 2, 'content', c.content, \
         'record_type', c.record_type, 'uid', c.uid, 'time_recorded', c.time_recorded, \
         'ehr_id', c.ehr_id, \
         'content_digest', sha256(convert_to(c.content, 'UTF8')), 'form', 1, \
         'shared', '\\x' || encode(c.shared, 'hex'))) AS forged \
         FROM (SELECT replace(content, '000000000003', '0000000000ff') AS content, shared, \
                      record_type, replace(uid::text, '000000000003', '0000000000ff') AS uid, \
                      time_recorded, replace(ehr_id::text, '000000000003', '0000000000ff') AS ehr_id \
                 FROM audit.access_log_record WHERE chain_seq = 4) c) AS f",
    "ALTER TABLE audit.access_log_entry ALTER COLUMN id SET GENERATED ALWAYS",
];

/// A record removed, inserted or reordered inside a sealed window breaks the
/// window's seal, and the verifier names the window's first entry id.
#[tokio::test]
async fn the_verifier_finds_a_removed_an_inserted_and_a_reordered_record() {
    let cases: [(&str, &[&str], i64); 3] = [
        (
            "removed",
            &["DELETE FROM audit.access_log_entry WHERE id = 2"],
            1,
        ),
        ("inserted", INSERTED, 1),
        (
            "reordered",
            &["UPDATE audit.access_log_entry e \
               SET content = o.content, content_digest = o.content_digest \
               FROM audit.access_log_entry o WHERE (e.id, o.id) IN ((2, 3), (3, 2))"],
            1,
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
         WHERE id = 2",
        "UPDATE audit.access_log_entry SET ehr_id = '00000000-0000-4000-8000-000000000000' \
         WHERE id = 2",
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

/// A search index row changed or removed under the index's cursor is found
/// at its entry, since it would hide the entry from a search (#6319).
#[tokio::test]
async fn the_verifier_finds_a_changed_and_a_missing_index_row() {
    for statement in [
        "UPDATE audit.access_log_index SET ehr_id = '00000000-0000-4000-8000-000000000000' \
         WHERE entry_id = 2",
        "UPDATE audit.access_log_index SET search_keys = search_keys || 4242::bigint WHERE entry_id = 2",
        "DELETE FROM audit.access_log_index WHERE entry_id = 2",
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
/// the uid, the system and the time, and its window verifies.
#[tokio::test]
async fn a_tombstone_replaces_an_entry_and_its_window_stays_intact() {
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
        "the window verifies over the tombstone"
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
    store
        .seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
        .await
        .expect("sealed");
    let first = written.first().expect("the first entry");

    let mut changed = first.clone();
    changed.is_emergency = true;
    let foreign: Value = json!({
        "_type": "ACCESS_LOG_TOMBSTONE",
        "uid": { "_type": "HIER_OBJECT_ID", "value": "0193f1a2-7c1e-7b4a-9f00-0000000000aa" },
        "system_id": first.system_id,
        "time_recorded": { "_type": "DV_DATE_TIME", "value": "2026-11-07T09:14:03.120Z" },
        "time_removed": { "_type": "DV_DATE_TIME", "value": "2031-11-08T00:00:00Z" },
        "grounds": [{ "_type": "DV_CODED_TEXT", "value": "retention period ended",
            "defining_code": { "terminology_id": { "value": "local" }, "code_string": "expired" } }]
    });
    let foreign: AccessLogRecord = serde_json::from_value(foreign).expect("the tombstone reads");
    let whole = serde_json::to_string(&AccessLogRecord::AccessLogEntry(entry(9)))
        .expect("the entry writes");

    for statement in [
        format!(
            "UPDATE audit.access_log_entry SET content = {} WHERE id = \
             1",
            literal(&changed)
        ),
        format!(
            "UPDATE audit.access_log_entry SET content = {} WHERE id = \
             1",
            literal(&foreign)
        ),
        "UPDATE audit.access_log_seal SET digest = decode(repeat('0', 64), 'hex')".to_owned(),
        "UPDATE audit.access_log_entry SET ehr_id = '00000000-0000-4000-8000-000000000000' \
         WHERE id = 1"
            .to_owned(),
        "DELETE FROM audit.access_log_seal".to_owned(),
        "DELETE FROM audit.access_log_entry \
         WHERE id = 2"
            .to_owned(),
        "TRUNCATE audit.access_log_seal".to_owned(),
        "TRUNCATE audit.access_log_entry".to_owned(),
        format!(
            "INSERT INTO audit.access_log_entry (content) VALUES ('{}')",
            whole.replace('\'', "''")
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
             1",
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

/// The access log and the `audit_event` records are kept apart: an entry
/// written between two `audit_event` records holds no position of the system
/// records, the `audit_event` records stay adjacent in windows of their own, and
/// each verifies from its own cursor, the access log's incremental run finding
/// a change to an entry sealed after its cursor.
#[tokio::test]
async fn each_log_verifies_apart_from_its_own_cursor_on() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let audit = AuditStore::new(pool.clone());
    let log = AccessLogStore::new(pool.clone());

    audit
        .insert(&legacy_event(Timestamp::now()), Some("patient-42"))
        .await
        .expect("an audit_event record");
    append(&log, 1..=2).await;
    audit
        .insert(&legacy_event(Timestamp::now()), Some("patient-42"))
        .await
        .expect("an audit_event record after the entries");

    let system: Vec<i64> = sqlx::query_scalar("SELECT seq FROM audit.audit_event ORDER BY seq")
        .fetch_all(&pool)
        .await
        .expect("the audit_event records");
    assert_eq!(
        system,
        [1, 2],
        "the system records hold no access-log entry"
    );
    let windows: Vec<(i64, i64)> =
        sqlx::query_as("SELECT first_id, last_id FROM audit.audit_event_seal ORDER BY window_no")
            .fetch_all(&pool)
            .await
            .expect("the system windows");
    assert_eq!(
        windows,
        [(1, 1), (2, 2)],
        "each audit_event record is sealed in a window of its own log, after the one before it"
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
    let windows = "SELECT s.last_id FROM audit.access_log_seal_verified v \
                   JOIN audit.access_log_seal s ON s.window_no = v.through_window";
    assert_eq!(
        cursor("SELECT through_window FROM audit.audit_event_seal_verified").await,
        2
    );
    assert_eq!(cursor(windows).await, 2);

    append(&log, 3..=4).await;
    log.seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
        .await
        .expect("sealed");
    assert!(
        audit
            .verify_chain_advancing(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
            .await
            .expect("verify")
            .is_empty()
    );
    assert_eq!(
        cursor(windows).await,
        4,
        "the access log's cursor moves to its newest window"
    );
    assert_eq!(
        cursor("SELECT through_window FROM audit.audit_event_seal_verified").await,
        2,
        "the system records' cursor stays where it was"
    );

    append(&log, 5..=5).await;
    tamper(
        &pool,
        &["UPDATE audit.access_log_entry \
           SET content = replace(content, '\"was_delivered\":true', '\"was_delivered\":false') \
           WHERE id = 5"],
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

    store
        .seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
        .await
        .expect("the app role seals");
    let mut conn = PgConnection::connect(&url).await.expect("connect");
    for statement in [
        "UPDATE audit.access_log_seal SET digest = decode(repeat('0', 64), 'hex')",
        "UPDATE audit.access_log_entry SET ehr_id = '00000000-0000-4000-8000-000000000000'",
        "DELETE FROM audit.access_log_seal",
        "DELETE FROM audit.access_log_entry",
        "TRUNCATE audit.access_log_seal",
        "TRUNCATE audit.access_log_entry",
        "ALTER TABLE audit.access_log_entry DISABLE TRIGGER USER",
        "ALTER TABLE audit.access_log_seal DISABLE TRIGGER USER",
    ] {
        let outcome = sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&mut conn)
            .await;
        assert!(
            outcome.is_err(),
            "the application role must not run `{statement}`"
        );
    }
    let clean = AccessLogStore::new(app.clone())
        .verify_windows(&ferroehr::access_log::seal::ACCESS_LOG, false)
        .await
        .expect("the application role verifies the access log");
    assert!(clean.is_empty(), "{clean:?}");
}
