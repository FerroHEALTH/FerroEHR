// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-log store: each record kept as its canonical form, chained by
//! its `ACCESS_LOG_LINK`, and an entry replaced in place by its tombstone
//! (#5125).
//!
//! The draft's integrity rules (`docs/plans/access-log.md` §Integrity) fix
//! the chain: the canonical form of a record is its ITS-JSON serialisation
//! without its link, canonicalised by RFC 8785; `content_digest` is the
//! SHA-256 of that form and `digest` the SHA-256 of `previous_digest`, the
//! sequence and `content_digest`, one per line. An entry is written to the
//! intake first ([`crate::access_log::intake`]) and the chainer
//! ([`crate::access_log::chainer`]) gives it its link in this system's own
//! access log, one chain from 1 with no gaps, under the head's lock, which
//! the insert trigger of `audit.access_log_link`
//! (`migrations/audit/0004_baseline_access_log.sql`) checks. The retention rules (§Retention) let an entry be replaced by
//! a tombstone that keeps its uid, `system_id`, `time_recorded` and link, the
//! one change the table's triggers admit.
//!
//! Verification is the access log's own: `audit.verify_access_log_chain()`
//! and [`crate::system_log::store::AuditStore::verify_chain_advancing`].

use std::collections::BTreeMap;
use std::sync::Arc;

use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::canonical_object_without;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use sqlx::postgres::PgRow;
use sqlx::{Connection, PgConnection, PgPool, Row};

use crate::access_log::chainer::Owed;
use crate::access_log::checkpoint::{PendingSeal, Sealer};
use crate::access_log::intake::{IntakeEntry, take_in};
use crate::access_log::stored_form;
use crate::db::acquire::{AcquireError, acquire};
use crate::versioning::signature::signer::{AttestError, Signer};
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::draft::access_log::access_log::access_log_tombstone::AccessLogTombstone;
use openehr_rm::draft::access_log::invariants::{check_entry, check_tombstone};
use openehr_rm::draft::invariants::Violations;

/// The member of a record's ITS-JSON form its canonical form leaves out.
const LINK: &str = "link";

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
    /// The chain reached the last position a link can name.
    #[error("the access-log chain has no position left")]
    Exhausted,
    /// The store wrote an entry and returned no link for its uid.
    #[error("the access-log store returned no link for the entry {uid}")]
    Unlinked {
        /// The entry's uid.
        uid: String,
    },
}

/// The canonical form of an access-log record or checkpoint: its ITS-JSON
/// serialisation without its link or its proof, canonicalised by RFC 8785.
///
/// The link is left out whatever it holds, since the chain computes it over
/// this form, and a checkpoint's proof since it signs this form. No record
/// has a proof and no checkpoint a link.
///
/// # Errors
/// [`AccessLogError::Canonical`] when the record does not serialise or holds
/// a number with no finite double form.
pub fn canonical_form(record: &impl serde::Serialize) -> Result<String, AccessLogError> {
    canonical_object_without(record, &[LINK, PROOF]).map_err(AccessLogError::Canonical)
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
         r.request_id, r.resource_id"
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
            ", r.algorithm, \
             encode(r.previous_digest, 'hex') AS previous_digest, \
             encode(r.content_digest, 'hex') AS content_digest, encode(r.digest, 'hex') AS digest \
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
    /// The keyring and system that seal the chain in signed checkpoints,
    /// whenever the keyring holds an `OpenPGP` key (#5704).
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

    /// The store that seals the chain in `ACCESS_LOG_CHECKPOINT`s of
    /// `system_id` over the last link a round wrote, signed with `signer`'s
    /// key in the round's own transaction (#5704, #5770).
    #[must_use]
    pub fn with_sealer(mut self, signer: Arc<Signer>, system_id: String) -> Self {
        self.sealer = Some(Sealer::new(signer, system_id));
        self
    }

    /// Append `entries` to the chain in their order and return them with the
    /// links the chain gave them: written to the intake in one transaction,
    /// then chained, with every row written before them, before this returns.
    ///
    /// The link an entry arrives with is replaced: its position and digests
    /// are the chain's to decide. Each entry is checked against the draft's
    /// invariants with its link.
    ///
    /// # Errors
    /// [`AccessLogError::Canonical`] for an entry with no canonical form,
    /// [`AccessLogError::Invalid`] for one that breaks an invariant, the
    /// errors of [`Self::settle`], and the store errors when a statement
    /// fails (a repeated uid among them).
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
        take_in(
            &mut tx,
            &intake,
            &[],
            None,
            &std::collections::BTreeSet::new(),
        )
        .await?;
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this
        // transaction's rows.
        tx.commit().await?;
        drop(conn);
        self.settle(Owed::default()).await?;
        self.linked(entries).await
    }

    /// `entries`, chained, each with the link the chain gave it.
    ///
    /// # Errors
    /// [`AccessLogError::Unlinked`] for an entry the log does not hold,
    /// [`AccessLogError::Invalid`] for one that breaks an invariant with its
    /// link, and the store errors.
    pub(super) async fn linked(
        &self,
        entries: Vec<AccessLogEntry>,
    ) -> Result<Vec<AccessLogEntry>, AccessLogError> {
        let uids = entries
            .iter()
            .map(|entry| uuid::Uuid::parse_str(entry.uid.value()).ok())
            .collect::<Vec<_>>();
        let rows = sqlx::query(concat!(records!(), " WHERE r.uid = ANY($1)"))
            .bind(&uids)
            .fetch_all(&mut *acquire(&self.pool).await?)
            .await?;
        let mut links = rows
            .iter()
            .map(|row| Ok((row.try_get::<uuid::Uuid, _>("uid")?, own_link(row)?)))
            .collect::<Result<BTreeMap<_, _>, sqlx::Error>>()?;
        let mut linked = entries;
        for (entry, uid) in linked.iter_mut().zip(uids) {
            entry.link =
                uid.and_then(|uid| links.remove(&uid))
                    .ok_or_else(|| AccessLogError::Unlinked {
                        uid: entry.uid.value().to_owned(),
                    })?;
            check_entry(entry)?;
        }
        Ok(linked)
    }

    /// The checkpoint a round that ends at the link `digest` at `sequence`
    /// owes, its signature begun; `None` without a sealer.
    ///
    /// # Errors
    /// The errors of starting the signature.
    pub(super) fn begin_seal(
        &self,
        sequence: i64,
        digest: &str,
    ) -> Result<Option<PendingSeal>, AccessLogError> {
        self.sealer
            .as_ref()
            .map(|sealer| sealer.begin(sequence, digest))
            .transpose()
    }

    /// The records whose `ehr_id` is `ehr_id`, in chain order, each with its
    /// link.
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`AccessLogError::Unreadable`] when a stored record does not read.
    pub async fn of_ehr(&self, ehr_id: &str) -> Result<Vec<AccessLogRecord>, AccessLogError> {
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
        rows.iter().map(stored).collect()
    }

    /// The links from `from` to `to`, both included, without the records'
    /// content: what a verifier of the records between needs.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn links(&self, from: i64, to: i64) -> Result<Vec<AccessLogLink>, AccessLogError> {
        let span = self.chain(from, to).await?;
        Ok(span.into_iter().map(|(_uid, link)| link).collect())
    }

    /// The record whose uid is `uid`, entry or tombstone, with its link.
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
    /// The tombstone keeps the entry's uid, `system_id`, `time_recorded` and
    /// link, so the chain verifies after the entry's content is gone. `None`
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
        let row = sqlx::query(concat!(
            records!(),
            " WHERE r.uid = $1 AND r.record_type = 'ACCESS_LOG_ENTRY' FOR UPDATE OF r"
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
    /// day it is kept (`docs/plans/access-log.md` §Retention); an entry that
    /// states none is kept without end. Entries expire in any order, in the
    /// middle of the log as at its start, and each tombstone names the grounds
    /// of the period that ended. The entries come oldest end first and one
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
        let rows = sqlx::query(concat!(
            records!(),
            " WHERE r.record_type = 'ACCESS_LOG_ENTRY' AND r.retain_until < $1::text::date \
                 ORDER BY r.retain_until, r.chain_seq LIMIT $2 FOR UPDATE OF r SKIP LOCKED"
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
/// `system_id`, `time_recorded` and link, and nothing else of it.
pub(crate) fn tombstone_of(
    entry: AccessLogEntry,
    time_removed: DvDateTime,
    grounds: NonEmptyVec<DvCodedText>,
) -> AccessLogTombstone {
    AccessLogTombstone {
        uid: entry.uid,
        system_id: entry.system_id,
        time_recorded: entry.time_recorded,
        link: entry.link,
        time_removed,
        grounds,
    }
}

/// Write each tombstone over the entry at its chain position on `tx`, in one
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
         FROM unnest($1::bigint[], $2::text[]) AS t(chain_seq, content) \
         JOIN audit.access_log_link l ON l.chain_seq = t.chain_seq \
         WHERE e.id = l.entry_id",
    )
    .bind(&positions)
    .bind(&contents)
    .execute(&mut *tx)
    .await?;
    Ok(true)
}

/// The link the columns of `row` hold, this system's: its links carry no
/// signature, since checkpoints sign the chain (#5704).
pub(super) fn own_link(row: &PgRow) -> Result<AccessLogLink, sqlx::Error> {
    Ok(AccessLogLink {
        sequence: row.try_get("chain_seq")?,
        algorithm: row.try_get("algorithm")?,
        previous_digest: row.try_get("previous_digest")?,
        content_digest: row.try_get("content_digest")?,
        digest: row.try_get("digest")?,
        signature: None,
    })
}

/// The link the columns of `row` hold with its `signature` column, an
/// imported log's, whose exporter may have signed its links.
pub(super) fn signed_link(row: &PgRow) -> Result<AccessLogLink, sqlx::Error> {
    Ok(AccessLogLink {
        signature: row.try_get("signature")?,
        ..own_link(row)?
    })
}

/// The record `row` of this system's log holds, read as a `T`: its canonical
/// form, joined from its stored parts ([`document_of`]), with its link. The
/// row holds `chain_seq`, the columns of [`entry_parts`] and the link's
/// columns.
///
/// # Errors
/// [`AccessLogError::Unreadable`] when the record does not read.
pub(crate) fn stored<T: serde::de::DeserializeOwned>(row: &PgRow) -> Result<T, AccessLogError> {
    let document = document_of(row)?;
    with_link(row, &document, &own_link(row)?)
}

/// The record `row` of an imported log holds, read as a `T`, as [`stored`]
/// from its whole canonical form in `document`, with the link's `signature`
/// column.
///
/// # Errors
/// As [`stored`].
pub(super) fn stored_signed<T: serde::de::DeserializeOwned>(
    row: &PgRow,
) -> Result<T, AccessLogError> {
    let document: String = row.try_get("document")?;
    with_link(row, &document, &signed_link(row)?)
}

/// The canonical form of the record `row` holds, without its link, joined
/// from the columns of [`entry_parts`]: the exact text its content digest
/// covers.
///
/// # Errors
/// [`AccessLogError::Unreadable`] when the parts do not join, and the store
/// errors when a column does not read.
pub(crate) fn document_of(row: &PgRow) -> Result<String, AccessLogError> {
    let chain_seq: i64 = row.try_get("chain_seq")?;
    let record_type: &str = row.try_get("entry_type")?;
    let columns = stored_form::parts::RowColumns {
        record_type,
        uid: row.try_get("entry_uid")?,
        time_recorded: row
            .try_get::<jiff_sqlx::Timestamp, _>("recorded_at")?
            .to_jiff(),
        ehr_id: row.try_get("entry_ehr")?,
        index: stored_form::parts::index::IndexColumns {
            request_id: row.try_get("request_id")?,
            resource_id: row.try_get("resource_id")?,
        },
    };
    stored_form::parts::whole(
        row.try_get("row_part")?,
        row.try_get("shared_part")?,
        &columns,
    )
    .map_err(|source| AccessLogError::Unreadable { chain_seq, source })
}

/// The record whose canonical form is `document`, with `link` joined, read
/// as a `T` with no JSON value of it built.
fn with_link<T: serde::de::DeserializeOwned>(
    row: &PgRow,
    document: &str,
    link: &AccessLogLink,
) -> Result<T, AccessLogError> {
    let chain_seq: i64 = row.try_get("chain_seq")?;
    let unreadable = |source| AccessLogError::Unreadable { chain_seq, source };
    let link = serde_json::to_string(link).map_err(unreadable)?;
    let open = document
        .strip_suffix('}')
        .ok_or_else(|| unreadable(serde::de::Error::custom("a record is no object")))?;
    let mut joined = String::with_capacity(document.len() + link.len() + 9);
    joined.push_str(open);
    if open != "{" {
        joined.push(',');
    }
    joined.push_str("\"link\":");
    joined.push_str(&link);
    joined.push('}');
    serde_json::from_str(&joined).map_err(unreadable)
}
