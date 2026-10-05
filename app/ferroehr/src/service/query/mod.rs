// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The QUERY component of the platform crate — the openEHR **Query** service
//! seam (`I_QUERY_SERVICE`, `i_query_service.adoc`; `master08-query_service.adoc`).
//!
//! Module tree, one file per concern:
//!
//! - [`request`] — the normalized execute-call request/outcome pair
//!   (`ADHOC_QUERY_EXECUTE_SPEC` / `STORED_QUERY_EXECUTE_SPEC` +
//!   the execute-call parameters).
//! - `execute` — the `I_QUERY_SERVICE` calls on `FerroEhrService` and the
//!   execution orchestration (parse → plan → execute → assemble), paging
//!   composition, `ehr_ids` resolution, and the per-query execution budget.
//!   The AQL *engine* it drives is [`crate::aql`].
//! - `result_set` — `RESULT_SET` / `RESULT_SET_COLUMN` / `RESULT_SET_ROW`
//!   assembly (`result_set.adoc`) + parameter substitution, isolated so the
//!   SM-vs-ITS-REST shape divergences (the `RESULT_SET.id` MUST)
//!   live in one spec-cited place.
//! - [`plan_cache`] — the bounded cache of lowered AQL plans keyed on query
//!   text (no openEHR spec governs it — our own performance design).
//! - [`config`] — the `[query]` tuning knobs (no openEHR spec governs
//!   configuration — our own design).

pub mod config;
mod execute;
pub mod plan_cache;
pub(crate) mod result_set;

pub mod request;
