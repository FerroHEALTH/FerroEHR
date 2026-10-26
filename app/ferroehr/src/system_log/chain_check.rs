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
//! A scheduled run verifies the records written since the last link a run
//! found intact, and a full pass runs every `full_verify_interval_seconds`
//! and after any run that found damage, so damage to an older record stays
//! reported until a full pass finds the trail intact again.
//!
//! A run that finds damage logs it at `ERROR`, adds to the
//! `atna_audit_chain_findings` counter and leaves the result in a
//! [`ChainCheck`] the `audit_chain` health indicator reads. A run that cannot
//! verify at all is reported the same way, because an unverifiable trail is
//! never a pass.

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use jiff::Timestamp;

use crate::system_log::store::{AuditChainFinding, AuditStore, VerifyScope};

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

    /// Verifies the whole chain once, reports the outcome, and returns it
    /// ([`Self::run_scope`] with [`VerifyScope::Full`]).
    pub async fn run(&self, store: &AuditStore) -> ChainCheckState {
        self.run_scope(store, VerifyScope::Full).await
    }

    /// Verifies the chain over `scope` once, reports the outcome, and returns
    /// it; an intact run moves the verification checkpoint to the newest
    /// record.
    ///
    /// Damage and a failed verification are each logged at `ERROR` and added
    /// to the `atna_audit_chain_findings` counter (attribute `kind` = `damage`
    /// or `unverifiable`); an intact trail is logged at `DEBUG`.
    pub async fn run_scope(&self, store: &AuditStore, scope: VerifyScope) -> ChainCheckState {
        let at = Timestamp::now();
        let state = match store.verify_chain_advancing(scope).await {
            Ok(findings) if findings.is_empty() => {
                tracing::debug!(?scope, "audit hash chain verified intact");
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
/// `interval`, each over the scope [`scope_for`] picks.
#[expect(
    clippy::infinite_loop,
    reason = "the chain check is a detached background task with no shutdown \
              channel — it ends when the runtime drops the task; declaring `-> !` \
              is not an option because `tokio::spawn` would then need the never \
              type as a type argument, which is unstable"
)]
pub(crate) async fn scheduled(
    store: AuditStore,
    check: ChainCheck,
    interval: Duration,
    full_every: Option<Duration>,
) {
    let first = tokio::time::Instant::now() + FIRST_RUN_DELAY.min(interval);
    let mut ticks = tokio::time::interval_at(first, interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_full = tokio::time::Instant::now();
    loop {
        ticks.tick().await;
        let now = tokio::time::Instant::now();
        let scope = scope_for(&check.state(), now.duration_since(last_full), full_every);
        if scope == VerifyScope::Full {
            last_full = now;
        }
        check.run_scope(&store, scope).await;
    }
}

/// The scope of the next scheduled run: a full pass after a run that found
/// damage, so damage before the checkpoint stays reported, and once
/// `full_every` has passed `since_full`; otherwise the records since the
/// checkpoint.
fn scope_for(
    previous: &ChainCheckState,
    since_full: Duration,
    full_every: Option<Duration>,
) -> VerifyScope {
    let damaged = matches!(previous, ChainCheckState::Damaged { .. });
    if damaged || full_every.is_some_and(|every| since_full >= every) {
        VerifyScope::Full
    } else {
        VerifyScope::SinceCheckpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run follows the checkpoint until the full interval has passed or a
    /// run found damage, and never runs full on a schedule that has none.
    #[test]
    fn the_scope_is_full_after_damage_or_the_full_interval() {
        let day = Duration::from_hours(24);
        let week = Some(Duration::from_hours(24 * 7));
        let intact = ChainCheckState::Intact {
            at: Timestamp::UNIX_EPOCH,
        };
        let damaged = ChainCheckState::Damaged {
            at: Timestamp::UNIX_EPOCH,
            total: 1,
            findings: Vec::new(),
        };
        assert_eq!(
            scope_for(&ChainCheckState::Pending, day, week),
            VerifyScope::SinceCheckpoint
        );
        assert_eq!(scope_for(&intact, day, week), VerifyScope::SinceCheckpoint);
        assert_eq!(scope_for(&intact, day * 7, week), VerifyScope::Full);
        assert_eq!(scope_for(&damaged, day, week), VerifyScope::Full);
        assert_eq!(
            scope_for(&intact, day * 70, None),
            VerifyScope::SinceCheckpoint
        );
        assert_eq!(scope_for(&damaged, day, None), VerifyScope::Full);
    }
}
