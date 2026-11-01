// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Ad-hoc + stored AQL execution,
//! and the orchestration behind it: parse → plan → execute → assemble the
//! ITS-REST 1.1.0 `RESULT_SET`, the paging composition, the `ehr_ids`
//! resolution, and the per-query execution budget.
//!
//! Paging (ITS-REST QUERY `Request.md`): `fetch` is the page size and "cannot
//! be combined with AQL-`TOP`" (the one prohibited pairing); `offset` is the
//! 0-based start row. REST paging composes OVER the AQL `LIMIT`/`OFFSET`
//! window — the page is cut out of the AQL-shaped result set.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL result rows are arbitrary \
              projections by specification (QUERY 1.1)"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ids::EhrId;
use serde_json::Value;
use uuid::Uuid;

use openehr_query::parser::parse_str;

use crate::aql;
use crate::aql::error::{AqlError, ExecError};
use crate::aql::ir::{Params, QueryDomain, QueryIr};
use crate::aql::sql::SqlCtx;
use crate::db::acquire::acquire;
use crate::service::FerroEhrService;
use crate::service::error::{ErrorChain, internal_fault_caused};
use crate::service::query::request::{AqlQueryRequest, QueryOutcome};
use crate::service::status::{QUERY_TIMEOUT_TAG, StatusError};
use crate::system_log::categories::{AccessedContent, ContentIds};

use super::result_set::{build_params, result_set_json, substitute_params};

impl FerroEhrService {
    /// `execute_ad_hoc_query` — execute an ad hoc query, supplying the query
    /// text.
    ///
    /// # Errors
    ///
    /// - a query that does not parse, an unsupported AQL construct, a path/
    ///   typing error, an unbound `$parameter`, a REST-vs-AQL paging
    ///   collision, or a malformed `ehr_id` → precondition (`400`);
    /// - a listed EHR that does not exist → `ehr_id_does_not_exist`;
    /// - a whole-object projection that would serve a stored version body the
    ///   active `spec_profile` cannot express → conflict (`409`), our own
    ///   extension (`crate::versioning::profile::gate_result_bodies`);
    /// - a database/assembly failure, or an execution-budget overrun (tagged
    ///   [`QUERY_TIMEOUT_TAG`], rendered `408` at the wire) → exception.
    pub async fn execute_ad_hoc_query(
        &self,
        aql: String,
        request: AqlQueryRequest,
    ) -> Result<QueryOutcome, StatusError> {
        self.execute_aql(&aql, None, &request).await
    }

    /// `execute_stored_query` — execute a query stored in the definition
    /// service by its qualified name (`version` a semver.org string, latest
    /// when absent).
    ///
    /// # Errors
    ///
    /// - a malformed `version` (`400`), no matching stored query, or a stored
    ///   record with no query text → the definition-store error / exception;
    /// - otherwise exactly the [`FerroEhrService::execute_ad_hoc_query`]
    ///   failure conditions (the stored text executes identically).
    pub async fn execute_stored_query(
        &self,
        qualified_query_name: String,
        version: Option<String>,
        request: AqlQueryRequest,
    ) -> Result<QueryOutcome, StatusError> {
        // The stored AQL text resolves through the DEFINITION store by
        // exact/partial SEMVER, or the latest.
        let stored = self
            .get_stored_query(&qualified_query_name, version.as_deref())
            .await?;
        let aql = stored
            .get("q")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StatusError::exception(format!(
                    "stored query `{qualified_query_name}` has no query text"
                ))
            })?
            .to_owned();
        self.execute_aql(&aql, Some(&qualified_query_name), &request)
            .await
    }

    /// Parses, plans, executes, and assembles an AQL query into an ITS-REST
    /// `RESULT_SET`; `name` is the stored-query name for the result metadata
    /// (`None` for an ad-hoc query).
    ///
    /// Emits `aql_query_duration_seconds{phase}` for `plan` and `execute`, each
    /// recorded only when its phase completes, and `aql_queries_total{outcome}`
    /// exactly once per call, every failure arm routing through [`Failure`].
    ///
    /// # Errors
    ///
    /// See [`FerroEhrService::execute_ad_hoc_query`] — this is the shared
    /// orchestration both public calls delegate to.
    pub(crate) async fn execute_aql(
        &self,
        aql: &str,
        name: Option<&str>,
        request: &AqlQueryRequest,
    ) -> Result<QueryOutcome, StatusError> {
        let outcome = self.execute_aql_inner(aql, name, request).await;
        let label = match &outcome {
            Ok(_) => "ok",
            Err(failure) => failure.outcome,
        };
        count_query(label);
        outcome.map_err(|failure| failure.error)
    }

    /// The fallible body of [`FerroEhrService::execute_aql`]: every failure
    /// carries its `aql_queries_total{outcome}` label so the caller counts
    /// uniformly.
    async fn execute_aql_inner(
        &self,
        aql: &str,
        name: Option<&str>,
        request: &AqlQueryRequest,
    ) -> Result<QueryOutcome, Failure> {
        let plan_start = Instant::now();
        let params = build_params(request);
        // The IR is a pure function of the query text — parameter values,
        // paging and scope all bind downstream — so a cached plan serves every
        // caller of the same text correctly.
        let ir = match self.plan_query(aql, &params).await? {
            Planned::Single(ir) => ir,
            Planned::Joined(plan) => {
                record_phase("plan", plan_start);
                let exec_start = Instant::now();
                let outcome = self
                    .execute_joined(&super::join::JoinRequest {
                        aql,
                        name,
                        request,
                        plan: &plan,
                        params: &params,
                    })
                    .await?;
                record_phase("execute", exec_start);
                return Ok(outcome);
            }
        };
        record_phase("plan", plan_start);

        // A demographic query reads no EHR, so an EHR scope cannot apply to it;
        // ignoring the scope would answer rows the caller asked to exclude.
        // No openEHR spec governs AQL over a demographic repository: our own
        // design.
        if ir.domain == QueryDomain::Demographic && !request.ehr_ids.is_empty() {
            return Err(Failure::analysis(StatusError::precondition(
                "a query rooted at a demographic class reads no EHR, so an `ehr_id` scope \
                 cannot apply to it; send the query without one"
                    .to_owned(),
            )));
        }
        let (limit, offset) = compose_paging(ir.limit, ir.offset, ir.limit_is_top, request)?;
        let limit = apply_result_ceiling(limit, self.query_result_ceiling)?;
        // Multi-EHR scoping (`ehr_ids: List<UUID>`): a malformed id is a
        // client precondition (`400`); a well-formed but absent id raises
        // `ehr_id_does_not_exist`.
        let ehr_ids = self.resolve_ehr_ids(&request.ehr_ids).await?;
        let ctx = SqlCtx {
            system_id: self.effective_system_id(),
            ehr_ids,
            ehr_scope_kind: request.ehr_scope_kind,
            subject_scope: request.subject_scope.clone().map(|id| {
                aql::sql::subject::SubjectScope {
                    id,
                    namespace: request.subject_namespace.clone(),
                }
            }),
            ehr_access: request.ehr_access.clone(),
            limit,
            offset,
            // The stored specialisation graph an archetype predicate widens a
            // parent query through (BASE Artefact Identification
            // §Supporting Archetype-based Querying): resolved here, once per execution,
            // because SQL building is synchronous.
            archetype_lineage: self.archetype_lineage().await,
        };

        let exec_start = Instant::now();
        // `[query].timeout_ms` bounds the DB execution so an over-long query is
        // `408` rather than hanging to the global request timeout. The ABAC
        // scope collection stays a separate statement — the main query is paged,
        // so folding the sets in would scope only the served page — but runs
        // concurrently under the same budget (our own extension).
        // The demographic domain runs the same SQL on the party pool, whose
        // `search_path` names the `party` schema.
        let pool = match ir.domain {
            QueryDomain::Clinical => &self.pool,
            QueryDomain::Demographic => &self.demographic_pool,
        };
        let run = async {
            let exec = aql::exec::execute(pool, &ir, &params, &ctx, self.spec_profile);
            if request.collect_attributes {
                let (result, scope) =
                    tokio::try_join!(exec, aql::exec::collect_scope(pool, &ir, &params, &ctx))?;
                Ok((result, Some(scope)))
            } else {
                Ok((exec.await?, None))
            }
        };
        let (result, scope) = match self.query_timeout {
            Some(budget) => tokio::time::timeout(budget, run)
                .await
                .map_err(|_elapsed| Failure::timeout(budget))?,
            None => run.await,
        }
        .map_err(Failure::exec)?;
        record_phase("execute", exec_start);

        // `_executed_aql` carries the parameter-SUBSTITUTED text; `q` keeps
        // the original query as submitted.
        let executed = substitute_params(aql, &params);
        let served_rows = u64::try_from(result.rows.len()).unwrap_or(u64::MAX);
        let served_ehrs = result
            .served_ehrs
            .iter()
            .map(|(ehr, count)| (ehr.to_string(), *count))
            .collect();
        let served_origins = result.served_origins.clone();
        let origin_count = result.origin_count;
        let (accessed, served_ehr_content) = accessed_content(&result);
        let mut outcome = QueryOutcome::plain(result_set_json(aql, &executed, name, result));
        outcome.served_ehrs = served_ehrs;
        outcome.served_rows = served_rows;
        outcome.served_origins = served_origins;
        outcome.origin_count = origin_count;
        outcome.accessed = Some(accessed);
        outcome.served_ehr_content = served_ehr_content;
        outcome.domain = ir.domain;
        if let Some(scope) = scope {
            outcome.ehr_ids = scope.ehr_ids;
            outcome.template_ids = scope.template_ids;
            outcome.subjects = scope.subjects;
        }
        Ok(outcome)
    }

    /// Parse, terminology-expand, and lower `aql` into a typed [`QueryIr`],
    /// reusing a cached plan on a repeat of the same query text. `params` is
    /// validated present against the (cached or fresh) plan on every call —
    /// the values themselves bind later at SQL-build time, so the cached plan
    /// is independent of them.
    ///
    /// A plan whose `matches` operands were resolved through the terminology
    /// service is not cached, the resolution being free to differ on a later
    /// execution (QUERY master03 §TERMINOLOGY), so such a query always re-parses
    /// and re-expands. A query that joins a demographic root to its EHR is cut
    /// into its two halves ([`aql::join::split`]) and not cached either: each
    /// half is lowered when it runs.
    ///
    /// # Errors
    ///
    /// [`Failure`] on: unparseable AQL (precondition), a terminology
    /// expansion / lowering / parameter-binding [`AqlError`] (mapped by
    /// [`map_plan_error`], labelled by [`outcome_of`]).
    async fn plan_query(&self, aql: &str, params: &Params) -> Result<Planned, Failure> {
        // Cache hit: the plan is request-independent, but the caller's
        // bindings still must satisfy it.
        if let Some(ir) = self.plan_cache.get(aql).await {
            aql::check_params(&ir, params).map_err(Failure::plan)?;
            return Ok(Planned::Single(ir));
        }
        // Miss: full parse → terminology expansion → lowering.
        let mut ast = parse_str(aql).map_err(|e| {
            let message = format!("invalid AQL: {}", ErrorChain::new(&e));
            Failure::analysis(StatusError::precondition(message).with_source(e))
        })?;
        // Semantic-analysis pre-pass: resolve every `TERMINOLOGY('expand', …)`
        // used in a `matches` operand through the terminology-service seam
        // and merge the codes into the value list, before planning
        // (QUERY master03 lines 756–759).
        let expanded = aql::terminology::expand_matches(&mut ast, self)
            .await
            .map_err(Failure::plan)?;
        if let Some(plan) = aql::join::split(&ast).map_err(Failure::plan)? {
            return Ok(Planned::Joined(Box::new(plan)));
        }
        let ir = Arc::new(aql::lower_query(&ast, self.spec_profile).map_err(Failure::plan)?);
        // Cache only terminology-free plans (a resolved expansion may change).
        if !expanded {
            self.plan_cache
                .insert(aql.to_owned(), Arc::clone(&ir))
                .await;
        }
        aql::check_params(&ir, params).map_err(Failure::plan)?;
        Ok(Planned::Single(ir))
    }

    /// Resolve the request's `ehr_ids` (string form) into the scoped `Uuid`
    /// set: a malformed
    /// id is a client precondition (`400`); a well-formed but non-existent id
    /// raises `ehr_id_does_not_exist`. An empty list scopes nothing (the
    /// population gate over `is_queryable` EHRs applies).
    ///
    /// # Errors
    ///
    /// [`Failure`] (all labelled `analysis_error`) on: an unparseable UUID
    /// (precondition), a database fault during the existence check
    /// (exception), or a listed EHR that does not exist
    /// (`ehr_id_does_not_exist`).
    async fn resolve_ehr_ids(&self, ehr_ids: &[String]) -> Result<Vec<EhrId>, Failure> {
        if ehr_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::with_capacity(ehr_ids.len());
        for id in ehr_ids {
            #[expect(
                clippy::map_err_ignore,
                reason = "the mapped error already echoes the rejected token; the \
                          discarded `uuid::Error` adds only its own wording, which \
                          is not part of the wire contract"
            )]
            let uuid = Uuid::parse_str(id).map_err(|_| {
                Failure::analysis(StatusError::precondition(format!("invalid ehr_id `{id}`")))
            })?;
            ids.push(EhrId(uuid));
        }
        // Verify existence in one round-trip; report the first absent id. A
        // set, not a list probe: a cohort scope names a hundred thousand EHRs
        // (#3159), and a probe per id made that check quadratic.
        let present: std::collections::HashSet<Uuid> =
            // NOTE: EHDS Art. 8 (`docs/law/eu/ehds/text.html`), an EHR under a whole-EHR
            // access restriction reads as absent while the request enforces it.
            sqlx::query_scalar::<_, EhrId>(
                "SELECT id FROM ehr WHERE id = ANY($1) \
                 AND (NOT $2 OR access_restricted_at IS NULL)",
            )
            .bind(&ids)
            .bind(crate::system_log::access_context::access_restriction_enforced())
                .fetch_all(
                    &mut *acquire(&self.pool)
                        .await
                        .map_err(|e| Failure::analysis(StatusError::from(e)))?,
                )
                .await
                .map_err(|e| {
                    Failure::analysis(internal_fault_caused("resolve the requested ehr_ids", &e))
                })?
                .into_iter()
                .map(|id| id.0)
                .collect();
        if let Some(missing) = ids.iter().find(|id| !present.contains(&id.0)) {
            return Err(Failure::analysis(StatusError::ehr_not_found(format!(
                "no EHR with id {missing}"
            ))));
        }
        Ok(ids)
    }
}

/// A planned query: one statement, or the two halves of a query that joins a
/// demographic root to its EHR.
enum Planned {
    /// One single-domain statement.
    Single(Arc<QueryIr>),
    /// The party and clinical halves of a joined query.
    Joined(Box<aql::join::JoinPlan>),
}

/// What a statement touched, for the access record's category classification:
/// the identifiers of the served versions, or the query's positive constraints
/// when the answer carries none; and the identifiers per served EHR.
///
/// No openEHR spec governs access-log classification — our own design/extension
/// (EHDS Annex II 3.2(c), `docs/law/eu/ehds/text.html`).
pub(super) fn accessed_content(
    result: &aql::exec::QueryResult,
) -> (
    AccessedContent,
    std::collections::BTreeMap<String, Vec<ContentIds>>,
) {
    let mut overall = std::collections::BTreeSet::new();
    let mut per_ehr: std::collections::BTreeMap<String, Vec<ContentIds>> =
        std::collections::BTreeMap::new();
    for served in &result.served_content {
        let ids = ContentIds::of_kind(
            &served.kind,
            served.template_id.clone(),
            served.root_archetype.clone(),
        );
        if let Some(ehr_id) = served.ehr_id {
            per_ehr
                .entry(ehr_id.to_string())
                .or_default()
                .push(ids.clone());
        }
        overall.insert(ids);
    }
    let accessed = if overall.is_empty() {
        AccessedContent::Query(result.constraints.clone())
    } else {
        AccessedContent::Objects(overall.into_iter().collect())
    };
    (accessed, per_ehr)
}

/// A failed execution step carrying both the ITS-REST-mapped error and the
/// `aql_queries_total{outcome}` label to record for it — so the orchestration
/// can fail from any point (parse, terminology expansion, lowering, parameter
/// binding, paging, scoping, execution, scope collection) and the caller
/// counts + returns uniformly, exactly once.
pub(super) struct Failure {
    outcome: &'static str,
    error: StatusError,
}

impl Failure {
    /// A failure with the fixed `analysis_error` label (client preconditions
    /// and pre-execution service faults — the historical bookkeeping).
    pub(super) fn analysis(error: StatusError) -> Self {
        Self {
            outcome: "analysis_error",
            error,
        }
    }

    /// A planning-stage [`AqlError`] (terminology expansion, lowering,
    /// parameter binding) — labelled by [`outcome_of`], mapped by
    /// [`map_plan_error`].
    pub(super) fn plan(e: AqlError) -> Self {
        Self {
            outcome: outcome_of(&e),
            error: map_plan_error(e),
        }
    }

    /// An execution-stage [`AqlError`] — labelled by [`outcome_of`], mapped
    /// by [`map_exec_error`].
    pub(super) fn exec(e: AqlError) -> Self {
        Self {
            outcome: outcome_of(&e),
            error: map_exec_error(e),
        }
    }

    /// The execution-budget overrun: an `exception` [`StatusError`] whose message
    /// is prefixed with [`QUERY_TIMEOUT_TAG`] so the REST adapter renders it
    /// as `408 Request Timeout` (`Requests_and_responses.md` §HTTP status
    /// codes). The tag is stripped at the wire, leaving the clean detail.
    pub(super) fn timeout(budget: Duration) -> Self {
        Self {
            outcome: "exec_error",
            error: StatusError::exception(format!(
                "{QUERY_TIMEOUT_TAG}query execution exceeded the maximum time of {}ms; \
                 the server aborted the query",
                budget.as_millis()
            )),
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
        return Err(Failure::analysis(StatusError::precondition(
            "the `fetch` query parameter cannot be combined with the deprecated AQL TOP \
             modifier (ITS-REST query Request.md §Common Headers and Query Parameters)"
                .to_owned(),
        )));
    }
    if let Some(f) = request.fetch
        && f < 0
    {
        return Err(Failure::analysis(StatusError::precondition(format!(
            "the `fetch` query parameter must be non-negative, got {f}"
        ))));
    }
    if let Some(o) = request.offset
        && o < 0
    {
        return Err(Failure::analysis(StatusError::precondition(format!(
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
            Err(Failure::analysis(StatusError::precondition(format!(
                "the requested page of {asked} rows exceeds the largest page this server \
                 serves ({ceiling} rows, `query.max_result_rows`); page with `offset` and \
                 a `fetch` at or below it"
            ))))
        }
        (bounded, _) => Ok(bounded),
    }
}

/// The `aql_queries_total{outcome}` label for an [`AqlError`] (spec-silent
/// observability — our own design). Closed label set: `feature_rejected`,
/// `analysis_error`, `exec_error` (+ `ok`, recorded by the caller).
fn outcome_of(e: &AqlError) -> &'static str {
    match e {
        AqlError::Feature(_) => "feature_rejected",
        AqlError::Analysis(_) | AqlError::Sql(_) => "analysis_error",
        AqlError::Exec(_) => "exec_error",
    }
}

/// Increment `aql_queries_total{outcome}` — called exactly once per
/// [`FerroEhrService::execute_aql`] call.
fn count_query(outcome: &'static str) {
    crate::telemetry::metrics::metrics()
        .aql_queries
        .add(1, &[opentelemetry::KeyValue::new("outcome", outcome)]);
}

/// Record `aql_query_duration_seconds{phase}` for a completed phase.
fn record_phase(phase: &'static str, start: Instant) {
    crate::telemetry::metrics::metrics()
        .aql_query_duration
        .record(
            start.elapsed().as_secs_f64(),
            &[opentelemetry::KeyValue::new("phase", phase)],
        );
}

/// Map a planning error to an ITS-REST status: an unsupported feature, a
/// path/typing error, or an unrenderable construct all mean the query as
/// written cannot be served → `400 Bad Request` (ITS-REST `400_QUERY.yaml`).
fn map_plan_error(e: AqlError) -> StatusError {
    match e {
        AqlError::Feature(_) | AqlError::Analysis(_) | AqlError::Sql(_) => {
            StatusError::precondition(e.to_string()).with_source(e)
        }
        AqlError::Exec(inner) => map_exec_error(inner.into()),
    }
}

/// Map an execution error: an assembly/terminology failure is the server's
/// fault (`500`); a SQL-lowering failure that surfaces here is still a bad
/// query (`400`); a raw driver failure is CLASSIFIED like every other database
/// leg (`crate::storage::error::classify_sqlx`), so a pool-acquire timeout on
/// the query path sheds as `503 service-overloaded` exactly as it does on the
/// write paths instead of reporting a blanket `500`.
///
/// The `500` legs carry the curated opaque message: a driver string names the
/// schema objects the generated SQL touched, an assembly failure names the
/// internal node-row shape, and a missing column alias names a generated SQL
/// alias — server-internal detail that goes to `tracing` instead
/// ([`internal_fault_caused`]).
/// The refusal of a value the query compares that the database cannot cast to
/// the type of the path it is compared with. The driver error does not say
/// whether the value was written in the query or bound from
/// `query_parameters`, so the message names neither.
const CAST_REFUSAL: &str = "a value in the query cannot be coerced to the type its predicate \
                            compares against (an invalid date/time or text representation); \
                            correct the value";

fn map_exec_error(e: AqlError) -> StatusError {
    match e {
        // A data-exception the DRIVER raises here is a value of the query (a
        // literal or a bound `query_parameters` value) failing a text→type
        // cast: 22007 / 22008 / 22P02 (PostgreSQL Appendix A, errcodes) are a
        // 400 naming the defect class, never an opaque 500.
        AqlError::Exec(ExecError::Database(db))
            if matches!(
                db.as_database_error()
                    .and_then(sqlx::error::DatabaseError::code)
                    .as_deref(),
                Some("22007" | "22008" | "22P02")
            ) =>
        {
            StatusError::precondition(CAST_REFUSAL).with_source(db)
        }
        // Class 54 `program_limit_exceeded` (`54001` statement_too_complex,
        // PostgreSQL Appendix A) is the query's own shape outgrowing a database
        // limit, so it is the invalid input `400_Query` names, never a `500`.
        AqlError::Exec(ExecError::Database(db))
            if db
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .is_some_and(|code| code.starts_with("54")) =>
        {
            StatusError::precondition(
                "the query is too complex for the database to run; split it or \
                 write a long OR over one path as MATCHES {...}",
            )
            .with_source(db)
        }
        // No connection could be obtained: no statement ran, so the request can
        // be repeated (`503` plus `Retry-After`).
        AqlError::Exec(
            ExecError::Acquire(e)
            | ExecError::Assembly(crate::storage::error::StorageError::Acquire(e)),
        ) => StatusError::from(e),
        AqlError::Exec(ExecError::Database(db)) => crate::storage::error::classify_sqlx(&db),
        AqlError::Exec(ExecError::Assembly(a)) => {
            internal_fault_caused("assemble the RESULT_SET", &a)
        }
        AqlError::Exec(ExecError::Terminology(msg)) => StatusError::exception(msg),
        // The `spec_profile` refusal is already the `conflict` the resource
        // reads carry (→ `409`), so it crosses unchanged: same cause, same
        // status, same remedy, whichever route reached the stored body.
        AqlError::Exec(ExecError::Profile(e)) => StatusError::from(e),
        AqlError::Exec(e @ ExecError::MissingColumnAlias { .. }) => {
            internal_fault_caused("bind a RESULT_SET column to its generated SQL alias", &e)
        }
        AqlError::Feature(_) | AqlError::Analysis(_) | AqlError::Sql(_) => {
            StatusError::precondition(e.to_string()).with_source(e)
        }
    }
}

impl From<AqlError> for StatusError {
    /// Bridges an AQL-engine failure to the call-status model as a query
    /// execution reports it (`map_exec_error`): a connection that cannot be
    /// obtained is `service_overloaded` (`503` plus `Retry-After`).
    fn from(e: AqlError) -> Self {
        map_exec_error(e)
    }
}

#[cfg(test)]
mod tests {
    use super::{AqlError, ExecError, map_exec_error};
    use crate::service::status::ServiceStatus;

    /// The query path's database leg is classified, not blanket-500: a
    /// pool-acquire timeout sheds as `service_overloaded` (→ 503 +
    /// `Retry-After`) exactly as every other database leg does, while a
    /// genuine driver fault stays an `exception` (→ 500) with the curated
    /// message.
    #[test]
    fn query_database_failures_are_classified() {
        let shed = map_exec_error(AqlError::Exec(ExecError::Database(
            sqlx::Error::PoolTimedOut,
        )));
        assert_eq!(
            shed.status,
            ServiceStatus::ServiceOverloaded,
            "a pool-acquire timeout on the query path sheds, got {shed:?}"
        );

        let fault = map_exec_error(AqlError::Exec(ExecError::Database(
            sqlx::Error::RowNotFound,
        )));
        assert_eq!(
            fault.status,
            ServiceStatus::Exception,
            "an unclassified driver failure is still a server fault, got {fault:?}"
        );
        assert!(
            !fault.message.contains("RowNotFound"),
            "the driver diagnostic never reaches the wire body, got {fault:?}"
        );
    }

    /// A cast the database refuses is the query's fault (`400`), and its
    /// message never claims the value came from a query parameter: a literal
    /// written in the query reaches the same cast (#4378).
    #[test]
    fn a_cast_refusal_names_no_query_parameter() {
        for sqlstate in ["22007", "22008", "22P02"] {
            let err = map_exec_error(AqlError::Exec(ExecError::Database(
                crate::storage::error::tests::db_error(sqlstate),
            )));
            assert_eq!(
                err.status,
                ServiceStatus::PreconditionViolation,
                "{sqlstate}"
            );
            assert!(
                !err.message.contains("parameter"),
                "{sqlstate}: {}",
                err.message
            );
        }
    }

    /// A statement PostgreSQL refuses for its own complexity limit is the
    /// query's fault (`400`), not the server's.
    #[test]
    fn a_program_limit_refusal_is_a_precondition() {
        for sqlstate in ["54000", "54001"] {
            let err = map_exec_error(AqlError::Exec(ExecError::Database(
                crate::storage::error::tests::db_error(sqlstate),
            )));
            assert_eq!(
                err.status,
                ServiceStatus::PreconditionViolation,
                "{sqlstate}"
            );
        }
    }

    /// A closed pool: every acquire on it fails before any statement runs.
    async fn closed_pool() -> sqlx::PgPool {
        let options = sqlx::postgres::PgConnectOptions::new()
            .host("ferroehr-acquire-test.invalid")
            .username("nobody")
            .database("nothing");
        let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy_with(options);
        pool.close().await;
        pool
    }

    /// An AQL execution that cannot obtain a connection fails in the acquire
    /// phase and answers `service_overloaded` (→ `503` plus `Retry-After`): no
    /// statement ran, so the request can be repeated.
    #[tokio::test]
    async fn an_acquire_failure_on_execution_answers_503() {
        let ast = super::parse_str("SELECT e/ehr_id/value FROM EHR e").expect("the query parses");
        let params = crate::aql::ir::Params::new();
        let profile = crate::config::profile::SpecProfile::default();
        let ir = crate::aql::plan(&ast, &params, profile).expect("the query plans");
        let ctx = crate::aql::sql::SqlCtx {
            system_id: "sys.example.com".to_owned(),
            ehr_ids: Vec::new(),
            ehr_scope_kind: crate::aql::sql::EhrScopeKind::Named,
            subject_scope: None,
            ehr_access: crate::aql::sql::EhrAccessScope::Unrestricted,
            limit: None,
            offset: None,
            archetype_lineage: std::sync::Arc::new(crate::aql::lineage::ArchetypeLineage::default()),
        };
        let pool = closed_pool().await;
        let failure = crate::aql::exec::execute(&pool, &ir, &params, &ctx, profile)
            .await
            .expect_err("a closed pool yields no connection");
        assert!(
            matches!(failure, AqlError::Exec(ExecError::Acquire(_))),
            "the failure carries the acquire phase, got {failure:?}"
        );
        assert_eq!(
            map_exec_error(failure).status,
            ServiceStatus::ServiceOverloaded
        );
        let collected = crate::aql::exec::collect_scope(&pool, &ir, &params, &ctx)
            .await
            .expect_err("the scope collection acquires too");
        assert!(
            matches!(collected, AqlError::Exec(ExecError::Acquire(_))),
            "got {collected:?}"
        );
    }

    /// A node-codec read that cannot obtain a connection answers `503` too.
    #[tokio::test]
    async fn an_acquire_failure_while_assembling_answers_503() {
        let failure = crate::db::acquire::acquire(&closed_pool().await)
            .await
            .expect_err("a closed pool yields no connection");
        let err = map_exec_error(AqlError::Exec(ExecError::Assembly(
            crate::storage::error::StorageError::Acquire(failure),
        )));
        assert_eq!(err.status, ServiceStatus::ServiceOverloaded);
    }
}
