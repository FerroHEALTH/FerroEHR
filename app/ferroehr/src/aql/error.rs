// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL planner errors.
//!
//! Two families, both surfaced before any SQL is built:
//!
//! * [`AqlFeatureError`] — the feature-envelope rejections. Every variant names
//!   the rejected construct and cites the governing QUERY 1.1 spec section, so a
//!   rejection is always explainable against the vendored spec
//!   (`docs/specs/openehr/QUERY/docs/AQL/`), which is the ONLY authority for the
//!   accept/reject envelope: a construct the released QUERY 1.1 text defines is
//!   accepted or rejected on that text alone. Other implementations' envelopes
//!   are prior art and never widen or narrow this one.
//! * [`AnalysisError`] — path analysis / typing failures (unknown class or
//!   variable, unresolvable attribute, type mismatch, unbound parameter).
//! * [`SqlError`] — IR→SQL lowering failures (a construct the planner accepted
//!   but the SQL package cannot yet render), surfaced before execution.
//! * [`ExecError`] — execution / `RESULT_SET` assembly failures (a database
//!   error, a reassembly failure, or a projected version body outside the
//!   active `spec_profile`), surfaced during execution.

use thiserror::Error;

/// The single error type returned by [`crate::aql::plan`] and the SQL/execution
/// packages ([`crate::aql::exec`]).
#[derive(Debug, Error)]
pub enum AqlError {
    /// A construct outside the accepted feature envelope.
    #[error(transparent)]
    Feature(#[from] AqlFeatureError),
    /// A path-analysis / typing failure.
    #[error(transparent)]
    Analysis(#[from] AnalysisError),
    /// An IR→SQL lowering failure.
    #[error(transparent)]
    Sql(#[from] SqlError),
    /// An execution / `RESULT_SET` assembly failure.
    #[error(transparent)]
    Exec(#[from] ExecError),
}

/// A construct that is syntactically valid AQL but outside the accepted feature
/// envelope. Each variant names the construct and its QUERY 1.1 spec section.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AqlFeatureError {
    /// `TERMINOLOGY(...) = true` — the Boolean value-expression form (an
    /// assertion tested server-side). QUERY §Functions/Other functions/
    /// TERMINOLOGY (master03 lines 762–767): a `TERMINOLOGY()` call in a
    /// position the pre-pass could not resolve — used other than as a
    /// `matches` operand or a Boolean value expression (`= true`/`!= true`),
    /// e.g. compared to a non-boolean or selected as a column.
    #[error(
        "TERMINOLOGY() may be used as a `matches` operand or as a Boolean \
         value expression (`= true`); this position is not supported \
         (QUERY §Functions/Other functions/TERMINOLOGY)"
    )]
    TerminologyFunction,

    /// A Boolean `TERMINOLOGY()` operation the terminology seam cannot
    /// evaluate to a truth value (`lookup`, `map`, or an unrecognised
    /// operation). QUERY §Functions/Other functions/TERMINOLOGY: only
    /// `validate` and `subsumes` have boolean semantics.
    #[error(
        "TERMINOLOGY() operation `{0}` has no boolean semantics \
         (`validate`/`subsumes` may be tested against true/false; \
         QUERY §Functions/Other functions/TERMINOLOGY)"
    )]
    UnsupportedTerminologyOperation(String),

    /// A `TERMINOLOGY()` `params_uri` missing an argument the operation needs
    /// (e.g. `validate` without `url=`/`code=`). QUERY §Functions/Other
    /// functions/TERMINOLOGY.
    #[error(
        "TERMINOLOGY() params_uri is missing the `{0}` argument \
         (QUERY §Functions/Other functions/TERMINOLOGY)"
    )]
    TerminologyParams(&'static str),

    /// A non-`expand` `TERMINOLOGY(...)` used as (or inside) a `matches`
    /// operand. Only `expand` is merged into the value list at semantic
    /// analysis; other operations are rejected. QUERY §Operators/Comparison
    /// operators/matches + §Other functions (master03 lines 748–767).
    #[error(
        "matches against a non-`expand` TERMINOLOGY() operand is not supported \
         (only `expand` merges codes into the value list) \
         (QUERY §Operators/matches, §Other functions/TERMINOLOGY)"
    )]
    MatchesTerminology,

    /// `matches { <uri> }` reached the planner unresolved — the pre-pass
    /// resolves terminology URIs (QUERY §matches/URI, master03 lines 367–402);
    /// this fires only when planning bypasses semantic analysis.
    #[error(
        "matches against a URI operand was not resolved at semantic analysis \
         (QUERY §Operators/matches)"
    )]
    MatchesUri,

    /// A `TERMINOLOGY()` `service_api` that names no configured terminology
    /// service: an unrecognised flavour, or a FHIR flavour with no provider
    /// configured. A query-side/config problem (→ 400), distinct from an
    /// upstream server fault ([`ExecError::Terminology`], → 500). QUERY
    /// §Functions/Other functions/TERMINOLOGY.
    #[error(
        "TERMINOLOGY() service_api `{0}` is not a configured terminology service \
         (QUERY §Functions/Other functions/TERMINOLOGY)"
    )]
    UnknownTerminologyService(String),

    /// A `TERMINOLOGY('expand', service_api, params_uri)` whose `params_uri`
    /// names a value set the terminology service does not know. A bad-query
    /// problem (→ 400). QUERY §Functions/Other functions/TERMINOLOGY.
    #[error(
        "TERMINOLOGY() value set `{value_set}` was not found via service_api \
         `{service_api}` (QUERY §Functions/Other functions/TERMINOLOGY)"
    )]
    TerminologyValueSetNotFound {
        /// The `service_api` argument.
        service_api: String,
        /// The `params_uri` argument (the value-set identifier).
        value_set: String,
    },

    /// A `matches` node predicate carrying a `{/regex/}` — cADL, not AQL value
    /// matching. QUERY §Predicates/Node predicate.
    #[error(
        "regex node predicates ({{/../}}) are not supported (QUERY §Predicates/Node predicate)"
    )]
    RegexNodePredicate,

    /// An `OR` combination inside a node predicate. QUERY §Predicates/Node
    /// predicate (the common forms are a code with an optional `AND name`
    /// criterion; a disjunctive node predicate is out of the accepted subset).
    #[error("OR node predicates are not supported (QUERY §Predicates/Node predicate)")]
    OrNodePredicate,

    /// A FROM source that is not an EHR / VERSION / in-scope RM structure class
    /// (e.g. a demographic PARTY/ROLE/ACTOR source). QUERY §FROM.
    #[error("FROM source class `{0}` is not in scope (demographic/off-scope class; QUERY §FROM)")]
    UnsupportedSourceClass(String),

    /// A `CONTAINS` pair the RM defines no containment relationship for — a
    /// versioned-object root under a parent that cannot contain it (e.g.
    /// `COMPOSITION CONTAINS COMPOSITION`, `FOLDER CONTAINS EHR_STATUS`).
    /// Folder containment of versioned objects is `FOLDER.items` reference
    /// resolution (RM common master05); everything else this refuses used to
    /// answer a silent cartesian product.
    #[error(
        "`{0} CONTAINS {1}` has no RM containment relationship (QUERY §FROM/Containment; RM common master05)"
    )]
    UnsupportedContainment(String, String),

    /// Branch (non-trunk) version addressing. Trunk-only per the storage design
    /// NOTE. QUERY §Predicates/Standard predicate (version).
    #[error("branch version addressing is not supported (trunk-only; QUERY §Predicates)")]
    BranchVersionAddressing,

    /// Both `TOP n` and `LIMIT n` in one query. QUERY §Query structure/LIMIT.
    #[error("TOP and LIMIT cannot be combined in one query (QUERY §Query structure/LIMIT)")]
    TopWithLimit,

    /// `TOP n BACKWARD` — the deprecated direction variant is not supported;
    /// the reject carries the spec's own rewrite guidance
    /// (QUERY §SELECT/TOP deprecation note).
    #[error(
        "TOP {0} BACKWARD is not supported: TOP is deprecated as of AQL 1.1.0 \
         (QUERY §SELECT/TOP) — rewrite the query with the recommended form, \
         e.g. `ORDER BY <path> DESC LIMIT {0}`"
    )]
    TopBackward(i64),

    /// A named scalar function that is not on the supported whitelist.
    /// QUERY §Functions.
    #[error("function `{0}` is not supported (QUERY §Functions)")]
    UnsupportedFunction(String),

    /// A version predicate addressing metadata the planner does not model.
    /// QUERY §Predicates/Standard predicate (version).
    #[error("version predicate on `{0}` is not supported (QUERY §Predicates/Standard predicate)")]
    UnsupportedVersionPredicate(String),

    /// An `e/ehr_status[/...]` path form the engine does not resolve. The whole
    /// `EHR_STATUS` object and inline/structure-child leaf extraction under it
    /// are supported; a predicate on the EHR variable or on `ehr_status` itself
    /// (`EHR_STATUS` is a singleton VO, not a filterable node set) is not.
    /// RM 1.2.0 `EHR.ehr_status` (`docs/specs/openehr/RM/docs/ehr/`).
    #[error("EHR path form `{0}` is not supported (RM EHR.ehr_status)")]
    UnsupportedEhrStatusPath(String),

    /// A SELECT mixing an aggregate function with a plain projection. QUERY
    /// master03 §Aggregate functions defines the five functions over the rows
    /// the FROM/WHERE clauses select and no grouping construct, explicit or
    /// implicit, so the mixed shape has no defined result under the released
    /// text; the engine refuses it rather than inventing `GROUP BY` semantics.
    #[error(
        "a SELECT cannot mix an aggregate function with a non-aggregated column \
         (AQL 1.1 defines no grouping; QUERY master03 §Aggregate functions)"
    )]
    MixedAggregateProjection,
}

/// A path-analysis / typing failure against the generated RM model
/// (`openehr_rm::v1_2::model`).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AnalysisError {
    /// A FROM class name absent from the RM model.
    #[error("unknown RM class `{0}` (not in the generated RM model)")]
    UnknownClass(String),

    /// A FROM class the ACTIVE `spec_profile`'s released RM generation does
    /// not define (the class exists only in a newer generation).
    #[error(
        "RM class `{class}` is not defined by RM {generation} (the active spec_profile is \
         `{profile}`)"
    )]
    ClassNotInProfile {
        /// The class name.
        class: String,
        /// The active profile token.
        profile: &'static str,
        /// The active RM generation's spec version.
        generation: &'static str,
    },

    /// A path attribute the ACTIVE `spec_profile`'s released RM generation
    /// does not define on any candidate class.
    #[error(
        "attribute `{attribute}` on {on} is not defined by RM {generation} (the active \
         spec_profile is `{profile}`)"
    )]
    AttributeNotInProfile {
        /// The attribute name.
        attribute: String,
        /// The candidate class description.
        on: String,
        /// The active profile token.
        profile: &'static str,
        /// The active RM generation's spec version.
        generation: &'static str,
    },

    /// An identified-path root that is not a variable bound in FROM.
    #[error("unknown variable `{0}` (not bound in the FROM clause)")]
    UnknownVariable(String),

    /// A variable name declared by more than one class expression — variable
    /// names must be unique within an AQL statement (QUERY master03
    /// §Variables/Syntax).
    #[error("variable `{0}` is declared more than once (variable names must be unique)")]
    DuplicateVariable(String),

    /// LIMIT/OFFSET/TOP bound violation: `row_count` minimum is 1, `offset`
    /// minimum is 0 (QUERY master03 §LIMIT/Syntax).
    #[error("invalid {clause} value {value} ({clause} minimum is {minimum})")]
    PagingBounds {
        /// `LIMIT` / `OFFSET` / `TOP`.
        clause: &'static str,
        /// The offending value.
        value: i64,
        /// The spec minimum.
        minimum: i64,
    },

    /// `SELECT DISTINCT` ordered by an expression that is not one of the
    /// selected columns. QUERY master03 §DISTINCT defines no semantics for
    /// sorting a de-duplicated projection by an unselected expression (and
    /// the DBMS requires ORDER BY expressions to appear in the select list).
    #[error(
        "ORDER BY with SELECT DISTINCT must sort by a selected column \
         (QUERY §SELECT/DISTINCT)"
    )]
    DistinctOrderByUnselected,

    /// An aggregate applied to a non-conforming input type (`SUM`/`AVG`
    /// accept Integer/Real input only — QUERY master03 §Functions/SUM, AVG).
    #[error("{func} requires a numeric (Integer/Real) input; the path selects {got}")]
    AggregateInputType {
        /// The aggregate name.
        func: &'static str,
        /// A human description of the selected leaf type.
        got: &'static str,
    },

    /// A scalar function called with the wrong number of arguments.
    #[error("{func} takes {expected} argument(s), got {got}")]
    FunctionArity {
        /// The function name.
        func: &'static str,
        /// The expected arity description.
        expected: &'static str,
        /// The supplied argument count.
        got: usize,
    },

    /// An attribute that no candidate type of the current path step declares.
    #[error("attribute `{attribute}` is not defined on {on} (RM model)")]
    UnresolvableAttribute {
        /// The unresolved attribute name.
        attribute: String,
        /// A human description of the type(s) it was looked up on.
        on: String,
    },

    /// A parameter (`$name`) referenced by the query but not supplied.
    #[error("unbound query parameter `${0}`")]
    UnboundParameter(String),

    /// An operand typing that cannot be reconciled (e.g. comparing a whole
    /// structure object to a scalar literal).
    #[error("type mismatch: {0}")]
    TypeMismatch(String),

    /// An `archetype_node_id = '<literal>'` criterion whose operand is neither
    /// an archetype identifier nor an archetype term code.
    ///
    /// QUERY `master03-syntax.adoc` §"Archetype predicate" and §"Node
    /// predicate" define the two shortcut forms — `[openEHR-EHR-OBSERVATION.…]`
    /// and `[at0002]` — as EQUIVALENT to the standard predicates
    /// `archetype_node_id = 'openEHR-EHR-OBSERVATION.…'` and
    /// `archetype_node_id=at0002` ("These predicates could also be written as
    /// standard predicates"). The admissible operand set of the standard form
    /// is therefore exactly the two the shortcut forms carry, which is also
    /// what the RM allows the attribute to hold (`LOCATABLE.archetype_node_id`:
    /// an archetype id at a root, an at-code inside). A third shape addresses
    /// nothing, so it is refused rather than silently planned as an archetype
    /// constraint that can never match.
    #[error(
        "archetype_node_id criterion `{0}` is neither an archetype identifier \
         nor an archetype node code (QUERY §Archetype predicate / §Node predicate)"
    )]
    MalformedArchetypeNodeId(String),
}

/// An IR→SQL lowering failure: a construct the planner accepted but the SQL
/// package cannot (yet) render. Distinct from [`AqlFeatureError`] (rejected at
/// planning time) — these surface at SQL build time.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SqlError {
    /// A SELECT / WHERE construct the SQL generator does not render yet.
    #[error("SQL generation for `{0}` is not supported yet")]
    Unsupported(String),

    /// A `$parameter` referenced during SQL generation had no supplied value
    /// (should have been caught by [`super::plan`]; a defensive guard).
    #[error("unbound query parameter `${0}` at SQL build time")]
    UnboundParameter(String),

    /// A temporal comparison bound that is no ISO-8601 date/time/date-time in
    /// any accepted precision — the caller's own value (→ 400). The message
    /// deliberately names the defect class only, never the value or a driver
    /// type.
    #[error(
        "a query value in a date/time comparison cannot be coerced to an \
         ISO-8601 date, time or date-time (reduced precision is accepted); \
         correct the value"
    )]
    UncoercibleTemporal,
}

/// An execution / `RESULT_SET` assembly failure.
#[derive(Debug, Error)]
pub enum ExecError {
    /// The database rejected or failed the generated query.
    #[error("query execution failed: {0}")]
    Database(#[from] sqlx::Error),

    /// A whole-object `RESULT_SET` cell could not be reassembled from its node
    /// subtree (a storage/codec failure).
    #[error("result assembly failed: {0}")]
    Assembly(#[from] crate::storage::error::StorageError),

    /// A terminology-server call failed while resolving a
    /// `TERMINOLOGY('expand', …)` operand (transport/HTTP/malformed response).
    /// The construct is accepted and the value set is routable; the upstream
    /// service failed → a server-side fault (→ 500), distinct from a bad
    /// query (400). QUERY §Functions/Other functions/TERMINOLOGY.
    #[error("terminology expansion failed: {0}")]
    Terminology(String),

    /// A whole-object `RESULT_SET` cell would have served a stored version
    /// body the ACTIVE `spec_profile`'s generation set cannot express, so the
    /// query is refused instead (`409`-class — the same refusal the resource
    /// reads carry, `crate::versioning::profile::gate_result_bodies`). No
    /// openEHR spec governs runtime generation selection — our own
    /// design/extension.
    #[error(transparent)]
    Profile(crate::service::error::ServiceError),

    /// A `RESULT_SET` column spec reached the executor without one of the
    /// generated SQL aliases its [`super::sql::CellKind`] declares (a
    /// defensive guard on a lowering defect, like
    /// [`SqlError::UnboundParameter`] — the executor rejects rather than
    /// panicking on a request path).
    #[error("result column `{column}` is missing generated SQL alias {index}")]
    MissingColumnAlias {
        /// The `RESULT_SET` column's name.
        column: String,
        /// The alias position the executor needed.
        index: usize,
    },
}
