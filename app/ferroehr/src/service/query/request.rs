// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Query execution requests and outcomes.
//!
//! The realization of `ADHOC_QUERY_EXECUTE_SPEC` /
//! `STORED_QUERY_EXECUTE_SPEC` (`adhoc_query_execute_spec.adoc`,
//! `stored_query_execute_spec.adoc`) plus the execute-call parameters
//! (`i_query_service.adoc`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL result rows are arbitrary \
              projections by specification (QUERY 1.1)"
)]

use std::collections::BTreeMap;

use serde_json::Value;

/// A normalized AQL query request: the paging window, the EHR scope, and the
/// `$parameter` bindings.
///
/// Realizes the SM execute-spec classes plus the execute-call parameters:
/// `query_parameters` (the spec types them `Hash<String, String>`; carried as
/// JSON [`Value`]s — the ITS-REST wire binds typed parameters, a documented
/// widening), `row_offset` / `rows_to_fetch`, and `ehr_ids`.
///
/// `formalism` (`ADHOC_QUERY_EXECUTE_SPEC.formalism`, default `"aql"`) is
/// fixed to AQL — another formalism is rejected typed, which the SM sanctions
/// ("matching one of: aql; any other string value").
#[derive(Debug, Clone, Default)]
pub struct AqlQueryRequest {
    /// `ehr_ids: List<UUID> [0..1]` — "Specific set of EHRs on which to
    /// execute the query. If none supplied, a full population query will be
    /// performed on all EHRs whose status has the `is_queryable` flag set to
    /// `True`" (`i_query_service.adoc`). Empty = unscoped. The ITS-REST
    /// single `ehr_id` parameter is the one-element case. Error
    /// `ehr_id_does_not_exist` when a listed EHR does not exist.
    pub ehr_ids: Vec<String>,
    /// `row_offset [0..1]` — "offset in query response rows to return …
    /// A zero or negative value means offset of zero."
    pub offset: Option<i64>,
    /// `rows_to_fetch [0..1]` — "number of query response rows to fetch …
    /// A zero or negative value means 'all'."
    pub fetch: Option<i64>,
    /// `query_parameters` — "Parameters to substitute in query … each tag
    /// must match a parameter name in the query" (`$name` binds, no `$`
    /// prefix).
    pub parameters: BTreeMap<String, Value>,
    /// The ABAC patient-scope subject id (no openEHR spec governs this — our
    /// own access-control extension): when set, the engine pre-filters every
    /// VO root to EHRs whose subject equals it.
    pub subject_scope: Option<String>,
    /// The caller's per-EHR access decision (`EHR_ACCESS`): the EHRs the query
    /// may read rows from, applied to every EHR-anchored source whatever the
    /// query scopes or projects.
    pub ehr_access: crate::aql::sql::EhrAccessScope,
    /// Whether the executor should collect the touched EHR-id / template-id
    /// sets for the ABAC query post-check (our own extension).
    pub collect_attributes: bool,
}

impl AqlQueryRequest {
    /// The single-EHR scope of the ITS-REST wire (`ehr_id` query parameter /
    /// `openehr-ehr-id` header), if exactly one EHR is scoped.
    #[must_use]
    pub fn single_ehr_id(&self) -> Option<&str> {
        match self.ehr_ids.as_slice() {
            [one] => Some(one),
            _ => None,
        }
    }
}

/// The outcome of an AQL execution.
///
/// Carries the assembled `RESULT_SET` (`result_set.adoc`; rendered as ITS-REST
/// canonical JSON) plus — when the caller asked for them
/// ([`AqlQueryRequest::collect_attributes`]) — the distinct EHR ids and
/// template ids the query touched, for the ABAC post-check.
#[derive(Debug, Clone, Default)]
pub struct QueryOutcome {
    /// The `RESULT_SET` (canonical JSON) the adapter renders.
    pub result_set: Value,
    /// The distinct EHR ids the query touched (empty unless collected).
    pub ehr_ids: Vec<String>,
    /// The distinct template ids the query touched (empty unless collected).
    pub template_ids: Vec<String>,
    /// The EHRs this execution actually SERVED, each with its served-row count.
    ///
    /// Distinct from [`Self::ehr_ids`], and the difference is the point.
    /// `ehr_ids` is what the query MATCHED, collected before paging so the
    /// authorization decision covers everything the statement could reach.
    /// This is what the caller was actually shown, after `LIMIT`, and it is
    /// what the access log records: a legal record of access must not name an
    /// EHR whose content was never disclosed.
    ///
    /// Empty when the plan carries no per-EHR breakdown
    /// (`crate::aql::sql::ACCESS_EHR_PREFIX` names those shapes).
    pub served_ehrs: Vec<(String, u64)>,
    /// How many rows this execution served — the access log's record of how
    /// much was disclosed, counted where the rows are rather than re-read from
    /// the assembled document.
    pub served_rows: u64,
    /// The distinct origins of the data the served rows came from (EHDS
    /// Annex II 3.2(e), #3212), capped; empty when no version row could be
    /// attributed (see `crate::aql::sql::ACCESS_EHR_PREFIX` for the shapes).
    pub served_origins: Vec<String>,
    /// The true number of distinct origins behind [`Self::served_origins`].
    pub origin_count: u64,
    /// What the statement touched, for the access record's category
    /// classification (EHDS Annex II 3.2(c), #3621): the identifiers of the
    /// served versions, or the query's positive constraints when the answer
    /// carries none. `None` when nothing was executed.
    pub accessed: Option<crate::system_log::categories::AccessedContent>,
    /// The identifiers of the versions served from each EHR in
    /// [`Self::served_ehrs`], keyed by EHR id, so each per-EHR access record
    /// is classified by what that EHR disclosed.
    pub served_ehr_content: BTreeMap<String, Vec<crate::system_log::categories::ContentIds>>,
}

impl QueryOutcome {
    /// An outcome with no collected attributes.
    #[must_use]
    pub fn plain(result_set: Value) -> Self {
        Self {
            result_set,
            ehr_ids: Vec::new(),
            template_ids: Vec::new(),
            served_ehrs: Vec::new(),
            served_rows: 0,
            served_origins: Vec::new(),
            origin_count: 0,
            accessed: None,
            served_ehr_content: BTreeMap::new(),
        }
    }
}
