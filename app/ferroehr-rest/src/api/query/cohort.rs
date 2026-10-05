// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Cohort execution — `POST /query/cohort` (`query_execute_cohort`).
//!
//! **OUR OWN EXTENSION: no openEHR spec governs this route.** ITS-REST 1.1.0
//! publishes an ad-hoc and a stored query and nothing else; AQL is defined over
//! the clinical domain alone and has no demographic source. The operation
//! selects a population in the demographic domain, resolves it to EHRs through
//! the linkage domain, and runs the caller's AQL over exactly those EHRs —
//! [`ferroehr::service::linkage::cohort`] carries the design and the
//! separations that make it safe.
//!
//! The body is JSON only, decoded through the group's own
//! [`response::decode_body`], and the paging/parameter members are spelled the
//! way the released `AdhocQueryExecute` spells them (`q`, `offset`, `fetch`,
//! `query_parameters`) so a client that already speaks the query API needs no
//! second vocabulary. There is no `ehr_id`: the cohort IS the scope.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL `$parameter` binds are arbitrary \
              typed scalars by specification (QUERY 1.1), exactly as on the released ad-hoc body"
)]

use std::collections::BTreeMap;

use openehr_its::rest::runtime::ApiError;
use serde::Deserialize;

use ferroehr::service::linkage::cohort::{CohortOutcome, CohortPredicate, CohortQueryRequest};
use ferroehr::service::query::request::AqlQueryRequest;

use super::response::{self, QueryScope};
use crate::api::RequestParts;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The `POST /query/cohort` request body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CohortExecute {
    /// The AQL to run over the cohort.
    q: String,
    /// The demographic predicates, intersected. Each names a key the
    /// deployment bound in `[cohort.predicates]`.
    cohort: Vec<CohortSelector>,
    /// The caller's declared purpose of use, recorded on the access event when
    /// no purpose header carried one.
    #[serde(default)]
    purpose: Option<String>,
    /// AQL `$name` binds, as on the ad-hoc body.
    #[serde(default)]
    query_parameters: Option<BTreeMap<String, serde_json::Value>>,
    /// The 0-based start row, as on the ad-hoc body.
    #[serde(default)]
    offset: Option<i64>,
    /// The page size, as on the ad-hoc body.
    #[serde(default)]
    fetch: Option<i64>,
}

/// One predicate in the request's cohort selection.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CohortSelector {
    /// The `[cohort.predicates]` key.
    name: String,
    /// The value to match, interpreted by the binding's configured kind.
    value: String,
}

/// Execute a cohort query.
///
/// # Errors
///
/// [`ApiError::NotFound`] when the deployment binds no cohort predicate (the
/// route serves nothing), the decode failures of [`response::decode_body`], and
/// whatever [`ferroehr::service::linkage::cohort::CohortError`] maps onto —
/// `400` for a malformed request, `422` above the configured cohort ceiling.
pub(super) async fn execute(
    state: &AppState,
    parts: &RequestParts,
    scope: &QueryScope,
) -> Result<CohortOutcome, RestError> {
    // Config gate: the surface is off until a predicate is bound, and an
    // unbound deployment answers as if the route were not mounted.
    if !state.backend().cohort_enabled() {
        return Err(RestError(ApiError::NotFound(
            "cohort queries are not configured on this server".to_owned(),
        )));
    }
    let body: CohortExecute = response::decode_body(&parts.headers, &parts.body)?;
    let inner = scope.apply(AqlQueryRequest {
        ehr_ids: Vec::new(),
        offset: body.offset,
        fetch: body.fetch,
        parameters: body.query_parameters.unwrap_or_default(),
        ..Default::default()
    });
    let request = CohortQueryRequest {
        aql: body.q,
        predicates: body
            .cohort
            .into_iter()
            .map(|selector| CohortPredicate {
                name: selector.name,
                value: selector.value,
            })
            .collect(),
        purpose: body.purpose,
        query: inner,
    };
    state
        .backend()
        .execute_cohort_query(request)
        .await
        .map_err(|error| RestError::from(ferroehr::service::status::SmError::from(error)))
}
