// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-log store: each record kept as its canonical form, written
//! once and read from that row, and an entry replaced in place by its
//! tombstone (#5125, #6242).
//!
//! The canonical form of a record is its ITS-JSON serialisation canonicalised
//! by RFC 8785, rendered from its stored parts by the renderer of its stored
//! form, and `content_digest` the SHA-256 of those stored facts
//! ([`stored_form::parts::facts::facts_digest`], #6322). An entry is
//! written by its request ([`crate::access_log::intake`]) and sealed in a
//! window ([`crate::access_log::seal`]). The retention rules let an entry be
//! replaced by a tombstone that keeps its uid, `system_id`, `time_recorded`
//! and content digest, the one change the table's triggers admit
//! (`migrations/audit/0004_baseline_access_log.sql`).

use std::sync::Arc;

use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::canonical_object_without;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use sqlx::postgres::PgRow;
use sqlx::{Connection, PgConnection, PgPool, Row};

use crate::access_log::checkpoint::Sealer;
use crate::access_log::intake::{IntakeEntry, take_in};
use crate::access_log::stored_form;
use crate::db::acquire::{AcquireError, acquire};
use crate::versioning::signature::signer::{AttestError, Signer};
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::draft::access_log::access_log::access_log_tombstone::AccessLogTombstone;
use openehr_rm::draft::access_log::invariants::check_tombstone;
use openehr_rm::draft::invariants::Violations;

/// The member of a checkpoint's ITS-JSON form its canonical form leaves out:
/// the proof is Void while the signature is computed (RM common
/// `master04-generic_package.adoc` §Attestation).
const PROOF: &str = "proof";

/// Why the access-log store refused or failed an operation.
#[derive(Debug, thiserror::Error)]
pub enum AccessLogError {
    /// A statement on the store failed.
    #[error("the access-log store failed")]
    Store(#[from] sqlx::Error),
    /// No connection to the store could be obtained.
    #[error("the access-log store is unavailable")]
    Acquire(#[from] AcquireError),
    /// The intake write this entry was gathered into failed, or its writer
    /// stopped before answering.
    #[error("the gathered intake write failed")]
    Gathered(#[source] Option<Arc<AccessLogError>>),
    /// A record has no canonical form: a number in it has no finite IEEE 754
    /// double form (RFC 8785 §3.2.2.3).
    #[error("the access-log record has no canonical form")]
    Canonical(#[source] serde_json::Error),
    /// The record at `chain_seq` does not read back as an access-log record.
    #[error("the access-log record at chain position {chain_seq} does not read")]
    Unreadable {
        /// The record's position in the chain.
        chain_seq: i64,
        /// Why it does not read.
        #[source]
        source: serde_json::Error,
    },
    /// The record breaks an invariant of the draft model, so it is not
    /// written.
    #[error(transparent)]
    Invalid(#[from] Violations),
    /// An identifier the log writes is not a valid `HIER_OBJECT_ID`.
    #[error("{value:?} is not a valid HIER_OBJECT_ID")]
    Identifier {
        /// The text.
        value: String,
        /// Why it is not one.
        #[source]
        source: openehr_base::v1_3::base_types::identification::lexical::IdError,
    },
    /// The audit chain's verifier failed.
    #[error("the audit chain could not be verified")]
    Chain(#[source] Box<crate::system_log::AuditError>),
    /// A checkpoint was to be signed and the keyring could not sign it.
    #[error("the access-log checkpoint could not be signed")]
    Sign(#[from] AttestError),
    /// The task that signs the checkpoint ended without an answer.
    #[error("the task signing the access-log checkpoint did not finish")]
    Signing(#[source] tokio::task::JoinError),
    /// The clock gave a time a checkpoint cannot name to the microsecond.
    #[error("the time of the access-log checkpoint is out of range")]
    Clock(#[source] jiff::Error),
    /// A window holds more entries than a seal can count.
    #[error("the access-log window holds more entries than a seal counts")]
    Exhausted,
}

/// The canonical form of an access-log record or checkpoint: its ITS-JSON
/// serialisation without its proof, canonicalised by RFC 8785.
///
/// A checkpoint's proof is left out since it signs this form; no record has
/// a proof.
///
/// # Errors
/// [`AccessLogError::Canonical`] when the record does not serialise or holds
/// a number with no finite double form.
pub fn canonical_form(record: &impl serde::Serialize) -> Result<String, AccessLogError> {
    canonical_object_without(record, &[PROOF]).map_err(AccessLogError::Canonical)
}

/// The columns a stored access-log record is read back from, over the record
/// `r` and its shared part `sh`: the row and shared parts and the columns
/// that hold the members the row part leaves out, which
/// [`document_of`] joins into the record's canonical form
/// (`migrations/audit/0004_baseline_access_log.sql`).
macro_rules! entry_parts {
    () => {
        "r.content AS row_part, sh.content AS shared_part, r.record_type AS entry_type, \
         r.uid AS entry_uid, r.time_recorded AS recorded_at, r.ehr_id AS entry_ehr, \
         r.request_id, r.resource_id, r.form AS stored_form"
    };
}
pub(crate) use entry_parts;

/// The records of this system's log, each as its stored parts, in the
/// columns [`stored`] reads.
macro_rules! records {
    () => {
        concat!(
            "SELECT r.uid, r.chain_seq, ",
            crate::access_log::store::entry_parts!(),
            ", encode(r.content_digest, 'hex') AS content_digest \
             FROM audit.access_log_record r \
             LEFT JOIN audit.access_log_shared sh ON sh.digest = r.shared"
        )
    };
}
pub(crate) use records;

/// The access-log records in the `audit` schema.
#[derive(Debug, Clone)]
pub struct AccessLogStore {
    pool: PgPool,
    /// The keyring and system that sign the seals, whenever the keyring
    /// holds an `OpenPGP` key (#5704, #6242).
    sealer: Option<Sealer>,
}

impl AccessLogStore {
    /// A store over the audit pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool, sealer: None }
    }

    /// This store over `pool`, sealing as `self` does.
    #[must_use]
    pub fn with_pool(&self, pool: PgPool) -> Self {
        Self {
            pool,
            sealer: self.sealer.clone(),
        }
    }

    /// The audit pool the records are kept in.
    pub(super) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The keyring and system that sign the seals, when the keyring holds a key.
    pub(super) fn sealer(&self) -> Option<&Sealer> {
        self.sealer.as_ref()
    }

    /// The store that signs each window's seal as the `ACCESS_LOG_CHECKPOINT`
    /// of `system_id` up to the window's last entry, with `signer`'s key
    /// (#5704, #6242).
    #[must_use]
    pub fn with_sealer(mut self, signer: Arc<Signer>, system_id: String) -> Self {
        self.sealer = Some(Sealer::new(signer, system_id));
        self
    }

    /// Append `entries` in their order, written in one transaction before
    /// this returns; each is checked against the draft's invariants.
    ///
    /// # Errors
    /// [`AccessLogError::Canonical`] for an entry with no canonical form,
    /// [`AccessLogError::Invalid`] for one that breaks an invariant, and the
    /// store errors when a statement fails (a repeated uid among them).
    pub async fn append(
        &self,
        entries: Vec<AccessLogEntry>,
    ) -> Result<Vec<AccessLogEntry>, AccessLogError> {
        if entries.is_empty() {
            return Ok(entries);
        }
        let mut entries = entries;
        let intake = entries
            .iter_mut()
            .map(|entry| IntakeEntry::of(entry, None))
            .collect::<Result<Vec<_>, _>>()?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        take_in(&mut tx, &intake, None, &std::collections::BTreeSet::new()).await?;
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this
        // transaction's rows.
        tx.commit().await?;
        Ok(entries)
    }

    /// The records whose `ehr_id` is `ehr_id`, in id order.
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`AccessLogError::Unreadable`] when a stored record does not read.
    pub async fn of_ehr(&self, ehr_id: &str) -> Result<Vec<AccessLogRecord>, AccessLogError> {
        let records = self.of_ehr_digested(ehr_id).await?;
        Ok(records
            .into_iter()
            .map(|(record, _digest)| record)
            .collect())
    }

    /// The records whose `ehr_id` is `ehr_id`, in id order, each with the
    /// SHA-256 of an entry's canonical form, or the content digest a
    /// tombstone keeps from its entry.
    ///
    /// # Errors
    /// As [`Self::of_ehr`].
    pub async fn of_ehr_digested(
        &self,
        ehr_id: &str,
    ) -> Result<Vec<(AccessLogRecord, String)>, AccessLogError> {
        let mut conn = acquire(&self.pool).await?;
        let rows = match stored_form::column_uuid(ehr_id) {
            Some(column) => {
                sqlx::query(concat!(
                    records!(),
                    " WHERE r.ehr_id = $1 ORDER BY r.chain_seq"
                ))
                .bind(column)
                .fetch_all(&mut *conn)
                .await?
            }
            None => {
                sqlx::query(concat!(
                    records!(),
                    " WHERE r.ehr_id IS NULL AND r.content::jsonb #>> '{ehr_id,value}' = $1 \
                     ORDER BY r.chain_seq"
                ))
                .bind(ehr_id)
                .fetch_all(&mut *conn)
                .await?
            }
        };
        rows.iter()
            .map(|row| {
                let document = document_of(row, row.try_get("shared_part")?)?;
                let record: AccessLogRecord = read_document(row, &document)?;
                // NOTE: our own design (#6322), the bundle names an entry by its canonical form's
                // digest, which the importing system recomputes; a tombstone keeps its entry's.
                let digest = match record {
                    AccessLogRecord::AccessLogEntry(_) => crate::digest::sha256_hex(&document),
                    AccessLogRecord::AccessLogTombstone(_) => row.try_get("content_digest")?,
                };
                Ok((record, digest))
            })
            .collect()
    }

    /// The record whose uid is `uid`, entry or tombstone.
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`AccessLogError::Unreadable`] when the stored record does not read.
    pub async fn read(&self, uid: &str) -> Result<Option<AccessLogRecord>, AccessLogError> {
        // NOTE: our own design (#5714); a uid that is no uuid names no record of this log.
        let Ok(uid) = uuid::Uuid::parse_str(uid) else {
            return Ok(None);
        };
        let row = sqlx::query(concat!(records!(), " WHERE r.uid = $1"))
            .bind(uid)
            .fetch_optional(&mut *acquire(&self.pool).await?)
            .await?;
        row.as_ref().map(stored).transpose()
    }

    /// Replace the entry whose uid is `uid` by its tombstone, removed at
    /// `time_removed` on `grounds`, and return the tombstone.
    ///
    /// The tombstone keeps the entry's uid, `system_id` and `time_recorded`,
    /// and the row keeps its content digest, so its window verifies after the
    /// entry's content is gone. `None`
    /// when no entry has the uid, which is also the answer for one already
    /// replaced: a tombstone is never replaced again.
    ///
    /// # Errors
    /// [`AccessLogError::Invalid`] for a tombstone that breaks an invariant,
    /// and the store and read errors of [`Self::read`].
    pub async fn tombstone(
        &self,
        uid: &str,
        time_removed: DvDateTime,
        grounds: NonEmptyVec<DvCodedText>,
    ) -> Result<Option<AccessLogTombstone>, AccessLogError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        let tombstone = Self::tombstone_in(&mut tx, uid, time_removed, grounds).await?;
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this transaction's
        // rows.
        tx.commit().await?;
        Ok(tombstone)
    }

    /// [`Self::tombstone`] inside the caller's transaction on `conn`, so the
    /// record of the change to the log commits with it (the draft's recording
    /// rule 7, what changes the log is recorded).
    ///
    /// # Errors
    /// As [`Self::tombstone`].
    pub async fn tombstone_in(
        conn: &mut PgConnection,
        uid: &str,
        time_removed: DvDateTime,
        grounds: NonEmptyVec<DvCodedText>,
    ) -> Result<Option<AccessLogTombstone>, AccessLogError> {
        // NOTE: our own design (#5714); a uid that is no uuid names no entry of this log.
        let Ok(uid) = uuid::Uuid::parse_str(uid) else {
            return Ok(None);
        };
        // NOTE: our own design (#6319), the record view is a union, which takes no row lock, so the
        // entry's row is locked in the arm that holds it in the statement's snapshot.
        let row = sqlx::query(concat!(
            "WITH indexed AS (SELECT e.id FROM audit.access_log_index i \
               JOIN audit.access_log_entry e ON e.id = i.entry_id \
              WHERE e.uid = $1 AND e.record_type = 'ACCESS_LOG_ENTRY' FOR UPDATE OF e), \
             unindexed AS (SELECT e.id FROM audit.access_log_entry e \
              WHERE e.id > (SELECT x.last_id FROM audit.access_log_index_state x WHERE x.singleton) \
                AND NOT EXISTS (SELECT 1 FROM audit.access_log_hold h WHERE h.entry_id = e.id) \
                AND e.uid = $1 AND e.record_type = 'ACCESS_LOG_ENTRY' FOR UPDATE OF e) ",
            records!(),
            " WHERE r.chain_seq IN (SELECT id FROM indexed UNION ALL SELECT id FROM unindexed)"
        ))
        .bind(uid)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let entry: AccessLogEntry = stored(&row)?;
        let tombstone = tombstone_of(entry, time_removed, grounds);
        let replaced = replace(conn, &[(row.try_get("chain_seq")?, &tombstone)]).await?;
        Ok(replaced.then_some(tombstone))
    }

    /// Replace up to `limit` entries whose retention has ended by their
    /// tombstones, removed at `time_removed`, and return the tombstones.
    ///
    /// An entry has expired once `today` is past its `retain_until`, the last
    /// day it is kept (`docs/specifications/access-log`, semantics §Retention); an entry that
    /// states none is kept without end. Entries expire in any order, in the
    /// middle of the log as at its start, and each tombstone names the grounds
    /// of the period that ended. An indexed entry a sink has not delivered
    /// waits one day more
    /// ([`crate::system_log::store::delivery::DELIVERY_GRACE_DAYS`]), and its
    /// tombstone parks it for that sink then. The entries come oldest end first and one
    /// call replaces them in one transaction; a caller repeats it until fewer
    /// than `limit` come back. Rows another run holds are skipped, so two
    /// replicas never replace the same entry.
    ///
    /// # Errors
    /// [`AccessLogError::Invalid`] for a tombstone that breaks an invariant,
    /// and the store and read errors of [`Self::read`].
    pub async fn tombstone_expired(
        &self,
        today: jiff::civil::Date,
        time_removed: &DvDateTime,
        limit: u16,
    ) -> Result<Vec<AccessLogTombstone>, AccessLogError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        // NOTE: our own design (#6319), the record view is a union, which takes no row lock, so
        // each arm locks its first entries and the oldest ends of the two are replaced.
        let rows = sqlx::query(concat!(
            "WITH indexed AS (SELECT i.entry_id AS id, i.retain_until FROM audit.access_log_index i \
               JOIN audit.access_log_entry e ON e.id = i.entry_id \
              WHERE e.record_type = 'ACCESS_LOG_ENTRY' AND i.retain_until < $1::text::date \
                AND (i.render_policy_id IS NULL OR i.retain_until < $1::text::date - 1 \
                     OR NOT EXISTS (SELECT 1 FROM audit.delivery_cursor c \
                                     WHERE c.chain = 'access_log' AND c.through_id < i.entry_id)) \
              ORDER BY i.retain_until, i.entry_id LIMIT $2 FOR UPDATE OF e SKIP LOCKED), \
             unindexed AS (SELECT e.id, e.retain_until FROM audit.access_log_entry e \
              WHERE e.id > (SELECT x.last_id FROM audit.access_log_index_state x WHERE x.singleton) \
                AND NOT EXISTS (SELECT 1 FROM audit.access_log_hold h WHERE h.entry_id = e.id) \
                AND e.record_type = 'ACCESS_LOG_ENTRY' AND e.retain_until < $1::text::date \
                AND (e.render_policy_id IS NULL OR e.retain_until < $1::text::date - 1 \
                     OR NOT EXISTS (SELECT 1 FROM audit.delivery_cursor c \
                                     WHERE c.chain = 'access_log' AND c.through_id < e.id)) \
              ORDER BY e.retain_until, e.id LIMIT $2 FOR UPDATE OF e SKIP LOCKED), \
             due AS (SELECT id, retain_until FROM indexed UNION ALL SELECT id, retain_until FROM unindexed \
              ORDER BY retain_until, id LIMIT $2) ",
            records!(),
            " WHERE r.chain_seq IN (SELECT id FROM due) ORDER BY r.retain_until, r.chain_seq"
        ))
        .bind(today.to_string())
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await?;
        let mut tombstones = Vec::with_capacity(rows.len());
        for row in &rows {
            let entry: AccessLogEntry = stored(row)?;
            let grounds = entry.retention.grounds.clone();
            tombstones.push((
                row.try_get::<i64, _>("chain_seq")?,
                tombstone_of(entry, time_removed.clone(), grounds),
            ));
        }
        let replacing = tombstones
            .iter()
            .map(|(chain_seq, tombstone)| (*chain_seq, tombstone))
            .collect::<Vec<_>>();
        replace(&mut tx, &replacing).await?;
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this transaction's
        // rows.
        tx.commit().await?;
        Ok(tombstones
            .into_iter()
            .map(|(_, tombstone)| tombstone)
            .collect())
    }
}

impl AccessLogStore {
    /// Remove up to `limit` shared parts whose `retain_until` is more than a
    /// day before `today` and that no entry names
    /// (`audit.access_log_shared_collect`), and return how many.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn collect_shared(
        &self,
        today: jiff::civil::Date,
        limit: u16,
    ) -> Result<u64, AccessLogError> {
        let removed: i64 =
            sqlx::query_scalar("SELECT audit.access_log_shared_collect($1::text::date, $2)")
                .bind(today.to_string())
                .bind(i32::from(limit))
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
        Ok(u64::try_from(removed).unwrap_or(0))
    }
}

/// The tombstone of `entry`, removed at `time_removed` on `grounds`: its uid,
/// `system_id` and `time_recorded`, and nothing else of it.
pub(crate) fn tombstone_of(
    entry: AccessLogEntry,
    time_removed: DvDateTime,
    grounds: NonEmptyVec<DvCodedText>,
) -> AccessLogTombstone {
    AccessLogTombstone {
        uid: entry.uid,
        system_id: entry.system_id,
        time_recorded: entry.time_recorded,
        time_removed,
        grounds,
    }
}

/// Write each tombstone over the entry of its id on `tx`, in one
/// statement, after checking every one against the draft's invariants;
/// whether any was written.
///
/// The entry table's update trigger admits the change only from an entry to
/// its own tombstone
/// (`migrations/audit/0004_baseline_access_log.sql`).
async fn replace(
    tx: &mut PgConnection,
    tombstones: &[(i64, &AccessLogTombstone)],
) -> Result<bool, AccessLogError> {
    if tombstones.is_empty() {
        return Ok(false);
    }
    let mut positions = Vec::with_capacity(tombstones.len());
    let mut contents = Vec::with_capacity(tombstones.len());
    for (chain_seq, tombstone) in tombstones {
        check_tombstone(tombstone)?;
        positions.push(*chain_seq);
        contents.push(canonical_form(*tombstone)?);
    }
    sqlx::query(
        "UPDATE audit.access_log_entry e SET content = t.content, shared = NULL \
         FROM unnest($1::bigint[], $2::text[]) AS t(id, content) \
         WHERE e.id = t.id",
    )
    .bind(&positions)
    .bind(&contents)
    .execute(&mut *tx)
    .await?;
    Ok(true)
}

/// The record `row` of this system's log holds, read as a `T`: its canonical
/// form, joined from its stored parts ([`document_of`]). The row holds
/// `chain_seq` (the entry id) and the columns of [`entry_parts`].
///
/// # Errors
/// [`AccessLogError::Unreadable`] when the record does not read.
pub(crate) fn stored<T: serde::de::DeserializeOwned>(row: &PgRow) -> Result<T, AccessLogError> {
    let shared = row.try_get("shared_part")?;
    stored_with(row, shared)
}

/// [`stored`] with `shared` as the record's shared part, which a page of
/// records carries once for every record naming it (#6244).
///
/// # Errors
/// [`AccessLogError::Unreadable`] when the record does not read.
pub(crate) fn stored_with<T: serde::de::DeserializeOwned>(
    row: &PgRow,
    shared: Option<&str>,
) -> Result<T, AccessLogError> {
    let document = document_of(row, shared)?;
    read_document(row, &document)
}

/// The canonical form of the record `row` holds, joined from the columns of
/// [`entry_parts`] and `shared`, the record's shared part, which a page of
/// records reads once for every record naming it (#6244): the exact text its
/// content digest covers.
///
/// # Errors
/// [`AccessLogError::Unreadable`] when the parts do not join, and the store
/// errors when a column does not read.
pub(crate) fn document_of(row: &PgRow, shared: Option<&str>) -> Result<String, AccessLogError> {
    let chain_seq: i64 = row.try_get("chain_seq")?;
    let columns = row_columns(row)?;
    let form: i16 = row.try_get("stored_form")?;
    if form != stored_form::parts::facts::FORM {
        return Err(AccessLogError::Unreadable {
            chain_seq,
            source: serde::de::Error::custom(format!("no renderer reads stored form {form}")),
        });
    }
    stored_form::parts::assemble::whole(row.try_get("row_part")?, shared, &columns)
        .map_err(|source| AccessLogError::Unreadable { chain_seq, source })
}

/// The columns of the record `row` holds that its renderer joins its stored
/// parts with, from the columns of [`entry_parts`].
///
/// # Errors
/// The store errors when a column does not read.
pub(crate) fn row_columns(
    row: &PgRow,
) -> Result<stored_form::parts::assemble::RowColumns<'_>, AccessLogError> {
    Ok(stored_form::parts::assemble::RowColumns {
        record_type: row.try_get("entry_type")?,
        uid: row.try_get("entry_uid")?,
        time_recorded: row
            .try_get::<jiff_sqlx::Timestamp, _>("recorded_at")?
            .to_jiff(),
        ehr_id: row.try_get("entry_ehr")?,
        index: stored_form::parts::index::IndexColumns {
            request_id: row.try_get("request_id")?,
            resource_id: row.try_get("resource_id")?,
        },
    })
}

/// The record whose canonical form is `document`, read as a `T` with no
/// JSON value of it built.
fn read_document<T: serde::de::DeserializeOwned>(
    row: &PgRow,
    document: &str,
) -> Result<T, AccessLogError> {
    let chain_seq: i64 = row.try_get("chain_seq")?;
    serde_json::from_str(document)
        .map_err(|source| AccessLogError::Unreadable { chain_seq, source })
}
