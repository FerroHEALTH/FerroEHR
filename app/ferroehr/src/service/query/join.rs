// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Execution of a query that joins a demographic root to the EHR its party is
//! the subject of ([`crate::aql::join`]), across the party, linkage and
//! clinical pools.
//!
//! No openEHR spec governs this: our own design. The plan runs in four
//! statements, one pool each, and never in one: the party query on the
//! demographic pool, the party-to-EHR map in force on the linkage pool, the
//! EHRs whose `EHR_STATUS.subject.external_ref` names one of those parties on
//! the clinical pool, and the clinical query on the clinical pool scoped to
//! the EHRs the two reads name (the cohort query's crossing,
//! `crate::service::linkage::cohort`). The rows are joined here on the union
//! of the two reads, sorted by the ranks each pool's own ordering gave the
//! keys, and paged. Each half is bounded by the server's result
//! ceiling (`query.max_result_rows`), and a half or a join larger than it is
//! refused rather than truncated: a silently shortened half would join to a
//! wrong answer with nothing in the `RESULT_SET` to say so.
//!
//! The clinical query carries the caller's per-EHR access decision and ABAC
//! patient scope as any query does; the party read and the crossing each
//! leave an access record in their own domain, the subject read on the
//! linkage record of the crossing as `ehr_id_for_subject` files it.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL result rows are arbitrary \
              projections by specification (QUERY 1.1)"
)]

use std::collections::BTreeMap;

use serde_json::Value;
use uuid::Uuid;

use crate::aql;
use crate::aql::exec::{ColumnMeta, QueryResult};
use crate::aql::ir::{Params, QueryDomain, QueryIr};
use crate::aql::join::{JoinPlan, Side};
use crate::aql::sql::{EhrAccessScope, EhrScopeKind, SqlCtx};
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::error::{internal_fault, internal_fault_caused};
use crate::service::linkage::LinkageError;
use crate::service::query::request::{AqlQueryRequest, QueryOutcome};
use crate::service::status::StatusError;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};

use super::execute::{Failure, accessed_content, apply_result_ceiling, compose_paging};
use super::result_set::{result_set_json, substitute_params};

/// The two halves of a joined query, lowered.
struct Halves {
    party: QueryIr,
    clinical: QueryIr,
}

/// One joined row: a party row and a clinical row of an EHR that party is the
/// subject of.
struct Joined<'a> {
    party: usize,
    clinical: usize,
    ehr: &'a str,
}

/// A joined query as the caller submitted it.
pub(super) struct JoinRequest<'a> {
    /// The AQL text as submitted.
    pub(super) aql: &'a str,
    /// The stored-query name for the result metadata, `None` for ad hoc.
    pub(super) name: Option<&'a str>,
    /// The paging, binds and scope of the execution.
    pub(super) request: &'a AqlQueryRequest,
    /// The two halves.
    pub(super) plan: &'a JoinPlan,
    /// The bound parameters.
    pub(super) params: &'a Params,
}

/// What the clinical half answered, with the ABAC scope it collected.
type ClinicalAnswer = (QueryResult, Option<aql::exec::QueryScope>);

impl FerroEhrService {
    /// Executes a joined query and assembles its `RESULT_SET`.
    ///
    /// # Errors
    /// [`Failure`] on: an `ehr_id` scope (a party is in no EHR), a paging
    /// collision, a half that does not plan, a half or a join above the result
    /// ceiling, a database failure, or an access record refused under
    /// `fail_mode = "closed"`.
    pub(super) async fn execute_joined(
        &self,
        job: &JoinRequest<'_>,
    ) -> Result<QueryOutcome, Failure> {
        if !job.request.ehr_ids.is_empty() {
            return Err(Failure::analysis(StatusError::precondition(
                "a query rooted at a demographic class names its EHRs through the party, so an \
                 `ehr_id` scope cannot apply to it; send the query without one"
                    .to_owned(),
            )));
        }
        let plan = job.plan;
        let (limit, offset) =
            compose_paging(plan.limit, plan.offset, plan.limit_is_top, job.request)?;
        let limit = apply_result_ceiling(limit, self.query_result_ceiling)?;
        let halves = Halves {
            party: lower_half(
                &plan.party,
                job.params,
                self.spec_profile,
                QueryDomain::Demographic,
            )?,
            clinical: lower_half(
                &plan.clinical,
                job.params,
                self.spec_profile,
                QueryDomain::Clinical,
            )?,
        };
        let run = self.run_joined(job, &halves, (limit, offset));
        match self.query_timeout {
            Some(budget) => tokio::time::timeout(budget, run)
                .await
                .map_err(|_elapsed| Failure::timeout(budget))?,
            None => run.await,
        }
    }

    /// The three statements and the join, under the caller's execution budget.
    async fn run_joined(
        &self,
        job: &JoinRequest<'_>,
        halves: &Halves,
        page: (Option<i64>, Option<i64>),
    ) -> Result<QueryOutcome, Failure> {
        let ceiling = self.query_result_ceiling;
        let party_ctx = SqlCtx {
            system_id: self.effective_system_id(),
            ehr_ids: Vec::new(),
            ehr_scope_kind: EhrScopeKind::Named,
            subject_scope: None,
            ehr_access: EhrAccessScope::Unrestricted,
            limit: ceiling.map(|c| c.saturating_add(1)),
            offset: None,
            archetype_lineage: self.archetype_lineage().await,
        };
        let party = aql::exec::execute(
            &self.demographic_pool,
            &halves.party,
            job.params,
            &party_ctx,
            self.spec_profile,
        )
        .await
        .map_err(Failure::exec)?;
        refuse_above(ceiling, party.rows.len(), "demographic")?;

        let parties = party_ids(&party.rows, job.plan.party_uid)?;
        let mut distinct: Vec<VoId> = parties.clone();
        distinct.sort_unstable_by_key(|p| p.0);
        distinct.dedup();
        let pairs = self.linked_ehrs(&distinct, &job.plan.root).await?;

        let (clinical, scope) = self.run_clinical(job, halves, party_ctx, &pairs).await?;
        refuse_above(ceiling, clinical.rows.len(), "clinical")?;

        let mut joined = join_rows(&parties, &pairs, &clinical.rows, job.plan.ehr_id)?;
        refuse_above(ceiling, joined.len(), "joined")?;
        sort_rows(&mut joined, job.plan, &party.rows, &clinical.rows);
        let page = cut_page(joined, page);
        self.assemble(job, halves, &page, (&party, &clinical), scope)
    }

    /// Runs the clinical half over the EHRs the map names, under the caller's
    /// per-EHR access decision and ABAC patient scope.
    async fn run_clinical(
        &self,
        job: &JoinRequest<'_>,
        halves: &Halves,
        party_ctx: SqlCtx,
        pairs: &[(VoId, EhrId)],
    ) -> Result<ClinicalAnswer, Failure> {
        let mut ehrs: Vec<EhrId> = pairs.iter().map(|(_, ehr)| *ehr).collect();
        ehrs.sort_unstable_by_key(|e| e.0);
        ehrs.dedup();
        // An empty map scopes nothing, and an unscoped clinical query is a
        // population query, the opposite of what was asked, so it never runs.
        if ehrs.is_empty() {
            return Ok((empty_result(), None));
        }
        // NOTE: RM `ehr_status.adoc` `is_queryable` ("included in population queries"):
        // the caller named no EHR, so the linked EHRs are a population subset.
        let ctx = SqlCtx {
            ehr_ids: ehrs,
            ehr_scope_kind: EhrScopeKind::PopulationSubset,
            subject_scope: job.request.subject_scope.clone().map(|id| {
                aql::sql::subject::SubjectScope {
                    id,
                    namespace: job.request.subject_namespace.clone(),
                }
            }),
            ehr_access: job.request.ehr_access.clone(),
            ..party_ctx
        };
        let exec = aql::exec::execute(
            &self.pool,
            &halves.clinical,
            job.params,
            &ctx,
            self.spec_profile,
        );
        if job.request.collect_attributes {
            let (result, scope) = tokio::try_join!(
                exec,
                aql::exec::collect_scope(&self.pool, &halves.clinical, job.params, &ctx)
            )
            .map_err(Failure::exec)?;
            Ok((result, Some(scope)))
        } else {
            Ok((exec.await.map_err(Failure::exec)?, None))
        }
    }

    /// Builds the served page's rows from the two halves, records the party
    /// read, and assembles the `RESULT_SET` and the access facts.
    fn assemble(
        &self,
        job: &JoinRequest<'_>,
        halves: &Halves,
        page: &[Joined<'_>],
        (party, clinical): (&QueryResult, &QueryResult),
        scope: Option<aql::exec::QueryScope>,
    ) -> Result<QueryOutcome, Failure> {
        let mut served: Vec<(Uuid, u64)> = Vec::new();
        let mut served_parties: Vec<usize> = Vec::new();
        let mut rows = Vec::with_capacity(page.len());
        for row in page {
            let ehr = Uuid::parse_str(row.ehr).map_err(|e| {
                Failure::analysis(internal_fault_caused("read the joined row's EHR id", &e))
            })?;
            match served.iter_mut().find(|(id, _)| *id == ehr) {
                Some((_, count)) => *count += 1,
                None => served.push((ehr, 1)),
            }
            if !served_parties.contains(&row.party) {
                served_parties.push(row.party);
            }
            rows.push(
                job.plan
                    .columns
                    .iter()
                    .map(|column| {
                        let source = match column.side {
                            Side::Party => party.rows.get(row.party),
                            Side::Clinical => clinical.rows.get(row.clinical),
                        };
                        source
                            .and_then(|cells| cells.get(column.index))
                            .cloned()
                            .unwrap_or(Value::Null)
                    })
                    .collect(),
            );
        }
        self.record_party_query(u64::try_from(served_parties.len()).unwrap_or(u64::MAX))
            .map_err(Failure::analysis)?;

        let result = QueryResult {
            columns: job
                .plan
                .columns
                .iter()
                .map(|c| ColumnMeta {
                    name: c.name.clone(),
                    path: c.path.clone(),
                })
                .collect(),
            rows,
            served_ehrs: served,
            served_origins: Vec::new(),
            origin_count: 0,
            served_content: Vec::new(),
            constraints: aql::constraints::positive_constraints(&halves.clinical, job.params),
        };
        let executed = substitute_params(job.aql, job.params);
        let served_ehrs = result
            .served_ehrs
            .iter()
            .map(|(ehr, count)| (ehr.to_string(), *count))
            .collect();
        let served_rows = u64::try_from(result.rows.len()).unwrap_or(u64::MAX);
        let (accessed, _) = accessed_content(&result);
        let mut outcome =
            QueryOutcome::plain(result_set_json(job.aql, &executed, job.name, result));
        outcome.served_ehrs = served_ehrs;
        outcome.served_rows = served_rows;
        outcome.accessed = Some(accessed);
        if let Some(scope) = scope {
            outcome.ehr_ids = scope.ehr_ids;
            outcome.template_ids = scope.template_ids;
            outcome.subjects = scope.subjects;
        }
        Ok(outcome)
    }

    /// The `(party, EHR)` pairs of the party-to-EHR map in force, read on the
    /// linkage pool, united with the EHRs whose subject names one of the
    /// `parties` as a `root`, read on the clinical pool, each pair once and
    /// ordered, with the linkage-domain access record the crossing owes.
    async fn linked_ehrs(
        &self,
        parties: &[VoId],
        root: &str,
    ) -> Result<Vec<(VoId, EhrId)>, Failure> {
        let mut pairs =
            crate::service::linkage::store::open_mapping_pairs(&self.linkage_pool, parties)
                .await
                .map_err(|e| Failure::analysis(linkage_failure(LinkageError::from(e))))?;
        pairs.extend(self.party_subject_ehrs(parties, root).await?);
        pairs.sort_unstable_by_key(|(party, ehr)| (party.0, ehr.0));
        pairs.dedup();
        self.emit_linkage_access(
            EventActionCode::Read,
            "party-ehr:query".to_owned(),
            None,
            u64::try_from(pairs.len()).unwrap_or(u64::MAX),
            true,
        )
        .map_err(|e| Failure::analysis(linkage_failure(e)))?;
        Ok(pairs)
    }

    /// The `(party, EHR)` pairs of the EHRs whose current `EHR_STATUS.subject`
    /// `external_ref` names one of `parties`, read on the clinical pool.
    ///
    /// The `id.value` is the party's versioned-object id, which is what
    /// `uid/value` carries before its first `::` (RM common master06 §Version
    /// Identification).
    async fn party_subject_ehrs(
        &self,
        parties: &[VoId],
        root: &str,
    ) -> Result<Vec<(VoId, EhrId)>, Failure> {
        let ids: Vec<String> = parties.iter().map(|party| party.0.to_string()).collect();
        // NOTE: BASE `OBJECT_REF.namespace` takes any name a deployment chooses, so the
        // namespace is not matched: the party's own id and its class decide.
        let found =
            crate::storage::ehr_repo::ehrs_by_subject_ids(&self.pool, &ids, subject_types(root))
                .await
                .map_err(|e| {
                    Failure::analysis(internal_fault_caused("read the EHRs of the subjects", &e))
                })?;
        found
            .into_iter()
            .map(|(subject, ehr)| {
                Uuid::parse_str(&subject)
                    .map(|party| (VoId(party), ehr))
                    .map_err(|e| {
                        Failure::analysis(internal_fault_caused("read a matched subject id", &e))
                    })
            })
            .collect()
    }

    /// Records the party read of a joined query as a demographic-domain access
    /// naming how many parties the served rows came from.
    ///
    /// # Errors
    /// [`StatusError`] when the record is refused under `fail_mode = "closed"`;
    /// the caller withholds its result.
    fn record_party_query(&self, parties: u64) -> Result<(), StatusError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let mut event = AuditEvent::new(
            EventActionCode::Execute,
            ObjectClass::Demographic,
            EventOutcome::Success,
        );
        event.domain = AccessDomain::Demographic;
        event.object_id = Some("party-query".to_owned());
        event.result_count = Some(parties);
        crate::service::linkage::stamp_requester(&mut event);
        event.legal_basis = self.audit_legal_basis().map(str::to_owned);
        self.record_access(event)
    }
}

/// The `OBJECT_REF.type` values a subject `external_ref` may carry to name a
/// party of class `root`.
fn subject_types(root: &str) -> &'static [&'static str] {
    // NOTE: BASE `OBJECT_REF.type` names "the class (concrete or abstract)" and
    // `PARTY_REF` admits `PARTY` and `ACTOR`, so a root matches itself and its RM ancestors.
    match root {
        "PERSON" => &["PERSON", "ACTOR", "PARTY"],
        "ORGANISATION" => &["ORGANISATION", "ACTOR", "PARTY"],
        "ROLE" => &["ROLE", "PARTY"],
        _ => &[],
    }
}

/// Lowers one half and checks it planned to the domain the split gave it.
fn lower_half(
    half: &openehr_query::ast::SelectQuery,
    params: &Params,
    profile: crate::config::profile::SpecProfile,
    domain: QueryDomain,
) -> Result<QueryIr, Failure> {
    let ir = aql::lower_query(half, profile).map_err(Failure::plan)?;
    aql::check_params(&ir, params).map_err(Failure::plan)?;
    if ir.domain != domain {
        return Err(Failure::analysis(internal_fault(
            "plan a joined query",
            &"a half lowered to the other domain",
        )));
    }
    Ok(ir)
}

/// Refuses a half or a join with more rows than the result ceiling.
fn refuse_above(ceiling: Option<i64>, rows: usize, what: &str) -> Result<(), Failure> {
    let Some(ceiling) = ceiling else {
        return Ok(());
    };
    if i64::try_from(rows).unwrap_or(i64::MAX) > ceiling {
        return Err(Failure::analysis(StatusError::precondition(format!(
            "the {what} half of a query that joins a demographic root to its EHR matches more \
             than {ceiling} rows (`query.max_result_rows`); narrow it with a WHERE condition"
        ))));
    }
    Ok(())
}

/// The party each party row came from, read off the root's `uid/value`
/// column: the versioned-object id before the first `::` (RM common master06
/// §Version Identification).
fn party_ids(rows: &[Vec<Value>], column: usize) -> Result<Vec<VoId>, Failure> {
    rows.iter()
        .map(|row| {
            let uid = row.get(column).and_then(Value::as_str).unwrap_or_default();
            let object = uid.split("::").next().unwrap_or_default();
            Uuid::parse_str(object).map(VoId).map_err(|e| {
                Failure::analysis(internal_fault_caused(
                    "read a party row's versioned-object id",
                    &e,
                ))
            })
        })
        .collect()
}

/// The joined rows, party-major: each party row with each clinical row of an
/// EHR its party is the subject of.
fn join_rows<'a>(
    parties: &[VoId],
    pairs: &[(VoId, EhrId)],
    clinical: &'a [Vec<Value>],
    ehr_column: usize,
) -> Result<Vec<Joined<'a>>, Failure> {
    let mut by_ehr: BTreeMap<&'a str, Vec<usize>> = BTreeMap::new();
    for (i, row) in clinical.iter().enumerate() {
        let ehr = row.get(ehr_column).and_then(Value::as_str).ok_or_else(|| {
            Failure::analysis(internal_fault(
                "read a clinical row's EHR id",
                &"the hidden EHR column is not a string",
            ))
        })?;
        by_ehr.entry(ehr).or_default().push(i);
    }
    let mut linked: BTreeMap<Uuid, Vec<String>> = BTreeMap::new();
    for (party, ehr) in pairs {
        linked.entry(party.0).or_default().push(ehr.0.to_string());
    }
    let mut out = Vec::new();
    for (p, party) in parties.iter().enumerate() {
        for ehr in linked.get(&party.0).into_iter().flatten() {
            if let Some((&key, rows)) = by_ehr.get_key_value(ehr.as_str()) {
                out.extend(rows.iter().map(|&c| Joined {
                    party: p,
                    clinical: c,
                    ehr: key,
                }));
            }
        }
    }
    Ok(out)
}

/// For each row of a half, the dense rank of each prefix of that half's order
/// keys, in the order the half's own `ORDER BY` returned the rows.
///
/// The `j`-th rank of a row counts how many distinct tuples of the first
/// `j + 1` keys precede it, so two rows that agree on every earlier key of
/// the half compare on key `j` exactly as the database ordered it.
fn prefix_ranks(rows: &[Vec<Value>], key_columns: &[usize]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::with_capacity(rows.len());
    let mut previous: Option<&Vec<Value>> = None;
    let mut counters = vec![0usize; key_columns.len()];
    for row in rows {
        if let Some(prev) = previous {
            let mut changed = false;
            for (j, &column) in key_columns.iter().enumerate() {
                changed |= row.get(column) != prev.get(column);
                if changed && let Some(counter) = counters.get_mut(j) {
                    *counter += 1;
                }
            }
        }
        out.push(counters.clone());
        previous = Some(row);
    }
    out
}

/// Sorts the joined rows by the `ORDER BY` keys, each compared through the
/// rank its own half's ordering assigned; ties keep the join's order.
fn sort_rows(
    joined: &mut [Joined<'_>],
    plan: &JoinPlan,
    party: &[Vec<Value>],
    clinical: &[Vec<Value>],
) {
    if plan.order.is_empty() {
        return;
    }
    let columns = |side: Side| -> Vec<usize> {
        (0..plan.half(side).order_by.len())
            .map(|rank| plan.key_column(side, rank))
            .collect()
    };
    let party_ranks = prefix_ranks(party, &columns(Side::Party));
    let clinical_ranks = prefix_ranks(clinical, &columns(Side::Clinical));
    let rank_of = |row: &Joined<'_>, side: Side, rank: usize| -> usize {
        let ranks = match side {
            Side::Party => party_ranks.get(row.party),
            Side::Clinical => clinical_ranks.get(row.clinical),
        };
        ranks.and_then(|r| r.get(rank)).copied().unwrap_or(0)
    };
    joined.sort_by(|a, b| {
        plan.order
            .iter()
            .map(|key| rank_of(a, key.side, key.rank).cmp(&rank_of(b, key.side, key.rank)))
            .find(|ordering| ordering.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// The page `(limit, offset)` cut out of the sorted joined rows.
fn cut_page(
    joined: Vec<Joined<'_>>,
    (limit, offset): (Option<i64>, Option<i64>),
) -> Vec<Joined<'_>> {
    let skip = usize::try_from(offset.unwrap_or(0)).unwrap_or(usize::MAX);
    let take = limit.map_or(usize::MAX, |l| usize::try_from(l).unwrap_or(usize::MAX));
    joined.into_iter().skip(skip).take(take).collect()
}

/// An empty clinical answer, for a party set no EHR is linked to.
fn empty_result() -> QueryResult {
    QueryResult {
        columns: Vec::new(),
        rows: Vec::new(),
        served_ehrs: Vec::new(),
        served_origins: Vec::new(),
        origin_count: 0,
        served_content: Vec::new(),
        constraints: crate::system_log::categories::QueryConstraints::default(),
    }
}

/// A linkage failure as the query surface reports it: a refused access record
/// keeps its own status, everything else is the store failing.
fn linkage_failure(error: LinkageError) -> StatusError {
    match error {
        LinkageError::Unrecorded(e) => e,
        other => internal_fault_caused("read the party-to-EHR map", &other),
    }
}
