// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The signed checkpoints of the access log, the verification of the chain
//! against them, the signature of each link, and the export and import of one
//! EHR's log between two FerroEHR databases (#5129), against real
//! `PostgreSQL` 18 databases.
//!
//! No openEHR spec governs log signing or the transfer: our own design.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use openehr_base::containers::NonEmptyVec;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use pgp::composed::{ArmorOptions, KeyType, SecretKeyParamsBuilder, SignedSecretKey};
use rand::rngs::OsRng;
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, Connection, PgPool};

use ferroehr::access_log::chain::{self, ChainFinding};
use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log::store::AccessLogError;
use ferroehr::access_log::transfer::{ImportRecord, LogExport, TransferError};
use ferroehr::versioning::signature::config::{Mode, SigningConfig};
use ferroehr::versioning::signature::signer::{AttestError, Signer};
use ferroehr::versioning::signature::trust::TrustedCertificates;
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
fn generate_key() -> SignedSecretKey {
    let params = SecretKeyParamsBuilder::default()
        .key_type(KeyType::Ed25519Legacy)
        .can_sign(true)
        .primary_user_id("access-log test <test@ferroehr.local>".into())
        .build()
        .expect("build key params");
    params.generate(OsRng).expect("generate key")
}

/// A `pgp`-mode keyring over `key`.
fn keyring(key: &SignedSecretKey) -> Arc<Signer> {
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
fn trusting(key: &SignedSecretKey) -> Arc<TrustedCertificates> {
    let armored = key
        .to_public_key()
        .to_armored_string(ArmorOptions::default())
        .expect("armor certificate");
    Arc::new(TrustedCertificates::from_armored([armored.as_str()]).expect("the certificate reads"))
}

/// The checkpoints of `pool` for `system_id` under `signer`.
fn checkpoints(pool: &PgPool, system_id: &str, signer: Arc<Signer>) -> Checkpoints {
    Checkpoints::new(
        pool.clone(),
        system_id.to_owned(),
        signer,
        Arc::new(TrustedCertificates::default()),
    )
}

/// The proposal's example entry, number `n`, of `system_id` about `ehr_id`.
fn entry(n: u32, system_id: &str, ehr_id: &str) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["system_id"] = json!(system_id);
    value["ehr_id"]["value"] = json!(ehr_id);
    serde_json::from_value(value).expect("the example entry reads")
}

/// Append four entries of `system_id`, alternating between the moved EHR and
/// the one that stays.
async fn append(checkpoints: &Checkpoints, system_id: &str) -> Vec<AccessLogEntry> {
    let entries = (1..=4)
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
async fn tamper(pool: &PgPool, table: &str, statements: &[&str]) {
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

/// The record of the import, by an operator, kept three years.
fn import_record() -> ImportRecord {
    let coded = |terminology: &str, value: &str| {
        json!({ "_type": "DV_CODED_TEXT", "value": value,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                    "code_string": value } })
    };
    ImportRecord {
        accessor: serde_json::from_value(json!({ "_type": "ACCESSOR",
            "kind": coded("openehr", "person"),
            "party": { "_type": "PARTY_IDENTIFIED", "name": "transfer operator" } }))
        .expect("the accessor reads"),
        retention: serde_json::from_value(json!({ "_type": "ACCESS_RETENTION",
            "retain_until": { "_type": "DV_DATE", "value": "2029-11-13" },
            "grounds": [coded("local", "ehds-art-9-2")] }))
        .expect("the retention reads"),
        request_id: Some("01JC2Q7N3X".to_owned()),
    }
}

/// A checkpoint seals the head of the chain, signed with the keyring's key,
/// and the log verifies against it; sealing an unchanged head writes nothing
/// new.
#[tokio::test]
async fn a_signed_checkpoint_verifies() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let entries = append(&log, SYSTEM_A).await;

    let checkpoint = log.seal().await.expect("seal").expect("a checkpoint");
    let head = entries.last().expect("an entry");
    assert_eq!(checkpoint.sequence, head.link.sequence);
    assert_eq!(checkpoint.digest, head.link.digest);
    assert_eq!(checkpoint.system_id, SYSTEM_A);
    assert!(checkpoint.proof.contains("BEGIN PGP SIGNATURE"));
    assert_eq!(
        log.seal().await.expect("seal again"),
        Some(checkpoint.clone()),
        "an unchanged head keeps its checkpoint"
    );

    let verification = log.verify(None).await.expect("verify");
    assert!(verification.intact(), "{verification:?}");
    assert_eq!(verification.checkpoint, Some(checkpoint));
}

/// An entry whose content changed after it was written fails the chain, a
/// checkpoint whose proof was replaced fails its signature, and a checkpoint
/// another key signed is not trusted.
#[tokio::test]
async fn a_tampered_entry_fails_verification() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    let entries = append(&log, SYSTEM_A).await;
    let checkpoint = log.seal().await.expect("seal").expect("a checkpoint");
    let target = entries.get(1).expect("a second entry").link.sequence;

    tamper(
        pool,
        "audit.access_log_record",
        &[&format!(
            "UPDATE audit.access_log_record SET content = replace(content, \
             '\"is_emergency\":false', '\"is_emergency\":true') WHERE chain_seq = {target}"
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

/// An exported entry whose content changed fails the span's verification at
/// its position, and a tombstone in the span verifies as recorded.
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
    log.store()
        .tombstone(
            first.uid.value(),
            first.time_recorded.clone(),
            NonEmptyVec::new(vec![ground]).expect("one ground"),
            None,
        )
        .await
        .expect("tombstone")
        .expect("the entry is replaced");

    // NOTE: the tombstone no longer names its EHR, so the moved EHR's export starts at its second entry.
    let export = log.export(MOVED).await.expect("the export verifies");
    assert_eq!(export.records.len(), 1, "{export:?}");
    let trusted = |data: &[u8], signature: &str| log.signed(data, signature);
    let whole = LogExport {
        records: vec![
            log.store()
                .read(first.uid.value())
                .await
                .expect("read")
                .expect("the tombstone"),
        ],
        links: log
            .store()
            .links(first.link.sequence, export.checkpoint.sequence)
            .await
            .expect("links"),
        ..export.clone()
    };
    assert!(matches!(
        whole.records.first(),
        Some(AccessLogRecord::AccessLogTombstone(_))
    ));
    assert_eq!(
        chain::verify(whole.span(), &trusted).expect("verify"),
        Vec::new()
    );

    let mut tampered: Value = serde_json::to_value(&export).expect("the export writes");
    tampered["records"][0]["is_emergency"] = json!(true);
    let tampered: LogExport = serde_json::from_value(tampered).expect("the export reads");
    let sequence = entries.get(2).expect("a third entry").link.sequence;
    assert_eq!(
        chain::verify(tampered.span(), &trusted).expect("verify"),
        vec![ChainFinding::ContentDigest { sequence }]
    );
}

/// One EHR's log leaves one database as JSON and enters another: the records
/// keep their system, links and `retain_until`, the import verifies against
/// the trusted certificate and is itself an entry of the importing log, and
/// an import from a system the deployment does not trust is refused.
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
        .expect_err("a checkpoint no trusted key signed is refused");
    assert!(
        matches!(&refused, TransferError::Unverified(findings)
        if findings.contains(&ChainFinding::CheckpointProof {
            sequence: export.checkpoint.sequence
        })),
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

    let stored: Vec<(String, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT system_id, sequence, ehr_id, retain_until::text \
         FROM audit.access_log_imported_record ORDER BY sequence",
    )
    .fetch_all(&target_db.pool())
    .await
    .expect("read the imported records");
    let expected: Vec<_> = export
        .records
        .iter()
        .map(|record| match record {
            AccessLogRecord::AccessLogEntry(entry) => (
                SYSTEM_A.to_owned(),
                entry.link.sequence,
                Some(MOVED.to_owned()),
                Some("2031-11-07".to_owned()),
            ),
            AccessLogRecord::AccessLogTombstone(_) => panic!("no tombstone was exported"),
        })
        .collect();
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
}

/// A local link is never signed on its own, even under an `OpenPGP` key: the
/// record keeps no signature and the checkpoint seals the chain (#5562).
#[tokio::test]
async fn a_local_link_carries_no_signature_and_the_checkpoint_seals_it() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = &testdb.pool();
    let log = checkpoints(pool, SYSTEM_A, keyring(&generate_key()));
    assert!(log.seals());
    let entries = append(&log, SYSTEM_A).await;
    assert!(entries.iter().all(|entry| entry.link.signature.is_none()));
    let first = entries.first().expect("an entry");
    let read = log
        .store()
        .read(first.uid.value())
        .await
        .expect("read")
        .expect("the entry");
    assert!(
        matches!(&read, AccessLogRecord::AccessLogEntry(entry) if entry.link.signature.is_none())
    );
    let column: Option<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = 'audit' AND table_name = 'access_log_record' \
         AND column_name = 'signature'",
    )
    .fetch_optional(pool)
    .await
    .expect("read the catalogue");
    assert_eq!(column, None, "a local record keeps no link signature");
    let checkpoint = log.seal().await.expect("seal").expect("a checkpoint");
    assert_eq!(
        checkpoint.sequence,
        entries.last().expect("an entry").link.sequence
    );
    let verification = log.verify(None).await.expect("verify");
    assert!(verification.intact(), "{verification:?}");
}

/// A link another system signed, as the draft's optional
/// `ACCESS_LOG_LINK.signature` lets it, verifies under a trusted key, and a
/// signature over another link is a finding.
#[tokio::test]
async fn a_foreign_signed_link_verifies_and_a_replaced_signature_does_not() {
    let testdb = testkit::db().await.expect("testkit database");
    let key_a = generate_key();
    let signer = keyring(&key_a);
    let source = checkpoints(&testdb.pool(), SYSTEM_A, Arc::clone(&signer));
    append(&source, SYSTEM_A).await;
    let mut export = source.export(MOVED).await.expect("export");
    for link in &mut export.links {
        link.signature = Some(
            signer
                .attest(link.digest.as_bytes())
                .expect("sign the link"),
        );
    }
    let trusted = |data: &[u8], signature: &str| signer.attests(data, signature);
    assert_eq!(
        chain::verify(export.span(), &trusted).expect("verify"),
        Vec::new()
    );

    let first = export.links.first().expect("a link").sequence;
    let foreign = export.links.get(1).and_then(|link| link.signature.clone());
    let replaced = export.links.first_mut().expect("a link");
    replaced.signature = foreign;
    assert_eq!(
        chain::verify(export.span(), &trusted).expect("verify"),
        vec![ChainFinding::LinkSignature { sequence: first }]
    );
}

/// A keyring with no `OpenPGP` key seals nothing.
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
