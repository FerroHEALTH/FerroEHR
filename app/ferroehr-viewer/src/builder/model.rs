// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The builder's editable state: selected template, columns, the n-ary AND/OR
//! criterion tree (deliberately richer than the binary-only prior art), and
//! the result shape.
//!
//! Plain serializable data — no components.

use serde::{Deserialize, Serialize};

/// What the query returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryShape {
    /// Whole COMPOSITIONs (`SELECT c …`).
    Compositions,
    /// Projected data points (`SELECT <columns> …`).
    DataValues,
    /// A match count (`SELECT COUNT(*) …`) — dashboard / query-group tiles.
    Count,
    /// The matching EHR ids (`SELECT DISTINCT e/ehr_id/value …`) — cohort
    /// queries: "which EHRs match these conditions".
    Ehrs,
}

/// One selected projection column (only meaningful for
/// [`QueryShape::DataValues`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedColumn {
    /// The archetype path relative to the COMPOSITION root, as the Web
    /// Template's `aqlPath` carries it (leading `/` optional).
    pub aql_path: String,
    /// Column alias (`AS`) — a valid AQL identifier; empty = no alias.
    pub alias: String,
}

/// The boolean connective of a criterion group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoolOp {
    /// All children must hold.
    And,
    /// Any child must hold.
    Or,
}

/// A node in the criterion tree: a leaf condition or an n-ary group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CriterionNode {
    /// One typed condition on one path.
    Leaf(Criterion),
    /// An n-ary boolean group (children joined by `op`; `negated` wraps the
    /// whole group in `NOT (…)`).
    Group {
        /// The connective.
        op: BoolOp,
        /// `NOT` the whole group.
        negated: bool,
        /// The children (empty groups are a validation error at lowering).
        children: Vec<CriterionNode>,
    },
}

/// One leaf condition: a path plus a typed constraint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Criterion {
    /// The Web Template leaf's `aqlPath` (COMPOSITION-relative). Per the
    /// Simplified Formats level-removal rules
    /// (`docs/specs/openehr/ITS-REST/docs/simplified_formats/master04` §Level
    /// Removal) the promoted leaf IS the `DATA_VALUE` node, so this path
    /// already ends at the DV instance (e.g. `…/items[at0004]/value`); each
    /// [`CriterionKind`] appends its DV-attribute suffix (`magnitude`,
    /// `defining_code/code_string`, …).
    pub aql_path: String,
    /// `NOT` this single condition.
    pub negated: bool,
    /// The typed constraint.
    pub kind: CriterionKind,
}

/// The per-RM-datatype constraint catalog (each variant lowers to a typed
/// AQL `WHERE` fragment; unsupported datatypes are rejected at lowering
/// with a typed error, never silently dropped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CriterionKind {
    /// `DV_QUANTITY`: magnitude range (inclusive bounds; either side
    /// optional) + optional units equality — independent `AND`-joined
    /// conditions.
    QuantityRange {
        /// Lower bound on `…/magnitude` (`>=`).
        min: Option<f64>,
        /// Upper bound (`<=`).
        max: Option<f64>,
        /// Required `…/units` (empty = unconstrained).
        units: String,
    },
    /// `DV_CODED_TEXT`: `…/defining_code/code_string MATCHES {codes}`
    /// with optional terminology-id equality.
    CodedIn {
        /// The accepted code strings (at least one).
        codes: Vec<String>,
        /// Required `…/defining_code/terminology_id/value` (empty = any).
        terminology: String,
    },
    /// `DV_TEXT`: exact `…/value = text`.
    TextEquals {
        /// The compared text.
        text: String,
    },
    /// `DV_TEXT`: `…/value LIKE pattern` (AQL `*`/`?` wildcards; the
    /// UI passes the pattern through verbatim).
    TextLike {
        /// The LIKE pattern.
        pattern: String,
    },
    /// `DV_DATE_TIME` / `DV_DATE` / `DV_TIME`: ISO-8601 string range on
    /// `…/value` (inclusive; either side optional).
    DateTimeRange {
        /// Lower bound (`>=`), ISO-8601 text.
        from: String,
        /// Upper bound (`<=`), ISO-8601 text; empty = open.
        to: String,
    },
    /// `DV_COUNT`: integer range on `…/magnitude`.
    CountRange {
        /// Lower bound (`>=`).
        min: Option<i64>,
        /// Upper bound (`<=`).
        max: Option<i64>,
    },
    /// `DV_ORDINAL`: `…/value MATCHES {values}` (the ordinal ints).
    OrdinalIn {
        /// Accepted ordinal values (at least one).
        values: Vec<i64>,
    },
    /// `DV_BOOLEAN`: `…/value = b`.
    BooleanIs {
        /// The required value.
        value: bool,
    },
    /// `DV_PROPORTION`: numeric range on the computed
    /// `…/numerator` (v1 keeps numerator-only, the common vital-sign
    /// use; denominator/type constraints are a recorded follow-up).
    ProportionNumeratorRange {
        /// Lower bound (`>=`).
        min: Option<f64>,
        /// Upper bound (`<=`).
        max: Option<f64>,
    },
    /// Any type: the node exists (`EXISTS <path>`).
    Exists,
}

/// Ordering of the result set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRule {
    /// COMPOSITION-relative path.
    pub aql_path: String,
    /// Descending when true.
    pub descending: bool,
}

/// The whole builder state for one query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuilderQuery {
    /// Restrict to COMPOSITIONs of this template
    /// (`c/archetype_details/template_id/value = id`); empty = any.
    pub template_id: String,
    /// Result shape.
    pub shape: QueryShape,
    /// Projection columns ([`QueryShape::DataValues`] only).
    pub columns: Vec<SelectedColumn>,
    /// The criterion tree; `None` = no WHERE clause beyond the template
    /// restriction.
    pub criteria: Option<CriterionNode>,
    /// ORDER BY rules.
    pub order_by: Vec<OrderRule>,
    /// LIMIT (fetch size); `None` = unlimited (the run surface still pages).
    pub limit: Option<u32>,
}

impl BuilderQuery {
    /// A fresh builder for a template.
    #[must_use]
    pub fn new(template_id: String) -> Self {
        Self {
            template_id,
            shape: QueryShape::Compositions,
            columns: Vec::new(),
            criteria: None,
            order_by: Vec::new(),
            limit: Some(50),
        }
    }
}
