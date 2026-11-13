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
    /// The keyring that signs each new link's digest, when the deployment
    /// asks for it (`[audit.access_log] sign_links`).
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

    /// The store that signs the digest of each link it appends with
    /// `signer`'s key, the optional `ACCESS_LOG_LINK.signature`.
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
    /// # Errors
    /// As [`Self::append`], and [`AccessLogError::Sign`] when a link is to be
    /// signed and the keyring cannot sign it.
    pub async fn append_in(
        &self,
        conn: &mut PgConnection,
        mut entries: Vec<AccessLogEntry>,
    ) -> Result<Vec<AccessLogEntry>, AccessLogError> {
        let contents = entries
            .iter()
            .map(canonical_form)
            .collect::<Result<Vec<_>, _>>()?;
        // NOTE: the ordinality orders the insert, which is the order the row trigger links the records in.
        let rows = sqlx::query(
            "INSERT INTO audit.access_log_record (content) \
             SELECT t.content FROM unnest($1::text[]) WITH ORDINALITY AS t(content, n) ORDER BY t.n \
             RETURNING uid, chain_seq, algorithm, previous_digest, content_digest, digest, signature",
        )
        .bind(&contents)
        .fetch_all(&mut *conn)
        .await?;
        let mut links = rows
            .iter()
            .map(|row| Ok((row.try_get::<String, _>("uid")?, link(row)?)))
            .collect::<Result<BTreeMap<_, _>, sqlx::Error>>()?;
        for entry in &mut entries {
            entry.link =
                links
                    .remove(entry.uid.value())
                    .ok_or_else(|| AccessLogError::Unlinked {
                        uid: entry.uid.value().to_owned(),
                    })?;
            if let Some(signer) = &self.link_signer {
                let signature = signer.attest(entry.link.digest.as_bytes())?;
                // NOTE: the trigger computes the digest a signature signs, so the link is signed once it exists.
                sqlx::query(
                    "UPDATE audit.access_log_record SET signature = $2 WHERE chain_seq = $1",
                )
                .bind(entry.link.sequence)
                .bind(&signature)
                .execute(&mut *conn)
                .await?;
                entry.link.signature = Some(signature);
            }
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
        let rows = sqlx::query(
            "SELECT chain_seq, algorithm, previous_digest, content_digest, digest, signature \
             FROM audit.access_log_record WHERE chain_seq BETWEEN $1 AND $2 ORDER BY chain_seq",
        )
        .bind(from)
        .bind(to)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?;
        let links = rows.iter().map(link).collect::<Result<Vec<_>, _>>()?;
        let expected = to.checked_sub(from).and_then(|span| span.checked_add(1));
        if i64::try_from(links.len()).ok() != expected {
            return Err(AccessLogError::Interleaved { from, to });
        }
        Ok(links)
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
    /// replaced.
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
        let chain_seq: i64 = row.try_get("chain_seq")?;
        let entry: AccessLogEntry = stored(&row)?;
        let tombstone = AccessLogTombstone {
            uid: entry.uid,
            system_id: entry.system_id,
            time_recorded: entry.time_recorded,
            link: entry.link,
            time_removed,
            grounds,
        };
        check_tombstone(&tombstone)?;
        sqlx::query("UPDATE audit.access_log_record SET content = $2 WHERE chain_seq = $1")
            .bind(chain_seq)
            .bind(canonical_form(&tombstone)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(tombstone))
    }
}

/// The link the columns of `row` hold.
fn link(row: &PgRow) -> Result<AccessLogLink, sqlx::Error> {
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
/// joined.
fn stored<T: serde::de::DeserializeOwned>(row: &PgRow) -> Result<T, AccessLogError> {
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
