// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One frontier round of both audit logs (#6242).
//!
//! The round moves the frontier ([`crate::access_log::frontier`]), owes the
//! records below it to the sinks that deliver from the store, and, when due,
//! seals each log in windows ([`crate::access_log::seal`]). No specification
//! governs it: our own design.

use crate::access_log::frontier::Frontier;
use crate::access_log::seal::{ACCESS_LOG, Log, SYSTEM, SealRound};
use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::store::chains::AuditChain;

use super::AuditStore;

/// The sinks that deliver from the store, owed each record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Owed {
    /// The ITI-20 ATX: FHIR Feed.
    pub fhir_feed: bool,
    /// The ITI-20 syslog route.
    pub syslog: bool,
}

impl AuditStore {
    /// Move `frontier`, owe the records of both logs below it to the
    /// configured sinks, and seal them when `seal`, in as many windows as
    /// they fill; returns how many records were owed and whether a window was
    /// sealed.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a statement fails, and
    /// [`AuditError::AccessLog`] when the seal fails.
    pub async fn frontier_round(
        &self,
        frontier: &mut Frontier,
        seal: bool,
    ) -> Result<(u64, bool), AuditError> {
        let reached = {
            let mut conn = acquire(&self.pool).await?;
            frontier.round(&mut conn).await?
        };
        let mut owed = 0;
        if self.owed.fhir_feed || self.owed.syslog {
            for (chain, owe, upto) in [
                (
                    AuditChain::AccessLog,
                    "SELECT owed_count, owed_through FROM audit.access_log_owe($1, $2, $3)",
                    reached.access_log,
                ),
                (
                    AuditChain::System,
                    "SELECT owed_count, owed_through FROM audit.audit_event_owe($1, $2, $3)",
                    reached.system,
                ),
            ] {
                if !frontier.owe_due(chain) {
                    continue;
                }
                let (count, through): (i64, i64) = sqlx::query_as(owe)
                    .bind(upto)
                    .bind(self.owed.fhir_feed)
                    .bind(self.owed.syslog)
                    .fetch_one(&mut *acquire(&self.pool).await?)
                    .await?;
                // NOTE: our own design (#6242), records a write holds stay owed until released.
                if through >= upto {
                    frontier.owed(chain);
                }
                owed += u64::try_from(count).unwrap_or(0);
            }
        }
        let mut sealed = false;
        if seal {
            sealed |= self
                .seal_through(frontier, &ACCESS_LOG, reached.access_log)
                .await?;
            sealed |= self.seal_through(frontier, &SYSTEM, reached.system).await?;
        }
        Ok((owed, sealed))
    }

    /// Seal `log` up to `upto`, window after window, when its frontier moved
    /// past the last window, timed under `audit_seal_duration`; returns
    /// whether a window was sealed.
    async fn seal_through(
        &self,
        frontier: &mut Frontier,
        log: &Log,
        upto: i64,
    ) -> Result<bool, AuditError> {
        if !frontier.seal_due(log.chain()) {
            return Ok(false);
        }
        let started = std::time::Instant::now();
        let sealed = self.seal_windows(frontier, log, upto).await;
        measured(log, started, &sealed);
        sealed
    }

    /// Seal `log` up to `upto`, window after window; returns whether a window
    /// was sealed.
    async fn seal_windows(
        &self,
        frontier: &mut Frontier,
        log: &Log,
        upto: i64,
    ) -> Result<bool, AuditError> {
        let mut sealed = false;
        // NOTE: our own design (#6242), a window holds a bounded span of ids, so a round seals
        // window after window until the frontier is reached and no backlog outgrows the rate.
        loop {
            match self.chaining_store().seal_to(log, upto).await? {
                SealRound::Sealed { entries, .. } => {
                    sealed = true;
                    let entries = u64::try_from(entries).unwrap_or(u64::MAX);
                    crate::telemetry::access_path::metrics().log_chained.add(
                        entries,
                        &[opentelemetry::KeyValue::new("chain", log.chain().as_str())],
                    );
                    if log.chain() == AuditChain::System {
                        crate::telemetry::metrics::metrics()
                            .atna_audit_sent
                            .add(entries, &[opentelemetry::KeyValue::new("sink", "store")]);
                    }
                }
                SealRound::Nothing { through } => {
                    // NOTE: our own design (#6242), records a write holds stay due until released.
                    if through >= upto {
                        frontier.sealed(log.chain());
                    }
                    return Ok(sealed);
                }
                SealRound::Busy => return Ok(sealed),
            }
        }
    }
}

/// Record one sealing of `log`, begun at `started`: whether it sealed a
/// window, found nothing to seal, or failed. No openEHR spec governs the
/// metrics: our own design (#5591).
fn measured(log: &Log, started: std::time::Instant, sealed: &Result<bool, AuditError>) {
    use opentelemetry::KeyValue;
    let metrics = crate::telemetry::access_path::metrics();
    let chain = log.chain().as_str();
    let outcome = match sealed {
        Ok(true) => "sealed",
        Ok(false) => "empty",
        Err(_) => "failed",
    };
    metrics.seal_duration.record(
        crate::telemetry::access_path::seconds_since(started),
        &[
            KeyValue::new("chain", chain),
            KeyValue::new("outcome", outcome),
        ],
    );
    if sealed.is_ok() {
        metrics.seal_last_success.record(
            crate::telemetry::access_path::unix_now(),
            &[KeyValue::new("chain", chain)],
        );
    }
}

impl AuditStore {
    /// This store's gathered write waiting up to `window` after the first
    /// entry for more (`[audit.store] gather_window`); set before the first
    /// write.
    #[must_use]
    #[cfg_attr(
        not(feature = "fhir"),
        expect(
            unused_mut,
            unused_variables,
            reason = "a slim build writes no entry through the gathered writer"
        )
    )]
    pub fn with_gather_window(mut self, window: std::time::Duration) -> Self {
        #[cfg(feature = "fhir")]
        {
            self.group = std::sync::Arc::new(crate::access_log::intake::Gathered::new(window));
        }
        self
    }
}
