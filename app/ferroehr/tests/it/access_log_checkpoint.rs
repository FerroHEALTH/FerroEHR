// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The signed checkpoints of the access log, one per sealed window (#5704,
//! #6242), the verification of the windows against them, and the export and
//! import of one EHR's log as a signed bundle between two FerroEHR databases
//! (#5129), against real `PostgreSQL` 19 databases.
//!
//! No openEHR spec governs log signing or the transfer: our own design.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use openehr_base::containers::NonEmptyVec;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use pgp::composed::{ArmorOptions, KeyType, SecretKeyParamsBuilder, SignedSecretKey};
use rand::rngs::OsRng;
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, Connection, PgPool};

use ferroehr::access_log::chain::ChainFinding;
use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log::publish::{Publication, PublicationState, PublishError, file_name};
use ferroehr::access_log::store::AccessLogError;
use ferroehr::access_log::transfer::{ImportRecord, LogExport, TransferError};
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::versioning::signature::config::{Mode, SigningConfig};
use ferroehr::versioning::signature::signer::{AttestError, Signer};
use ferroehr::versioning::signature::trust::TrustedCertificates;
use openehr_rm::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

/// The system the exporting database records as.
const SYSTEM_A: &str = "cdr-a.example.org";
/// The system the importing database records as.
const SYSTEM_B: &str = "cdr-b.example.org";
/// The EHR whose log moves.
const MOVED: &str = "7d44b88c-4199-4bad-97dc-000000000001";
/// An EHR whose log stays.
const STAYS: &str = "7d44b88c-4199-4bad-97dc-000000000002";

/// A generated Ed25519 `OpenPGP` secret key.
pub(crate) fn generate_key() -> SignedSecretKey {
    let params = SecretKeyParamsBuilder::default()
        .key_type(KeyType::Ed25519Legacy)
        .can_sign(true)
        .primary_user_id("access-log test <test@ferroehr.local>".into())
        .build()
        .expect("build key params");
    params.generate(OsRng).expect("generate key")
}

/// A `pgp`-mode keyring over `key`.
pub(crate) fn keyring(key: &SignedSecretKey) -> Arc<Signer> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ferroehr-access-log-{}-{n}.asc",
        std::process::id()
    ));
    let armored = key
        .to_armored_string(ArmorOptions::default())
        .expect("armor key");
    std::fs::write(&path, armored).expect("write key");
    let config = SigningConfig {
        mode: Mode::Pgp,
        key_file: Some(path),
        ..SigningConfig::default()
    };
    Arc::new(Signer::from_config(&config).expect("the pgp keyring loads"))
}

/// The certificates trusting `key`'s public certificate.
pub(crate) fn trusting(key: &SignedSecretKey) -> Arc<TrustedCertificates> {
    let armored = key
        .to_public_key()
        .to_armored_string(ArmorOptions::default())
        .expect("armor certificate");
    Arc::new(TrustedCertificates::from_armored([armored.as_str()]).expect("the certificate reads"))
}

/// The checkpoints of `pool` for `system_id` under `signer`.
pub(crate) fn checkpoints(pool: &PgPool, system_id: &str, signer: Arc<Signer>) -> Checkpoints {
    Checkpoints::new(
        pool.clone(),
        system_id.to_owned(),
        signer,
        Arc::new(TrustedCertificates::default()),
    )
}

/// The proposal's example entry, number `n`, of `system_id` about `ehr_id`.
pub(crate) fn entry(n: u32, system_id: &str, ehr_id: &str) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["system_id"] = json!(system_id);
    value["ehr_id"]["value"] = json!(ehr_id);
    serde_json::from_value(value).expect("the example entry reads")
}

/// Append four entries of `system_id`, alternating between the moved EHR and
/// the one that stays.
async fn append(checkpoints: &Checkpoints, system_id: &str) -> Vec<AccessLogEntry> {
    append_from(checkpoints, system_id, 1).await
}

/// Append four entries of `system_id` numbered from `first`, alternating
/// between the moved EHR and the one that stays.
async fn append_from(
    checkpoints: &Checkpoints,
    system_id: &str,
    first: u32,
) -> Vec<AccessLogEntry> {
    let entries = (first..first + 4)
        .map(|n| entry(n, system_id, if n % 2 == 1 { MOVED } else { STAYS }))
        .collect();
    checkpoints
        .store()
        .append(entries)
        .await
        .expect("the entries append")
}

/// Run `statements` with the table's triggers off, as only its owner can: the
/// attacker the chain and the checkpoint detect.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
pub(crate) async fn tamper(pool: &PgPool, table: &str, statements: &[&str]) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {table} DISABLE TRIGGER USER"
    )))
    .execute(&mut *tx)
    .await
    .expect("disable the triggers");
    for statement in statements {
        sqlx::query(AssertSqlSafe((*statement).to_owned()))
            .execute(&mut *tx)
            .await
            .expect("the tampering statement itself must succeed");
    }
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {table} ENABLE TRIGGER USER"
    )))
    .execute(&mut *tx)
    .await
    .expect("enable the triggers");
    tx.commit().await.expect("commit");
}

/// The login of the operator who imports the log.
const OPERATOR: &str = "transfer-operator";
/// The request that carried the import.
const IMPORT_REQUEST: &str = "01JC2Q7N3X";

/// The record of the import, by an operator, kept three years.
pub(crate) fn import_record() -> ImportRecord {
    let coded = |terminology: &str, value: &str| {
        json!({ "_type": "DV_CODED_TEXT", "value": value,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                    "code_string": value } })
    };
    let mut caller = AuditEvent::new(
        EventActionCode::Create,
        ObjectClass::Extract,
        EventOutcome::Success,
    );
    OPERATOR.clone_into(&mut caller.user_id);
    caller.request_id = Some(IMPORT_REQUEST.to_owned());
    ImportRecord {
        caller,
        retention: serde_json::from_value(json!({ "_type": "ACCESS_RETENTION",
            "retain_until": { "_type": "DV_DATE", "value": "2029-11-13" },
            "grounds": [coded("local", "ehds-art-9-2")] }))
        .expect("the retention reads"),
    }
}

/// The id of the entry whose uid is that of `entry`.
async fn id_of(pool: &PgPool, entry: &AccessLogEntry) -> i64 {
    sqlx::query_scalar("SELECT id FROM audit.access_log_entry WHERE uid = $1::uuid")
        .bind(entry.uid.value())
        .fetch_one(pool)
        .await
        .expect("the entry")
}

/// The last entry ids of the signed seals, in order.
async fn sealed_positions(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT last_id FROM audit.access_log_seal WHERE proof IS NOT NULL ORDER BY window_no",
    )
    .fetch_all(pool)
    .await
    .expect("the seals are read")
}

/// A checkpoint signs the seal of the window up to the newest entry with the
/// keyring's key, and the log verifies against it; sealing again with
/// nothing new returns the same checkpoint.
#[tokio::test]
async fn a_signed_checkpoint_verifies() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let entries = append(&log, SYSTEM_A).await;

    let checkpoint = log.seal().await.expect("seal").expect("a checkpoint");
    let head = id_of(pool, entries.last().expect("an entry")).await;
    assert_eq!(checkpoint.sequence, head);
    let digest: String = sqlx::query_scalar(
        "SELECT encode(digest, 'hex') FROM audit.access_log_seal WHERE last_id = $1",
    )
    .bind(head)
    .fetch_one(pool)
    .await
    .expect("the seal");
    assert_eq!(checkpoint.digest, digest);
    assert_eq!(checkpoint.system_id, SYSTEM_A);
    assert!(checkpoint.proof.contains("BEGIN PGP SIGNATURE"));
    assert_eq!(
        log.seal().await.expect("seal again"),
        Some(checkpoint.clone()),
        "nothing new keeps the checkpoint"
    );

    let verification = log.verify(None).await.expect("verify");
    assert!(verification.intact(), "{verification:?}");
    assert_eq!(verification.checkpoint, Some(checkpoint));
}

/// An entry whose content changed after it was written fails its window, a
/// checkpoint whose digest was replaced fails its signature, and a checkpoint
/// another key signed is not trusted.
#[tokio::test]
async fn a_tampered_entry_fails_verification() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let entries = append(&log, SYSTEM_A).await;
    let checkpoint = log.seal().await.expect("seal").expect("a checkpoint");
    let target = id_of(pool, entries.get(1).expect("a second entry")).await;

    tamper(
        pool,
        "audit.access_log_entry",
        &[&format!(
            "UPDATE audit.access_log_entry SET content = replace(content, \
             '\"was_delivered\":true', '\"was_delivered\":false') WHERE id = {target}"
        )],
    )
    .await;
    let verification = log.verify(None).await.expect("verify");
    assert!(!verification.intact());
    assert!(
        verification
            .chain
            .iter()
            .any(|finding| finding.chain_seq == Some(target)),
        "{verification:?}"
    );

    let mut forged = checkpoint.clone();
    forged.digest = "0".repeat(64);
    let verification = log.verify(Some(forged)).await.expect("verify");
    assert!(
        verification
            .findings
            .contains(&ChainFinding::CheckpointProof {
                sequence: checkpoint.sequence
            }),
        "{verification:?}"
    );

    let stranger = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let verification = stranger
        .verify(Some(checkpoint.clone()))
        .await
        .expect("verify");
    assert!(
        verification
            .findings
            .contains(&ChainFinding::CheckpointProof {
                sequence: checkpoint.sequence
            }),
        "a key the deployment does not trust: {verification:?}"
    );
}

/// A bundle holding a tombstone verifies, its content digest taken as
/// recorded, and an exported entry whose content changed fails at its uid.
#[tokio::test]
async fn an_export_verifies_with_a_tombstone_and_fails_with_a_tampered_entry() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let entries = append(&log, SYSTEM_A).await;
    let first = entries.first().expect("an entry");
    let ground: DvCodedText = serde_json::from_value(json!({
        "_type": "DV_CODED_TEXT", "value": "retention period ended",
        "defining_code": { "terminology_id": { "value": "local" }, "code_string": "expired" }
    }))
    .expect("the ground reads");
    let tombstone_digest: String = sqlx::query_scalar(
        "SELECT encode(content_digest, 'hex') FROM audit.access_log_entry WHERE uid = $1::uuid",
    )
    .bind(first.uid.value())
    .fetch_one(pool)
    .await
    .expect("the digest");
    log.store()
        .tombstone(
            first.uid.value(),
            first.time_recorded.clone(),
            NonEmptyVec::new(vec![ground]).expect("one ground"),
        )
        .await
        .expect("tombstone")
        .expect("the entry is replaced");

    // NOTE: the tombstone no longer names its EHR, so the moved EHR's export holds its second entry.
    let export = log.export(MOVED).await.expect("the export");
    assert_eq!(export.records.len(), 1, "{export:?}");
    assert_eq!(log.verify_bundle(&export).expect("verify"), Vec::new());

    let mut whole = export.clone();
    whole.records.insert(
        0,
        log.store()
            .read(first.uid.value())
            .await
            .expect("read")
            .expect("the tombstone"),
    );
    whole.content_digests.insert(0, tombstone_digest);
    whole.proof = String::new();
    whole.proof = log
        .sign(
            ferroehr::access_log::store::canonical_form(&whole)
                .expect("a canonical form")
                .as_bytes(),
        )
        .expect("signed");
    assert!(matches!(
        whole.records.first(),
        Some(AccessLogRecord::AccessLogTombstone(_))
    ));
    assert_eq!(log.verify_bundle(&whole).expect("verify"), Vec::new());

    let mut tampered: Value = serde_json::to_value(&export).expect("the export writes");
    tampered["records"][0]["is_emergency"] = json!(true);
    let tampered: LogExport = serde_json::from_value(tampered).expect("the export reads");
    let uid = entries
        .get(2)
        .expect("a third entry")
        .uid
        .value()
        .to_owned();
    let findings = log.verify_bundle(&tampered).expect("verify");
    assert!(
        findings.contains(&ChainFinding::ContentDigest { uid }),
        "{findings:?}"
    );
    assert!(
        findings.contains(&ChainFinding::BundleProof),
        "{findings:?}"
    );
}

/// An imported record's system, uid, EHR, `retain_until` and content digest.
type ImportedRow = (String, String, Option<String>, Option<String>, String);

/// One EHR's log leaves one database as JSON and enters another: the records
/// keep their system, `retain_until` and content digest, the import verifies
/// against the trusted certificate and is itself an entry of the importing
/// log, and an import from a system the deployment does not trust is refused.
#[tokio::test]
async fn an_ehrs_log_round_trips_through_export_and_import() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key_a = generate_key();
    let source = checkpoints(&source_db.pool(), SYSTEM_A, keyring(&key_a));
    append(&source, SYSTEM_A).await;

    let export = source.export(MOVED).await.expect("export");
    assert_eq!(export.records.len(), 2);
    assert_eq!(export.system_id, SYSTEM_A);
    let wire = serde_json::to_string(&export).expect("the export writes");
    let received: LogExport = serde_json::from_str(&wire).expect("the export reads");
    assert_eq!(received, export);

    let untrusting = checkpoints(&target_db.pool(), SYSTEM_B, keyring(&generate_key()));
    let refused = untrusting
        .import(&received, import_record())
        .await
        .expect_err("a bundle no trusted key signed is refused");
    assert!(
        matches!(&refused, TransferError::Unverified(findings)
        if findings.contains(&ChainFinding::BundleProof)),
        "{refused:?}"
    );

    let target = Checkpoints::new(
        target_db.pool(),
        SYSTEM_B.to_owned(),
        keyring(&generate_key()),
        trusting(&key_a),
    );
    let imported = target
        .import(&received, import_record())
        .await
        .expect("the import verifies and stores");
    assert_eq!(imported.system_id, SYSTEM_A);
    assert_eq!(imported.records, 2);
    assert_eq!(imported.entry.system_id, SYSTEM_B);
    assert_eq!(imported.entry.action.value, "import");
    assert_eq!(
        imported.entry.ehr_id.as_ref().map(HierObjectId::value),
        Some(MOVED)
    );
    // The import's entry is built by the one recording path, so it names its
    // caller by login and its request as every other access's entry does.
    let accessor = &imported.entry.accessor;
    assert_eq!(accessor.kind.value, "person");
    let PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) = &accessor.party
    else {
        panic!("the operator is an identified party: {accessor:?}");
    };
    let login = party
        .identifiers
        .as_ref()
        .and_then(|ids| ids.first())
        .map(|id| id.id.as_str());
    assert_eq!(login, Some(OPERATOR), "{accessor:?}");
    assert_eq!(
        imported
            .entry
            .channel
            .as_ref()
            .and_then(|channel| channel.request_id.as_deref()),
        Some(IMPORT_REQUEST)
    );

    let stored: Vec<ImportedRow> = sqlx::query_as(
        "SELECT system_id, uid, ehr_id, retain_until::text, content_digest \
         FROM audit.access_log_imported_record ORDER BY uid",
    )
    .fetch_all(&target_db.pool())
    .await
    .expect("read the imported records");
    let mut expected: Vec<_> = export
        .records
        .iter()
        .zip(&export.content_digests)
        .map(|(record, digest)| match record {
            AccessLogRecord::AccessLogEntry(entry) => (
                SYSTEM_A.to_owned(),
                entry.uid.value().to_owned(),
                Some(MOVED.to_owned()),
                Some("2031-11-07".to_owned()),
                digest.clone(),
            ),
            AccessLogRecord::AccessLogTombstone(_) => panic!("no tombstone was exported"),
        })
        .collect();
    expected.sort();
    assert_eq!(stored, expected);

    let again = target
        .import(&received, import_record())
        .await
        .expect("a repeated import is stored once and recorded again");
    assert_ne!(again.entry.uid, imported.entry.uid);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_imported_record")
        .fetch_one(&target_db.pool())
        .await
        .expect("count");
    assert_eq!(count, 2);

    let own = target.verify(None).await.expect("verify the importing log");
    assert!(own.chain.is_empty(), "{own:?}");
    assert!(own.imported.is_empty(), "{own:?}");

    tamper(
        &target_db.pool(),
        "audit.access_log_imported_record",
        &[
            "UPDATE audit.access_log_imported_record SET content = replace(content, \
           '\"was_delivered\":true', '\"was_delivered\":false')",
        ],
    )
    .await;
    let changed = target.verify(None).await.expect("verify the importing log");
    assert!(
        !changed.imported.is_empty(),
        "a changed imported record is found: {changed:?}"
    );
}

/// Under an `OpenPGP` key every sealed window is signed as the checkpoint of
/// the log up to its last entry, and each later window follows the one
/// before it.
#[tokio::test]
async fn every_sealed_window_is_signed_and_follows_the_one_before() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    assert!(log.seals());
    let entries = append(&log, SYSTEM_A).await;
    log.seal().await.expect("seal");
    let head = id_of(pool, entries.last().expect("an entry")).await;
    assert_eq!(sealed_positions(pool).await, [head]);
    let checkpoint = log
        .at(head)
        .await
        .expect("read")
        .expect("the window's checkpoint");
    let verification = log.verify(Some(checkpoint)).await.expect("verify");
    assert!(verification.intact(), "{verification:?}");

    let more = append_from(&log, SYSTEM_A, 5).await;
    log.seal().await.expect("seal");
    let next = id_of(pool, more.last().expect("an entry")).await;
    assert_eq!(sealed_positions(pool).await, [head, next]);
    let follows: bool = sqlx::query_scalar(
        "SELECT b.previous_digest = a.digest FROM audit.access_log_seal a \
         JOIN audit.access_log_seal b ON b.window_no = a.window_no + 1 LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("the seals");
    assert!(follows, "the second window follows the first");
}

/// A seal is stored once: the table refuses to change or remove one.
#[tokio::test]
async fn a_stored_seal_is_never_changed_or_removed() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let entries = append(&log, SYSTEM_A).await;
    log.seal().await.expect("seal");
    let head = id_of(pool, entries.last().expect("an entry")).await;
    for statement in [
        "UPDATE audit.access_log_seal SET proof = 'x'",
        "DELETE FROM audit.access_log_seal",
        "TRUNCATE audit.access_log_seal",
    ] {
        let refused = sqlx::query(statement)
            .execute(pool)
            .await
            .expect_err(statement);
        assert!(refused.to_string().contains("append-only"), "{refused}");
    }
    assert_eq!(sealed_positions(pool).await, [head]);
}

/// An intake write whose transaction rolls back leaves nothing to seal: no
/// entry and no window that holds one.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn a_rolled_back_intake_leaves_no_entry_and_no_seal() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    let intake =
        ferroehr::access_log::intake::IntakeEntry::of(&mut entry(1, SYSTEM_A, MOVED), None)
            .expect("the entry has a stored form");
    ferroehr::access_log::intake::take_in(
        &mut tx,
        &[intake],
        &[],
        None,
        &std::collections::BTreeSet::new(),
        &ferroehr::access_log::intake::Named::default(),
    )
    .await
    .expect("the intake write");
    tx.rollback().await.expect("rollback");
    log.store()
        .seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
        .await
        .expect("the sealer runs");
    let sealed: i64 =
        sqlx::query_scalar("SELECT coalesce(sum(entries), 0)::bigint FROM audit.access_log_seal")
            .fetch_one(pool)
            .await
            .expect("the seals");
    assert_eq!(sealed, 0);
    let entries: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_record")
        .fetch_one(pool)
        .await
        .expect("the entries are counted");
    assert_eq!(entries, 0);
}

/// A record removed from a sealed window is a finding, and the newest
/// windows removed together with their seals fail a checkpoint kept outside
/// the database.
#[tokio::test]
async fn a_deleted_record_fails_verification() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let entries = append(&log, SYSTEM_A).await;
    log.seal().await.expect("seal");
    let target = id_of(pool, entries.get(1).expect("a second entry")).await;
    tamper(
        pool,
        "audit.access_log_entry",
        &[&format!(
            "DELETE FROM audit.access_log_entry WHERE id = {target}"
        )],
    )
    .await;
    let verification = log.verify(None).await.expect("verify");
    assert!(!verification.intact(), "{verification:?}");

    let other_db = testkit::db().await.expect("a second database");
    let other = &other_db.pool();
    let log = checkpoints(other, SYSTEM_A, keyring(&generate_key()));
    append(&log, SYSTEM_A).await;
    let kept = log
        .seal()
        .await
        .expect("seal")
        .expect("the first checkpoint");
    let newest = append_from(&log, SYSTEM_A, 5).await;
    let published = log
        .seal()
        .await
        .expect("seal")
        .expect("the newest checkpoint");
    let from = id_of(other, newest.first().expect("an entry")).await;
    let to = id_of(other, newest.last().expect("an entry")).await;
    tamper(
        other,
        "audit.access_log_entry",
        &[&format!(
            "DELETE FROM audit.access_log_entry WHERE id BETWEEN {from} AND {to}"
        )],
    )
    .await;
    tamper(
        other,
        "audit.access_log_seal",
        &[&format!(
            "DELETE FROM audit.access_log_seal WHERE last_id = {to}"
        )],
    )
    .await;
    tamper(
        other,
        "audit.access_log_seal_state",
        &[&format!(
            "UPDATE audit.access_log_seal_state SET last_id = {}, window_no = window_no - 1, \
             digest = decode('{}', 'hex')",
            kept.sequence, kept.digest
        )],
    )
    .await;
    let inside = log.verify(None).await.expect("verify");
    assert!(
        inside.chain.is_empty(),
        "nothing left in the database shows the removal: {inside:?}"
    );
    let outside = log.verify(Some(published)).await.expect("verify");
    assert!(
        outside
            .findings
            .contains(&ChainFinding::CheckpointDigest { sequence: to }),
        "the published checkpoint names a seal the log no longer holds: {outside:?}"
    );
}

/// Without an `OpenPGP` key the windows are sealed unsigned.
#[tokio::test]
async fn a_keyring_without_a_key_signs_no_seal() {
    let testdb = testkit::db().await.expect("testkit database");
    let log = checkpoints(&testdb.pool(), SYSTEM_A, Arc::new(Signer::digest_default()));
    assert!(!log.seals());
    append(&log, SYSTEM_A).await;
    log.store()
        .seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
        .await
        .expect("sealed unsigned");
    assert_eq!(sealed_positions(&testdb.pool()).await, Vec::<i64>::new());
    let unsigned: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_seal")
        .fetch_one(&testdb.pool())
        .await
        .expect("the seals");
    assert!(unsigned >= 1);
}

/// A keyring with no `OpenPGP` key gives no checkpoint.
#[tokio::test]
async fn a_keyring_without_a_key_seals_no_checkpoint() {
    let testdb = testkit::db().await.expect("testkit database");
    let log = checkpoints(&testdb.pool(), SYSTEM_A, Arc::new(Signer::digest_default()));
    append(&log, SYSTEM_A).await;
    let refused = log.seal().await.expect_err("no key, no checkpoint");
    assert!(
        matches!(refused, AccessLogError::Sign(AttestError::NoKey)),
        "{refused:?}"
    );
}

/// A fresh directory under the system's temporary directory.
fn publish_directory() -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("ferroehr-checkpoints-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&path).expect("the publication directory");
    path
}

/// The newest checkpoint is published once as its own file, which verifies
/// the log; a later window publishes the next one beside it, and a file of
/// the same name holding another checkpoint is a conflict, never rewritten
/// (#5704).
#[tokio::test]
async fn the_newest_checkpoint_is_published_once_outside_the_database() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let directory = publish_directory();
    let publication = Publication::new(Some(directory.clone()));
    assert_eq!(publication.state(), PublicationState::Pending);
    assert_eq!(publication.publish(&log).await.expect("an empty log"), None);

    let first = append(&log, SYSTEM_A).await;
    log.seal().await.expect("seal");
    let head = id_of(pool, first.last().expect("an entry")).await;
    publication.run(&log).await;
    assert!(
        matches!(publication.state(), PublicationState::Published { sequence: Some(s), .. } if s == head),
        "{:?}",
        publication.state()
    );
    let checkpoint = log.at(head).await.expect("read").expect("the checkpoint");
    let path = file_name(&directory, &checkpoint);
    let published: AccessLogCheckpoint =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the published file"))
            .expect("the published checkpoint reads");
    assert_eq!(published, checkpoint);
    assert!(log.verify(Some(published)).await.expect("verify").intact());

    let more = append_from(&log, SYSTEM_A, 5).await;
    log.seal().await.expect("seal");
    let next = id_of(pool, more.last().expect("an entry")).await;
    assert_eq!(
        publication.publish(&log).await.expect("publish"),
        Some(next)
    );
    let files = std::fs::read_dir(&directory)
        .expect("the directory")
        .count();
    assert_eq!(files, 2, "one file per published checkpoint");

    let newest = log.at(next).await.expect("read").expect("the checkpoint");
    let other = publish_directory();
    let mut forged = newest.clone();
    forged.time_sealed.value = "2000-01-01T00:00:00Z".to_owned();
    std::fs::write(
        file_name(&other, &newest),
        serde_json::to_string(&forged).expect("the forged checkpoint writes"),
    )
    .expect("a file at the same name");
    let refused = Publication::new(Some(other.clone()))
        .publish(&log)
        .await
        .expect_err("another checkpoint at the same name");
    assert!(
        matches!(refused, PublishError::Conflict { .. }),
        "{refused:?}"
    );
    std::fs::remove_dir_all(directory).expect("clean up");
    std::fs::remove_dir_all(other).expect("clean up");
}

/// A bundle verifies under the exporter's key, is refused by a system that
/// trusts another key, and is refused when its proof is replaced by the
/// proof of another bundle the same key signed (#6242).
#[tokio::test]
async fn a_bundle_under_a_foreign_key_or_a_replaced_proof_is_refused() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    append(&log, SYSTEM_A).await;
    let export = log.export(MOVED).await.expect("the export");
    assert_eq!(log.verify_bundle(&export).expect("verify"), Vec::new());

    let stranger = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    assert_eq!(
        stranger.verify_bundle(&export).expect("verify"),
        vec![ChainFinding::BundleProof],
        "a key the receiving system does not trust"
    );

    let other = log.export(MOVED).await.expect("a second export");
    assert_ne!(
        other.time_signed, export.time_signed,
        "each export is signed anew"
    );
    let mut replaced = export.clone();
    replaced.proof = other.proof;
    assert_eq!(
        log.verify_bundle(&replaced).expect("verify"),
        vec![ChainFinding::BundleProof],
        "a proof over another bundle"
    );
}
