// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The retention and verification of the access logs imported from other
//! systems (#5664).
//!
//! An imported entry keeps the `retain_until` it states
//! (`docs/plans/access-log.md` §Retention), and the retention run replaces it
//! by its tombstone once that day has passed, as a local one is. Its
//! tombstone is kept as long again. Within each span between two imported
//! checkpoints of a system, the positions before the first record still kept
//! then fold into one `access_log_imported_gap` per run that holds a
//! tombstone, the links of other EHRs' records with them, so every kept
//! imported record stays hash-chained to the checkpoint after it whether or
//! not the exporter signed its links.
//!
//! The stored imported log verifies per imported checkpoint through
//! [`chain::verify`], the verifier an import runs before it stores anything.
//! No openEHR spec governs audit retention: our own design.

use std::collections::BTreeMap;

use sqlx::{Connection, Row};

use crate::access_log::chain::{self, ChainFinding, Span};
use crate::access_log::checkpoint::{self, Checkpoints};
use crate::access_log::gap::{ChainGap, gap};
use crate::access_log::store::{
    AccessLogError, AccessLogStore, canonical_form, link, stored, tombstone_of,
};
use crate::db::acquire::acquire;
use openehr_rm::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::draft::access_log::invariants::check_tombstone;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;

/// The imported records joined with their links, in the columns [`stored`]
/// reads.
macro_rules! imported_records {
    () => {
        "SELECT r.system_id, l.sequence AS chain_seq, r.content, l.algorithm, \
         l.previous_digest, l.content_digest, l.digest, l.signature \
         FROM audit.access_log_imported_record r JOIN audit.access_log_imported_link l \
         ON l.system_id = r.system_id AND l.sequence = r.sequence"
    };
}

/// One thing an imported log fails against one of its checkpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFinding {
    /// The system the imported log belongs to.
    pub system_id: String,
    /// The sequence of the imported checkpoint the finding is against.
    pub checkpoint: i64,
    /// What fails.
    pub finding: ChainFinding,
}

impl std::fmt::Display for ImportedFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the log imported from {:?}, against its checkpoint at {}: {}",
            self.system_id, self.checkpoint, self.finding
        )
    }
}

impl AccessLogStore {
    /// Replace up to `limit` imported entries whose retention ended before
    /// `today` by their tombstones, removed at `time_removed`, in one
    /// transaction, and return how many were replaced.
    ///
    /// Each tombstone keeps the entry's uid, `system_id`, `time_recorded` and
    /// its link in the exporting system's chain, and names the grounds of
    /// the period that ended. Rows another run holds are skipped, so two
    /// replicas never replace the same entry; a caller repeats the call until
    /// fewer than `limit` come back.
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
        let rows = sqlx::query(concat!(
            imported_records!(),
            " WHERE r.record_type = 'ACCESS_LOG_ENTRY' AND r.retain_until < $1::text::date \
             ORDER BY r.retain_until, r.system_id, r.sequence LIMIT $2 \
             FOR UPDATE OF r SKIP LOCKED",
        ))
        .bind(today.to_string())
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await?;
        let mut systems = Vec::with_capacity(rows.len());
        let mut sequences = Vec::with_capacity(rows.len());
        let mut contents = Vec::with_capacity(rows.len());
        for row in &rows {
            let entry: AccessLogEntry = stored(row)?;
            let grounds = entry.retention.grounds.clone();
            let tombstone = tombstone_of(entry, time_removed.clone(), grounds);
            check_tombstone(&tombstone)?;
            systems.push(row.try_get::<String, _>("system_id")?);
            sequences.push(row.try_get::<i64, _>("chain_seq")?);
            contents.push(canonical_form(&tombstone)?);
        }
        if !rows.is_empty() {
            sqlx::query(
                "UPDATE audit.access_log_imported_record r SET content = t.content \
                 FROM unnest($1::text[], $2::bigint[], $3::text[]) \
                   AS t(system_id, sequence, content) \
                 WHERE r.system_id = t.system_id AND r.sequence = t.sequence",
            )
            .bind(&systems)
            .bind(&sequences)
            .bind(&contents)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(u64::try_from(rows.len()).unwrap_or(u64::MAX))
    }
}

/// One exporting system's imported log as stored.
struct Stored {
    checkpoints: Vec<AccessLogCheckpoint>,
    links: BTreeMap<i64, AccessLogLink>,
    gaps: Vec<ChainGap>,
    records: Vec<AccessLogRecord>,
}

impl Checkpoints {
    /// Verify every imported log against each of its checkpoints.
    ///
    /// For each imported checkpoint, the span from the checkpoint before it
    /// (of the same system) to it is read back from the stored links, gaps
    /// and records and verified by [`chain::verify`]: each link recomputes
    /// and links to the one before it, the last link or gap is the one the
    /// checkpoint seals, the checkpoint and every signed link and gap verify
    /// under the keyring and the trusted certificates, and every record in
    /// the span carries the span's link over its own content. Where an
    /// import stored a link and a gap for the same positions, the link is
    /// read, so a kept record keeps its link in the span.
    ///
    /// # Errors
    /// The store errors, and [`AccessLogError::Unreadable`] when a stored
    /// record, gap or checkpoint does not read.
    pub async fn verify_imported(&self) -> Result<Vec<ImportedFinding>, AccessLogError> {
        let systems: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT system_id FROM audit.access_log_imported_checkpoint \
             ORDER BY system_id",
        )
        .fetch_all(&mut *acquire(self.pool()).await?)
        .await?;
        let mut findings = Vec::new();
        for system_id in systems {
            let log = self.stored_import(&system_id).await?;
            let mut after = 0_i64;
            for checkpoint in &log.checkpoints {
                let (links, gaps) = cover(&log, after, checkpoint.sequence);
                let records: Vec<AccessLogRecord> = log
                    .records
                    .iter()
                    .filter(|record| {
                        let sequence = record_sequence(record);
                        sequence > after && sequence <= checkpoint.sequence
                    })
                    .cloned()
                    .collect();
                let span = Span {
                    records: &records,
                    links: &links,
                    gaps: &gaps,
                    checkpoint,
                };
                let found = chain::verify(span, &|data, signature| self.signed(data, signature))?;
                findings.extend(found.into_iter().map(|finding| ImportedFinding {
                    system_id: system_id.clone(),
                    checkpoint: checkpoint.sequence,
                    finding,
                }));
                after = checkpoint.sequence;
            }
        }
        Ok(findings)
    }

    /// The imported log of `system_id` as stored.
    async fn stored_import(&self, system_id: &str) -> Result<Stored, AccessLogError> {
        let mut conn = acquire(self.pool()).await?;
        let checkpoints = sqlx::query(
            "SELECT sequence, content FROM audit.access_log_imported_checkpoint \
             WHERE system_id = $1 ORDER BY sequence",
        )
        .bind(system_id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(|row| {
            checkpoint::read(
                row.try_get("sequence")?,
                &row.try_get::<String, _>("content")?,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
        let links = sqlx::query(
            "SELECT sequence AS chain_seq, algorithm, previous_digest, content_digest, digest, \
             signature FROM audit.access_log_imported_link WHERE system_id = $1 \
             ORDER BY sequence",
        )
        .bind(system_id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(|row| link(row).map(|link| (link.sequence, link)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
        let gaps = sqlx::query(
            "SELECT first_seq, last_seq, link, signature, grounds::text AS grounds, time_folded \
             FROM audit.access_log_imported_gap WHERE system_id = $1 \
             ORDER BY last_seq, first_seq",
        )
        .bind(system_id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(gap)
        .collect::<Result<Vec<_>, _>>()?;
        let records = sqlx::query(concat!(
            imported_records!(),
            " WHERE r.system_id = $1 ORDER BY r.sequence"
        ))
        .bind(system_id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(stored)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(Stored {
            checkpoints,
            links,
            gaps,
            records,
        })
    }
}

/// The sequence of a record's link.
fn record_sequence(record: &AccessLogRecord) -> i64 {
    match record {
        AccessLogRecord::AccessLogEntry(entry) => entry.link.sequence,
        AccessLogRecord::AccessLogTombstone(tombstone) => tombstone.link.sequence,
    }
}

/// The links and gaps that hold the positions after `after` up to `last`,
/// walked back from `last`: the link at a position where there is one, else
/// the shortest gap that ends there. A position neither holds while
/// something is still stored before it in the span is a hole: everything
/// stored there is added, so the verifier reports it.
fn cover(log: &Stored, after: i64, last: i64) -> (Vec<AccessLogLink>, Vec<ChainGap>) {
    let mut links = Vec::new();
    let mut gaps = Vec::new();
    let mut position = last;
    while position > after {
        if let Some(link) = log.links.get(&position) {
            links.push(link.clone());
            position = position.saturating_sub(1);
            continue;
        }
        let shortest = log
            .gaps
            .iter()
            .filter(|gap| gap.last_sequence == position)
            .max_by_key(|gap| gap.first_sequence);
        let Some(gap) = shortest else {
            break;
        };
        gaps.push(gap.clone());
        position = gap.first_sequence.saturating_sub(1);
    }
    if position > after {
        links.extend(
            log.links
                .range(after.saturating_add(1)..=position)
                .map(|(_, link)| link.clone()),
        );
        gaps.extend(
            log.gaps
                .iter()
                .filter(|gap| gap.last_sequence > after && gap.last_sequence <= position)
                .cloned(),
        );
    }
    links.sort_by_key(|link| link.sequence);
    gaps.sort_by_key(|gap| (gap.last_sequence, gap.first_sequence));
    (links, gaps)
}
