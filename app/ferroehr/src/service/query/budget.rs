// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The bounds of one AQL execution, which the clinical, the joined and the
//! access-log paths share: the page the AQL clauses and the REST paging
//! compose, the result-row ceiling, and the execution time budget.

use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::service::query::request::AqlQueryRequest;

use super::execute::Failure;

impl FerroEhrService {
    /// `run`'s output, or the timeout failure when it overruns
    /// `[record.query] timeout`, so an over-long query is `408` rather than
    /// hanging to the global request timeout.
    ///
    /// # Errors
    /// [`Failure::timeout`] when the budget runs out first.
    pub(super) async fn within_budget<T>(
        &self,
        run: impl Future<Output = T>,
    ) -> Result<T, Failure> {
        // Boxed, so the caller's future holds the statement once and never beside its timer.
        let run = Box::pin(run);
        match self.query_timeout {
            Some(budget) => tokio::time::timeout(budget, run)
                .await
                .map_err(|_elapsed| Failure::timeout(budget)),
            None => Ok(run.await),
        }
    }
}

/// Compose the effective `(limit, offset)` from the AQL clauses and the REST
/// paging parameters.
///
/// The REST layer pages OVER the result set the AQL `LIMIT`/`OFFSET` clauses
/// define: `offset` is "the row number in result-set to start result-set
/// from" and `fetch` is "the number of rows to fetch" (ITS-REST query
/// `Request.md` §Common Headers and Query Parameters) — the two layers
/// compose; the only prohibited combination is `fetch` with the deprecated
/// AQL `TOP` modifier, which `Request.md` names explicitly.
pub(super) fn compose_paging(
    aql_limit: Option<i64>,
    aql_offset: Option<i64>,
    limit_is_top: bool,
    request: &AqlQueryRequest,
) -> Result<(Option<i64>, Option<i64>), Failure> {
    if request.fetch.is_some() && limit_is_top {
        return Err(Failure::analysis(ServiceError::precondition(
            "the `fetch` query parameter cannot be combined with the deprecated AQL TOP \
             modifier (ITS-REST query Request.md §Common Headers and Query Parameters)"
                .to_owned(),
        )));
    }
    if let Some(f) = request.fetch
        && f < 0
    {
        return Err(Failure::analysis(ServiceError::precondition(format!(
            "the `fetch` query parameter must be non-negative, got {f}"
        ))));
    }
    if let Some(o) = request.offset
        && o < 0
    {
        return Err(Failure::analysis(ServiceError::precondition(format!(
            "the `offset` query parameter must be non-negative, got {o}"
        ))));
    }
    let rest_offset = request.offset.unwrap_or(0);
    let offset = match (aql_offset, request.offset) {
        (None, None) => None,
        _ => Some(aql_offset.unwrap_or(0).saturating_add(rest_offset)),
    };
    let limit = match aql_limit {
        None => request.fetch,
        Some(l) => {
            // The page window sits inside the AQL-limited result set: skip
            // `rest_offset` rows of it, then take at most `fetch`.
            let remaining = l.saturating_sub(rest_offset).max(0);
            Some(request.fetch.map_or(remaining, |f| f.min(remaining)))
        }
    };
    Ok((limit, offset))
}

/// Apply the configured result-row ceiling to the composed page `limit`.
///
/// A query nothing else bounds takes the ceiling as its `LIMIT`: without it
/// the SQL would carry no `LIMIT` and materialise every matching row before
/// the `RESULT_SET` is built, one request an unbounded caller-chosen
/// allocation. A page asked for explicitly, by AQL `LIMIT` or the `fetch`
/// parameter, is served as written up to the ceiling and REFUSED above it:
/// a silent clamp would hand a client paging with its own `fetch` as the
/// stride a shorter page, and the rows between the clamped page and the next
/// `offset` would vanish with nothing in the `RESULT_SET` to say so. ITS-REST
/// leaves both the `fetch` default and its maximum to the implementation
/// (query `Request.md` §Common Headers and Query Parameters), and a refused
/// page is the released `400` branch ("invalid input … at least one of the
/// parameters has an invalid syntax", `400_Query`).
pub(super) fn apply_result_ceiling(
    limit: Option<i64>,
    ceiling: Option<i64>,
) -> Result<Option<i64>, Failure> {
    match (limit, ceiling) {
        (None, Some(ceiling)) => Ok(Some(ceiling)),
        (Some(asked), Some(ceiling)) if asked > ceiling => {
            Err(Failure::analysis(ServiceError::precondition(format!(
                "the requested page of {asked} rows exceeds the largest page this server \
                 serves ({ceiling} rows, `query.max_result_rows`); page with `offset` and \
                 a `fetch` at or below it"
            ))))
        }
        (bounded, _) => Ok(bounded),
    }
}
