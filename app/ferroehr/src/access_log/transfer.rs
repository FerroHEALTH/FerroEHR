// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The export of one EHR's access log and its import by the system the EHR
//! moves to (#5129).
//!
//! EHDS Annex II 2.6 forbids features that place an undue burden on the
//! export of an EHR for the replacement of the EHR system, and Art 9(2)
//! keeps the access information available for three years from each access,
//! whichever system then holds the EHR (`docs/law/eu/ehds/text.html`). The
//! export carries the EHR's records with their links, every link and gap
//! (#5619) from the first of them to the checkpoint they verify against, and
//! that checkpoint.
//! The importing system verifies them, keeps their `system_id`, links and
//! `retain_until` apart from its own chain, and records the import as an
//! entry of its own log. No openEHR spec governs the transfer: our own
//! design (`docs/plans/access-log.md` §Moving the log with the EHR).

use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row};

use crate::access_log::chain::{self, ChainFinding, Span};
use crate::access_log::checkpoint::Checkpoints;
use crate::access_log::entry;
use crate::access_log::gap::ChainGap;
use crate::access_log::store::{AccessLogError, canonical_form};
use crate::db::acquire::acquire;
use crate::system_log::event::{
    AccessDecision, AccessDomain, ActingMode, AuditEvent, EntryScope, EventActionCode,
    EventOutcome, ObjectClass,
};
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_rm::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::draft::access_log::access_log::access_retention::AccessRetention;
use openehr_rm::v1_2::common::generic::party_identified::{PartyIdentified, PartyIdentifiedData};
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;

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
    /// Every link from the first record's to the checkpoint's sequence but
    /// those of folded runs, in sequence.
    pub links: Vec<AccessLogLink>,
    /// Every gap between the links: a run of folded tombstones, which the
    /// link after it links to.
    pub gaps: Vec<ChainGap>,
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
            gaps: &self.gaps,
            checkpoint: &self.checkpoint,
        }
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
    /// Export the access log of the EHR `ehr_id`: its records, the links and
    /// gaps to the newest checkpoint that covers them, sealing one when none
    /// does, and that checkpoint, verified before it is returned.
    ///
    /// The access log holds only the accesses to EHR data, in a chain of its
    /// own, so the span carries the links of the other EHRs' records between
    /// and never a record outside the log.
    ///
    /// # Errors
    /// [`TransferError::NoRecords`] for an EHR the log holds nothing of,
    /// [`TransferError::Unverified`] when the span does not verify, and the
    /// store and signing errors.
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
        let gaps = store.gaps(first, checkpoint.sequence).await?;
        let export = LogExport {
            system_id: self.system_id().to_owned(),
            ehr_id: ehr_id.to_owned(),
            records,
            links,
            gaps,
            checkpoint,
        };
        self.verified(&export)?;
        Ok(export)
    }

    /// Refuse `export` unless its span verifies against its checkpoint under
    /// the keyring and the trusted certificates.
    fn verified(&self, export: &LogExport) -> Result<(), TransferError> {
        let findings = chain::verify(export.span(), &|data, signature| {
            self.signed(data, signature)
        })?;
        if findings.is_empty() {
            Ok(())
        } else {
            Err(TransferError::Unverified(findings))
        }
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
    /// imported, or a link or gap ends where a stored one ends with another
    /// digest, and the store and signing errors.
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
        self.verified(export)?;
        let mut entry = import_entry(self.system_id(), &export.ehr_id, record)?;
        let mut conn = acquire(self.pool()).await.map_err(AccessLogError::from)?;
        let mut tx = conn.begin().await.map_err(AccessLogError::from)?;
        store_links(&mut tx, system_id, &export.links).await?;
        store_gaps(&mut tx, system_id, &export.gaps).await?;
        consistent(&mut tx, system_id).await?;
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
        crate::access_log::intake::take_in(
            &mut tx,
            vec![crate::access_log::intake::IntakeEntry::of(
                &mut entry, None,
            )?],
        )
        .await?;
        tx.commit().await.map_err(AccessLogError::from)?;
        self.store()
            .settle(crate::access_log::chainer::Owed::default())
            .await?;
        let entry = self
            .store()
            .linked(vec![entry])
            .await?
            .pop()
            .ok_or_else(|| AccessLogError::Unlinked {
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

/// Store `gaps` of `system_id`'s chain, keeping any already stored.
async fn store_gaps(
    conn: &mut sqlx::PgConnection,
    system_id: &str,
    gaps: &[ChainGap],
) -> Result<(), TransferError> {
    if gaps.is_empty() {
        return Ok(());
    }
    let firsts: Vec<i64> = gaps.iter().map(|gap| gap.first_sequence).collect();
    let lasts: Vec<i64> = gaps.iter().map(|gap| gap.last_sequence).collect();
    let links: Vec<String> = gaps.iter().map(|gap| gap.link.clone()).collect();
    let signatures: Vec<Option<String>> = gaps.iter().map(|gap| gap.signature.clone()).collect();
    let grounds = gaps
        .iter()
        .map(|gap| serde_json::to_string(&gap.grounds))
        .collect::<Result<Vec<_>, _>>()
        .map_err(AccessLogError::Canonical)?;
    let folded: Vec<String> = gaps
        .iter()
        .map(|gap| gap.time_folded.value.clone())
        .collect();
    sqlx::query(
        "INSERT INTO audit.access_log_imported_gap (system_id, first_seq, last_seq, link, \
         signature, grounds, time_folded) \
         SELECT $1, t.first_seq, t.last_seq, t.link, t.signature, t.grounds::jsonb, \
         t.time_folded::timestamptz FROM unnest($2::bigint[], $3::bigint[], $4::text[], \
         $5::text[], $6::text[], $7::text[]) \
         AS t(first_seq, last_seq, link, signature, grounds, time_folded) \
         ON CONFLICT (system_id, first_seq, last_seq) DO NOTHING",
    )
    .bind(system_id)
    .bind(&firsts)
    .bind(&lasts)
    .bind(&links)
    .bind(&signatures)
    .bind(&grounds)
    .bind(&folded)
    .execute(&mut *conn)
    .await
    .map_err(AccessLogError::from)?;
    Ok(())
}

/// Refuse when a stored link or gap of `system_id`'s chain ends where
/// another ends with another digest: each position has one link digest,
/// whichever import brought it and whichever system folded it (#5664).
async fn consistent(conn: &mut sqlx::PgConnection, system_id: &str) -> Result<(), TransferError> {
    let differs: Option<i64> = sqlx::query_scalar(
        "SELECT g.last_seq FROM audit.access_log_imported_gap g \
         WHERE g.system_id = $1 AND ( \
           EXISTS (SELECT 1 FROM audit.access_log_imported_gap o \
                    WHERE o.system_id = g.system_id AND o.last_seq = g.last_seq \
                      AND o.link <> g.link) \
           OR EXISTS (SELECT 1 FROM audit.access_log_imported_link l \
                       WHERE l.system_id = g.system_id AND l.sequence = g.last_seq \
                         AND l.digest <> g.link)) \
         ORDER BY g.last_seq LIMIT 1",
    )
    .bind(system_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AccessLogError::from)?;
    if let Some(sequence) = differs {
        return Err(TransferError::Conflict {
            system_id: system_id.to_owned(),
            sequence,
        });
    }
    Ok(())
}

/// Store `records` of `system_id`'s chain as their canonical forms, keeping
/// any already stored. A tombstone or an expired entry at a position a
/// stored gap already holds is not stored again: its retention ended here,
/// and a repeated import does not bring its content back (#5664).
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
         WHERE NOT ((d.doc ->> '_type' = 'ACCESS_LOG_TOMBSTONE' \
                     OR (d.doc #>> '{retention,retain_until,value}')::date < current_date) \
                    AND EXISTS (SELECT 1 FROM audit.access_log_imported_gap g \
                                 WHERE g.system_id = $1 \
                                   AND t.sequence BETWEEN g.first_seq AND g.last_seq)) \
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

/// A party named `name` and identified no further, such as a system.
pub(crate) fn named_party(name: &str) -> PartyProxy {
    PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(PartyIdentifiedData {
        external_ref: None,
        name: Some(name.to_owned()),
        identifiers: None,
    }))
}

/// The entry of this system's log that records the import of `ehr_id`'s log:
/// the caller's access record, as an import of that EHR's extract, through
/// [`entry::entry_of`].
#[expect(
    clippy::expect_used,
    reason = "the event is an access to EHR data by construction (an EXTRACT under \
              `EntryScope::ByDomain`), which `entry::writes_entry` always admits"
)]
fn import_entry(
    system_id: &str,
    ehr_id: &str,
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
    event.object_id = None;
    Ok(entry::entry_of(&event, None, system_id, record.retention)
        .expect("an import of an EHR's extract should write an access-log entry"))
}
