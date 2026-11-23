// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! IR to SQL lowering.
//!
//! No openEHR spec governs the execution: openEHR defines the language, not its
//! lowering, so the SQL shapes are our own design. The construct-by-construct
//! mapping to QUERY master03 lives in each submodule: `from` (FROM, containment
//! and scope gates), `stream` (the LIMIT-streaming FROM shape), `select` (SELECT and aggregates), `predicate` (WHERE and
//! functions), `value` (the path split and coercions), and `expr` (the typed
//! building blocks and the `LIKE`/archetype translations).
//!
//! A demographic query ([`QueryDomain::Demographic`]) runs the same SQL on the
//! party pool, whose `search_path` names the `party` schema rendered from the
//! same DDL. It carries none of the EHR gates (the population gate, the
//! `ehr_ids` scope, the per-EHR access decision),
//! because the `party` schema has no `ehr` relation and a party is no EHR's
//! content; the restriction mark, the policy-party mark of a caller who is no
//! administrator (`gates`) and `LATEST_VERSION` ride the head join as for
//! every root. No openEHR spec governs AQL over a demographic repository:
//! our own design.

mod access;
mod anchor;
mod anchorable;
mod archetype;
mod attestation;
mod care;
mod caseload;
mod coerce;
mod compact;
mod condition;
mod contained;
pub(crate) mod expr;
mod fold;
mod from;
mod gates;
mod head;
pub(crate) mod jsonpath;
pub mod own_records;
mod param;
mod pinned;
mod predicate;
mod project;
pub mod readers;
mod scope;
mod select;
pub mod shape;
mod stream;
mod value;

use crate::db::iden::{Ehr, Node};
use crate::ids::EhrId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sea_query::{PostgresQueryBuilder, Query, SelectStatement};
use sea_query_sqlx::{SqlxBinder as _, SqlxValues};

use super::error::{AqlError, SqlError};
use super::ir::{Bind, Coercion, ParamValue, Params, QueryDomain, QueryIr};
use super::lineage::ArchetypeLineage;

use expr::col;
use readers::{EhrAccessReaders, PolicyParties};

/// Static execution context the SQL generator needs beyond the IR: the CDR
/// system id (for `OBJECT_VERSION_ID` synthesis), the optional REST `ehr_id`
/// scope, and the composed effective paging window.
#[derive(Debug, Clone)]
pub struct SqlCtx {
    /// The openEHR system id stamped into synthesized `OBJECT_VERSION_ID`s.
    pub system_id: String,
    /// The set of EHRs the query is scoped to (the `ehr_ids`
    /// list). The
    /// ITS-REST single `ehr_id` parameter is the one-element case. Empty = no
    /// explicit scope (the population gate over `is_queryable` EHRs applies).
    pub ehr_ids: Vec<EhrId>,
    /// Whether a non-empty [`Self::ehr_ids`] is a named target or a subset of
    /// the population, which the population gate still applies to.
    pub ehr_scope_kind: EhrScopeKind,
    /// The access evaluator's condition for the caller (`EHR_ACCESS`, RM ehr
    /// `ehr_access.adoc`: "All access decisions to data in the EHR must be
    /// made in accordance with the policies and rules in this object"): every
    /// VO root and EHR source is restricted by it, so a row the caller may
    /// not read is never fetched ([`crate::access_eval::condition`]).
    pub ehr_access: crate::access_eval::condition::AccessCondition,
    /// The effective row limit (AQL `LIMIT`/`TOP` or REST `fetch`, pre-composed).
    pub limit: Option<i64>,
    /// The effective row offset (AQL `OFFSET` or REST `offset`, pre-composed).
    pub offset: Option<i64>,
    /// The stored archetype specialisation graph the `archetype_node_id`
    /// predicate resolves an AOM2-era identifier's descendants through
    /// (BASE Artefact Identification, Referencing §Supporting Archetype-based
    /// Querying: for
    /// specialised archetypes the lineage "can only be obtained from the
    /// operational form of the archetype"). Resolved once per execution from
    /// the ADL2/OPT2 artefact store (the query service's cached
    /// `archetype_lineage` read); an empty index leaves the predicate at exact
    /// + ADL 1.4 concept-prefix matching.
    pub archetype_lineage: Arc<ArchetypeLineage>,
    /// Whether a demographic query serves the policy parties
    /// ([`crate::party_binding::policy`]) or leaves them out; a clinical query
    /// reads no party, so it never applies there.
    pub policy_parties: PolicyParties,
    /// The EHRs whose `EHR_ACCESS` objects the query reads rows of.
    pub access_settings: EhrAccessReaders,
    /// The caller's own records by the party binding, on which the EHDS
    /// Art. 3(3) delay withholds held data from the query (#6108); `None`
    /// where the delay holds on no record or on every one.
    pub own_records: Option<own_records::OwnRecords>,
    /// The caseload ([`crate::access_eval::caseload`]) narrowing the query.
    pub caseload: Option<Arc<crate::access_eval::caseload::Caseload>>,
}

impl SqlCtx {
    /// Whether the population gate over `is_queryable` EHRs applies: on an
    /// unscoped query and on a [`EhrScopeKind::PopulationSubset`] scope.
    #[must_use]
    pub fn population_gated(&self) -> bool {
        self.ehr_ids.is_empty() || self.ehr_scope_kind == EhrScopeKind::PopulationSubset
    }

    /// Whether the query is aimed at EHRs its caller named
    /// ([`EhrScopeKind::Named`] with at least one id), whose content is read
    /// from those EHRs' objects inward.
    #[must_use]
    pub fn names_ehrs(&self) -> bool {
        !self.population_gated()
    }
}

/// What an explicit [`SqlCtx::ehr_ids`] scope stands for, which decides
/// whether the population gate still applies to it.
///
/// RM `EHR_STATUS.is_queryable` (`org.openehr.rm.ehr.ehr_status.adoc`): "True
/// if this EHR should be included in population queries". A query aimed at
/// EHRs its caller named is not a population query; a query over a set of
/// EHRs the service selected out of the population is one. An empty
/// `ehr_ids` is the whole population whatever the kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EhrScopeKind {
    /// The caller named the EHRs (the ITS-REST `ehr_id` parameter, the one
    /// EHR an EXTRACT is built from): the query targets them, gate or not.
    #[default]
    Named,
    /// The service selected the EHRs out of the population (a cohort's
    /// subjects, the EHRs a demographic join links to): the population gate
    /// applies to them as to an unscoped query.
    PopulationSubset,
}

/// How one `RESULT_SET` cell is read back from the query rows.
#[derive(Debug, Clone, PartialEq)]
pub enum CellKind {
    /// A single SQL column holding the cell as `jsonb`: what a `SELECT
    /// DISTINCT` and an attestation list project, whose values PostgreSQL
    /// compares or aggregates.
    Scalar,
    /// A whole structure object: four locator columns (`vo_id`, `sys_version`,
    /// `num`, `num_cap`) the executor reassembles through the node codec.
    WholeObject,
    /// A data leaf read in Rust off the stored columns of a node row
    /// ([`NodeGroup`]): the fragment rebuilt and its path evaluated
    /// ([`crate::aql::fragment`]).
    Fragment(FragmentCell),
    /// One typed SQL value, rendered in Rust ([`crate::aql::cell`]); a
    /// `timestamptz` carries its session offset in a second column.
    Column(ColumnRender),
    /// An AQL literal, rendered from the query with no SQL column.
    Literal(crate::aql::ir::TypedLit),
}

/// A data leaf read off a node group.
#[derive(Debug, Clone, PartialEq)]
pub struct FragmentCell {
    /// The index of the node group in [`Layout::node_groups`].
    pub group: usize,
    /// The fragment path, `None` for the whole fragment.
    pub path: Option<crate::aql::fragment::FragmentPath>,
    /// Whether every match is one array cell (`NULL` for none) rather than
    /// the first match.
    pub list: bool,
    /// Whether the first SQL column is the source's root predicate, which
    /// must hold for the cell to carry a value.
    pub guarded: bool,
}

/// How a [`CellKind::Column`] value is rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnRender {
    /// By the column's SQL type.
    Typed,
    /// A `timestamptz` beside its session offset in seconds.
    Timestamp,
    /// The `OBJECT_VERSION_ID` object of a version uid text.
    VersionUid,
}

/// The stored columns of one node row a statement returns for the cells
/// read off it: `data`, the fragment columns and the root identity
/// (`crate::storage::node_repo`), consecutive from the column `first`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeGroup {
    /// The SQL alias of the group's first column.
    pub first: String,
}

/// The read-back plan + `RESULT_SET` metadata for one SELECT column.
#[derive(Debug, Clone, PartialEq)]
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
    /// In a debug build, the SQL alias of the jsonb cell PostgreSQL builds
    /// for a rendered column, which the rendered cell must equal byte for
    /// byte (`crate::aql::read`); `None` otherwise.
    pub reference: Option<String>,
}

/// A built, bindable query: the SQL text, its bound values, and the per-column
/// read-back plan.
pub struct PreparedQuery {
    /// The generated SQL, shared with the statement shape it was bound from
    /// ([`shape`]).
    pub sql: Arc<str>,
    /// The bound parameter values (`sea-query-sqlx` → `sqlx`).
    pub values: SqlxValues,
    /// How the executor reads the rows back.
    pub layout: Arc<Layout>,
    /// Whether the text is a statement shape's ([`shape`]), one of the
    /// bounded set a repeated request binds, rather than built for this
    /// request alone.
    pub shaped: bool,
}

/// How the executor reads the rows of a statement back: the `RESULT_SET`
/// columns and the hidden access-logging columns beside them.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// The `RESULT_SET` column read-back plan, in SELECT order.
    pub columns: Vec<ColumnSpec>,
    /// The node rows whose stored columns the fragment cells read.
    pub node_groups: Vec<NodeGroup>,
    /// The SQL aliases of the hidden access-logging columns, one per bound
    /// `EHR` source and per bound VO root, appended after the projection.
    ///
    /// Empty when the plan cannot carry them (see [`ACCESS_EHR_PREFIX`]); the
    /// executor then reports no per-EHR breakdown rather than a wrong one.
    pub access_ehr_cols: Vec<String>,
    /// The SQL aliases of the hidden columns that say, beside each access-EHR
    /// column, whether its EHR carries `EHR_ACCESS` settings
    /// ([`ACCESS_SETTLED_PREFIX`]).
    pub access_settled_cols: Vec<String>,
    /// Whether each access column holds, per row, the array of the EHRs an
    /// aggregate projection read in place of one EHR id (#5639).
    pub access_aggregated: bool,
    /// The SQL aliases of the hidden version-locator columns, one
    /// `(vo_id, sys_version)` pair per bound VO root, appended beside the
    /// access-EHR columns so the origins of the served data can be aggregated
    /// over the version rows (#3212). Empty in the same shapes.
    pub access_version_cols: Vec<(String, String)>,
    /// Whether the statement selects the policy and definition revisions as
    /// [`POLICY_REVISION_COL`] and [`DEFINITION_REVISION_COL`] (#6320): a
    /// clinical query a request judges whose served versions are not read
    /// after it, a read that selects them itself.
    pub revisions: bool,
}

/// The alias prefix of the hidden per-row EHR columns the access log reads.
///
/// An AQL statement is one access to every EHR it actually served, and NEN 7513
/// and EHDS Art. 9 ask which records were accessed rather than how many. The
/// executed statement therefore carries the EHR id of each bound `EHR` source
/// and each bound VO root beside the projection, so the served set is read off
/// the rows that were served, and a statement that projects only an EHR names
/// it too. It is never read off a second, unpaged scan, which would name EHRs
/// the query matched and never disclosed.
///
/// `SELECT DISTINCT` cannot carry it, since an extra column changes which
/// rows are distinct, and is recorded as a statement-level access with a row
/// count and no per-EHR breakdown. An aggregate projection, where an
/// ungrouped column is not valid SQL, carries per row the array of the
/// distinct EHRs its aggregate read, so every EHR whose data shaped the
/// result is named (EHDS Art. 9, #5639).
///
/// **No openEHR spec governs this — our own design/extension.**
pub const ACCESS_EHR_PREFIX: &str = "access_ehr_";

/// The alias of the hidden column holding the policy revision a judged
/// clinical statement read (`Layout::revisions`), the name every statement
/// that selects `access_revision!` gives it.
pub const POLICY_REVISION_COL: &str = "policy_revision";

/// The alias of the hidden column holding the definition revision beside
/// [`POLICY_REVISION_COL`].
pub const DEFINITION_REVISION_COL: &str = "definition_revision";

/// The alias of the hidden column holding, in a ride, the party store's
/// grant, facts and binding revisions beside [`POLICY_REVISION_COL`] (#6320).
pub const PARTY_REVISIONS_COL: &str = "party_revisions";

/// The alias prefix of the hidden column of whether an EHR carries settings.
///
/// It sits beside each access-EHR column and says whether that EHR carries
/// `EHR_ACCESS` settings, so the access record of an EHR a query served reads
/// its settings only when it has some (#5588). Same two excluded shapes. No
/// openEHR spec governs this: our own design.
pub const ACCESS_SETTLED_PREFIX: &str = "access_set_";

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
            .field("layout", &self.layout)
            .field("shaped", &self.shaped)
            .finish()
    }
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
    /// A mixed-type (`Raw`) leaf's number: a JSON number, or the magnitude of
    /// a whole numeric `DV_ORDERED` ([`coerce::raw_numeric`]); NULL otherwise.
    RawNumeric,
    /// A mixed-type leaf's text when it is a JSON string, number or Boolean;
    /// NULL for a whole object ([`coerce::raw_text`]).
    RawText,
    /// The instant of a mixed-type leaf that is a whole date/time value of the
    /// kinds given; NULL otherwise ([`coerce::raw_temporal`]).
    RawTemporal(coerce::TemporalKinds),
    /// A mixed-type leaf's amount for `SUM` and `AVG`: a JSON number, or the
    /// magnitude of a whole `DV_AMOUNT`; any other value fails the statement
    /// ([`coerce::raw_amount`]).
    RawAmount,
}

/// The shared SQL-builder state; its `impl` blocks are fanned across the
/// submodules (`from`/`select`/`predicate`/`value`) — idiomatic, one struct.
///
/// Its alias maps are ordered (`BTreeMap`/`BTreeSet`), so any iteration over
/// them emits SQL in source-id order and the statement is byte-deterministic.
struct Builder<'a> {
    ir: &'a QueryIr,
    params: &'a Params,
    ctx: &'a SqlCtx,
    q: SelectStatement,
    node_alias: BTreeMap<usize, String>,
    audit_alias: BTreeMap<String, String>,
    ehr_alias: BTreeMap<usize, String>,
    /// The `EHR_STATUS` root-node alias joined for an EHR source's `ehr_status`
    /// path (keyed by EHR source id; joined once, lazily, on first use).
    ehr_status_node: BTreeMap<usize, String>,
    /// The `version` alias of each joined `EHR_STATUS` root (keyed by EHR source
    /// id).
    ehr_status_vo: BTreeMap<usize, String>,
    /// The `version` alias each versioned-object-root RM source opened
    /// (keyed by source id). Used to synthesize the server-assigned
    /// `OBJECT_VERSION_ID` for a `uid[/value]` path on a VO-root variable
    /// (RM common master06 §Version Identification), which is not stored in the
    /// canonical fragment.
    vo_alias: BTreeMap<usize, String>,
    version_vo: BTreeMap<usize, String>,
    /// The head aliases joined ahead of their version row, whose scope adds
    /// only its predicates (`head::HeadPlacement::Joined`).
    joined_heads: BTreeSet<String>,
    /// Node aliases that root a VO group (targets of the REST `ehr_id` filter).
    group_roots: Vec<String>,
    /// The subset of `group_roots` whose rows are join-linked to a bound EHR
    /// alias (`node.ehr_id = e.id`), each with that alias: the population gate
    /// on the EHR alias already covers them, so gating the root again would be
    /// a duplicate full-population subquery per query.
    roots_linked_to_ehr: BTreeMap<String, String>,
    /// The `version` alias for each entry in `group_roots` (parallel vec).
    group_vos: Vec<String>,
    /// The `(node, version)` left-binding aliases of each `OR` branch operand
    /// bound as a versioned object of its own: reported to the access log like
    /// a group root, but never filtered as one,
    /// since a `NULL` binding is a row whose branch did not match.
    branch_roots: Vec<(String, String)>,
    /// Fresh-alias counter for anchor subqueries / anti-joins.
    sub_ctr: usize,
    /// Whether the FROM was built in the STREAMING shape (one `version`
    /// FROM item, everything else a join) — lazily summoned tables
    /// (`ensure_audit`, the population gate, the `EHR_STATUS` root) must
    /// then JOIN instead of adding comma-separated FROM items, or a later
    /// `LATERAL` could no longer reference them (SQL join-tree scoping).
    streaming: bool,
    /// How the anchor of a range comparison enters the statement.
    range_entry: anchor::RangeEntry,
    /// Whether a whole-object read only has to say that the object is there:
    /// set while a plain `COUNT` lowers its argument, so the row stands for
    /// its rebuilt fragment, which is never `NULL` (`crate::storage::slim`).
    presence_only: bool,
    /// The EHDS Art. 8 and Art. 3(3) access gates, read once when the builder
    /// starts (`crate::system_log::access_context`).
    gates: gates::Gates<'a>,
    /// The structure steps the SELECT paths bind once per row
    /// ([`Builder::bind_select_paths`]), one entry per distinct step prefix.
    step_bindings: Vec<StepBinding>,
    /// The node aliases whose stored columns the projection returns, in
    /// group order ([`NodeGroup`]).
    node_groups: Vec<(String, NodeGroup)>,
    /// Whether each rendered cell also selects the jsonb cell PostgreSQL
    /// builds for it ([`build_executed`]).
    references: bool,
}

/// One bound structure-step prefix: the steps from `source` down, and the
/// alias of the node the last of them binds in this row.
#[derive(Debug, Clone)]
struct StepBinding {
    source: usize,
    steps: Vec<super::ir::StructureStep>,
    alias: String,
}

impl<'a> Builder<'a> {
    fn new(ir: &'a QueryIr, params: &'a Params, ctx: &'a SqlCtx) -> Self {
        Self {
            ir,
            params,
            ctx,
            q: Query::select(),
            node_alias: BTreeMap::new(),
            audit_alias: BTreeMap::new(),
            ehr_alias: BTreeMap::new(),
            ehr_status_node: BTreeMap::new(),
            ehr_status_vo: BTreeMap::new(),
            vo_alias: BTreeMap::new(),
            version_vo: BTreeMap::new(),
            joined_heads: BTreeSet::new(),
            group_roots: Vec::new(),
            roots_linked_to_ehr: BTreeMap::new(),
            group_vos: Vec::new(),
            branch_roots: Vec::new(),
            sub_ctr: 0,
            streaming: false,
            range_entry: anchor::RangeEntry::default(),
            presence_only: false,
            gates: gates::Gates::current(
                ir.domain,
                ctx.policy_parties,
                &ctx.access_settings,
                &ctx.ehr_access,
                ctx.own_records.as_ref(),
                ctx.caseload.as_deref(),
            ),
            step_bindings: Vec::new(),
            node_groups: Vec::new(),
            references: false,
        }
    }

    /// Every bound versioned-object root as a `(node, version)` alias pair:
    /// the group roots, then the `OR` branch roots.
    fn served_roots(&self) -> Vec<(String, String)> {
        self.group_roots
            .iter()
            .cloned()
            .zip(self.group_vos.iter().cloned())
            .chain(self.branch_roots.iter().cloned())
            .collect()
    }

    /// The EHR id of every EHR a row of this query is drawn from: each bound
    /// `EHR` source's `id`, then the `ehr_id` of each served root no source
    /// covers.
    ///
    /// A query that reads only the `ehr` row (`FROM EHR e` with no
    /// containment, or an `ehr_status` path) binds no versioned-object root
    /// and still serves the EHR its row names, so the access log
    /// ([`ACCESS_EHR_PREFIX`]) reads this one list. A root linked to an `EHR`
    /// source names that source's EHR, so its column would repeat the
    /// source's and its settings probe the source's (#5782). No openEHR spec
    /// governs this: our own design.
    fn served_ehr_ids(&self) -> Vec<sea_query::Expr> {
        self.ehr_alias
            .values()
            .map(|alias| col(alias, Ehr::Id))
            .chain(
                self.served_roots()
                    .iter()
                    .filter(|(root, _)| !self.roots_linked_to_ehr.contains_key(root))
                    .map(|(root, _)| col(root, Node::EhrId)),
            )
            .collect()
    }

    pub(super) fn next_ctr(&mut self) -> usize {
        let n = self.sub_ctr;
        self.sub_ctr += 1;
        n
    }
}

/// The EHR id every row of a query belongs to, or `None`.
///
/// Its containment tree is rooted at one `EHR` source, and that source's own
/// predicate or a top-level conjunct of its `WHERE` clause holds
/// `ehr_id/value` equal to a string literal or a bound string parameter.
///
/// The access evaluator decides care for that one EHR instead of binding the
/// caller's whole care census into the statement (#5699). No openEHR spec
/// governs this: our own design.
#[must_use]
pub fn pinned_ehr(ir: &QueryIr, params: &Params) -> Option<String> {
    use crate::aql::ir::{
        ContainsTree, EhrField, Expr as IrExpr, Operand, PathTarget, Source, TypedLit,
    };
    use openehr_query::lexer::CompOp;
    fn conjunct(filter: &IrExpr, ehr: usize) -> Option<&Operand> {
        match filter {
            IrExpr::And(a, b) => conjunct(a, ehr).or_else(|| conjunct(b, ehr)),
            IrExpr::Compare {
                lhs:
                    Operand::Path(PathTarget::Ehr {
                        source,
                        field: EhrField::EhrId | EhrField::Whole,
                    }),
                op: CompOp::Eq,
                rhs,
                ..
            } if source.0 == ehr => Some(rhs),
            _ => None,
        }
    }
    let ContainsTree::Operand { source, .. } = &ir.contains else {
        return None;
    };
    let Some(Source::Ehr(ehr)) = ir.sources.get(source.0) else {
        return None;
    };
    let text = |lit: &TypedLit| match lit {
        TypedLit::String(s) => Some(s.clone()),
        _ => None,
    };
    let param = |name: &str| match params.get(name) {
        Some(ParamValue::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let in_predicate = ehr
        .predicates
        .iter()
        .filter(|p| p.field == EhrField::EhrId && p.op == CompOp::Eq)
        .find_map(|p| match &p.value {
            Bind::Literal(lit) => text(lit),
            Bind::Param(name) => param(name),
        });
    in_predicate.or_else(
        || match ir.filter.as_ref().and_then(|f| conjunct(f, source.0))? {
            Operand::Literal(lit) => text(lit),
            Operand::Param(name) => param(name),
            _ => None,
        },
    )
}

/// Lowers a planned [`QueryIr`] to SQL.
///
/// # Errors
///
/// [`crate::aql::error::AqlFeatureError`] for a read of a variable that binds
/// no row (`NOT CONTAINS`), and [`SqlError`] for a planner-accepted construct
/// the SQL generator does not render.
pub fn build(ir: &QueryIr, params: &Params, ctx: &SqlCtx) -> Result<PreparedQuery, AqlError> {
    let builder = Builder::new(ir, params, ctx);
    Builder::lower(builder)
}

/// The statement the executor runs for `ir`: [`build`], and in a debug
/// build each rendered cell beside the jsonb cell PostgreSQL builds for it,
/// which the executor holds the rendered cell to byte for byte
/// (`crate::aql::read`), so every AQL test checks the rendering.
///
/// # Errors
/// As [`build`].
pub(crate) fn build_executed(
    ir: &QueryIr,
    params: &Params,
    ctx: &SqlCtx,
) -> Result<PreparedQuery, AqlError> {
    let mut builder = Builder::new(ir, params, ctx);
    builder.references =
        cfg!(debug_assertions) && REFERENCE_CELLS.load(std::sync::atomic::Ordering::Relaxed);
    Builder::lower(builder)
}

/// Whether a debug build selects the jsonb reference cells ([`build_executed`]).
static REFERENCE_CELLS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Stops a debug build of this process from selecting the jsonb reference
/// cells beside the rendered ones, so a measurement harness counts the
/// statements and allocations a release build makes.
pub fn without_reference_cells() {
    REFERENCE_CELLS.store(false, std::sync::atomic::Ordering::Relaxed);
}

impl Builder<'_> {
    /// Lowers the builder's query to its statement ([`build`]).
    ///
    /// # Errors
    /// As [`build`].
    fn lower(mut b: Self) -> Result<PreparedQuery, AqlError> {
        let (ir, ctx) = (b.ir, b.ctx);
        from::refuse_negated_reads(ir)?;
        // NOTE: our own design (#5699), the service reads care before a statement is built, so an
        // unread care is refused loudly rather than withholding every cared row.
        if ctx.ehr_access.reads_care()
            && ctx.ehr_access.groups().is_some_and(|groups| {
                groups.cared == crate::access_eval::condition::care::Care::Unresolved
            })
        {
            return Err(SqlError::Unsupported(
                "an access condition whose care the service has not read".to_owned(),
            )
            .into());
        }
        // A query aimed at named EHRs, or pinned to one by its own text, reads
        // only those EHRs' versions, which the spine enters through the
        // `ehr_id` index, so it streams whatever its clauses (#5076).
        let plan = stream::streaming_plan(ir).filter(|plan| {
            stream::streaming_eligible(ir, ctx)
                || ctx.names_ehrs()
                || (ctx.ehr_ids.is_empty() && (plan.counts_roots(ir) || plan.pins_one_ehr(ir)))
        });
        match plan {
            Some(plan) => {
                b.range_entry = if stream::streaming_eligible(ir, ctx) && !plan.pins_one_ehr(ir) {
                    anchor::RangeEntry::Leading
                } else {
                    anchor::RangeEntry::Off
                };
                b.build_from_streaming(&plan)?;
            }
            None => b.build_from(&ir.contains)?,
        }
        let clinical = ir.domain == QueryDomain::Clinical;
        if clinical {
            b.apply_ehr_scope();
            b.apply_population_gate();
        }
        b.bind_select_paths()?;
        let columns = b.build_select()?;
        let (access_ehr_cols, access_settled_cols) = if clinical {
            b.build_access_ehr_columns(ir)
        } else {
            (Vec::new(), Vec::new())
        };
        let access_version_cols = b.build_access_version_columns(ir);
        // NOTE: our own design (#6320), a statement whose served versions are read after it
        // leaves the revisions to that one-row read, so no row of its own carries them.
        let revisions =
            clinical && ctx.ehr_access.groups().is_some() && access_version_cols.is_empty();
        if revisions {
            b.build_revision_columns(crate::party_binding::request::reads_door());
        }
        b.build_where()?;
        b.build_order_by()?;
        b.build_paging();
        if ir.distinct {
            b.q.distinct();
        }
        let (sql, values) = match care::census(&ctx.ehr_access) {
            Some(census) => b.q.with(census).build_sqlx(PostgresQueryBuilder),
            None => b.q.build_sqlx(PostgresQueryBuilder),
        };
        Ok(PreparedQuery {
            sql: sql.into(),
            values,
            shaped: false,
            layout: Arc::new(Layout {
                columns,
                node_groups: b.node_groups.into_iter().map(|(_, group)| group).collect(),
                access_aggregated: !access_ehr_cols.is_empty()
                    && ir.select.iter().any(|column| {
                        matches!(column.value, crate::aql::ir::SelectValue::Aggregate { .. })
                    }),
                access_ehr_cols,
                access_settled_cols,
                access_version_cols,
                revisions,
            }),
        })
    }
}
