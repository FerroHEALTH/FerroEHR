// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The typed AQL query IR (our own engine).
//!
//! This is a relational-algebra-flavoured, fully typed intermediate
//! representation of an AQL `SELECT` query over the greenfield node store. It is
//! produced by [`crate::aql::plan`] (path analysis in `super::analyze` plus
//! lowering in `super::lower`) and consumed by the IR-to-SQL package. No SQL
//! strings appear anywhere in this module: the IR records typed intent
//! (structure hops, fragment jsonpaths, coercions, version scope, wildcard
//! semantics) and leaves every SQL-shaped decision to the SQL package.
//!
//! AQL expresses version-at-time only as a standard predicate on version
//! metadata, so every version-selection predicate lowers uniformly to
//! [`VersionScope::Predicate`] and the common at-time case is recognised via
//! [`VersionScope::is_at_time`]. `Coercion::Magnitude` covers both `DV_ORDERED`
//! value objects (extracted via `ext.openehr_magnitude`) and numeric primitives
//! (a direct numeric cast); the analyzer records the candidate leaf [`TypeSet`]
//! on the [`LeafPath`] and the SQL package picks the exact extraction from it.

use std::collections::HashMap;

use openehr_query::ast::BuiltinFunction;
use openehr_query::lexer::CompOp;

/// A dense index identifying a [`Source`] within a [`QueryIr`]'s `sources` vec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub usize);

/// A set of RM/primitive type names (sorted, de-duplicated). Primitives use the
/// model's spec names (`"Integer"`, `"Real"`, `"String"`, `"Boolean"`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeSet(Vec<String>);

impl TypeSet {
    /// Build a normalized (sorted, de-duplicated) type set.
    #[must_use]
    pub fn new(mut types: Vec<String>) -> Self {
        types.sort();
        types.dedup();
        Self(types)
    }

    /// A single-type set.
    #[must_use]
    pub fn single(name: impl Into<String>) -> Self {
        Self(vec![name.into()])
    }

    /// The type names, sorted.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.0
    }

    /// Whether the set is empty (unresolved).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether exactly one type name is present.
    #[must_use]
    pub fn is_singleton(&self) -> bool {
        self.0.len() == 1
    }

    /// Whether `name` is in the set.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.0.iter().any(|n| n == name)
    }
}

// ── FROM: sources + containment tree ─────────────────────────────────────────

/// One FROM operand: an EHR, a versioned RM object, or a VERSION envelope.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// `EHR e[predicates]`.
    Ehr(EhrSource),
    /// A versioned-object / RM structure-class operand, e.g.
    /// `COMPOSITION c[openEHR-EHR-COMPOSITION.report.v1]` or `OBSERVATION o`.
    Rm(RmSource),
    /// A `VERSION v[LATEST_VERSION | ALL_VERSIONS | <standard predicate>]`
    /// envelope; addresses version metadata and sets the scope of the VO(s) it
    /// contains.
    Version(VersionSource),
}

impl Source {
    /// The source's dense id.
    #[must_use]
    pub fn id(&self) -> SourceId {
        match self {
            Source::Ehr(s) => s.id,
            Source::Rm(s) => s.id,
            Source::Version(s) => s.id,
        }
    }

    /// The bound variable name, if any.
    #[must_use]
    pub fn var(&self) -> Option<&str> {
        match self {
            Source::Ehr(s) => s.var.as_deref(),
            Source::Rm(s) => s.var.as_deref(),
            Source::Version(s) => s.var.as_deref(),
        }
    }
}

/// `EHR e[ehr_id/value=$id]` — the EHR operand.
#[derive(Debug, Clone, PartialEq)]
pub struct EhrSource {
    /// Dense id.
    pub id: SourceId,
    /// The bound variable (`e`), if any.
    pub var: Option<String>,
    /// Standard predicates on EHR metadata (currently `ehr_id/value`).
    pub predicates: Vec<EhrPredicate>,
}

/// A resolved standard predicate on an EHR operand.
#[derive(Debug, Clone, PartialEq)]
pub struct EhrPredicate {
    /// The addressed EHR field.
    pub field: EhrField,
    /// The comparison operator.
    pub op: CompOp,
    /// The right-hand value.
    pub value: Bind,
}

/// An addressable field of an EHR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EhrField {
    /// `ehr_id/value` → the `ehr.id` key.
    EhrId,
    /// `system_id/value`.
    SystemId,
    /// `time_created[/value]`.
    TimeCreated,
    /// The whole EHR object (bare `e`).
    Whole,
}

/// A versioned RM structure-class operand.
#[derive(Debug, Clone, PartialEq)]
pub struct RmSource {
    /// Dense id.
    pub id: SourceId,
    /// The bound variable, if any.
    pub var: Option<String>,
    /// The resolved concrete RM types this operand may bind (the class name
    /// expanded through the model when abstract).
    pub rm_type: TypeSet,
    /// An archetype/at-code constraint from the operand's predicate.
    pub archetype: Option<ArchetypeConstraint>,
    /// A `name/value` constraint from the operand's predicate.
    pub name: Option<NameConstraint>,
    /// General standard predicates on the operand (`[path op value]`).
    pub standard: Vec<StdPredicate>,
    /// The version scope selecting which versions of this VO participate.
    /// Defaults to [`VersionScope::Latest`]; set by an enclosing VERSION.
    pub scope: VersionScope,
}

/// A `VERSION v[...]` envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionSource {
    /// Dense id.
    pub id: SourceId,
    /// The bound variable, if any.
    pub var: Option<String>,
    /// The version scope this envelope imposes.
    pub scope: VersionScope,
}

/// How versions of a versioned object are scoped.
#[derive(Debug, Clone, PartialEq)]
pub enum VersionScope {
    /// `LATEST_VERSION` (the default) — the current version
    /// (the head row's `trunk_head_sys_version`).
    Latest,
    /// `ALL_VERSIONS` — every version (the temporal table unfiltered; supported
    /// from day one by design).
    All,
    /// A standard predicate on version metadata, e.g.
    /// `commit_audit/time_committed <= $t` (version-at-time) or `uid/value=$v`.
    Predicate(VersionMetaPredicate),
}

impl VersionScope {
    /// Whether this scope is an at-a-point-in-time selection (a comparison on
    /// `commit_audit/time_committed`). This is the design's `AtTime` case,
    /// recognised rather than represented as its own variant.
    #[must_use]
    pub fn is_at_time(&self) -> bool {
        matches!(
            self,
            VersionScope::Predicate(p) if p.field == VersionField::TimeCommitted
        )
    }
}

/// A resolved standard predicate on version metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionMetaPredicate {
    /// The addressed version-metadata field.
    pub field: VersionField,
    /// The comparison operator.
    pub op: CompOp,
    /// The right-hand value.
    pub value: Bind,
}

/// A version-metadata field addressable on a VERSION variable (maps to
/// `version` / `audit` / `contribution` columns in the SQL package).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionField {
    /// `uid/value` — the object version id (`vo_id::sys_version`).
    Uid,
    /// `commit_audit/time_committed`.
    TimeCommitted,
    /// `commit_audit/system_id`.
    SystemId,
    /// `commit_audit/change_type/defining_code/code_string` — the stored
    /// numeric group code (e.g. `"249"`).
    ChangeType,
    /// `commit_audit/change_type/value` — the rubric (e.g. `"creation"`),
    /// rendered from the openEHR `audit_change_type` group.
    ChangeTypeRubric,
    /// `commit_audit/change_type/defining_code/terminology_id/value` — the
    /// constant `openehr` terminology.
    ChangeTypeTerminology,
    /// `commit_audit/committer[/...]`.
    Committer,
    /// `commit_audit/description` — the whole `DV_TEXT` (or its
    /// `DV_CODED_TEXT` subtype) as stored.
    Description,
    /// `commit_audit/description/value` — the description's display text.
    DescriptionValue,
    /// `commit_audit/description/defining_code/code_string` — the code of a
    /// `DV_CODED_TEXT` description, `NULL` on a plain `DV_TEXT`.
    DescriptionCode,
    /// `commit_audit/description/defining_code/terminology_id/value` — the
    /// terminology of a `DV_CODED_TEXT` description.
    DescriptionTerminology,
    /// `contribution/id[/value]`.
    ContributionId,
    /// `lifecycle_state/defining_code/code_string` — the stored numeric
    /// group code (e.g. `"532"`).
    LifecycleState,
    /// `lifecycle_state/value` — the rubric (e.g. `"complete"`), rendered
    /// from the openEHR `version_lifecycle_state` group.
    LifecycleStateRubric,
    /// `lifecycle_state/defining_code/terminology_id/value` — the constant
    /// `openehr` terminology.
    LifecycleStateTerminology,
}

/// The FROM containment tree: a boolean tree whose leaves are source
/// operands, each optionally constraining a nested containment.
///
/// Lowered to nested-set interval self-joins (`d.num BETWEEN a.num AND
/// a.num_cap` within a version) or `ehr_id` joins (EHR→VO) by the SQL
/// package; `NotContains` is an anti-join.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainsTree {
    /// A source operand with an optional nested `[NOT] CONTAINS <sub>`.
    Operand {
        /// The operand's source.
        source: SourceId,
        /// The nested containment constraint, if any.
        contained: Option<Box<Contained>>,
    },
    /// `<a> AND <b>` over containment subtrees.
    And(Box<ContainsTree>, Box<ContainsTree>),
    /// `<a> OR <b>` over containment subtrees.
    Or(Box<ContainsTree>, Box<ContainsTree>),
}

/// A `[NOT] CONTAINS <tree>` tail.
#[derive(Debug, Clone, PartialEq)]
pub struct Contained {
    /// The containment link kind.
    pub link: Link,
    /// The contained subtree.
    pub tree: ContainsTree,
}

/// The kind of a containment edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// `CONTAINS` — an interval/`ehr_id` join.
    Contains,
    /// `NOT CONTAINS` — an anti-join (`NOT EXISTS`).
    NotContains,
}

// ── Paths: the analysis-time split ───────────────────────────────────────────

/// A fully analysed data path, split into a structure-node anchor and a residual
/// JSONB fragment path (design §Core insight: the path split).
#[derive(Debug, Clone, PartialEq)]
pub struct LeafPath {
    /// The source (CONTAINS variable) the path is rooted at.
    pub source: SourceId,
    /// A predicate on the root node itself (`o[name/value=$x]/...`).
    pub root_predicate: Option<NodeConstraint>,
    /// The structure-node descent from the source to the leaf-bearing node.
    /// Empty ⇒ the leaf lives in the source node's own fragment.
    pub anchor: Vec<StructureStep>,
    /// The residual jsonpath into the anchor node's `data` JSONB. Empty ⇒ the
    /// path addresses the whole anchor structure object.
    pub fragment: Vec<FragmentStep>,
    /// The candidate RM/primitive leaf types (abstract slots expanded to
    /// concrete descendants).
    pub types: TypeSet,
    /// The typed extraction/comparison strategy.
    pub coercion: Coercion,
}

impl LeafPath {
    /// Whether this path addresses a whole structure object (no fragment tail).
    #[must_use]
    pub fn is_whole_object(&self) -> bool {
        self.fragment.is_empty()
    }

    /// Whether the fragment tail crosses a list/set-valued attribute — the
    /// extraction then yields several items within ONE anchor node, and the
    /// SQL layer lowers predicates existentially and projections as an array
    /// cell.
    #[must_use]
    pub fn fragment_multi_valued(&self) -> bool {
        self.fragment.iter().any(|s| s.multi_valued)
    }
}

/// One structure hop: an attribute step whose resolved node is a structure root
/// (its own `node` row), resolved by the SQL package via nested-set containment
/// + promoted columns.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureStep {
    /// The RM attribute name (e.g. `"events"`).
    pub attribute: String,
    /// The concrete RM types the stepped-to node may be.
    pub node_types: TypeSet,
    /// A node predicate (at-code / archetype / name / standard) from the path.
    pub predicate: Option<NodeConstraint>,
    /// Whether the attribute is a container (list/set) — informs multiplicity.
    pub multi_valued: bool,
}

/// One fragment step: an attribute below the anchor node, addressed inside the
/// anchor's `data` JSONB.
#[derive(Debug, Clone, PartialEq)]
pub struct FragmentStep {
    /// The attribute name.
    pub name: String,
    /// An optional predicate carried on the step (rare within a fragment).
    pub predicate: Option<NodeConstraint>,
    /// Whether the attribute is a container.
    pub multi_valued: bool,
}

/// A resolved node predicate (the analysed form of an AST `PathPredicate`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeConstraint {
    /// An `archetype_node_id` constraint (an at/id code or an archetype HRID).
    pub archetype: Option<ArchetypeConstraint>,
    /// A `name/value` (or coded-name) constraint.
    pub name: Option<NameConstraint>,
    /// General standard sub-predicates (`[path op value]`).
    pub standard: Vec<StdPredicate>,
}

/// An `archetype_node_id` constraint.
#[derive(Debug, Clone, PartialEq)]
pub enum ArchetypeConstraint {
    /// An `at`/`id` node code (`at0001`, `id9`).
    NodeCode(String),
    /// An archetype HRID (`openEHR-EHR-OBSERVATION.bp.v1`).
    Archetype(String),
    /// A `$parameter` resolving to an archetype id at bind time.
    Param(String),
}

/// A `name`-attribute constraint on a node.
#[derive(Debug, Clone, PartialEq)]
pub enum NameConstraint {
    /// `name/value = '<text>'`.
    Value(String),
    /// `name/value = $param`.
    Param(String),
    /// The coded-name shortcut, decomposed per its canonical expansion
    /// (QUERY master03 §Node predicate: `[at0002, terminology::code|value|]`
    /// ≡ `name/defining_code/code_string = '<code>' AND
    /// name/defining_code/terminology_id/value = '<terminology>'`; the
    /// `|value|` tail is informational and takes no part in matching).
    TermCode {
        /// `name/defining_code/terminology_id/value`.
        terminology: String,
        /// `name/defining_code/code_string`.
        code: String,
    },
}

/// A resolved standard predicate (`[objectPath op operand]`) on a node — the
/// path is left as raw steps for the SQL package to compile into a fragment
/// jsonpath filter.
#[derive(Debug, Clone, PartialEq)]
pub struct StdPredicate {
    /// The `/`-joined object path (relative to the node), e.g.
    /// `value/defining_code/code_string`.
    pub path: Vec<String>,
    /// The operator.
    pub op: CompOp,
    /// The right-hand value.
    pub value: Bind,
}

/// The typed extraction/comparison strategy for a leaf (design §Coercion table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coercion {
    /// Numeric/ordered comparison: a `DV_ORDERED` value object (via
    /// `ext.openehr_magnitude`) or a numeric primitive (a direct numeric cast).
    /// The SQL package distinguishes the two from the leaf [`TypeSet`].
    Magnitude,
    /// String comparison (`#>> '{}'`); the leaf is a String or has a String
    /// representation.
    Text,
    /// Temporal comparison via jsonpath item methods (`.datetime()`, …).
    Temporal,
    /// Boolean leaf.
    Boolean,
    /// Mixed / unknown candidate set. In a comparison or `matches` against a
    /// numeric literal the leaf is extracted numerically (non-number
    /// occurrences yield `NULL` and fail the test — never a silent lexical
    /// miscompare); otherwise, and for projection/ordering, it is read as text
    /// (QUERY master03 §Comparison operators). The SQL package resolves this
    /// dispatch from the comparison partner (`sql::predicate`, `sql::value`).
    Raw,
}

// ── Expressions (WHERE / predicates) ─────────────────────────────────────────

/// A typed WHERE predicate tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A comparison `lhs op rhs` with a resolved coercion.
    Compare {
        /// Left operand.
        lhs: Operand,
        /// Operator.
        op: CompOp,
        /// Right operand.
        rhs: Operand,
        /// The typed comparison strategy.
        coercion: Coercion,
    },
    /// `EXISTS <path>`.
    Exists(PathTarget),
    /// A constant boolean (a `TERMINOLOGY()` Boolean value expression resolved
    /// at semantic analysis — QUERY master03 §TERMINOLOGY).
    Const(bool),
    /// `<path> LIKE <pattern>` (AQL wildcard semantics recorded on the pattern).
    Like {
        /// The addressed path.
        path: PathTarget,
        /// The pattern.
        pattern: LikePattern,
    },
    /// `<path> MATCHES { v1, v2, … }`.
    Matches {
        /// The addressed path.
        path: PathTarget,
        /// The candidate values.
        values: Vec<Bind>,
        /// The typed comparison strategy for the members.
        coercion: Coercion,
    },
    /// `<a> AND <b>`.
    And(Box<Expr>, Box<Expr>),
    /// `<a> OR <b>`.
    Or(Box<Expr>, Box<Expr>),
    /// `NOT <a>`.
    Not(Box<Expr>),
}

/// An operand of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// An identified path (data leaf / version field / EHR field).
    Path(PathTarget),
    /// A literal.
    Literal(TypedLit),
    /// A `$parameter` bind.
    Param(String),
    /// A scalar function applied to operands.
    Function {
        /// The AQL built-in function.
        func: BuiltinFunction,
        /// The argument operands.
        args: Vec<Operand>,
    },
}

/// A resolved identified path, discriminated by what it addresses.
#[derive(Debug, Clone, PartialEq)]
pub enum PathTarget {
    /// A data leaf (or whole structure object) through the node store.
    Data(Box<LeafPath>),
    /// Version metadata on a VERSION variable.
    Version {
        /// The VERSION source.
        source: SourceId,
        /// The addressed metadata field.
        field: VersionField,
    },
    /// An EHR attribute (`ehr_id/value`, `time_created`, …).
    Ehr {
        /// The EHR source.
        source: SourceId,
        /// The addressed field.
        field: EhrField,
    },
    /// A path into the EHR's current `EHR_STATUS` versioned object
    /// (`e/ehr_status[/...]`). `EHR` is not a `node` in the store and
    /// `EHR_STATUS` is a *separate* versioned object, so this addresses it via
    /// an engine-level join (`version.ehr_id = ehr.id`, `kind = EHR_STATUS`,
    /// latest version) rather than a node-tree walk. The wrapped [`LeafPath`] is
    /// analysed relative to the `EHR_STATUS` root (`leaf.source` = the EHR
    /// source), so the SQL package reuses the whole-object / anchor-walk /
    /// fragment-extraction machinery once the root node is joined in. RM 1.2.0
    /// `EHR.ehr_status` (`docs/specs/openehr/RM/docs/ehr/`).
    EhrStatus(Box<LeafPath>),
}

/// A `LIKE` pattern with its AQL wildcard semantics preserved for the SQL
/// package to translate (`*` → `%`, `?` → `_`; the conversion itself is the SQL
/// package's job). QUERY §Operators/LIKE.
#[derive(Debug, Clone, PartialEq)]
pub enum LikePattern {
    /// A literal AQL pattern (raw, wildcards not yet translated).
    Literal(String),
    /// A `$parameter` supplying the pattern at bind time.
    Param(String),
}

/// A literal or parameter used as a right-hand value.
#[derive(Debug, Clone, PartialEq)]
pub enum Bind {
    /// A typed literal.
    Literal(TypedLit),
    /// A `$parameter`.
    Param(String),
}

/// A typed AQL literal. `Temporal` is a quoted value the analyzer retyped from
/// its comparison context (QUERY §Built-in Types/Dates and Times).
#[derive(Debug, Clone, PartialEq)]
pub enum TypedLit {
    /// An integer literal.
    Integer(i64),
    /// A real literal.
    Real(f64),
    /// A boolean literal.
    Boolean(bool),
    /// A string literal.
    String(String),
    /// A date/time/duration literal (a quoted value typed by context).
    Temporal(String),
    /// `NULL`.
    Null,
}

// ── SELECT / ORDER BY ────────────────────────────────────────────────────────

/// One SELECT column.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectColumn {
    /// The projected value.
    pub value: SelectValue,
    /// An optional `AS alias` (also the `RESULT_SET` column name).
    pub alias: Option<String>,
    /// The `RESULT_SET` column `path`: the SELECT expression's path exactly as
    /// written in the query (minus the root variable; `"/"` for a bare
    /// variable), when the column is a path expression. ITS-REST 1.1.0
    /// `RESULT_SET.columns[].path`; the CNF query goldens compare it verbatim.
    pub path: Option<String>,
}

/// A projected SELECT value.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectValue {
    /// An identified path.
    Path(PathTarget),
    /// A literal.
    Literal(TypedLit),
    /// An aggregate function.
    Aggregate {
        /// Which aggregate.
        func: AggFunc,
        /// The argument path (`None` for `COUNT(*)`).
        arg: Option<PathTarget>,
        /// `DISTINCT` present.
        distinct: bool,
    },
    /// A scalar function.
    Function {
        /// The AQL built-in function.
        func: BuiltinFunction,
        /// The argument operands.
        args: Vec<Operand>,
    },
}

/// An aggregate function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggFunc {
    /// `COUNT`.
    Count,
    /// `MIN`.
    Min,
    /// `MAX`.
    Max,
    /// `SUM`.
    Sum,
    /// `AVG`.
    Avg,
}

/// One ORDER BY key.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderKey {
    /// The path to order by.
    pub path: PathTarget,
    /// Ascending (`true`) or descending; defaults to ascending.
    pub ascending: bool,
}

// ── The query IR ─────────────────────────────────────────────────────────────

/// A fully lowered, typed AQL query.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryIr {
    /// Every FROM operand, addressable by [`SourceId`] (= index).
    pub sources: Vec<Source>,
    /// The FROM containment tree over `sources`.
    pub contains: ContainsTree,
    /// The WHERE predicate, if any.
    pub filter: Option<Expr>,
    /// The SELECT projection.
    pub select: Vec<SelectColumn>,
    /// The ORDER BY keys.
    pub order_by: Vec<OrderKey>,
    /// `SELECT DISTINCT` present.
    pub distinct: bool,
    /// The row limit (from `LIMIT` or a mapped `TOP`).
    pub limit: Option<i64>,
    /// Whether [`Self::limit`] came from the deprecated `TOP` modifier — the
    /// one construct the REST `fetch` parameter may not combine with
    /// (ITS-REST query `Request.md` §Common Headers and Query Parameters).
    pub limit_is_top: bool,
    /// The row offset (`OFFSET`).
    pub offset: Option<i64>,
    /// Every `$parameter` name referenced by the query (validated present in
    /// the supplied [`Params`]).
    pub params: Vec<String>,
}

/// The supplied query parameters (`$name` binds), keyed by name (no `$`).
#[derive(Debug, Clone, Default)]
pub struct Params {
    values: HashMap<String, ParamValue>,
}

impl Params {
    /// An empty parameter set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `name` (without the leading `$`) to `value`.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: ParamValue) -> Self {
        self.values.insert(name.into(), value);
        self
    }

    /// Bind `name` to `value` in place.
    pub fn insert(&mut self, name: impl Into<String>, value: ParamValue) {
        self.values.insert(name.into(), value);
    }

    /// Whether `name` is bound.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// The bound value for `name`, if any.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ParamValue> {
        self.values.get(name)
    }

    /// Whether no parameter is bound.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// A typed query-parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// An integer.
    Int(i64),
    /// A real.
    Real(f64),
    /// A boolean.
    Bool(bool),
    /// A string (also covers codes / archetype ids / temporals on the wire).
    Str(String),
    /// A JSON null.
    Null,
}
