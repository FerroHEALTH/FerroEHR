// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-log store: each record kept as its canonical form, chained by
//! its `ACCESS_LOG_LINK`, and an entry replaced in place by its tombstone
//! (#5125).
//!
//! The draft's integrity rules (`docs/plans/access-log/antora/modules/
//! access_log/pages/semantics.adoc` §Integrity) fix the chain: the canonical
//! form of a record is its ITS-JSON serialisation without its link,
//! canonicalised by RFC 8785; `content_digest` is the SHA-256 of that form and
//! `digest` the SHA-256 of `previous_digest`, the sequence and
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

use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_object;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{Connection, PgPool, Row};

use crate::access_log_draft::access_log::access_log_entry::AccessLogEntry;
use crate::access_log_draft::access_log::access_log_link::AccessLogLink;
use crate::access_log_draft::access_log::access_log_record::AccessLogRecord;
use crate::access_log_draft::access_log::access_log_tombstone::AccessLogTombstone;
use crate::access_log_invariants::{Violations, check_entry, check_tombstone};
use crate::db::acquire::{AcquireError, acquire};

/// The member of a record's ITS-JSON form its canonical form leaves out.
const LINK: &str = "link";

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
    /// The store wrote an entry and returned no link for its uid.
    #[error("the access-log store returned no link for the entry {uid}")]
    Unlinked {
        /// The entry's uid.
        uid: String,
    },
}

/// The canonical form of an access-log record: its ITS-JSON serialisation
/// without its link, canonicalised by RFC 8785.
///
/// The link is left out whatever it holds, since the chain computes it over
/// this form.
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
                .filter(|(name, _)| name.as_str() != LINK)
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
}

impl AccessLogStore {
    /// A store over the audit pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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
        mut entries: Vec<AccessLogEntry>,
    ) -> Result<Vec<AccessLogEntry>, AccessLogError> {
        if entries.is_empty() {
            return Ok(entries);
        }
        let contents = entries
            .iter()
            .map(canonical_form)
            .collect::<Result<Vec<_>, _>>()?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        // NOTE: the ordinality orders the insert, which is the order the row trigger links the records in.
        let rows = sqlx::query(
            "INSERT INTO audit.access_log_record (content) \
             SELECT t.content FROM unnest($1::text[]) WITH ORDINALITY AS t(content, n) ORDER BY t.n \
             RETURNING uid, chain_seq, algorithm, previous_digest, content_digest, digest, signature",
        )
        .bind(&contents)
        .fetch_all(&mut *tx)
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
        }
        for entry in &entries {
            check_entry(entry)?;
        }
        tx.commit().await?;
        Ok(entries)
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
