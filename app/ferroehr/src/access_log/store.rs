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
//! sequence and
//! `content_digest`, one per line. The store writes the canonical form, and
//! the insert trigger of `audit.access_log_record`
//! (`migrations/audit/0027_access_log_record.sql`) gives it its link in the
//! chain the `audit_event` records share, so the database is the one place a
//! position is decided. The retention rules (§Retention) let an entry be
//! replaced by a tombstone that keeps its uid, `system_id`, `time_recorded`
//! and link, the one change the table's triggers admit.
//!
//! Verification is the audit chain's: `audit.verify_audit_chain()` and
//! [`crate::system_log::store::AuditStore::verify_chain_advancing`] walk the
//! `audit_event` records and these in one pass.

#![expect(
    clippy::disallowed_types,
    reason = "the canonical form and the stored record are computed over the record's ITS-JSON \
              value, with its link left out or joined (#1694 boundary class: canonical form)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_object;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{Connection, PgConnection, PgPool, Row};

use crate::access_log::chain::link_digest;
use crate::db::acquire::{AcquireError, acquire};
use crate::digest::sha256_hex;
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
    /// A link was to be signed and the keyring could not sign it.
    #[error("the access-log link could not be signed")]
    Sign(#[from] AttestError),
    /// The chain between two positions holds a record of the `audit_event`
    /// form, whose link only the database that wrote it recomputes, so the
    /// span cannot be carried apart from it.
    #[error(
        "the chain between positions {from} and {to} holds audit_event records, which cannot be \
         verified apart from the database that wrote them"
    )]
    Interleaved {
        /// The first position of the span.
        from: i64,
        /// The last position of the span.
        to: i64,
    },
    /// The chain gave a link another digest than the one its signature was
    /// made over, so the transaction is refused.
    #[error("the access-log link at {sequence} is not the link that was signed")]
    Unforeseen {
        /// The link's position in the chain.
        sequence: i64,
    },
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
        other => {
            openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json(other)
        }
    };
    canonical.map_err(AccessLogError::Canonical)
}

/// The access-log records in the `audit` schema.
#[derive(Debug, Clone)]
pub struct AccessLogStore {
    pool: PgPool,
    /// The keyring that signs each new link's digest, whenever it holds an
    /// `OpenPGP` key (#5562).
    link_signer: Option<Arc<Signer>>,
}

impl AccessLogStore {
    /// A store over the audit pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            link_signer: None,
        }
    }

    /// The audit pool the records are kept in.
    pub(super) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The store that signs the digest of each link it appends with
    /// `signer`'s key, the optional `ACCESS_LOG_LINK.signature`, written
    /// with the row.
    #[must_use]
    pub fn with_link_signer(mut self, signer: Arc<Signer>) -> Self {
        self.link_signer = Some(signer);
        self
    }

    /// Append `entries` to the chain in their order, in one transaction, and
    /// return them with the links the chain gave them.
    ///
    /// The link an entry arrives with is replaced: its position and digests
    /// are the chain's to decide. Each entry is checked against the draft's
    /// invariants with its link before the transaction commits, so an entry
    /// that breaks one leaves no record.
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
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        let entries = self.append_in(&mut tx, entries).await?;
        tx.commit().await?;
        Ok(entries)
    }

    /// [`Self::append`] inside the caller's transaction on `conn`, so the
    /// entries commit with what the caller writes beside them.
    ///
    /// With a keyring that holds an `OpenPGP` key each link is signed before
    /// its row is written and stored by the same `INSERT`: the head of the
    /// chain is locked first, so the links the trigger gives are the ones
    /// computed and signed before the insert.
    ///
    /// # Errors
    /// As [`Self::append`], [`AccessLogError::Sign`] when the keyring cannot
    /// sign a link, and [`AccessLogError::Unforeseen`] when the chain gave a
    /// link another digest than the one signed.
    pub async fn append_in(
        &self,
        conn: &mut PgConnection,
        mut entries: Vec<AccessLogEntry>,
    ) -> Result<Vec<AccessLogEntry>, AccessLogError> {
        let contents = entries
            .iter()
            .map(canonical_form)
            .collect::<Result<Vec<_>, _>>()?;
        let foreseen = match &self.link_signer {
            Some(signer) => Self::foresee(conn, signer, &contents).await?,
            None => Vec::new(),
        };
        let signatures: Vec<Option<String>> = (0..contents.len())
            .map(|index| foreseen.get(index).map(|(_, signature)| signature.clone()))
            .collect();
        // NOTE: the ordinality orders the insert, which is the order the row trigger links the records in.
        let rows = sqlx::query(
            "INSERT INTO audit.access_log_record (content, signature) \
             SELECT t.content, t.signature \
             FROM unnest($1::text[], $2::text[]) WITH ORDINALITY AS t(content, signature, n) \
             ORDER BY t.n \
             RETURNING uid, chain_seq, algorithm, previous_digest, content_digest, digest, signature",
        )
        .bind(&contents)
        .bind(&signatures)
        .fetch_all(&mut *conn)
        .await?;
        let mut links = rows
            .iter()
            .map(|row| Ok((row.try_get::<String, _>("uid")?, link(row)?)))
            .collect::<Result<BTreeMap<_, _>, sqlx::Error>>()?;
        for (index, entry) in entries.iter_mut().enumerate() {
            entry.link =
                links
                    .remove(entry.uid.value())
                    .ok_or_else(|| AccessLogError::Unlinked {
                        uid: entry.uid.value().to_owned(),
                    })?;
            if let Some((digest, _)) = foreseen.get(index)
                && *digest != entry.link.digest
            {
                return Err(AccessLogError::Unforeseen {
                    sequence: entry.link.sequence,
                });
            }
        }
        for entry in &entries {
            check_entry(entry)?;
        }
        Ok(entries)
    }

    /// The digest and signature of the link each of `contents` will get,
    /// in order, computed from the head of the chain, which this locks until
    /// the transaction on `conn` ends.
    ///
    /// The digests follow the draft's chain rule
    /// ([`crate::access_log::chain::link_digest`]) that the insert trigger
    /// applies, so the `INSERT` that follows stores each signature with its
    /// row.
    async fn foresee(
        conn: &mut PgConnection,
        signer: &Signer,
        contents: &[String],
    ) -> Result<Vec<(String, String)>, AccessLogError> {
        let head = sqlx::query("SELECT head_seq, head_digest FROM audit.audit_chain_reserve()")
            .fetch_one(&mut *conn)
            .await?;
        let mut sequence: i64 = head.try_get("head_seq")?;
        let mut previous: String = head.try_get("head_digest")?;
        let mut foreseen = Vec::with_capacity(contents.len());
        for content in contents {
            sequence = sequence
                .checked_add(1)
                .ok_or(AccessLogError::Unforeseen { sequence })?;
            let digest = link_digest(&previous, sequence, &sha256_hex(content));
            let signature = signer.attest(digest.as_bytes())?;
            previous.clone_from(&digest);
            foreseen.push((digest, signature));
        }
        Ok(foreseen)
    }

    /// The records whose `ehr_id` is `ehr_id`, in chain order, each with its
    /// link.
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`AccessLogError::Unreadable`] when a stored record does not read.
    pub async fn of_ehr(&self, ehr_id: &str) -> Result<Vec<AccessLogRecord>, AccessLogError> {
        let rows = sqlx::query(
            "SELECT chain_seq, content, algorithm, previous_digest, content_digest, digest, \
             signature FROM audit.access_log_record WHERE ehr_id = $1 ORDER BY chain_seq",
        )
        .bind(ehr_id)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?;
        rows.iter().map(stored).collect()
    }

    /// The links from `from` to `to`, both included, without the records'
    /// content: what a verifier of the records between needs.
    ///
    /// # Errors
    /// [`AccessLogError::Interleaved`] when a position between holds no
    /// access-log record, and the store errors when the statement fails.
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
        let row = sqlx::query(
            "SELECT chain_seq, content, algorithm, previous_digest, content_digest, digest, \
             signature FROM audit.access_log_record WHERE uid = $1",
        )
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
    /// `recorded`, the entry that records the change to the log, is appended
    /// in the same transaction, so the log holds the tombstone and the record
    /// of its making or neither.
    ///
    /// # Errors
    /// [`AccessLogError::Invalid`] for a tombstone or a record that breaks an
    /// invariant, and the store and read errors of [`Self::read`].
    pub async fn tombstone(
        &self,
        uid: &str,
        time_removed: DvDateTime,
        grounds: NonEmptyVec<DvCodedText>,
        recorded: Option<Box<AccessLogEntry>>,
    ) -> Result<Option<AccessLogTombstone>, AccessLogError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        let row = sqlx::query(
            "SELECT chain_seq, content, algorithm, previous_digest, content_digest, digest, \
             signature FROM audit.access_log_record \
             WHERE uid = $1 AND record_type = 'ACCESS_LOG_ENTRY' FOR UPDATE",
        )
        .bind(uid)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let entry: AccessLogEntry = stored(&row)?;
        let tombstone = tombstone_of(entry, time_removed, grounds);
        let replaced = replace(&mut tx, &[(row.try_get("chain_seq")?, &tombstone)]).await?;
        self.append_in(&mut tx, recorded.into_iter().map(|entry| *entry).collect())
            .await?;
        tx.commit().await?;
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
        let rows = sqlx::query(
            "SELECT chain_seq, content, algorithm, previous_digest, content_digest, digest, \
             signature FROM audit.access_log_record \
             WHERE record_type = 'ACCESS_LOG_ENTRY' AND retain_until < $1::text::date \
             ORDER BY retain_until, chain_seq LIMIT $2 FOR UPDATE SKIP LOCKED",
        )
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
fn tombstone_of(
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
        "UPDATE audit.access_log_record r SET content = t.content \
         FROM unnest($1::bigint[], $2::text[]) AS t(chain_seq, content) \
         WHERE r.chain_seq = t.chain_seq",
    )
    .bind(&positions)
    .bind(&contents)
    .execute(&mut *tx)
    .await?;
    Ok(true)
}

/// The link the columns of `row` hold.
pub(super) fn link(row: &PgRow) -> Result<AccessLogLink, sqlx::Error> {
    Ok(AccessLogLink {
        sequence: row.try_get("chain_seq")?,
        algorithm: row.try_get("algorithm")?,
        previous_digest: row.try_get("previous_digest")?,
        content_digest: row.try_get("content_digest")?,
        digest: row.try_get("digest")?,
        signature: row.try_get("signature")?,
    })
}

/// The record `row` holds, read as a `T`: its canonical form with its link
/// joined. The row holds `chain_seq`, `content` and the link's columns.
///
/// # Errors
/// [`AccessLogError::Unreadable`] when the record does not read.
pub(crate) fn stored<T: serde::de::DeserializeOwned>(row: &PgRow) -> Result<T, AccessLogError> {
    let chain_seq: i64 = row.try_get("chain_seq")?;
    let unreadable = |source| AccessLogError::Unreadable { chain_seq, source };
    let content: String = row.try_get("content")?;
    let mut value: Value = serde_json::from_str(&content).map_err(unreadable)?;
    let link = serde_json::to_value(link(row)?).map_err(unreadable)?;
    if let Value::Object(members) = &mut value {
        members.insert(LINK.to_owned(), link);
    }
    serde_json::from_value(value).map_err(unreadable)
}
