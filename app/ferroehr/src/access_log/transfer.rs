// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The export of one EHR's access log and its import by the system the EHR
//! moves to (#5129, #6242).
//!
//! EHDS Annex II 2.6 forbids features that place an undue burden on the
//! export of an EHR for the replacement of the EHR system, and Art 9(2)
//! keeps the access information available for three years from each access,
//! whichever system then holds the EHR (`docs/law/eu/ehds/text.html`). The
//! export is a bundle: the EHR's records with each one's content digest,
//! signed whole by the exporting system's keyring over its RFC 8785 form
//! without its proof ([`crate::access_log::chain`]). It carries no record or
//! digest of another person. The importing system verifies the signature and
//! every entry's digest, keeps the records with their `system_id`,
//! `retain_until` and content digest, and records the import as an entry of
//! its own log that names the bundle's digest and its records digest, so its
//! seal covers them. No openEHR spec governs the transfer: our own design.

use serde::{Deserialize, Serialize};
use sqlx::Connection;

use crate::access_log::chain::{ChainFinding, records_digest};
use crate::access_log::checkpoint::Checkpoints;
use crate::access_log::entry;
use crate::access_log::store::{AccessLogError, canonical_form};
use crate::db::acquire::acquire;
use crate::digest::sha256_hex;
use crate::system_log::event::{
    AccessDecision, AccessDomain, ActingMode, AuditEvent, EntryScope, EventActionCode,
    EventOutcome, ObjectClass,
};
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::draft::access_log::access_log::access_retention::AccessRetention;
use openehr_rm::v1_2::common::generic::party_identified::{PartyIdentified, PartyIdentifiedData};
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;

/// One EHR's access log as one system exports it: a bundle signed whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogExport {
    /// The system whose log the records belong to.
    pub system_id: String,
    /// The EHR whose records these are.
    pub ehr_id: String,
    /// The EHR's records, entries and tombstones, in the order the log holds
    /// them.
    pub records: Vec<AccessLogRecord>,
    /// The content digest of each record, by position: the SHA-256 of an
    /// entry's canonical form, and a tombstone's entry's.
    pub content_digests: Vec<String>,
    /// When the exporting system signed the bundle, to the microsecond.
    pub time_signed: String,
    /// The signature over the bundle's canonical form without its proof.
    pub proof: String,
}

impl LogExport {
    /// The bundle's digest: the SHA-256 of its canonical form without its
    /// proof, the text its proof signs.
    ///
    /// # Errors
    /// [`AccessLogError::Canonical`] when the bundle has no canonical form.
    pub fn digest(&self) -> Result<String, AccessLogError> {
        Ok(sha256_hex(&canonical_form(self)?))
    }

    /// The digest over the records' uids and content digests.
    #[must_use]
    pub fn records_digest(&self) -> String {
        let pairs: Vec<(String, String)> = self
            .records
            .iter()
            .zip(&self.content_digests)
            .map(|(record, digest)| (uid(record).to_owned(), digest.clone()))
            .collect();
        records_digest(&pairs)
    }
}

/// A record's uid.
fn uid(record: &AccessLogRecord) -> &str {
    match record {
        AccessLogRecord::AccessLogEntry(entry) => entry.uid.value(),
        AccessLogRecord::AccessLogTombstone(tombstone) => tombstone.uid.value(),
    }
}

/// A record's system.
fn system(record: &AccessLogRecord) -> &str {
    match record {
        AccessLogRecord::AccessLogEntry(entry) => &entry.system_id,
        AccessLogRecord::AccessLogTombstone(tombstone) => &tombstone.system_id,
    }
}

/// Who imported a log, and how long the record of the import is kept: what
/// the entry of the import states beside the import itself.
///
/// The entry is built from `caller` by [`entry::entry_of`], the one path every
/// access-log entry takes, so an import is recorded as every other access is.
#[derive(Debug, Clone)]
pub struct ImportRecord {
    /// The access record of whoever asked for the import: its caller fields
    /// (`user_id`, `issuer`, `actor`, `binding`, `request_id`, `client_ip`).
    /// The import itself states the action, the class and the EHR.
    pub caller: AuditEvent,
    /// How long the import's entry is kept, and why.
    pub retention: AccessRetention,
}

impl ImportRecord {
    /// The record of an import the system's operator runs, kept until three
    /// years after `today`: the floor EHDS Art 9(2) sets from the date of
    /// access (`docs/law/eu/ehds/text.html`).
    ///
    /// The accessor is the system itself, since the operator acts at the
    /// console and presents no credential; its ground is Art. 9(2), coded by
    /// its European Legislation Identifier (our own design).
    #[must_use]
    pub fn by_operator(system_id: &str, today: jiff::civil::Date) -> Self {
        let retain_until = today.saturating_add(jiff::Span::new().years(3));
        let mut caller = AuditEvent::new(
            EventActionCode::Create,
            ObjectClass::Extract,
            EventOutcome::Success,
        );
        system_id.clone_into(&mut caller.user_id);
        caller.actor.mode = Some(ActingMode::System);
        Self {
            caller,
            retention: crate::access_log::retention::stated(
                retain_until.tomorrow().ok(),
                &crate::system_log::retention::LogRetention::default(),
            ),
        }
    }
}

/// What an import stored.
#[derive(Debug, Clone)]
pub struct Imported {
    /// The system the imported records belong to.
    pub system_id: String,
    /// How many records the export carried.
    pub records: usize,
    /// The entry of this system's log that records the import.
    pub entry: AccessLogEntry,
}

/// Why an export or an import was refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    /// The store failed or refused.
    #[error(transparent)]
    Log(#[from] AccessLogError),
    /// The EHR has no record in the log.
    #[error("the access log holds no record of the EHR {ehr_id}")]
    NoRecords {
        /// The EHR.
        ehr_id: String,
    },
    /// The bundle does not verify.
    #[error("the access-log bundle does not verify ({} findings)", .0.len())]
    Unverified(Vec<ChainFinding>),
    /// The export names this system, whose own log is never imported.
    #[error("the export's system {system_id:?} is not one whose log this system imports")]
    System {
        /// The system the export names.
        system_id: String,
    },
    /// An entry of the export belongs to another EHR.
    #[error("the entry {uid} belongs to another EHR than the export's")]
    OtherEhr {
        /// The entry's uid.
        uid: String,
    },
    /// A record of the export was imported before with another content
    /// digest.
    #[error("the record {uid} of {system_id:?} differs from the one already imported")]
    Conflict {
        /// The system the record belongs to.
        system_id: String,
        /// The record's uid.
        uid: String,
    },
}

impl Checkpoints {
    /// Export the access log of the EHR `ehr_id` as a bundle signed by this
    /// system's keyring.
    ///
    /// # Errors
    /// [`TransferError::NoRecords`] for an EHR the log holds nothing of,
    /// [`AccessLogError::Sign`] when the keyring cannot sign, and the store
    /// errors.
    pub async fn export(&self, ehr_id: &str) -> Result<LogExport, TransferError> {
        let held = self.store().of_ehr_digested(ehr_id).await?;
        if held.is_empty() {
            return Err(TransferError::NoRecords {
                ehr_id: ehr_id.to_owned(),
            });
        }
        let (records, content_digests): (Vec<_>, Vec<_>) = held.into_iter().unzip();
        let signed = jiff::Timestamp::from_microsecond(jiff::Timestamp::now().as_microsecond())
            .map_err(AccessLogError::Clock)?;
        let mut export = LogExport {
            system_id: self.system_id().to_owned(),
            ehr_id: ehr_id.to_owned(),
            records,
            content_digests,
            time_signed: signed.to_string(),
            proof: String::new(),
        };
        export.proof = self.sign(canonical_form(&export)?.as_bytes())?;
        Ok(export)
    }

    /// The findings of `export`: its proof under the keyring and the trusted
    /// certificates, every record its system's, one digest per record, and
    /// each entry's content digest recomputed.
    ///
    /// # Errors
    /// [`AccessLogError::Canonical`] when a record has no canonical form.
    pub fn verify_bundle(&self, export: &LogExport) -> Result<Vec<ChainFinding>, AccessLogError> {
        let mut findings = Vec::new();
        if !self.signed(canonical_form(export)?.as_bytes(), &export.proof) {
            findings.push(ChainFinding::BundleProof);
        }
        if export.records.len() != export.content_digests.len() {
            findings.push(ChainFinding::Digests {
                records: export.records.len(),
                digests: export.content_digests.len(),
            });
        }
        for (record, digest) in export.records.iter().zip(&export.content_digests) {
            if system(record) != export.system_id {
                findings.push(ChainFinding::OtherSystem {
                    uid: uid(record).to_owned(),
                    system_id: system(record).to_owned(),
                });
            }
            if let AccessLogRecord::AccessLogEntry(_) = record
                && sha256_hex(&canonical_form(record)?) != *digest
            {
                findings.push(ChainFinding::ContentDigest {
                    uid: uid(record).to_owned(),
                });
            }
        }
        Ok(findings)
    }

    /// Import the access log `export` of an EHR moved from another system,
    /// and record the import as an entry of this system's log naming the
    /// bundle's digest and its records digest, in one transaction.
    ///
    /// A record already imported with the same content digest is kept, so an
    /// import repeated is stored once and recorded again.
    ///
    /// # Errors
    /// [`TransferError::System`] for an export of this system,
    /// [`TransferError::OtherEhr`] for an entry of another EHR,
    /// [`TransferError::Unverified`] when the bundle does not verify,
    /// [`TransferError::Conflict`] when a record was imported before with
    /// another content digest, and the store errors.
    pub async fn import(
        &self,
        export: &LogExport,
        record: ImportRecord,
    ) -> Result<Imported, TransferError> {
        let system_id = &export.system_id;
        if *system_id == self.system_id() {
            return Err(TransferError::System {
                system_id: system_id.clone(),
            });
        }
        for held in &export.records {
            if let AccessLogRecord::AccessLogEntry(entry) = held
                && entry.ehr_id.as_ref().map(HierObjectId::value) != Some(export.ehr_id.as_str())
            {
                return Err(TransferError::OtherEhr {
                    uid: entry.uid.value().to_owned(),
                });
            }
        }
        let findings = self.verify_bundle(export)?;
        if !findings.is_empty() {
            return Err(TransferError::Unverified(findings));
        }
        let bundle = export.digest()?;
        let records = export.records_digest();
        let mut entry = import_entry(self.system_id(), &export.ehr_id, &bundle, &records, record)?;
        let mut conn = acquire(self.pool()).await.map_err(AccessLogError::from)?;
        let mut tx = conn.begin().await.map_err(AccessLogError::from)?;
        sqlx::query(
            "INSERT INTO audit.access_log_imported_bundle (digest, system_id, ehr_id, \
             records_digest, time_signed, proof, entry_uid) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::uuid) ON CONFLICT (digest) DO NOTHING",
        )
        .bind(&bundle)
        .bind(system_id)
        .bind(&export.ehr_id)
        .bind(&records)
        .bind(&export.time_signed)
        .bind(&export.proof)
        .bind(entry.uid.value())
        .execute(&mut *tx)
        .await
        .map_err(AccessLogError::from)?;
        store_records(&mut tx, system_id, &bundle, export).await?;
        crate::access_log::intake::take_in(
            &mut tx,
            &[crate::access_log::intake::IntakeEntry::of(
                &mut entry, None,
            )?],
            None,
            &std::collections::BTreeSet::new(),
        )
        .await?;
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this
        // transaction's rows.
        tx.commit().await.map_err(AccessLogError::from)?;
        Ok(Imported {
            system_id: system_id.clone(),
            records: export.records.len(),
            entry,
        })
    }
}

/// Store the records of `export` under `bundle`, keeping any already stored
/// with the same content digest, and refuse one stored with another.
async fn store_records(
    conn: &mut sqlx::PgConnection,
    system_id: &str,
    bundle: &str,
    export: &LogExport,
) -> Result<(), TransferError> {
    let uids: Vec<String> = export.records.iter().map(|r| uid(r).to_owned()).collect();
    let contents = export
        .records
        .iter()
        .map(canonical_form)
        .collect::<Result<Vec<_>, _>>()?;
    sqlx::query(
        "INSERT INTO audit.access_log_imported_record (system_id, uid, bundle, record_type, \
         ehr_id, retain_until, content, content_digest) \
         SELECT $1, t.uid, $2, d.doc ->> '_type', d.doc #>> '{ehr_id,value}', \
         (d.doc #>> '{retention,retain_until,value}')::date, t.content, t.digest \
         FROM unnest($3::text[], $4::text[], $5::text[]) AS t(uid, content, digest) \
         CROSS JOIN LATERAL (SELECT t.content::jsonb AS doc) AS d \
         WHERE NOT EXISTS (SELECT 1 FROM audit.access_log_imported_folded f \
                            WHERE f.system_id = $1 AND f.uid = t.uid) \
         ON CONFLICT (system_id, uid) DO NOTHING",
    )
    .bind(system_id)
    .bind(bundle)
    .bind(&uids)
    .bind(&contents)
    .bind(&export.content_digests)
    .execute(&mut *conn)
    .await
    .map_err(AccessLogError::from)?;
    let differs: Option<String> = sqlx::query_scalar(
        "SELECT r.uid FROM audit.access_log_imported_record r \
         JOIN unnest($2::text[], $3::text[]) AS t(uid, digest) ON t.uid = r.uid \
         WHERE r.system_id = $1 AND r.content_digest <> t.digest ORDER BY r.uid LIMIT 1",
    )
    .bind(system_id)
    .bind(&uids)
    .bind(&export.content_digests)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AccessLogError::from)?;
    if let Some(uid) = differs {
        return Err(TransferError::Conflict {
            system_id: system_id.to_owned(),
            uid,
        });
    }
    Ok(())
}

/// A party named `name` and identified no further, such as a system.
pub(crate) fn named_party(name: &str) -> PartyProxy {
    PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(PartyIdentifiedData {
        external_ref: None,
        name: Some(name.to_owned()),
        identifiers: None,
    }))
}

/// The entry of this system's log that records the import of `ehr_id`'s log:
/// the caller's access record, as an import of that EHR's extract naming the
/// bundle's digest and its records digest, through [`entry::entry_of`].
#[expect(
    clippy::expect_used,
    reason = "the event is an access to EHR data by construction (an EXTRACT under \
              `EntryScope::ByDomain`), which `entry::writes_entry` always admits"
)]
fn import_entry(
    system_id: &str,
    ehr_id: &str,
    bundle: &str,
    records: &str,
    record: ImportRecord,
) -> Result<AccessLogEntry, AccessLogError> {
    HierObjectId::new(ehr_id.to_owned()).map_err(|source| AccessLogError::Identifier {
        value: ehr_id.to_owned(),
        source,
    })?;
    let mut event = record.caller;
    event.action = EventActionCode::Create;
    event.object = ObjectClass::Extract;
    event.domain = AccessDomain::of(ObjectClass::Extract);
    event.entry = EntryScope::ByDomain;
    event.decision = AccessDecision::Admitted;
    event.outcome = EventOutcome::Success;
    event.ehr_id = Some(ehr_id.to_owned());
    // NOTE: our own design (#6242), the entry names the bundle and its records digest, so its
    // seal covers the imported records.
    event.object_id = Some(format!("access-log-bundle:{bundle}:{records}"));
    Ok(entry::entry_of(&event, None, system_id, record.retention)
        .expect("an import of an EHR's extract should write an access-log entry"))
}
