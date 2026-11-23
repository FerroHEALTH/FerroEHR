// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One frontier round of both audit logs (#6242).
//!
//! The round moves the frontier ([`crate::access_log::frontier`]), moves the
//! delivery frontier the sinks deliver up to, copies the access log's search
//! columns up to it ([`crate::access_log::index`]), and, when due,
//! seals each log in windows ([`crate::access_log::seal`]). No specification
//! governs it: our own design.

use crate::access_log::frontier::Frontier;
use crate::access_log::seal::{ACCESS_LOG, Log, SYSTEM, SealRound};
use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::store::chains::AuditChain;

use super::AuditStore;

/// The statement that moves each log's delivery frontier to `$1` (the access
/// log) and `$2` (the system records), below every record a write holds;
/// returns how many logs' frontiers moved, and where each frontier stands.
const ADVANCE: &str = "WITH v(chain, upto) AS (VALUES \
       ('access_log', least($1::bigint, coalesce( \
            (SELECT min(h.entry_id) - 1 FROM audit.access_log_hold h), $1::bigint))), \
       ('system', least($2::bigint, coalesce( \
            (SELECT min(h.seq) - 1 FROM audit.audit_event_hold h), $2::bigint)))), \
     moved AS (UPDATE audit.delivery_frontier f SET through_id = v.upto FROM v \
                WHERE f.chain = v.chain AND f.through_id < v.upto RETURNING f.chain), \
     now_at AS (SELECT f.chain, greatest(f.through_id, v.upto) AS through_id \
                  FROM audit.delivery_frontier f JOIN v ON v.chain = f.chain) \
     SELECT (SELECT count(*) FROM moved), \
            (SELECT through_id FROM now_at WHERE chain = 'access_log'), \
            (SELECT through_id FROM now_at WHERE chain = 'system')";

/// The sinks that deliver from the store, owed each record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Owed {
    /// The ITI-20 ATX: FHIR Feed.
    pub fhir_feed: bool,
    /// The ITI-20 syslog route.
    pub syslog: bool,
}

impl AuditStore {
    /// Move `frontier`, move the delivery frontier of both logs to it while
    /// a sink is configured, and seal them when `seal`, in as many windows as
    /// they fill; returns how many logs' delivery frontiers moved and whether
    /// a window was sealed.
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
        // NOTE: our own design (#6242), the delivery frontier moves only while a sink is configured.
        if (self.owed.fhir_feed || self.owed.syslog)
            && (frontier.owe_due(AuditChain::AccessLog) || frontier.owe_due(AuditChain::System))
        {
            let (moved, access_log, system): (i64, i64, i64) = sqlx::query_as(ADVANCE)
                .bind(reached.access_log)
                .bind(reached.system)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
            // NOTE: our own design (#6242), records a write holds stay owed until released.
            if access_log >= reached.access_log {
                frontier.owed(AuditChain::AccessLog);
            }
            if system >= reached.system {
                frontier.owed(AuditChain::System);
            }
            owed = u64::try_from(moved).unwrap_or(0);
        }
        // NOTE: our own design (#6319), a round that fails to index leaves the entries past the
        // cursor to the readers' key range, which stays exact, and the next round copies them.
        if let Err(error) = self.chaining_store().index_to(reached.access_log).await {
            tracing::error!(
                error = %crate::error::ErrorChain::new(&error),
                "the access-log search index did not move; the next round copies its entries"
            );
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
