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
//! sequence and `content_digest`, one per line. The store writes the
//! canonical form, and the insert trigger of `audit.access_log_record`
//! (`migrations/audit/0031_access_log_own_chain.sql`) gives it its link in
//! this system's own access log, one chain from 1 with no gaps, apart from
//! the records outside the log, so the database is the one place a position
//! is decided. The retention rules (§Retention) let an entry be replaced by
//! a tombstone that keeps its uid, `system_id`, `time_recorded` and link, the
//! one change the table's triggers admit.
//!
//! Verification is the access log's own: `audit.verify_access_log_chain()`
//! and [`crate::system_log::store::AuditStore::verify_chain_advancing`].

#![expect(
    clippy::disallowed_types,
    reason = "the canonical form and the stored record are computed over the record's ITS-JSON \
              value, with its link left out or joined (#1694 boundary class: canonical form)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::{
    canonical_json, canonical_object,
};
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{Connection, PgConnection, PgPool, Row};

use crate::access_log::checkpoint::{PendingSeal, Sealer};
use crate::access_log::stored_form;
use crate::db::acquire::{AcquireError, acquire};
use crate::digest::sha256_hex;
use crate::telemetry::access_path;
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
    let value = serde_json::to_value(record).map_err(AccessLogError::Canonical)?;
    let canonical = match &value {
        Value::Object(members) => canonical_object(
            members
                .iter()
                .filter(|(name, _)| name.as_str() != LINK && name.as_str() != PROOF)
                .map(|(name, member)| (name.as_str(), member)),
        ),
        other => canonical_json(other),
    };
    canonical.map_err(AccessLogError::Canonical)
}

/// The records of this system's log joined with their shared parts, in the
/// columns [`stored`] reads.
macro_rules! records {
    () => {
        "SELECT r.chain_seq, r.content, sh.content AS shared, r.algorithm, \
         r.previous_digest, r.content_digest, r.digest \
         FROM audit.access_log_record r \
         LEFT JOIN audit.access_log_shared sh ON sh.digest = r.shared"
    };
}
pub(crate) use records;

/// An entry as the store writes it ([`stored_form`]).
#[derive(Debug)]
struct Stored {
    /// The row part's canonical form.
    row: String,
    /// The shared part's digest and canonical form.
    shared: Option<(String, String)>,
    /// The SHA-256 of the whole canonical form.
    content_digest: String,
}

impl Stored {
    /// The parts and the content digest of `entry`.
    fn of(entry: &AccessLogEntry) -> Result<Self, AccessLogError> {
        let value = serde_json::to_value(entry).map_err(AccessLogError::Canonical)?;
        let Value::Object(mut members) = value else {
            let whole = canonical_form(entry)?;
            return Ok(Self {
                content_digest: sha256_hex(&whole),
                row: whole,
                shared: None,
            });
        };
        members.remove(LINK);
        let whole =
            canonical_json(&Value::Object(members.clone())).map_err(AccessLogError::Canonical)?;
        let content_digest = sha256_hex(&whole);
        let form = stored_form::split(&members);
        let row = canonical_json(&form.row).map_err(AccessLogError::Canonical)?;
        let shared = form
            .shared
            .map(|shared| {
                canonical_json(&shared)
                    .map(|text| (sha256_hex(&text), text))
                    .map_err(AccessLogError::Canonical)
            })
            .transpose()?;
        Ok(Self {
            row,
            shared,
            content_digest,
        })
    }
}

/// The entries an append wrote, with the checkpoint its transaction still
/// owes when the store seals.
#[derive(Debug)]
#[must_use = "an append owes its checkpoint in its own transaction: seal it before the commit"]
pub struct Appended {
    entries: Vec<AccessLogEntry>,
    seal: Option<PendingSeal>,
}

impl Appended {
    /// The entries, each with the link the chain gave it.
    #[must_use]
    pub fn entries(&self) -> &[AccessLogEntry] {
        &self.entries
    }

    /// Write the checkpoint the append owes on `conn`, the transaction it
    /// was made in, and return the entries.
    ///
    /// # Errors
    /// [`AccessLogError::Sign`] when the keyring cannot sign the checkpoint,
    /// [`AccessLogError::Signing`] when the task signing it ends without an
    /// answer, [`AccessLogError::Invalid`] for a checkpoint that breaks an
    /// invariant, and the store errors when the insert fails.
    pub async fn seal(
        self,
        conn: &mut PgConnection,
    ) -> Result<Vec<AccessLogEntry>, AccessLogError> {
        if let Some(seal) = self.seal {
            seal.write(conn).await?;
        }
        Ok(self.entries)
    }
}

/// The access-log records in the `audit` schema.
#[derive(Debug, Clone)]
pub struct AccessLogStore {
    pool: PgPool,
    /// The keyring and system that seal each append in a signed checkpoint,
    /// whenever the keyring holds an `OpenPGP` key (#5704).
    sealer: Option<Sealer>,
}

impl AccessLogStore {
    /// A store over the audit pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool, sealer: None }
    }

    /// The audit pool the records are kept in.
    pub(super) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The store that ends each append with an `ACCESS_LOG_CHECKPOINT` of
    /// `system_id` over the last link it wrote, signed with `signer`'s key in
    /// the append's own transaction (#5704).
    #[must_use]
    pub fn with_sealer(mut self, signer: Arc<Signer>, system_id: String) -> Self {
        self.sealer = Some(Sealer::new(signer, system_id));
        self
    }

    /// Append `entries` to the chain in their order, in one transaction, and
    /// return them with the links the chain gave them.
    ///
    /// The link an entry arrives with is replaced: its position and digests
    /// are the chain's to decide. Each entry is checked against the draft's
    /// invariants with its link before the transaction commits, so an entry
    /// that breaks one leaves no record. With a sealer the transaction ends
    /// with the checkpoint of the last link.
    ///
    /// # Errors
    /// [`AccessLogError::Canonical`] for an entry with no canonical form,
    /// [`AccessLogError::Invalid`] for one that breaks an invariant, the
    /// sealing errors of [`Appended::seal`], and the store errors when a
    /// statement fails (a repeated uid among them).
    pub async fn append(
        &self,
        entries: Vec<AccessLogEntry>,
    ) -> Result<Vec<AccessLogEntry>, AccessLogError> {
        if entries.is_empty() {
            return Ok(entries);
        }
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        let appended = self.append_in(&mut tx, entries).await?;
        let entries = appended.seal(&mut tx).await?;
        tx.commit().await?;
        Ok(entries)
    }

    /// [`Self::append`] inside the caller's transaction on `conn`, so the
    /// entries commit with what the caller writes beside them; the caller
    /// ends the append with [`Appended::seal`] before it commits.
    ///
    /// The first insert of the transaction locks the head of the chain until
    /// the transaction ends. With a sealer the checkpoint of the last link is
    /// signed on the blocking pool while the caller writes the rest of its
    /// transaction, so the signature adds to the time the head is held only
    /// what outlasts that work.
    ///
    /// # Errors
    /// As [`Self::append`] before the seal.
    pub async fn append_in(
        &self,
        conn: &mut PgConnection,
        entries: Vec<AccessLogEntry>,
    ) -> Result<Appended, AccessLogError> {
        let started = std::time::Instant::now();
        let count = u64::try_from(entries.len()).unwrap_or(u64::MAX);
        let appended = self.link_in(conn, entries).await;
        let metrics = access_path::metrics();
        let outcome = access_path::outcome(&appended);
        metrics.log_append_duration.record(
            access_path::seconds_since(started),
            std::slice::from_ref(&outcome),
        );
        metrics.log_entries.add(count, &[outcome]);
        let entries = appended?;
        let seal = match (&self.sealer, entries.last()) {
            (Some(sealer), Some(last)) => {
                Some(sealer.begin(last.link.sequence, &last.link.digest)?)
            }
            _ => None,
        };
        Ok(Appended { entries, seal })
    }

    /// Link and insert `entries` on `conn`, the work of [`Self::append_in`].
    async fn link_in(
        &self,
        conn: &mut PgConnection,
        mut entries: Vec<AccessLogEntry>,
    ) -> Result<Vec<AccessLogEntry>, AccessLogError> {
        let mut contents = Vec::with_capacity(entries.len());
        let mut parts = Vec::with_capacity(entries.len());
        let mut digests = Vec::with_capacity(entries.len());
        let mut shared = BTreeMap::new();
        for entry in &entries {
            let stored = Stored::of(entry)?;
            if let Some((digest, content)) = stored.shared {
                parts.push(Some(digest.clone()));
                shared.insert(digest, content);
            } else {
                parts.push(None);
            }
            contents.push(stored.row);
            digests.push(stored.content_digest);
        }
        let (keys, texts): (Vec<String>, Vec<String>) = shared.into_iter().unzip();
        sqlx::query(
            "INSERT INTO audit.access_log_shared (digest, content) \
             SELECT * FROM unnest($1::text[], $2::text[]) ON CONFLICT (digest) DO NOTHING",
        )
        .bind(&keys)
        .bind(&texts)
        .execute(&mut *conn)
        .await?;
        // NOTE: the ordinality orders the insert, which is the order the row trigger links the records in.
        let rows = sqlx::query(
            "INSERT INTO audit.access_log_record (content, shared, content_digest) \
             SELECT t.content, t.shared, t.content_digest \
             FROM unnest($1::text[], $2::text[], $3::text[]) WITH ORDINALITY \
             AS t(content, shared, content_digest, n) \
             ORDER BY t.n \
             RETURNING uid, chain_seq, algorithm, previous_digest, content_digest, digest",
        )
        .bind(&contents)
        .bind(&parts)
        .bind(&digests)
        .fetch_all(&mut *conn)
        .await?;
        let mut links = rows
            .iter()
            .map(|row| Ok((row.try_get::<String, _>("uid")?, own_link(row)?)))
            .collect::<Result<BTreeMap<_, _>, sqlx::Error>>()?;
        for entry in &mut entries {
            entry.link =
                links
                    .remove(entry.uid.value())
                    .ok_or_else(|| AccessLogError::Unlinked {
                        uid: entry.uid.value().to_owned(),
                    })?;
        }
        for entry in &entries {
            check_entry(entry)?;
        }
        Ok(entries)
    }

    /// The records whose `ehr_id` is `ehr_id`, in chain order, each with its
    /// link.
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`AccessLogError::Unreadable`] when a stored record does not read.
    pub async fn of_ehr(&self, ehr_id: &str) -> Result<Vec<AccessLogRecord>, AccessLogError> {
        let rows = sqlx::query(concat!(
            records!(),
            " WHERE r.ehr_id = $1 ORDER BY r.chain_seq"
        ))
        .bind(ehr_id)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?;
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
        tx.commit().await?;
        Ok(tombstones
            .into_iter()
            .map(|(_, tombstone)| tombstone)
            .collect())
    }
}

/// The tombstone of `entry`, removed at `time_removed` on `grounds`: its uid,
/// `system_id`, `time_recorded` and link, and nothing else of it.
pub(super) fn tombstone_of(
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
/// The table's update trigger admits the change only from an entry to its
/// own tombstone (`migrations/audit/0027_access_log_record.sql`).
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
        "UPDATE audit.access_log_record r SET content = t.content, shared = NULL \
         FROM unnest($1::bigint[], $2::text[]) AS t(chain_seq, content) \
         WHERE r.chain_seq = t.chain_seq",
    )
    .bind(&positions)
    .bind(&contents)
    .execute(&mut *tx)
    .await?;
    Ok(true)
}

/// The link the columns of `row` hold, this system's: its links carry no
/// signature, since a checkpoint signs each append (#5704).
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

/// The record `row` of this system's log holds, read as a `T`: its row and
/// shared parts joined into its canonical form, with its link. The row holds
/// `chain_seq`, `content`, `shared` and the link's columns.
///
/// # Errors
/// [`AccessLogError::Unreadable`] when the record does not read.
pub(crate) fn stored<T: serde::de::DeserializeOwned>(row: &PgRow) -> Result<T, AccessLogError> {
    joined(row, own_link(row)?)
}

/// The record `row` of an imported log holds, read as a `T`, as [`stored`]
/// with the link's `signature` column.
///
/// # Errors
/// As [`stored`].
pub(super) fn stored_signed<T: serde::de::DeserializeOwned>(
    row: &PgRow,
) -> Result<T, AccessLogError> {
    joined(row, signed_link(row)?)
}

/// The record the `content` and `shared` columns of `row` store, with `link`
/// joined.
fn joined<T: serde::de::DeserializeOwned>(
    row: &PgRow,
    link: AccessLogLink,
) -> Result<T, AccessLogError> {
    let chain_seq: i64 = row.try_get("chain_seq")?;
    let unreadable = |source| AccessLogError::Unreadable { chain_seq, source };
    let content: String = row.try_get("content")?;
    let own: Value = serde_json::from_str(&content).map_err(unreadable)?;
    let shared = row
        .try_get::<Option<String>, _>("shared")?
        .map(|text| serde_json::from_str::<Value>(&text))
        .transpose()
        .map_err(unreadable)?;
    let mut value = stored_form::join(own, shared);
    let link = serde_json::to_value(link).map_err(unreadable)?;
    if let Value::Object(members) = &mut value {
        members.insert(LINK.to_owned(), link);
    }
    serde_json::from_value(value).map_err(unreadable)
}
