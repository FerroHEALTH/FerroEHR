// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scheduled verification of the local store's hash chain.
//!
//! No openEHR spec governs audit tamper detection — our own design/extension.
//! `docs/law/eu/cra/text.html Annex I Part I(2)(f)` asks a product to "protect
//! the integrity of stored … data … and report on corruptions": the chain makes
//! a corruption detectable, and this task makes the server report one without
//! an operator running `SELECT * FROM audit.verify_audit_chain()` by hand.
//!
//! A run that finds damage logs it at `ERROR`, adds to the
//! `atna_audit_chain_findings` counter and leaves the result in a
//! [`ChainCheck`] the `audit_chain` health indicator reads. A run that cannot
//! verify at all is reported the same way, because an unverifiable trail is
//! never a pass.

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use jiff::Timestamp;

use crate::system_log::store::{AuditChainFinding, AuditStore};

/// How many findings one run keeps for the health surface and logs one by one;
/// the total is always reported in full.
pub const KEPT_FINDINGS: usize = 20;

/// The delay before the first scheduled run, so a booting server is not slowed
/// by a full pass over the trail; shortened to the interval when that is less.
const FIRST_RUN_DELAY: Duration = Duration::from_mins(1);

/// The outcome of the most recent verification.
#[derive(Debug, Clone, Default)]
pub enum ChainCheckState {
    /// No run has finished yet.
    #[default]
    Pending,
    /// The last run found the trail intact.
    Intact {
        /// When the run finished.
        at: Timestamp,
    },
    /// The last run found damage.
    Damaged {
        /// When the run finished.
        at: Timestamp,
        /// How many findings the run returned.
        total: usize,
        /// The first [`KEPT_FINDINGS`] findings, in chain order.
        findings: Vec<AuditChainFinding>,
    },
    /// The last run could not verify the trail.
    Unverifiable {
        /// When the run finished.
        at: Timestamp,
        /// Why the verification query failed.
        reason: String,
    },
}

/// The shared result of the scheduled verification: written by the task,
/// read by the health indicator.
#[derive(Debug, Clone, Default)]
pub struct ChainCheck {
    state: Arc<RwLock<ChainCheckState>>,
}

impl ChainCheck {
    /// Creates a check whose state is [`ChainCheckState::Pending`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the outcome of the most recent run.
    #[must_use]
    pub fn state(&self) -> ChainCheckState {
        self.state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Verifies the chain once, reports the outcome, and returns it.
    ///
    /// Damage and a failed verification are each logged at `ERROR` and added
    /// to the `atna_audit_chain_findings` counter (attribute `kind` = `damage`
    /// or `unverifiable`); an intact trail is logged at `DEBUG`.
    pub async fn run(&self, store: &AuditStore) -> ChainCheckState {
        let at = Timestamp::now();
        let state = match store.verify_chain().await {
            Ok(findings) if findings.is_empty() => {
                tracing::debug!("audit hash chain verified intact");
                ChainCheckState::Intact { at }
            }
            Ok(mut findings) => {
                let total = findings.len();
                record(u64::try_from(total).unwrap_or(u64::MAX), "damage");
                tracing::error!(
                    findings = total,
                    "the audit hash chain is damaged: records were modified or deleted outside \
                     retention; see audit.verify_audit_chain()"
                );
                findings.truncate(KEPT_FINDINGS);
                for finding in &findings {
                    tracing::error!(
                        chain_seq = ?finding.chain_seq,
                        record_id = ?finding.record_id,
                        recorded_at = ?finding.recorded_at,
                        "audit hash chain finding: {}",
                        finding.finding
                    );
                }
                ChainCheckState::Damaged {
                    at,
                    total,
                    findings,
                }
            }
            Err(e) => {
                record(1, "unverifiable");
                tracing::error!(error = %e, "the audit hash chain could not be verified");
                ChainCheckState::Unverifiable {
                    at,
                    reason: e.to_string(),
                }
            }
        };
        *self.state.write().unwrap_or_else(PoisonError::into_inner) = state.clone();
        state
    }
}

/// Adds `count` to the findings counter under `kind`.
fn record(count: u64, kind: &'static str) {
    crate::telemetry::metrics::metrics()
        .atna_audit_chain_findings
        .add(count, &[opentelemetry::KeyValue::new("kind", kind)]);
}

/// The scheduled verification: one run after a short delay, then one per
/// `interval`.
#[expect(
    clippy::infinite_loop,
    reason = "the chain check is a detached background task with no shutdown \
              channel — it ends when the runtime drops the task; declaring `-> !` \
              is not an option because `tokio::spawn` would then need the never \
              type as a type argument, which is unstable"
)]
pub(crate) async fn scheduled(store: AuditStore, check: ChainCheck, interval: Duration) {
    let first = tokio::time::Instant::now() + FIRST_RUN_DELAY.min(interval);
    let mut ticks = tokio::time::interval_at(first, interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        check.run(&store).await;
    }
}
