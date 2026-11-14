// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The execution of the `query` API group — ad-hoc + stored AQL execution
//! (ITS-REST 1.1.0 QUERY API). Every operation runs the same frame: the
//! access evaluator's condition, its ad-hoc ([`super::adhoc`]), stored
//! ([`super::stored`]) or cohort ([`super::cohort`]) execution path, and
//! renders the assembled `RESULT_SET` (with its spec-mandated `ETag`) via
//! [`super::response`].
//!
//! Spec: the six released operations, their parameter lists, and their
//! `RESULT_SET` response are `query-codegen.openapi.yaml` +
//! `docs/query/{Request,Response}.md`. The seventh, `query_execute_cohort`
//! ([`super::cohort`]), is our own extension — no openEHR spec governs it.
//! The request-normalization + response-rendering shared by both paths lives in
//! [`super::response`].

use axum::response::Response;

use crate::api::RequestParts;
use crate::overview::error::RestError;

use super::response::{self, QueryScope};
use super::{adhoc, cohort, stored};
use crate::state::AppState;

/// The execution path of one query operation.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Execution {
    /// `query_execute_adhoc_query`.
    Adhoc,
    /// `query_execute_adhoc_query_body`.
    AdhocBody,
    /// `query_execute_stored_query`.
    Stored,
    /// `query_execute_stored_query_body`.
    StoredBody,
    /// `query_execute_stored_query_version`.
    StoredVersion,
    /// `query_execute_stored_query_version_body`.
    StoredVersionBody,
    /// `query_execute_cohort`, our own extension.
    Cohort,
}

/// Run one query operation along `execution`: the access scope first,
/// then the execution and the rendered `RESULT_SET`.
///
/// # Errors
/// The refusals of the execution path and of the result-set rendering.
pub(crate) async fn serve(
    state: AppState,
    parts: RequestParts,
    execution: Execution,
) -> Result<Response, RestError> {
    // EHR_ACCESS: the access evaluator's verdict the path routes apply, held
    // over every row the query could reach (RM ehr `ehr_access.adoc`
    // §`EHR_ACCESS` Class).
    let ehr_access = match crate::extensions::access::ehr_access::query_condition(&state).await {
        Ok(ehr_access) => ehr_access,
        Err(deny) => return Ok(*deny),
    };
    let scope = QueryScope {
        ehr_access,
        log_standing: crate::api::access_log_query::standing(&state),
    };

    let outcome = match execution {
        Execution::Adhoc => adhoc::get(&state, &parts, &scope).await?,
        Execution::AdhocBody => adhoc::post(&state, &parts, &scope).await?,
        Execution::Stored => stored::get(&state, &parts, &scope).await?,
        Execution::StoredBody => stored::post(&state, &parts, &scope).await?,
        Execution::StoredVersion => stored::version_get(&state, &parts, &scope).await?,
        Execution::StoredVersionBody => stored::version_post(&state, &parts, &scope).await?,
        // The cohort extension: the same access condition, rendering and access
        // extensions as the released operations, over a scope resolved across
        // the pseudonymisation boundary rather than supplied.
        Execution::Cohort => {
            let outcome = cohort::execute(&state, &parts, &scope).await?;
            let suppressed = outcome.suppressed;
            let mut query = outcome.query;
            if suppressed {
                // A suppressed response discloses no EHR, so the access record
                // must name none: the served-EHR extension is what the ATNA
                // layer writes one access per served record from.
                query.served_ehrs.clear();
            }
            query
        }
    };

    let mut resp = response::respond_result_set(&parts.headers, &outcome.result_set);
    if let Some(log_read) = &outcome.log_read {
        // A query over the access log is a read of the log, and its record writes its
        // own access-log entry (the draft's recording rule 4), naming the EHR whose log
        // was read when there is one.
        resp.extensions_mut()
            .insert(crate::system_log::middleware::AuditObject {
                ehr_id: log_read.ehr_id.clone(),
                result_count: Some(outcome.served_rows),
                query: outcome.statement.clone(),
                entry: ferroehr::system_log::event::EntryScope::LogRead,
                ..crate::system_log::middleware::AuditObject::default()
            });
        return Ok(resp);
    }
    // A query rooted at a demographic class reads the party domain, and its
    // record is filed there with no clinical category, as a Demographic API
    // read is (no openEHR spec governs access logging: our own design).
    let demographic = outcome.domain == ferroehr::aql::ir::QueryDomain::Demographic;
    // The access log records one access per EHR the statement served, plus the
    // statement's own operation record. Both ride the response extensions the
    // ATNA middleware reads, so the caller identity stays in one place.
    resp.extensions_mut()
        .insert(crate::system_log::middleware::AuditObject {
            ehr_id: None,
            uid: None,
            result_count: Some(outcome.served_rows),
            domain: demographic.then_some(ferroehr::system_log::event::AccessDomain::Demographic),
            origins: outcome.served_origins.clone(),
            origin_count: (!outcome.served_origins.is_empty()).then_some(outcome.origin_count),
            content: if demographic {
                None
            } else {
                outcome.accessed.clone()
            },
            query: outcome.statement.clone(),
            ..crate::system_log::middleware::AuditObject::default()
        });
    if !outcome.served_ehrs.is_empty() {
        resp.extensions_mut()
            .insert(crate::system_log::served::AuditServedEhrs {
                served: outcome.served_ehrs,
                content: outcome.served_ehr_content,
                query: outcome.statement,
            });
    }
    Ok(resp)
}
