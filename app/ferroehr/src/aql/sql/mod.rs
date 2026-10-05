// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! IR to SQL lowering.
//!
//! Turns a typed [`QueryIr`] into one `SELECT` over the greenfield
//! `node`/`version`/`ehr`/`audit` store, built entirely with `sea-query`'s
//! typed expression API and `sea-query-sqlx`, with no string-concatenated SQL.
//! Every table and column reference is an [`sea_query::Expr::col`], every
//! literal binds through `Expr::val`, and the PostgreSQL-specific pieces use the
//! sanctioned typed escape hatches: [`sea_query::Func::cust`] for functions
//! sea-query does not model (`jsonb_path_query_first`, `to_jsonb`, `upper_inf`,
//! `openehr_magnitude`), the typed Postgres operators from
//! [`sea_query::extension::postgres::PgExpr`], the built-in aggregates, and
//! `cast_as` for casts. The only string-operator escapes are the sanctioned
//! [`sea_query::BinOper::Custom`] set (`#>>`, `->>`). Runtime functions resolve
//! unqualified (the domain's own `search_path`, which ends in `ext, public`).
//!
//! No openEHR spec governs the execution: openEHR defines the language, not its
//! lowering, so the SQL shapes are our own design. The construct-by-construct
//! mapping to QUERY master03 lives in each submodule: `from` (FROM, containment
//! and scope gates), `select` (SELECT and aggregates), `predicate` (WHERE and
//! functions), `value` (the path split and coercions), and `expr` (the typed
//! building blocks and the `LIKE`/archetype translations).
//!
//! ## Coupling to the storage schema
//!
//! The builder references the `node`/`version`/`ehr`/`audit` column
//! vocabulary directly and encodes the nested-set, tier and
//! `branch_number` semantics of the greenfield store;
//! the planner's own structure-root notion is the RM model's
//! ([`openehr_rm::v1_2::model::is_structure_root`], kept in lockstep with the
//! vendored BMM by the `emit-rm-model` generator), which covers the clinical
//! content every AQL scope reaches today.
//! [`crate::storage::structure::is_structure_type`] is that set plus the
//! demographic party roots and the containers nested in them, which no FROM
//! class resolves to (`from::is_vo_root_type`).
//!
//! Every column is named through the schema catalog ([`crate::db::iden`]),
//! whose own tests pin each name against the clinical migration set and refuse
//! a column spelled as a string here, so a schema rename surfaces as a failing
//! test rather than a runtime SQL error.

mod expr;
mod from;
mod predicate;
mod select;
mod value;

use crate::db::iden::{Node, VersionRow};
use crate::ids::EhrId;
use std::collections::HashMap;
use std::sync::Arc;

use sea_query::{Alias, PostgresQueryBuilder, Query, SelectStatement, Value};
use sea_query_sqlx::{SqlxBinder as _, SqlxValues};

use super::error::{AqlError, SqlError};
use super::ir::{Bind, Coercion, ParamValue, Params, QueryIr};
use super::lineage::ArchetypeLineage;

use expr::{col, literal_value};

/// Static execution context the SQL generator needs beyond the IR: the CDR
/// system id (for `OBJECT_VERSION_ID` synthesis), the optional REST `ehr_id`
/// scope, and the composed effective paging window.
#[derive(Debug, Clone)]
pub struct SqlCtx {
    /// The openEHR system id stamped into synthesized `OBJECT_VERSION_ID`s.
    pub system_id: String,
    /// The set of EHRs the query is scoped to (`I_QUERY_SERVICE.execute_*`
    /// `ehr_ids: List<UUID> [0..1]`,
    /// `docs/specs/openehr/SM/docs/UML/classes/i_query_service.adoc`). The
    /// ITS-REST single `ehr_id` parameter is the one-element case. Empty = no
    /// explicit scope (the population gate over `is_queryable` EHRs applies).
    pub ehr_ids: Vec<EhrId>,
    /// The ABAC patient-scope subject id (no openEHR spec governs this, our own
    /// extension): when set, every
    /// VO root is restricted to EHRs whose `subject_id` equals it, so rows the
    /// caller may not see are never fetched — independent of what the query
    /// projects.
    pub subject_scope: Option<String>,
    /// The per-EHR access decision the caller's principal is held to
    /// (`EHR_ACCESS`, RM ehr `ehr_access.adoc`: "All access decisions to data
    /// in the EHR must be made in accordance with the policies and rules in
    /// this object"): every VO root and EHR source is restricted by it, so an
    /// EHR the principal may not read contributes no rows.
    pub ehr_access: EhrAccessScope,
    /// The effective row limit (AQL `LIMIT`/`TOP` or REST `fetch`, pre-composed).
    pub limit: Option<i64>,
    /// The effective row offset (AQL `OFFSET` or REST `offset`, pre-composed).
    pub offset: Option<i64>,
    /// The stored archetype specialisation graph the `archetype_node_id`
    /// predicate resolves an AOM2-era identifier's descendants through
    /// (AM `Identification` master07 §Supporting Archetype-based Querying: for
    /// specialised archetypes the lineage "can only be obtained from the
    /// operational form of the archetype"). Resolved once per execution from
    /// the ADL2/OPT2 artefact store (the query service's cached
    /// `archetype_lineage` read); an empty index leaves the predicate at exact
    /// + ADL 1.4 concept-prefix matching.
    pub archetype_lineage: Arc<ArchetypeLineage>,
}

/// Which EHRs a query may read rows from under the caller's per-EHR access
/// decision.
///
/// The REST adapter derives it from the `EHR_ACCESS` settings and the
/// authenticated principal; the SQL generator applies it to every EHR-anchored
/// source. No openEHR spec governs how the decision reaches a query — our own
/// design.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EhrAccessScope {
    /// Every EHR is readable.
    #[default]
    Unrestricted,
    /// Every EHR except these is readable.
    Excluding(Vec<EhrId>),
    /// Only these EHRs are readable; an empty list reads none.
    Only(Vec<EhrId>),
}

/// How one `RESULT_SET` cell is read back from the query rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellKind {
    /// A single SQL column read as one canonical-JSON value.
    Scalar,
    /// A whole structure object: four locator columns (`vo_id`, `sys_version`,
    /// `num`, `num_cap`) the executor reassembles through the node codec.
    WholeObject,
}

/// The read-back plan + `RESULT_SET` metadata for one SELECT column.
#[derive(Debug, Clone)]
pub struct ColumnSpec {
    /// The `RESULT_SET` column name (the `AS` alias, else `#{index}`).
    pub name: String,
    /// The `RESULT_SET` column path (the AQL path expression), if reconstructable.
    pub path: Option<String>,
    /// How the executor reads this column.
    pub kind: CellKind,
    /// The generated SQL column alias(es): one for [`CellKind::Scalar`], four
    /// (`vo`, `sv`, `num`, `cap`) for [`CellKind::WholeObject`].
    pub sql_cols: Vec<String>,
}

/// A built, bindable query: the SQL text, its bound values, and the per-column
/// read-back plan.
pub struct PreparedQuery {
    /// The generated SQL.
    pub sql: String,
    /// The bound parameter values (`sea-query-sqlx` → `sqlx`).
    pub values: SqlxValues,
    /// The `RESULT_SET` column read-back plan, in SELECT order.
    pub columns: Vec<ColumnSpec>,
    /// The SQL aliases of the hidden access-logging columns, one per bound VO
    /// root, appended after the projection.
    ///
    /// Empty when the plan cannot carry them (see [`ACCESS_EHR_PREFIX`]); the
    /// executor then reports no per-EHR breakdown rather than a wrong one.
    pub access_ehr_cols: Vec<String>,
    /// The SQL aliases of the hidden version-locator columns, one
    /// `(vo_id, sys_version)` pair per bound VO root, appended beside the
    /// access-EHR columns so the origins of the served data can be aggregated
    /// over the version rows (#3212). Empty in the same shapes.
    pub access_version_cols: Vec<(String, String)>,
}

/// The alias prefix of the hidden per-root EHR column the access log reads.
///
/// An AQL statement is one access to every EHR it actually served, and NEN 7513
/// and EHDS Art. 9 ask which records were accessed rather than how many. The
/// executed statement therefore carries each bound VO root's `ehr_id` beside
/// the projection, so the served set is read off the rows that were served —
/// not off a second, unpaged scan, which would name EHRs the query matched and
/// never disclosed.
///
/// Two plan shapes cannot carry it and get none: `SELECT DISTINCT`, where an
/// extra column changes which rows are distinct, and an aggregate projection,
/// where an ungrouped column is not even valid SQL. Both are recorded as a
/// statement-level access with a row count and no per-EHR breakdown.
///
/// **No openEHR spec governs this — our own design/extension.**
pub const ACCESS_EHR_PREFIX: &str = "access_ehr_";

/// The alias prefix of the hidden per-root `vo_id` locator column.
///
/// Appended beside the access-EHR columns with [`ACCESS_SV_PREFIX`], so the
/// access log can aggregate the origins of the served data over the version
/// rows it came from (EHDS Annex II 3.2(e), #3212). Same two excluded shapes.
pub const ACCESS_VO_PREFIX: &str = "access_vo_";
/// See [`ACCESS_VO_PREFIX`].
pub const ACCESS_SV_PREFIX: &str = "access_sv_";

// `SqlxValues` is not `Debug`; project the bound-value count instead so the
// struct still satisfies the workspace `missing_debug_implementations` lint.
impl std::fmt::Debug for PreparedQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedQuery")
            .field("sql", &self.sql)
            .field("value_count", &self.values.0.0.len())
            .field("columns", &self.columns)
            .field("access_ehr_cols", &self.access_ehr_cols)
            .field("access_version_cols", &self.access_version_cols)
            .finish()
    }
}

/// A built scope-collection query for the ABAC query post-check.
///
/// `SELECT DISTINCT` of every bound VO root's `ehr_id` + `template_id` over the
/// same containment/filter as the main query (no openEHR spec governs ABAC —
/// our own extension). Its rows carry the set of EHRs and templates the query
/// touches — across **all** bound variables and **independent of the
/// projection**.
#[derive(Debug)]
pub struct ScopeQuery {
    /// The generated SQL.
    pub sql: String,
    /// The bound parameter values.
    pub values: SqlxValues,
    /// The `(ehr-id column, template-id column)` alias pairs, one per bound VO
    /// root, the executor reads each result row's ehr/template from.
    pub columns: Vec<(String, String)>,
}

/// A VO "group": the node alias that roots it and the `version` alias its
/// nodes belong to. Content sources contained within it share the `vo` alias and
/// interval-join into `node`.
#[derive(Debug, Clone)]
struct VoGroup {
    node: String,
    vo: String,
    /// The resolved RM types of the operand that created this group — the
    /// containment-edge classifier's parent side.
    types: crate::aql::ir::TypeSet,
}

/// The extraction/coercion mode of a value expression.
#[derive(Debug, Clone, Copy)]
enum ValueMode {
    /// Raw jsonb value for a `RESULT_SET` cell.
    Projection,
    /// A coerced scalar for comparison / ordering / aggregation.
    Value(Coercion),
    /// A NULL-guarded numeric extraction for a mixed-type (`Raw`) leaf compared
    /// or matched against a numeric literal.
    RawNumeric,
}

/// The shared SQL-builder state; its `impl` blocks are fanned across the
/// submodules (`from`/`select`/`predicate`/`value`) — idiomatic, one struct.
struct Builder<'a> {
    ir: &'a QueryIr,
    params: &'a Params,
    ctx: &'a SqlCtx,
    q: SelectStatement,
    node_alias: HashMap<usize, String>,
    audit_alias: HashMap<String, String>,
    ehr_alias: HashMap<usize, String>,
    /// The `EHR_STATUS` root-node alias joined for an EHR source's `ehr_status`
    /// path (keyed by EHR source id; joined once, lazily, on first use).
    ehr_status_node: HashMap<usize, String>,
    /// The `version` alias each versioned-object-root RM source opened
    /// (keyed by source id). Used to synthesize the server-assigned
    /// `OBJECT_VERSION_ID` for a `uid[/value]` path on a VO-root variable
    /// (RM common master06 §Version Identification), which is not stored in the
    /// canonical fragment.
    vo_alias: HashMap<usize, String>,
    version_vo: HashMap<usize, String>,
    /// Node aliases that root a VO group (targets of the REST `ehr_id` filter).
    group_roots: Vec<String>,
    /// The subset of `group_roots` whose rows are join-linked to a bound EHR
    /// alias (`node.ehr_id = e.id`) — the population gate on the EHR alias
    /// already covers them, so gating the root again would be a duplicate
    /// full-population subquery per query.
    roots_linked_to_ehr: std::collections::HashSet<String>,
    /// The `version` alias for each entry in `group_roots` (parallel vec) —
    /// the source of the touched `template_id` for the ABAC scope collection.
    group_vos: Vec<String>,
    /// Fresh-alias counter for anchor subqueries / anti-joins.
    sub_ctr: usize,
    /// Whether the FROM was built in the STREAMING shape (one `version`
    /// FROM item, everything else a join) — lazily summoned tables
    /// (`ensure_audit`, the population gate, the `EHR_STATUS` root) must
    /// then JOIN instead of adding comma-separated FROM items, or a later
    /// `LATERAL` could no longer reference them (SQL join-tree scoping).
    streaming: bool,
}

impl<'a> Builder<'a> {
    fn new(ir: &'a QueryIr, params: &'a Params, ctx: &'a SqlCtx) -> Self {
        Self {
            ir,
            params,
            ctx,
            q: Query::select(),
            node_alias: HashMap::new(),
            audit_alias: HashMap::new(),
            ehr_alias: HashMap::new(),
            ehr_status_node: HashMap::new(),
            vo_alias: HashMap::new(),
            version_vo: HashMap::new(),
            group_roots: Vec::new(),
            roots_linked_to_ehr: std::collections::HashSet::new(),
            group_vos: Vec::new(),
            sub_ctr: 0,
            streaming: false,
        }
    }

    pub(super) fn next_ctr(&mut self) -> usize {
        let n = self.sub_ctr;
        self.sub_ctr += 1;
        n
    }

    pub(super) fn bind_value(&self, b: &Bind) -> Result<Value, AqlError> {
        match b {
            Bind::Literal(lit) => Ok(literal_value(lit)),
            Bind::Param(p) => self.param_value(p),
        }
    }

    pub(super) fn param_value(&self, name: &str) -> Result<Value, AqlError> {
        match self.params.get(name) {
            Some(ParamValue::Int(i)) => Ok(Value::from(*i)),
            Some(ParamValue::Real(r)) => Ok(Value::from(*r)),
            Some(ParamValue::Bool(b)) => Ok(Value::from(*b)),
            Some(ParamValue::Str(s)) => Ok(Value::from(s.clone())),
            Some(ParamValue::Null) => Ok(Value::from(Option::<String>::None)),
            None => Err(SqlError::UnboundParameter(name.to_owned()).into()),
        }
    }

    pub(super) fn param_str(&self, name: &str) -> Result<String, AqlError> {
        match self.params.get(name) {
            Some(ParamValue::Str(s)) => Ok(s.clone()),
            Some(ParamValue::Int(i)) => Ok(i.to_string()),
            Some(ParamValue::Real(r)) => Ok(r.to_string()),
            Some(ParamValue::Bool(b)) => Ok(b.to_string()),
            Some(ParamValue::Null) | None => {
                Err(SqlError::UnboundParameter(name.to_owned()).into())
            }
        }
    }
}

/// Whether the LIMIT-streaming FROM shape may apply. QUERY master03 §LIMIT:
/// without `ORDER BY`, "deterministic behavior" is explicitly not required —
/// which rows return is unconstrained, so a lazy streaming scan is
/// conformant. With `ORDER BY`, `DISTINCT`, or aggregates the full row set
/// is needed anyway; an EHR-scoped query (`ehr_ids`) is already bounded by
/// the `ehr_id` indexes and keeps the flat shape.
fn streaming_eligible(ir: &QueryIr, ctx: &SqlCtx) -> bool {
    ctx.limit.is_some()
        && ctx.ehr_ids.is_empty()
        && ir.order_by.is_empty()
        && !ir.distinct
        && !ir
            .select
            .iter()
            .any(|c| matches!(c.value, super::ir::SelectValue::Aggregate { .. }))
}

/// Lower a planned [`QueryIr`] to SQL.
///
/// # Errors
///
/// [`SqlError`] for a planner-accepted construct the SQL generator does not
/// render yet.
pub fn build(ir: &QueryIr, params: &Params, ctx: &SqlCtx) -> Result<PreparedQuery, AqlError> {
    let mut b = Builder::new(ir, params, ctx);
    match streaming_eligible(ir, ctx)
        .then(|| from::streaming_plan(ir))
        .flatten()
    {
        Some(plan) => b.build_from_streaming(&plan)?,
        None => b.build_from(&ir.contains)?,
    }
    b.apply_ehr_scope();
    b.apply_population_gate();
    let columns = b.build_select()?;
    let access_ehr_cols = b.build_access_ehr_columns(ir);
    let access_version_cols = b.build_access_version_columns(ir);
    b.build_where()?;
    b.build_order_by()?;
    b.build_paging();
    if ir.distinct {
        b.q.distinct();
    }
    let (sql, values) = b.q.build_sqlx(PostgresQueryBuilder);
    Ok(PreparedQuery {
        sql,
        values,
        columns,
        access_ehr_cols,
        access_version_cols,
    })
}

/// Build the ABAC scope-collection query for `ir`. `None` when the query has no
/// VO root (nothing to collect).
///
/// # Errors
/// [`SqlError`] for a construct the SQL generator does not render.
pub fn build_scope(
    ir: &QueryIr,
    params: &Params,
    ctx: &SqlCtx,
) -> Result<Option<ScopeQuery>, AqlError> {
    let mut b = Builder::new(ir, params, ctx);
    b.build_from(&ir.contains)?;
    b.apply_ehr_scope();
    b.apply_population_gate();
    b.build_where()?;
    let roots: Vec<(String, String)> = b
        .group_roots
        .clone()
        .into_iter()
        .zip(b.group_vos.clone())
        .collect();
    if roots.is_empty() {
        return Ok(None);
    }
    // One (ehr_id, template_id) column pair per bound VO root — the row-wise
    // cartesian of the joined roots, DISTINCT-ed, so every touched EHR/template
    // across every bound variable is collected.
    let mut columns = Vec::with_capacity(roots.len());
    for (i, (node, vo)) in roots.iter().enumerate() {
        let ehr_col = format!("scope_ehr_{i}");
        let template_col = format!("scope_template_{i}");
        b.q.expr_as(col(node, Node::EhrId), Alias::new(ehr_col.as_str()));
        b.q.expr_as(
            col(vo, VersionRow::TemplateId),
            Alias::new(template_col.as_str()),
        );
        columns.push((ehr_col, template_col));
    }
    b.q.distinct();
    let (sql, values) = b.q.build_sqlx(PostgresQueryBuilder);
    Ok(Some(ScopeQuery {
        sql,
        values,
        columns,
    }))
}
