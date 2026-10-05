// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[query]` section — AQL execution knobs.
//!
//! No openEHR spec governs these tuning knobs — our own operational extension. A field of the one config tree
//! ([`crate::config::FerroEhrConfig`]); no loader of its own. These replace the
//! two former raw-env reads (`FERROEHR__QUERY__PLAN_CACHE_CAPACITY` /
//! `FERROEHR__QUERY__TIMEOUT_MS`) whose parse failures were silently swallowed —
//! they are now ordinary typed fields, so a bad value is a boot error like
//! everywhere else.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// AQL execution configuration (`[query]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueryConfig {
    /// Max distinct cached query plans; `0` disables the plan cache (every
    /// lookup misses and the full parse→lower path runs).
    pub plan_cache_capacity: u64,
    /// Per-query DB execution budget in milliseconds; `0` disables it. Overrun
    /// reports `408` (ITS-REST `Requests_and_responses` §HTTP status codes).
    ///
    /// On by default. It was off, which left the HTTP request timeout as the
    /// only guard — and that one answers the CLIENT without cancelling the
    /// statement, so overrunning queries kept holding pooled connections after
    /// their callers had been given up on. The database-side
    /// [`statement_timeout`](crate::db::DbConfig::statement_timeout_ms) is the
    /// outer backstop; this budget is deliberately the tighter of the two so an
    /// overrun surfaces as this engine's typed refusal rather than a driver
    /// error.
    pub timeout_ms: u64,
    /// The largest page of rows one query execution serves; `0` means
    /// unbounded.
    ///
    /// A query that neither the AQL nor the request bounds takes it as its
    /// page: without that, a query carrying no `LIMIT`, called with no `fetch`,
    /// generates SQL with no `LIMIT` and materialises every matching row before
    /// the `RESULT_SET` is built — so a one-line request is an unbounded,
    /// caller-chosen allocation (the OWASP Denial of Service Cheat Sheet's
    /// "input-based resource allocation control"). A page asked for explicitly,
    /// by AQL `LIMIT` or the `fetch` parameter, is served as written up to the
    /// ceiling and refused above it with a `400` naming the ceiling, never
    /// silently shortened.
    ///
    /// ITS-REST leaves both the `fetch` default and its maximum to the
    /// implementation — query `Request.md` §Common Headers and Query
    /// Parameters: "the default depends on the implementation".
    pub max_result_rows: i64,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            plan_cache_capacity: 256,
            // Three orders of magnitude above the measured ad-hoc query p99 on
            // the reference SUT (tens of milliseconds — the committed step-load
            // record), so no legitimate query is cut short, while an unbounded
            // one is still bounded. A deployment with heavier queries raises it
            // deliberately rather than discovering there was no limit.
            timeout_ms: 30_000,
            // The largest example composition set the conformance corpus holds
            // is thousands of rows, and a clinical UI pages; 10_000 is far above
            // any interactive answer while keeping one request's worst case
            // bounded. A bulk consumer asks for more explicitly, which is the
            // point — the ceiling makes the unbounded case deliberate.
            max_result_rows: 10_000,
        }
    }
}

impl QueryConfig {
    /// The default result-row ceiling, or `None` when unbounded
    /// (`max_result_rows == 0`).
    #[must_use]
    pub const fn result_ceiling(&self) -> Option<i64> {
        if self.max_result_rows > 0 {
            Some(self.max_result_rows)
        } else {
            None
        }
    }

    /// The per-query execution budget as a [`Duration`], or `None` when
    /// disabled (`timeout_ms == 0`).
    #[must_use]
    pub fn timeout(&self) -> Option<Duration> {
        (self.timeout_ms > 0).then(|| Duration::from_millis(self.timeout_ms))
    }
}

#[cfg(test)]
mod tests {
    /// The ceiling is on by default: an unbounded query must not be able to ask
    /// for every row in the repository (OWASP Denial of Service Cheat Sheet
    /// §Input Validation — input-based resource-allocation control).
    #[test]
    fn the_result_ceiling_is_on_by_default() {
        let cfg = QueryConfig::default();
        assert_eq!(cfg.result_ceiling(), Some(cfg.max_result_rows));
        assert!(cfg.max_result_rows > 0);
    }

    /// Zero means unbounded, deliberately: an operator who wants no ceiling must
    /// say so, and the value that says so is the one a reader would guess.
    #[test]
    fn zero_disables_the_ceiling() {
        let cfg = QueryConfig {
            max_result_rows: 0,
            ..QueryConfig::default()
        };
        assert_eq!(cfg.result_ceiling(), None);
    }

    /// The execution budget is on by default, and stays TIGHTER than the
    /// database-side `statement_timeout` so an overrun surfaces as this engine's
    /// typed refusal rather than a driver error.
    #[test]
    fn the_execution_budget_is_on_and_tighter_than_the_database_backstop() {
        let query = QueryConfig::default();
        let db = crate::db::DbConfig::default();
        assert!(query.timeout().is_some());
        assert!(
            query.timeout_ms < db.statement_timeout_ms,
            "the engine budget ({}ms) must fire before the database backstop ({}ms)",
            query.timeout_ms,
            db.statement_timeout_ms
        );
    }

    use super::*;

    #[test]
    fn defaults_have_the_cache_and_the_budget_on() {
        let c = QueryConfig::default();
        assert_eq!(c.plan_cache_capacity, 256);
        // The budget used to default to 0 (off) and this test pinned that. It is
        // on now, deliberately: with it off, the HTTP request timeout was the
        // only guard, and that answers the client without cancelling the
        // statement — so overrunning queries kept holding pooled connections
        // after their callers had been given up on.
        assert!(c.timeout_ms > 0);
        assert!(c.timeout().is_some());
    }

    /// Zero still means off, and must keep meaning off: an operator who wants no
    /// budget has to be able to say so.
    #[test]
    fn zero_still_disables_the_budget() {
        let c = QueryConfig {
            timeout_ms: 0,
            ..QueryConfig::default()
        };
        assert!(c.timeout().is_none());
    }

    #[test]
    fn positive_timeout_maps_to_duration() {
        let c = QueryConfig {
            timeout_ms: 1500,
            ..QueryConfig::default()
        };
        assert_eq!(c.timeout(), Some(Duration::from_millis(1500)));
    }
}
