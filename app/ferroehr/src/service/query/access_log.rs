// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An AQL query over `ACCESS_LOG_ENTRY` (#5131), run against the access log
//! under the rules of its ITS-REST resources (#5130).
//!
//! A reviewer the admin class admits reads every entry in the full view,
//! narrowed by the request's `ehr_id` scope; the subject of care and their
//! representative read the entries of the EHR they are confined to, in the
//! subject view, without those about data held from them under EHDS Art. 3(3)
//! (`docs/law/eu/ehds/text.html`); any other caller is refused. The log's
//! access control is its own, so no clinical grant reaches it (the draft's
//! recording rule 8). No openEHR spec governs AQL over the access log: our own
//! design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL result rows are arbitrary \
              projections by specification (QUERY 1.1)"
)]

use jiff::Timestamp;
use openehr_query::ast::Primitive;
use serde_json::Value;

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_its::rest::stored_query::QualifiedQueryName;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

use crate::access_log::view::{LOG_REFUSED, LogReader, LogStanding, view_of, withheld};
use crate::aql::access_log::sql::{LogScope, statement};
use crate::aql::access_log::{LogCell, LogPlan};
use crate::aql::exec::{ColumnMeta, QueryResult};
use crate::aql::ir::Params;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::admin::access_log_read::log_failure;
use crate::service::query::request::{AqlQueryRequest, LogRead, QueryOutcome};

use super::execute::{Failure, apply_result_ceiling, compose_paging};
use super::result_set::{result_set_json, substitute_params};

impl FerroEhrService {
    /// Run the access-log query `plan`, written as `aql`, for the caller
    /// `request` stands for, and assemble its `RESULT_SET`.
    ///
    /// # Errors
    /// - `not_permitted` (`403`): the caller is neither the person nor a
    ///   reviewer;
    /// - `not_found` (`404`): the person scopes an EHR other than their own,
    ///   or the reviewer one that does not exist;
    /// - `precondition` (`400`): a paging collision, a value of another type
    ///   than its path's, or an unbound parameter;
    /// - `conflict` (`409`): the local audit store is off;
    /// - `Timeout` (`408`): the statement overran the query budget;
    /// - `exception`: the store failed.
    pub(super) async fn execute_access_log(
        &self,
        plan: &LogPlan,
        aql: &str,
        name: Option<&QualifiedQueryName>,
        request: &AqlQueryRequest,
        params: &Params,
    ) -> Result<QueryOutcome, Failure> {
        let (reader, ehrs) = reader_of(request)?;
        let log = self.access_log_store().map_err(Failure::analysis)?;
        if reader == LogReader::Reviewer {
            self.check_ehr_ids(&ehrs).await?;
        }
        let (limit, offset) = compose_paging(plan.limit, plan.offset, plan.limit_is_top, request)?;
        let limit = apply_result_ceiling(limit, self.query_result_ceiling)?;
        let now = Timestamp::now();
        let holds = match (reader, ehrs.as_slice()) {
            (LogReader::Person, [own]) => self
                .person_holds(&[own.to_string()], now)
                .await
                .map_err(Failure::analysis)?,
            _ => None,
        };
        // NOTE: EHDS Art. 3(3), while a hold of the person is in force every entry the
        // query selects is read, so the page is cut from what the person is shown.
        let window = if holds.is_some() {
            (None, None)
        } else {
            (limit, offset)
        };
        let scope = LogScope {
            reader,
            ehrs: &ehrs,
            limit: window.0,
            offset: window.1,
        };
        let (sql, values) = statement(plan, params, &scope).map_err(Failure::plan)?;
        let run = log.select(sql, values);
        let records = self
            .within_budget(run)
            .await?
            .map_err(|error| Failure::analysis(log_failure(error)))?;
        let mut entries: Vec<AccessLogRecord> = records
            .into_iter()
            .filter(|record| matches!(record, AccessLogRecord::AccessLogEntry(_)))
            .collect();
        if let Some(holds) = &holds {
            entries.retain(|record| match record {
                AccessLogRecord::AccessLogEntry(entry) => !withheld(entry, holds, now),
                AccessLogRecord::AccessLogTombstone(_) => false,
            });
            let skip = offset.and_then(|o| usize::try_from(o).ok()).unwrap_or(0);
            let take = limit
                .and_then(|l| usize::try_from(l).ok())
                .unwrap_or(usize::MAX);
            entries = entries.into_iter().skip(skip).take(take).collect();
        }
        let rows: Vec<Vec<Value>> = entries
            .iter()
            .map(|record| row(plan, record, reader))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                Failure::analysis(ServiceError::internal("render the access-log entry", error))
            })?;
        let served_rows = u64::try_from(rows.len()).unwrap_or(u64::MAX);
        let log_read = LogRead {
            ehr_id: match reader {
                LogReader::Person => ehrs.first().map(EhrId::to_string),
                LogReader::Reviewer => one_ehr(&entries),
            },
        };
        let executed = substitute_params(aql, params);
        let mut outcome =
            QueryOutcome::plain(result_set_json(aql, &executed, name, result(plan, rows)));
        outcome.served_rows = served_rows;
        outcome.log_read = Some(log_read);
        Ok(outcome)
    }
}

/// The reader `request`'s standing makes the caller, and the EHRs whose log
/// they read; empty reads every EHR's.
///
/// # Errors
/// A `403` for a refused caller, and a `404` for the person scoping an EHR
/// other than their own.
fn reader_of(request: &AqlQueryRequest) -> Result<(LogReader, Vec<EhrId>), Failure> {
    match request.log_standing {
        LogStanding::Refused => Err(Failure::analysis(ServiceError::not_permitted(LOG_REFUSED))),
        LogStanding::Person(own) => match request.ehr_ids.iter().find(|ehr| **ehr != own) {
            // NOTE: EHDS Art. 9(2), another EHR answers as an unknown one does.
            Some(other) => Err(Failure::analysis(ServiceError::not_found(
                Missing::Ehr,
                format!("no EHR with id {other}"),
            ))),
            None => Ok((LogReader::Person, vec![own])),
        },
        LogStanding::Reviewer => Ok((LogReader::Reviewer, request.ehr_ids.clone())),
    }
}

/// The result of `plan` that served `rows`.
fn result(plan: &LogPlan, rows: Vec<Vec<Value>>) -> QueryResult {
    let columns = plan
        .columns
        .iter()
        .map(|column| ColumnMeta {
            name: column.name.clone(),
            path: match &column.value {
                LogCell::Path(path) => Some(path.text.clone()),
                LogCell::Literal(_) => None,
            },
        })
        .collect();
    QueryResult::of_rows(columns, rows)
}

/// The row `plan` selects of `record`, read out of the view `reader` is
/// served.
fn row(
    plan: &LogPlan,
    record: &AccessLogRecord,
    reader: LogReader,
) -> Result<Vec<Value>, serde_json::Error> {
    let view = view_of(record, reader)?.unwrap_or(Value::Null);
    Ok(plan
        .columns
        .iter()
        .map(|column| match &column.value {
            LogCell::Path(path) => path.read(&view),
            LogCell::Literal(primitive) => match primitive {
                Primitive::String(text) => Value::from(text.as_str()),
                Primitive::Integer(number) => Value::from(*number),
                Primitive::Real(number) => Value::from(*number),
                Primitive::Boolean(flag) => Value::from(*flag),
                Primitive::Null => Value::Null,
            },
        })
        .collect())
}

/// The EHR every entry of `entries` is about, when they are all about one.
fn one_ehr(entries: &[AccessLogRecord]) -> Option<String> {
    let mut ehrs = entries.iter().map(|record| match record {
        AccessLogRecord::AccessLogEntry(entry) => entry.ehr_id.as_ref().map(HierObjectId::value),
        AccessLogRecord::AccessLogTombstone(_) => None,
    });
    let first = ehrs.next()??;
    ehrs.all(|ehr| ehr == Some(first)).then(|| first.to_owned())
}
