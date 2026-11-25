// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two logs of the audit store and their verification (#5573, #6242).
//!
//! The access log (`audit.access_log_record`) holds the accesses to EHR data;
//! the records outside it (`audit.audit_event`: an authentication, a
//! template, party or application operation) are the system records, named
//! the system chain. Each log has its own id sequence, its own sealed windows,
//! its own verification and its own verification cursor
//! ([`crate::access_log::seal`]). A record is named by its log and its
//! position in it. No openEHR spec governs audit tamper detection: our own
//! design.

use uuid::Uuid;

use crate::system_log::AuditError;
use crate::system_log::store::AuditStore;
use crate::system_log::store::chain_walk::StepPace;

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
    /// Checks the tamper evidence of `chain` over `scope`, and moves the chain's
    /// verification cursor to its newest record when the run finds nothing.
    ///
    /// [`VerifyScope::SinceCheckpoint`] recomputes only the windows sealed after
    /// the newest intact one, in steps sized to a time budget ([`StepPace`]),
    /// each moving the cursor ([`Self::chain_step`]); an earlier change needs
    /// [`VerifyScope::Full`]. An empty result means intact. The seals are
    /// unkeyed, so the signed seals published outside the database fix them
    /// against a writer who could recompute them.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the verification query cannot run, which is
    /// itself a finding, never a pass.
    pub async fn verify_chain_advancing(
        &self,
        chain: AuditChain,
        scope: VerifyScope,
    ) -> Result<Vec<AuditChainFinding>, AuditError> {
        let mut walk = self.chain_walk(chain, scope).await?;
        let mut pace = StepPace::default();
        let mut findings = Vec::new();
        while !walk.done() {
            findings.extend(self.chain_step_paced(&mut walk, &mut pace).await?.0);
        }
        Ok(findings)
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
