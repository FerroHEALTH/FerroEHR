// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The node rebuild: the repair the storage-parity sweep had no counterpart
//! for.
//!
//! NOTE: no openEHR spec governs storage mechanics — our own design/extension;
//! the storage keeps a version's content twice, as `version.body` and as the
//! decomposed `node` rows, and the sweep ([`super`]) reports where the two
//! disagree — in content, or in the shape the rows were decomposed into.
//!
//! Every defect the sweep reports is repaired the same way, because there is
//! only one repair: the rows are written again from the body. That covers a
//! [`StorageParityDefect::StaleDecomposition`] exactly as it covers damage,
//! since writing the rows again is what re-decomposing an old shape into the
//! current one takes. A release that changes the decomposition is therefore an
//! upgrade an operator performs and verifies.
//!
//! Which copy is authoritative is not a choice this module makes, it is a
//! property of the storage: `body` is the version's canonical serialized form,
//! the bytes the digital signature was taken over (RM common
//! `master06-change_control_package.adoc` §Digital Signature) and the bytes a
//! point read serves; the node rows are a derived index over it, produced by
//! `crate::storage::codec::decompose`. So a disagreement is repaired in one
//! direction only — the rows are re-derived from the body — and a version whose
//! BODY is the damaged copy is refused rather than propagated into the index.
//!
//! Dump and load re-decomposes too, but cannot serve as the repair: its dump
//! side reads each version through `node_repo::read_version_canonical`, from
//! the node rows themselves, so it would reproduce exactly the damage it is
//! asked to fix. It is also a whole-repository round trip.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use std::time::Instant;

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::admin::integrity::{
    StorageDomain, StorageParityDefect, StorageParityEvent, StorageParityScope,
};
use crate::service::error::ServiceError;
use crate::service::status::SmError;
use crate::storage::codec::decompose;
use crate::storage::node_repo::{delete_version_nodes, read_version_canonical_tx, write_nodes};
use crate::storage::version_repo::tier;

/// How many rebuild records the returned report carries in full.
///
/// The same cap the sweep applies to its mismatches, for the same reason:
/// every record is also logged, so nothing is lost when a repair of a badly
/// damaged store passes it.
const MAX_REPORTED_RECORDS: usize = 1000;

/// What happened to one version the rebuild reached.
///
/// NOTE: no openEHR spec governs storage mechanics — our own design/extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeRebuildOutcome {
    /// The node rows were replaced from the body, and the version re-derived
    /// from the new rows equals that body. The count is how many rows the
    /// rebuild wrote, which is zero for a logical delete.
    Rebuilt {
        /// How many `node` rows the version now has.
        node_rows: u32,
    },
    /// Nothing was written. The version's own body is the damaged copy, so
    /// re-deriving the index from it would spread the damage rather than
    /// repair it.
    Refused {
        /// What is wrong with the body, in one line.
        reason: String,
    },
}

impl NodeRebuildOutcome {
    /// Returns the stable wire token for this outcome.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match *self {
            Self::Rebuilt { .. } => "rebuilt",
            Self::Refused { .. } => "refused",
        }
    }
}

/// One damaged version the rebuild reached, and what happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRebuildRecord {
    /// Which domain's storage the version lives in.
    pub domain: StorageDomain,
    /// The versioned object the version belongs to.
    pub vo_id: Uuid,
    /// The per-object storage commit ordinal of the version.
    pub sys_version: i32,
    /// The `version.kind` discriminator (`COMPOSITION` / `EHR_STATUS` /
    /// `FOLDER` / a demographic PARTY class / …).
    pub kind: String,
    /// What the sweep found, which is what selected this version.
    pub defect: StorageParityDefect,
    /// What the rebuild did about it.
    pub outcome: NodeRebuildOutcome,
}

/// The outcome of one node rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRebuildReport {
    /// How many stored versions the underlying sweep read.
    pub versions_checked: u64,
    /// How many of them it found damaged, whatever `records` holds.
    pub versions_damaged: u64,
    /// How many were rebuilt.
    pub versions_rebuilt: u64,
    /// How many were refused because their body is the damaged copy.
    pub versions_refused: u64,
    /// The per-version records, up to the reporting cap.
    pub records: Vec<NodeRebuildRecord>,
    /// Whether `records` was cut short by the cap.
    pub truncated: bool,
    /// Wall-clock duration of the rebuild, in milliseconds.
    pub elapsed_ms: u64,
}

impl NodeRebuildReport {
    /// Returns `true` when the rebuild repaired everything it found.
    ///
    /// A run that found nothing is complete: the scope was already clean.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.versions_refused == 0
    }
}

/// One stored version's body plus the storage context its rows carry, read
/// under the transaction's row lock.
struct VersionBody {
    ehr_id: Option<EhrId>,
    /// `None` for a logical delete (data Void, RM common
    /// `master06-change_control_package.adoc` §Logical Deletion), which
    /// rebuilds to zero node rows.
    body: Option<String>,
}

impl FerroEhrService {
    /// Re-derives the `node` rows of every damaged version in `scope` from its
    /// `version.body`, one transaction per version.
    ///
    /// The scope is the sweep's own ([`StorageParityScope`]): the whole
    /// repository, one EHR, one versioned object, one version of it, or
    /// everything committed since an instant. Whatever the scope covers, only
    /// the versions the sweep reports are written — a clean version is read and
    /// left alone, so running this over a healthy repository writes nothing.
    /// That includes a version reported
    /// [`StorageParityDefect::StaleDecomposition`], whose rows are intact but
    /// in the shape an older decomposition wrote: writing them again from the
    /// body is what brings it to the current one.
    ///
    /// Each version is repaired in its own transaction: the body is read under
    /// a row lock, decomposed, the old rows deleted, the new set inserted, and
    /// the version re-derived from what was just written and compared with the
    /// body BEFORE the commit. A body that does not decompose, or that the
    /// rebuilt rows do not reproduce, rolls that transaction back, so the
    /// version's rows are left exactly as they were and the refusal is
    /// reported. One refused version never stops the rebuild that found it.
    ///
    /// An ARCHIVED object is thawed into the primary tier for the repair and
    /// re-frozen with its archive marker in the same transaction, because the
    /// tier's own rule is that a write always thaws first
    /// (`crate::storage::version_repo::tier`) — a versioned object is never
    /// split across tiers.
    ///
    /// NOTE: no openEHR spec governs storage mechanics — our own
    /// design/extension (the module docs carry the full derivation).
    ///
    /// # Errors
    /// - `exception` — a database fault while enumerating or reading versions.
    ///   A version the rebuild refuses is a REPORTED record, not an error.
    pub async fn rebuild_version_nodes(
        &self,
        scope: StorageParityScope,
    ) -> Result<NodeRebuildReport, SmError> {
        Ok(self.collect_node_rebuild(scope).await?)
    }

    /// Drive the sweep, repairing each version it reports.
    async fn collect_node_rebuild(
        &self,
        scope: StorageParityScope,
    ) -> Result<NodeRebuildReport, ServiceError> {
        let started = Instant::now();
        let mut sweep = self.storage_parity_sweep(scope);
        let mut report = NodeRebuildReport {
            versions_checked: 0,
            versions_damaged: 0,
            versions_rebuilt: 0,
            versions_refused: 0,
            records: Vec::new(),
            truncated: false,
            elapsed_ms: 0,
        };
        while let Some(events) = sweep.next_batch().await? {
            for event in events {
                match event {
                    StorageParityEvent::Mismatch(mismatch) => {
                        let outcome = self
                            .rebuild_one_version(
                                mismatch.domain,
                                VoId(mismatch.vo_id),
                                mismatch.sys_version,
                            )
                            .await?;
                        report.versions_damaged += 1;
                        match outcome {
                            NodeRebuildOutcome::Rebuilt { .. } => report.versions_rebuilt += 1,
                            NodeRebuildOutcome::Refused { .. } => report.versions_refused += 1,
                        }
                        // The log line carries identifiers and tokens only: a
                        // body fragment would put clinical content into an
                        // operational log.
                        tracing::info!(
                            domain = mismatch.domain.as_str(),
                            vo_id = %mismatch.vo_id,
                            sys_version = mismatch.sys_version,
                            kind = %mismatch.kind,
                            defect = mismatch.defect.as_str(),
                            outcome = outcome.as_str(),
                            "node rebuild: a damaged stored version was re-derived from its body"
                        );
                        if report.records.len() < MAX_REPORTED_RECORDS {
                            report.records.push(NodeRebuildRecord {
                                domain: mismatch.domain,
                                vo_id: mismatch.vo_id,
                                sys_version: mismatch.sys_version,
                                kind: mismatch.kind,
                                defect: mismatch.defect,
                                outcome,
                            });
                        } else {
                            report.truncated = true;
                        }
                    }
                    StorageParityEvent::Progress { .. } => {}
                    StorageParityEvent::Summary { counts, .. } => {
                        report.versions_checked = counts.versions_checked;
                    }
                }
            }
        }
        report.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(report)
    }

    /// Repair one version in one transaction, or leave it untouched.
    async fn rebuild_one_version(
        &self,
        domain: StorageDomain,
        vo_id: VoId,
        sys_version: i32,
    ) -> Result<NodeRebuildOutcome, ServiceError> {
        // The repair runs in the domain that holds the damage: both schemas
        // carry relations of the same names, and the pool's search path is
        // what decides which one this transaction reaches.
        let pool = match domain {
            StorageDomain::Clinical => &self.pool,
            StorageDomain::Demographic => &self.demographic_pool,
        };
        let mut tx = pool.begin().await?;

        // The tier's own rule: a write always thaws first, so a versioned
        // object is never split across tiers. The marker is captured before
        // the thaw drops it and re-inserted before the re-freeze, so an
        // archived object comes out of this exactly as archived as it went in.
        let archive_reason: Option<Option<String>> = sqlx::query_scalar(
            "SELECT archive_reason FROM vo_head WHERE vo_id = $1 AND tier = 'cold' FOR UPDATE",
        )
        .bind(vo_id)
        .fetch_optional(&mut *tx)
        .await?;
        if archive_reason.is_some() {
            tier::thaw(&mut tx, &[vo_id]).await?;
        }

        let outcome = rebuild_in_tx(&mut tx, vo_id, sys_version).await?;

        if matches!(outcome, NodeRebuildOutcome::Refused { .. }) {
            // Nothing this transaction wrote may survive: not the rows, and
            // not the thaw either.
            tx.rollback().await?;
            return Ok(outcome);
        }

        if let Some(reason) = archive_reason {
            tier::freeze(&mut tx, &[vo_id], reason.as_deref()).await?;
        }
        tx.commit().await?;
        Ok(outcome)
    }
}

/// The repair itself, inside the caller's transaction and the primary tier.
///
/// Returns [`NodeRebuildOutcome::Refused`] without propagating an error for
/// every way the BODY can be the damaged copy, because the caller answers a
/// refusal by rolling back rather than by failing the whole run.
async fn rebuild_in_tx(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
) -> Result<NodeRebuildOutcome, ServiceError> {
    let Some(version) = locked_body(tx, vo_id, sys_version).await? else {
        return Ok(NodeRebuildOutcome::Refused {
            reason: "the version is no longer stored".to_owned(),
        });
    };

    // A logical delete stores no body and must therefore have no node rows;
    // re-deriving it from nothing is exactly the delete below.
    let Some(text) = version.body else {
        delete_version_nodes(tx, vo_id, sys_version).await?;
        return Ok(NodeRebuildOutcome::Rebuilt { node_rows: 0 });
    };

    let body: Value = match serde_json::from_str(&text) {
        Ok(body) => body,
        Err(e) => {
            return Ok(NodeRebuildOutcome::Refused {
                reason: format!("the stored body is not valid JSON: {e}"),
            });
        }
    };
    let rows = match decompose(body.clone()) {
        Ok(rows) => rows,
        Err(e) => {
            return Ok(NodeRebuildOutcome::Refused {
                reason: format!("the stored body does not decompose: {e}"),
            });
        }
    };
    let node_rows = u32::try_from(rows.len()).unwrap_or(u32::MAX);

    // The row set is replaced wholesale: a nested-set numbering is only
    // meaningful as a whole (`crate::storage::codec`).
    delete_version_nodes(tx, vo_id, sys_version).await?;
    write_nodes(tx, vo_id, sys_version, version.ehr_id, &rows).await?;

    // The parity comparison the sweep runs, over the rows just written and
    // before they are committed. A rebuild that did not restore parity must
    // not report success, and the read is transaction-scoped precisely so it
    // sees this transaction's own uncommitted rows.
    match read_version_canonical_tx(tx, vo_id, sys_version).await {
        Ok(rebuilt) if rebuilt == body => Ok(NodeRebuildOutcome::Rebuilt { node_rows }),
        Ok(_) => Ok(NodeRebuildOutcome::Refused {
            reason: "the rows re-derived from the stored body do not reproduce it".to_owned(),
        }),
        Err(e) => Ok(NodeRebuildOutcome::Refused {
            reason: format!("the rebuilt rows do not reassemble: {e}"),
        }),
    }
}

/// Read one primary-tier version's body and storage context under a row lock,
/// so a concurrent commit on the same version cannot interleave with the
/// replacement of its rows.
async fn locked_body(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
) -> Result<Option<VersionBody>, ServiceError> {
    let row: Option<(Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT ehr_id, body FROM version \
         WHERE vo_id = $1 AND sys_version = $2 FOR UPDATE",
    )
    .bind(vo_id)
    .bind(sys_version)
    .fetch_optional(&mut *tx)
    .await?;
    Ok(row.map(|(ehr_id, body)| VersionBody {
        ehr_id: ehr_id.map(EhrId),
        body,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_tokens_are_the_documented_wire_values() {
        assert_eq!(
            NodeRebuildOutcome::Rebuilt { node_rows: 7 }.as_str(),
            "rebuilt"
        );
        assert_eq!(
            NodeRebuildOutcome::Refused {
                reason: "x".to_owned()
            }
            .as_str(),
            "refused"
        );
    }

    #[test]
    fn a_report_with_no_refusal_is_complete() {
        let mut report = NodeRebuildReport {
            versions_checked: 4,
            versions_damaged: 2,
            versions_rebuilt: 2,
            versions_refused: 0,
            records: Vec::new(),
            truncated: false,
            elapsed_ms: 1,
        };
        assert!(report.is_complete());
        report.versions_refused = 1;
        assert!(!report.is_complete());
    }
}
