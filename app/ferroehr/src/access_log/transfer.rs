// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The export of one EHR's access log and its import by the system the EHR
//! moves to (#5129).
//!
//! EHDS Annex II 2.6 forbids features that place an undue burden on the
//! export of an EHR for the replacement of the EHR system, and Art 9(2)
//! keeps the access information available for three years from each access,
//! whichever system then holds the EHR (`docs/law/eu/ehds/text.html`). The
//! export carries the EHR's records with their links, every link from the
//! first of them to the checkpoint they verify against, and that checkpoint.
//! The importing system verifies them, keeps their `system_id`, links and
//! `retain_until` apart from its own chain, and records the import as an
//! entry of its own log. No openEHR spec governs the transfer: our own
//! design (`docs/plans/access-log.md` §Moving the log with the EHR).

#![expect(
    clippy::disallowed_types,
    reason = "the import entry is built from its ITS-JSON form (#1694 boundary class: canonical \
              form)"
)]

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Connection, Row};

use crate::access_log::chain::{self, ChainFinding, Span};
use crate::access_log::checkpoint::Checkpoints;
use crate::access_log::store::{AccessLogError, canonical_form};
use crate::access_log_draft::access_log::access_log_checkpoint::AccessLogCheckpoint;
use crate::access_log_draft::access_log::access_log_entry::AccessLogEntry;
use crate::access_log_draft::access_log::access_log_link::AccessLogLink;
use crate::access_log_draft::access_log::access_log_record::AccessLogRecord;
use crate::access_log_draft::access_log::access_retention::AccessRetention;
use crate::access_log_draft::access_log::accessor::Accessor;
use crate::db::acquire::acquire;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;

/// One EHR's access log as one system exports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogExport {
    /// The system whose log the records belong to.
    pub system_id: String,
    /// The EHR whose records these are.
    pub ehr_id: String,
    /// The EHR's records, entries and tombstones, in chain order, each with
    /// its link.
    pub records: Vec<AccessLogRecord>,
    /// Every link from the first record's to the checkpoint's, consecutive.
    pub links: Vec<AccessLogLink>,
    /// The checkpoint the records verify against.
    pub checkpoint: AccessLogCheckpoint,
}

impl LogExport {
    /// The span of the log this export holds.
    #[must_use]
    pub fn span(&self) -> Span<'_> {
        Span {
            records: &self.records,
            links: &self.links,
            checkpoint: &self.checkpoint,
        }
    }
}

/// Who imported a log, and how long the record of the import is kept: what
/// the entry of the import states beside the import itself.
#[derive(Debug, Clone)]
pub struct ImportRecord {
    /// Who asked for the import.
    pub accessor: Accessor,
    /// How long the import's entry is kept, and why.
    pub retention: AccessRetention,
    /// The request that carried the import, when one did.
    pub request_id: Option<String>,
}

impl ImportRecord {
    /// The record of an import the system's operator runs, kept until three
    /// years after `today`: the floor EHDS Art 9(2) sets from the date of
    /// access (`docs/law/eu/ehds/text.html`).
    ///
    /// The accessor is the system itself, since the operator acts at the
    /// console and presents no credential; its ground is a `local` code,
    /// since the retention codes are the retention register's (our own
    /// design).
    ///
    /// # Errors
    /// [`AccessLogError::Canonical`] when the date has no RM form, which a
    /// civil date always has.
    pub fn by_operator(system_id: &str, today: jiff::civil::Date) -> Result<Self, AccessLogError> {
        let retain_until = today.saturating_add(jiff::Span::new().years(3));
        let coded = |terminology: &str, value: &str| {
            json!({ "_type": "DV_CODED_TEXT", "value": value,
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                        "code_string": value } })
        };
        let accessor = json!({ "_type": "ACCESSOR",
            "kind": coded("openehr", "system"),
            "party": { "_type": "PARTY_IDENTIFIED", "name": system_id } });
        let retention = json!({ "_type": "ACCESS_RETENTION",
            "retain_until": { "_type": "DV_DATE", "value": retain_until.to_string() },
            "grounds": [coded("local", "ehds-art-9-2")] });
        Ok(Self {
            accessor: serde_json::from_value(accessor).map_err(AccessLogError::Canonical)?,
            retention: serde_json::from_value(retention).map_err(AccessLogError::Canonical)?,
            request_id: None,
        })
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
    /// The export does not verify against its checkpoint.
    #[error("the access log does not verify against its checkpoint ({} findings)", .0.len())]
    Unverified(Vec<ChainFinding>),
    /// The export names a system other than its checkpoint's, or this
    /// system, whose own chain is never imported.
    #[error("the export's system {system_id:?} is not one whose log this system imports")]
    System {
        /// The system the export names.
        system_id: String,
    },
    /// An entry of the export belongs to another EHR.
    #[error("the entry at {sequence} belongs to another EHR than the export's")]
    OtherEhr {
        /// The entry's sequence.
        sequence: i64,
    },
    /// A link of the export differs from the link an earlier import stored at
    /// its position.
    #[error("the link at {sequence} of {system_id:?} differs from the one already imported")]
    Conflict {
        /// The system the link belongs to.
        system_id: String,
        /// The link's sequence.
        sequence: i64,
    },
}

/// The sequence of a record's link.
fn sequence(record: &AccessLogRecord) -> i64 {
    match record {
        AccessLogRecord::AccessLogEntry(entry) => entry.link.sequence,
        AccessLogRecord::AccessLogTombstone(tombstone) => tombstone.link.sequence,
    }
}

impl Checkpoints {
    /// Export the access log of the EHR `ehr_id`: its records, the links to
    /// the newest checkpoint that covers them, sealing one when none does,
    /// and that checkpoint, verified before it is returned.
    ///
    /// # Errors
    /// [`TransferError::NoRecords`] for an EHR the log holds nothing of,
    /// [`AccessLogError::Interleaved`] when the span holds `audit_event`
    /// records, [`TransferError::Unverified`] when the span does not verify,
    /// and the store and signing errors.
    pub async fn export(&self, ehr_id: &str) -> Result<LogExport, TransferError> {
        let store = self.store();
        let records = store.of_ehr(ehr_id).await?;
        let no_records = || TransferError::NoRecords {
            ehr_id: ehr_id.to_owned(),
        };
        let first = records.first().map(sequence).ok_or_else(no_records)?;
        let last = records.last().map(sequence).ok_or_else(no_records)?;
        let checkpoint = match self.latest(Some(last)).await? {
            Some(checkpoint) => checkpoint,
            None => self.seal().await?.ok_or_else(no_records)?,
        };
        let links = store.links(first, checkpoint.sequence).await?;
        let export = LogExport {
            system_id: self.system_id().to_owned(),
            ehr_id: ehr_id.to_owned(),
            records,
            links,
            checkpoint,
        };
        let findings = chain::verify(export.span(), &|data, signature| {
            self.signed(data, signature)
        })?;
        if !findings.is_empty() {
            return Err(TransferError::Unverified(findings));
        }
        Ok(export)
    }

    /// Import the access log `export` of an EHR moved from another system,
    /// and record the import as an entry of this system's log, in one
    /// transaction.
    ///
    /// The export is verified first: its span against its checkpoint, and
    /// the checkpoint's proof against the keyring and the trusted
    /// certificates. A record or link already imported is kept, so an import
    /// repeated is stored once and recorded again.
    ///
    /// # Errors
    /// [`TransferError::System`] for an export of this system or one whose
    /// checkpoint names another, [`TransferError::OtherEhr`] for an entry of
    /// another EHR, [`TransferError::Unverified`] when it does not verify,
    /// [`TransferError::Conflict`] when a link differs from one already
    /// imported, and the store and signing errors.
    pub async fn import(
        &self,
        export: &LogExport,
        record: ImportRecord,
    ) -> Result<Imported, TransferError> {
        let system_id = &export.system_id;
        if *system_id == self.system_id() || *system_id != export.checkpoint.system_id {
            return Err(TransferError::System {
                system_id: system_id.clone(),
            });
        }
        for held in &export.records {
            if let AccessLogRecord::AccessLogEntry(entry) = held
                && entry.ehr_id.as_ref().map(HierObjectId::value) != Some(export.ehr_id.as_str())
            {
                return Err(TransferError::OtherEhr {
                    sequence: entry.link.sequence,
                });
            }
        }
        let findings = chain::verify(export.span(), &|data, signature| {
            self.signed(data, signature)
        })?;
        if !findings.is_empty() {
            return Err(TransferError::Unverified(findings));
        }
        let entry = import_entry(self.system_id(), &export.ehr_id, record)?;
        let mut conn = acquire(self.pool()).await.map_err(AccessLogError::from)?;
        let mut tx = conn.begin().await.map_err(AccessLogError::from)?;
        store_links(&mut tx, system_id, &export.links).await?;
        store_records(&mut tx, system_id, &export.records).await?;
        sqlx::query(
            "INSERT INTO audit.access_log_imported_checkpoint (system_id, sequence, digest, \
             content) VALUES ($1, $2, $3, $4) ON CONFLICT (system_id, sequence) DO NOTHING",
        )
        .bind(system_id)
        .bind(export.checkpoint.sequence)
        .bind(&export.checkpoint.digest)
        .bind(serde_json::to_string(&export.checkpoint).map_err(AccessLogError::Canonical)?)
        .execute(&mut *tx)
        .await
        .map_err(AccessLogError::from)?;
        let mut appended = self.store().append_in(&mut tx, vec![entry]).await?;
        tx.commit().await.map_err(AccessLogError::from)?;
        let entry = appended.pop().ok_or_else(|| AccessLogError::Unlinked {
            uid: export.ehr_id.clone(),
        })?;
        Ok(Imported {
            system_id: system_id.clone(),
            records: export.records.len(),
            entry,
        })
    }
}

/// Store `links` of `system_id`'s chain, keeping any already stored, and
/// refuse when one already stored differs.
async fn store_links(
    conn: &mut sqlx::PgConnection,
    system_id: &str,
    links: &[AccessLogLink],
) -> Result<(), TransferError> {
    let column = |field: fn(&AccessLogLink) -> &str| -> Vec<String> {
        links.iter().map(|link| field(link).to_owned()).collect()
    };
    let sequences: Vec<i64> = links.iter().map(|link| link.sequence).collect();
    let signatures: Vec<Option<String>> = links.iter().map(|link| link.signature.clone()).collect();
    sqlx::query(
        "INSERT INTO audit.access_log_imported_link (system_id, sequence, algorithm, \
         previous_digest, content_digest, digest, signature) \
         SELECT $1, t.sequence, t.algorithm, t.previous_digest, t.content_digest, t.digest, \
         t.signature FROM unnest($2::bigint[], $3::text[], $4::text[], $5::text[], $6::text[], \
         $7::text[]) AS t(sequence, algorithm, previous_digest, content_digest, digest, signature) \
         ON CONFLICT (system_id, sequence) DO NOTHING",
    )
    .bind(system_id)
    .bind(&sequences)
    .bind(column(|link| &link.algorithm))
    .bind(column(|link| &link.previous_digest))
    .bind(column(|link| &link.content_digest))
    .bind(column(|link| &link.digest))
    .bind(&signatures)
    .execute(&mut *conn)
    .await
    .map_err(AccessLogError::from)?;
    let rows = sqlx::query(
        "SELECT l.sequence FROM audit.access_log_imported_link l \
         JOIN unnest($2::bigint[], $3::text[]) AS t(sequence, digest) ON t.sequence = l.sequence \
         WHERE l.system_id = $1 AND l.digest <> t.digest ORDER BY l.sequence LIMIT 1",
    )
    .bind(system_id)
    .bind(&sequences)
    .bind(column(|link| &link.digest))
    .fetch_optional(&mut *conn)
    .await
    .map_err(AccessLogError::from)?;
    if let Some(row) = rows {
        return Err(TransferError::Conflict {
            system_id: system_id.to_owned(),
            sequence: row.try_get("sequence").map_err(AccessLogError::from)?,
        });
    }
    Ok(())
}

/// Store `records` of `system_id`'s chain as their canonical forms, keeping
/// any already stored.
async fn store_records(
    conn: &mut sqlx::PgConnection,
    system_id: &str,
    records: &[AccessLogRecord],
) -> Result<(), TransferError> {
    let sequences: Vec<i64> = records.iter().map(sequence).collect();
    let contents = records
        .iter()
        .map(canonical_form)
        .collect::<Result<Vec<_>, _>>()?;
    sqlx::query(
        "INSERT INTO audit.access_log_imported_record (system_id, sequence, uid, record_type, \
         ehr_id, retain_until, content) \
         SELECT $1, t.sequence, d.doc #>> '{uid,value}', d.doc ->> '_type', \
         d.doc #>> '{ehr_id,value}', (d.doc #>> '{retention,retain_until,value}')::date, \
         t.content FROM unnest($2::bigint[], $3::text[]) AS t(sequence, content) \
         CROSS JOIN LATERAL (SELECT t.content::jsonb AS doc) AS d \
         ON CONFLICT (system_id, sequence) DO NOTHING",
    )
    .bind(system_id)
    .bind(&sequences)
    .bind(&contents)
    .execute(&mut *conn)
    .await
    .map_err(AccessLogError::from)?;
    Ok(())
}

/// The entry of this system's log that records the import of `ehr_id`'s log.
fn import_entry(
    system_id: &str,
    ehr_id: &str,
    record: ImportRecord,
) -> Result<AccessLogEntry, AccessLogError> {
    let coded = |value: &str, terminology: &str| {
        json!({ "_type": "DV_CODED_TEXT", "value": value,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                    "code_string": value } })
    };
    let mut entry = json!({
        "_type": "ACCESS_LOG_ENTRY",
        "uid": { "_type": "HIER_OBJECT_ID", "value": uuid::Uuid::now_v7().to_string() },
        "system_id": system_id,
        "time_recorded": { "_type": "DV_DATE_TIME", "value": jiff::Timestamp::now().to_string() },
        "action": coded("import", "openehr"),
        "outcome": { "_type": "ACCESS_OUTCOME", "result": coded("granted", "openehr") },
        "accessor": serde_json::to_value(&record.accessor).map_err(AccessLogError::Canonical)?,
        "ehr_id": { "_type": "HIER_OBJECT_ID", "value": ehr_id },
        "is_emergency": false,
        "retention": serde_json::to_value(&record.retention).map_err(AccessLogError::Canonical)?,
        "link": { "_type": "ACCESS_LOG_LINK", "sequence": 1, "algorithm": chain::ALGORITHM,
            "previous_digest": "", "content_digest": "", "digest": "" }
    });
    if let Some(request_id) = record.request_id
        && let Some(members) = entry.as_object_mut()
    {
        members.insert(
            "channel".to_owned(),
            json!({ "_type": "ACCESS_CHANNEL", "request_id": request_id }),
        );
    }
    serde_json::from_value(entry).map_err(AccessLogError::Canonical)
}
