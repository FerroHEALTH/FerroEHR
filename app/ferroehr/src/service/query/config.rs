// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[record.query]` section — AQL execution knobs.
//!
//! No openEHR spec governs these tuning knobs: our own operational extension. A
//! field of the one config tree ([`crate::config::FerroEhrConfig`]), so a bad
//! value is a boot error.

use std::time::Duration;

use ferrohealth_config::units;
use serde::{Deserialize, Serialize};

/// AQL execution configuration (`[record.query]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct QueryConfig {
    /// Max distinct cached query plans; `0` disables the plan cache (every
    /// lookup misses and the full parse→lower path runs).
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
    pub plan_cache_capacity: u64,
    /// Per-query DB execution budget; `"0s"` disables it. Overrun
    /// reports `408` (ITS-REST `Requests_and_responses` §HTTP status codes).
    ///
    /// On by default. It was off, which left the HTTP request timeout as the
    /// only guard — and that one answers the CLIENT without cancelling the
    /// statement, so overrunning queries kept holding pooled connections after
    /// their callers had been given up on. The database-side
    /// [`statement_timeout`](crate::db::DbConfig::statement_timeout) is the
    /// outer backstop; this budget is deliberately the tighter of the two so an
    /// overrun surfaces as this engine's typed refusal rather than a driver
    /// error.
    pub timeout: units::Duration,
    /// The largest page of rows one query execution serves; `0` means
    /// unbounded.
    ///
    /// A query neither the AQL nor the request bounds takes it as its page, so
    /// no request is an unbounded allocation (OWASP Denial of Service Cheat
    /// Sheet, "input-based resource allocation control"). A page asked for by
    /// AQL `LIMIT` or `fetch` above it is refused with a `400` naming the
    /// ceiling. ITS-REST leaves the `fetch` maximum to the implementation (query
    /// `Request.md` §Common Headers and Query Parameters).
    pub max_result_rows: i64,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            plan_cache_capacity: 256,
            // Far above any legitimate ad-hoc query, so an unbounded one is
            // still bounded; a deployment with heavier queries raises it.
            timeout: units::Duration::from_secs(30),
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
    /// disabled (`timeout = "0s"`).
    #[must_use]
    pub fn budget(&self) -> Option<Duration> {
        (!self.timeout.is_zero()).then(|| self.timeout.get())
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
        assert!(query.budget().is_some());
        assert!(
            query.timeout < db.statement_timeout,
            "the engine budget ({}) must fire before the database backstop ({})",
            query.timeout,
            db.statement_timeout
        );
    }

    use super::*;

    #[test]
    fn defaults_have_the_cache_and_the_budget_on() {
        let c = QueryConfig::default();
        assert_eq!(c.plan_cache_capacity, 256);
        // With no budget the HTTP timeout answers the client but leaves the
        // statement holding its pooled connection.
        assert!(!c.timeout.is_zero());
        assert!(c.budget().is_some());
    }

    /// Zero still means off, and must keep meaning off: an operator who wants no
    /// budget has to be able to say so.
    #[test]
    fn zero_still_disables_the_budget() {
        let c = QueryConfig {
            timeout: units::Duration::ZERO,
            ..QueryConfig::default()
        };
        assert!(c.budget().is_none());
    }

    #[test]
    fn positive_timeout_maps_to_duration() {
        let c = QueryConfig {
            timeout: units::Duration::from_millis(1500),
            ..QueryConfig::default()
        };
        assert_eq!(c.budget(), Some(Duration::from_millis(1500)));
    }
}
