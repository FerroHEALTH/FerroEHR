// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The retention and verification of the access logs imported from other
//! systems (#5664, #6242).
//!
//! An imported entry keeps the `retain_until` it states, and the retention
//! run replaces it by its tombstone once that day has passed, as a local one
//! is; its tombstone is kept as long again and then folds, leaving its uid,
//! bundle and content digest in `access_log_imported_folded`. Each stored
//! bundle verifies by its records digest over its records and folded
//! tombstones, by each entry's content digest, and by the import's own entry,
//! which a seal of this system covers and which names the bundle's digest and
//! records digest. No openEHR spec governs audit retention: our own design.

use sqlx::{Connection, Row};

use crate::access_log::chain::{ChainFinding, records_digest};
use crate::access_log::checkpoint::Checkpoints;
use crate::access_log::store::{AccessLogError, AccessLogStore, canonical_form, tombstone_of};
use crate::db::acquire::acquire;
use crate::digest::sha256_hex;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::invariants::check_tombstone;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;

/// One thing a stored imported bundle fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFinding {
    /// The system the imported log belongs to.
    pub system_id: String,
    /// The bundle's digest.
    pub bundle: String,
    /// What fails.
    pub finding: ImportedFailure,
}

/// What a stored imported bundle fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportedFailure {
    /// Its records and folded tombstones no longer give its records digest.
    Records,
    /// No entry of this system's log names the bundle and its records digest.
    Unrecorded,
    /// What a record fails.
    Record(ChainFinding),
}

impl std::fmt::Display for ImportedFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match &self.finding {
            ImportedFailure::Records => {
                "a record was changed, removed or inserted since the import".to_owned()
            }
            ImportedFailure::Unrecorded => "no entry of this log records its import".to_owned(),
            ImportedFailure::Record(finding) => finding.to_string(),
        };
        write!(
            f,
            "the log imported from {:?} in the bundle {}: {what}",
            self.system_id, self.bundle
        )
    }
}

impl AccessLogStore {
    /// Replace up to `limit` imported entries whose retention ended before
    /// `today` by their tombstones, removed at `time_removed`, in one
    /// transaction, and return how many were replaced.
    ///
    /// Each tombstone keeps the entry's uid, `system_id` and `time_recorded`,
    /// and the row keeps its content digest. Rows another run holds are
    /// skipped; a caller repeats the call until fewer than `limit` come back.
    ///
    /// # Errors
    /// [`AccessLogError::Invalid`] for a tombstone that breaks an invariant,
    /// [`AccessLogError::Unreadable`] for a stored entry that does not read,
    /// and the store errors.
    pub async fn tombstone_expired_imported(
        &self,
        today: jiff::civil::Date,
        time_removed: &DvDateTime,
        limit: u16,
    ) -> Result<u64, AccessLogError> {
        let mut conn = acquire(self.pool()).await?;
        let mut tx = conn.begin().await?;
        let rows = sqlx::query(
            "SELECT r.system_id, r.uid, r.content FROM audit.access_log_imported_record r \
             WHERE r.record_type = 'ACCESS_LOG_ENTRY' AND r.retain_until < $1::text::date \
             ORDER BY r.retain_until, r.system_id, r.uid LIMIT $2 FOR UPDATE SKIP LOCKED",
        )
        .bind(today.to_string())
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await?;
        let mut systems = Vec::with_capacity(rows.len());
        let mut uids = Vec::with_capacity(rows.len());
        let mut contents = Vec::with_capacity(rows.len());
        for row in &rows {
            let content: String = row.try_get("content")?;
            let entry: AccessLogEntry =
                serde_json::from_str(&content).map_err(|source| AccessLogError::Unreadable {
                    chain_seq: 0,
                    source,
                })?;
            let grounds = entry.retention.grounds.clone();
            let tombstone = tombstone_of(entry, time_removed.clone(), grounds);
            check_tombstone(&tombstone)?;
            systems.push(row.try_get::<String, _>("system_id")?);
            uids.push(row.try_get::<String, _>("uid")?);
            contents.push(canonical_form(&tombstone)?);
        }
        if !rows.is_empty() {
            sqlx::query(
                "UPDATE audit.access_log_imported_record r SET content = t.content \
                 FROM unnest($1::text[], $2::text[], $3::text[]) AS t(system_id, uid, content) \
                 WHERE r.system_id = t.system_id AND r.uid = t.uid",
            )
            .bind(&systems)
            .bind(&uids)
            .bind(&contents)
            .execute(&mut *tx)
            .await?;
        }
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this
        // transaction's rows.
        tx.commit().await?;
        Ok(u64::try_from(rows.len()).unwrap_or(u64::MAX))
    }
}

impl Checkpoints {
    /// Verify every stored imported bundle: its records digest over its
    /// records and folded tombstones, each entry's content digest, and the
    /// entry of this system's log that names it.
    ///
    /// # Errors
    /// The store errors when a statement fails; a finding is never an error.
    pub async fn verify_imported(&self) -> Result<Vec<ImportedFinding>, AccessLogError> {
        let mut conn = acquire(self.pool()).await?;
        let bundles: Vec<(String, String, String, uuid::Uuid)> = sqlx::query_as(
            "SELECT digest, system_id, records_digest, entry_uid \
             FROM audit.access_log_imported_bundle ORDER BY imported_at, digest",
        )
        .fetch_all(&mut *conn)
        .await?;
        let mut findings = Vec::new();
        for (bundle, system_id, records, entry_uid) in bundles {
            let finding = |finding| ImportedFinding {
                system_id: system_id.clone(),
                bundle: bundle.clone(),
                finding,
            };
            let rows: Vec<(String, String, String, String)> = sqlx::query_as(
                "SELECT uid, content_digest, record_type, content \
                 FROM audit.access_log_imported_record WHERE bundle = $1 \
                 UNION ALL \
                 SELECT uid, content_digest, 'FOLDED', '' \
                 FROM audit.access_log_imported_folded WHERE bundle = $1",
            )
            .bind(&bundle)
            .fetch_all(&mut *conn)
            .await?;
            let mut pairs = Vec::with_capacity(rows.len());
            for (uid, digest, record_type, content) in rows {
                if record_type == "ACCESS_LOG_ENTRY" && sha256_hex(&content) != digest {
                    findings.push(finding(ImportedFailure::Record(
                        ChainFinding::ContentDigest { uid: uid.clone() },
                    )));
                }
                pairs.push((uid, digest));
            }
            if records_digest(&pairs) != records {
                findings.push(finding(ImportedFailure::Records));
            }
            let marker = format!("access-log-bundle:{bundle}:{records}");
            let recorded: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM audit.access_log_entry e \
                 WHERE e.uid = $1 AND e.record_type = 'ACCESS_LOG_ENTRY' \
                   AND strpos(e.content, $2) > 0)",
            )
            .bind(entry_uid)
            .bind(&marker)
            .fetch_one(&mut *conn)
            .await?;
            if !recorded {
                findings.push(finding(ImportedFailure::Unrecorded));
            }
        }
        Ok(findings)
    }
}
