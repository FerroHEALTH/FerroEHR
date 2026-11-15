// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two hash chains of the audit store and their verification (#5573).
//!
//! The access log (`audit.access_log_record`) holds the accesses to EHR data
//! in a chain of its own, from 1 with no gaps; the records outside it
//! (`audit.audit_event`: an authentication, a template, party or application
//! operation) form the system chain, whose retention leaves recorded gaps.
//! Each chain has its own head, locked by its writers, its own verification
//! and its own verification cursor
//! (`migrations/audit/0031_access_log_own_chain.sql`). A record is named by
//! its chain and its position in it. No openEHR spec governs audit tamper
//! detection: our own design.

use jiff_sqlx::Timestamp;
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::digest::sha256_hex;
use crate::system_log::AuditError;
use crate::system_log::store::AuditStore;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json;

/// One of the two hash chains of the audit store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuditChain {
    /// The records outside the access log, `audit.audit_event`.
    System,
    /// The access log, `audit.access_log_record`.
    AccessLog,
}

impl AuditChain {
    /// Both chains, in the order a verification reports them.
    pub const ALL: [Self; 2] = [Self::System, Self::AccessLog];

    /// The chain's name as the store's `chain` columns hold it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::AccessLog => "access_log",
        }
    }

    /// The chain the store's `chain` column names; `None` for a name no
    /// chain has.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|chain| chain.as_str() == name)
    }

    /// The incremental verification of the chain, which moves its cursor.
    const fn incremental(self) -> &'static str {
        match self {
            Self::System => {
                "SELECT chain_seq, record_id, recorded_at, finding \
                 FROM audit.verify_audit_chain_incremental($1)"
            }
            Self::AccessLog => {
                "SELECT chain_seq, record_id, recorded_at, finding \
                 FROM audit.verify_access_log_chain_incremental($1, $2, $3)"
            }
        }
    }
}

/// How much of a hash chain one verification recomputes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyScope {
    /// Every record, from the start of the chain.
    Full,
    /// The records written after the chain's verification cursor.
    SinceCheckpoint,
}

/// One damaged record, or one damaged chain boundary, reported by
/// [`AuditStore::verify_chain_advancing`].
#[derive(Debug, Clone)]
pub struct AuditChainFinding {
    /// The chain the finding is about.
    pub chain: AuditChain,
    /// The chain position the finding is about; `None` when the chain state
    /// row itself is missing.
    pub chain_seq: Option<i64>,
    /// The damaged record's id, when the finding is about a record that is
    /// still present rather than about a gap or a boundary.
    pub record_id: Option<Uuid>,
    /// The damaged record's event time, when known.
    pub recorded_at: Option<jiff::Timestamp>,
    /// What is wrong, in the operator's words.
    pub finding: String,
}

impl AuditStore {
    /// Check the tamper evidence of `chain` over `scope`, and move the
    /// chain's verification cursor to its newest record when the run finds
    /// nothing.
    ///
    /// [`VerifyScope::SinceCheckpoint`] recomputes only the records written
    /// after the newest link an earlier run found intact, so its cost follows
    /// the records written since rather than the size of the chain; a change
    /// to a record before that link needs [`VerifyScope::Full`], the pass
    /// `SELECT * FROM audit.verify_audit_chain()` (the system chain) or
    /// `audit.verify_access_log_chain()` (the access log) makes from `psql`.
    /// Both check the chain head.
    ///
    /// An empty result means the chain is intact. The chain is unkeyed, so
    /// it detects modification and deletion but cannot stop a party with
    /// unrestricted write access from recomputing it wholesale; the signed
    /// seals and checkpoints fix its head against that.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the verification query cannot run, which is
    /// itself a finding, never a pass.
    pub async fn verify_chain_advancing(
        &self,
        chain: AuditChain,
        scope: VerifyScope,
    ) -> Result<Vec<AuditChainFinding>, AuditError> {
        let full = scope == VerifyScope::Full;
        let rows = match chain {
            AuditChain::System => {
                sqlx::query(chain.incremental())
                    .bind(full)
                    .fetch_all(&mut *acquire(&self.pool).await?)
                    .await?
            }
            AuditChain::AccessLog => {
                let (through, modified) = self.access_log_contents(full).await?;
                sqlx::query(chain.incremental())
                    .bind(full)
                    .bind(through)
                    .bind(&modified)
                    .fetch_all(&mut *acquire(&self.pool).await?)
                    .await?
            }
        };
        rows.iter().map(|row| finding(chain, row)).collect()
    }

    /// [`Self::verify_chain_advancing`] over both chains, the system chain's
    /// findings first.
    ///
    /// # Errors
    /// As [`Self::verify_chain_advancing`].
    pub async fn verify_chains_advancing(
        &self,
        scope: VerifyScope,
    ) -> Result<Vec<AuditChainFinding>, AuditError> {
        let mut findings = Vec::new();
        for chain in AuditChain::ALL {
            findings.extend(self.verify_chain_advancing(chain, scope).await?);
        }
        Ok(findings)
    }
}

/// The finding about `chain` a verification row holds.
/// How many entries one page of the content check reads.
const CONTENT_PAGE: i64 = 1000;

impl AuditStore {
    /// The content check of the access log the database cannot run, since
    /// only the server writes an entry's canonical form (#5703): each entry
    /// after the verification cursor, or every entry when `full`, joined from
    /// its row and shared parts and digested again. Returns the newest
    /// position the check covered, `None` for an empty log, and the positions
    /// whose digest differs from their link's `content_digest`. It writes
    /// nothing, so it also serves a check that moves no cursor.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a statement fails.
    pub async fn access_log_contents(
        &self,
        full: bool,
    ) -> Result<(Option<i64>, Vec<i64>), AuditError> {
        let mut conn = acquire(&self.pool).await?;
        let through: Option<i64> =
            sqlx::query_scalar("SELECT max(chain_seq) FROM audit.access_log_record")
                .fetch_one(&mut *conn)
                .await?;
        let Some(through) = through else {
            return Ok((None, Vec::new()));
        };
        let mut after: i64 = if full {
            0
        } else {
            sqlx::query_scalar(
                "SELECT coalesce((SELECT through_seq FROM audit.access_log_chain_verified \
                 WHERE singleton), 0)",
            )
            .fetch_one(&mut *conn)
            .await?
        };
        let mut modified = Vec::new();
        loop {
            let rows = sqlx::query(
                "SELECT r.chain_seq, r.content, sh.content AS shared, r.content_digest \
                 FROM audit.access_log_record r \
                 LEFT JOIN audit.access_log_shared sh ON sh.digest = r.shared \
                 WHERE r.chain_seq > $1 AND r.chain_seq <= $2 \
                 AND r.record_type = 'ACCESS_LOG_ENTRY' ORDER BY r.chain_seq LIMIT $3",
            )
            .bind(after)
            .bind(through)
            .bind(CONTENT_PAGE)
            .fetch_all(&mut *conn)
            .await?;
            let Some(last) = rows.last() else {
                return Ok((Some(through), modified));
            };
            after = last.try_get("chain_seq")?;
            for row in &rows {
                if !content_intact(row)? {
                    modified.push(row.try_get("chain_seq")?);
                }
            }
        }
    }
}

/// Whether the entry `row` stores still digests to its `content_digest`; an
/// entry whose parts no longer read as JSON does not.
fn content_intact(row: &PgRow) -> Result<bool, AuditError> {
    let parse = |text: &str| serde_json::from_str::<serde_json::Value>(text);
    let content: String = row.try_get("content")?;
    let shared: Option<String> = row.try_get("shared")?;
    let recorded: String = row.try_get("content_digest")?;
    // NOTE: our own design; a part that no longer parses was changed after it was written.
    let (Ok(own), Ok(shared)) = (parse(&content), shared.as_deref().map(parse).transpose()) else {
        return Ok(false);
    };
    let joined = crate::access_log::stored_form::join(own, shared);
    let Ok(form) = canonical_json(&joined) else {
        return Ok(false);
    };
    Ok(sha256_hex(&form) == recorded)
}

fn finding(chain: AuditChain, row: &PgRow) -> Result<AuditChainFinding, AuditError> {
    Ok(AuditChainFinding {
        chain,
        chain_seq: row.try_get("chain_seq")?,
        record_id: row.try_get("record_id")?,
        recorded_at: row
            .try_get::<Option<Timestamp>, _>("recorded_at")?
            .map(Timestamp::to_jiff),
        finding: row.try_get("finding")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each chain reads back from the name its column holds.
    #[test]
    fn a_chain_reads_back_from_its_name() {
        for chain in AuditChain::ALL {
            assert_eq!(AuditChain::named(chain.as_str()), Some(chain));
        }
        assert_eq!(AuditChain::named("audit_event"), None);
    }
}
