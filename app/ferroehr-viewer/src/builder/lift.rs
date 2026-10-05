// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lifting stored AQL back into [`BuilderQuery`] — the exact inverse of
//! [`crate::builder::lower`], and nothing more.
//!
//! The builder emits ONE query shape (`SELECT … FROM EHR e CONTAINS
//! COMPOSITION c …` with a typed criterion tree). This module recognizes
//! exactly that shape and REFUSES everything else with a typed
//! [`LiftError`] naming what it could not express — a query outside the
//! envelope belongs in the raw editor, never in a lossy half-lift.
//!
//! Recognition runs on the real AQL syntax tree (`openehr_query::parser`, the
//! same grammar `lower` renders through), so there is no second grammar to keep
//! in step. The refusal is then made total by a **round trip**: the lifted state
//! is lowered again, re-parsed, and compared with the original parse. Anything
//! that would come back different is refused, so an accepted lift is provably
//! equivalent to the stored definition. String escaping is symmetric by the
//! query crate's own contract: the parser decodes a literal's escapes into the
//! AST, and `openehr_query::printer` re-encodes them at emission — the AST
//! (and therefore the builder state) always holds the decoded value.

use openehr_query::ast::{
    AggregateCall, ClassExprOperand, ColumnExpr, CompareOperand, ContainsExpr, IdentifiedExpr,
    IdentifiedPath, LikeOperand, Limit, MatchesOperand, OrderByExpr, Primitive, SelectClause,
    SelectExpr, SelectQuery, SortOrder, Terminal, ValueListItem, WhereExpr,
};
use openehr_query::lexer::CompOp;

use crate::builder::lower::BuilderError;
use crate::builder::model::{
    BoolOp, BuilderQuery, Criterion, CriterionKind, CriterionNode, OrderRule, QueryShape,
    SelectedColumn,
};

/// The variable the builder binds the EHR to (mirrors `lower`).
const EHR_VAR: &str = "e";
/// The variable the builder binds the COMPOSITION to (mirrors `lower`).
const COMP_VAR: &str = "c";
/// The cohort shape's fixed column path and alias.
const EHR_ID_PATH: &str = "/ehr_id/value";
/// The alias `lower` gives the cohort column.
const EHR_ID_ALIAS: &str = "ehr_id";
/// The path the template restriction constrains.
const TEMPLATE_PATH: &str = "/archetype_details/template_id/value";

/// Why a stored query cannot be opened in the point-and-click builder.
///
/// Every variant names a property of the QUERY, so the notice the screen renders
/// tells the reader what to do about it (edit it as raw AQL, or run it on the
/// stored-query runner). Never crosses a server-fn boundary — the lift is pure
/// and runs on whichever target holds the AQL.
///
/// NOTE: `NotAql` carries the parse failure as text because the refusal is held
/// in an `RwSignal` and compared, so the type must be `Clone + Eq` (Leptos book
/// `reactivity/working_with_signals`) — which the parser's error is not.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LiftError {
    /// The stored text is not valid AQL at all.
    #[error("this stored query is not valid AQL: {0}")]
    NotAql(String),
    /// The query binds `$parameters`.
    #[error(
        "this query takes parameters ($…), which the point-and-click builder has no fields for — \
         run it on the stored-query runner, which prompts for each parameter"
    )]
    Parameterised,
    /// The `FROM` clause is not the builder's fixed `EHR e CONTAINS
    /// COMPOSITION c`.
    #[error(
        "the builder always queries `FROM EHR e CONTAINS COMPOSITION c`; this query selects from \
         something else (another RM class, a further CONTAINS level, a predicate, or other \
         variable names)"
    )]
    UnsupportedFrom,
    /// The `SELECT` list is none of the four builder output shapes.
    #[error(
        "the builder's output shapes are whole compositions, projected data-value columns, \
         `COUNT(*)`, and the distinct EHR ids; this query selects something else"
    )]
    UnsupportedSelect,
    /// The deprecated `TOP` row limit.
    #[error(
        "this query uses the deprecated `TOP` row limit; the builder expresses a limit as `LIMIT`"
    )]
    UnsupportedTop,
    /// `LIMIT … OFFSET …`.
    #[error(
        "this query carries a `LIMIT … OFFSET …`; the builder's limit has no offset (the run surface pages instead)"
    )]
    UnsupportedOffset,
    /// A `LIMIT` outside the builder's range.
    #[error("the row limit {0} is outside the range the builder's limit field accepts")]
    LimitOutOfRange(i64),
    /// An `ORDER BY` term the builder cannot hold.
    #[error(
        "the builder cannot express the sort term `{0}`: it orders composition-relative paths, each explicitly ASC or DESC"
    )]
    UnsupportedOrderBy(String),
    /// A `WHERE` fragment with no counterpart in the criterion catalog.
    #[error("the builder has no condition for `{0}`")]
    UnsupportedCondition(String),
    /// The lifted state does not lower back at all.
    #[error("this query lifted into a builder state that is itself invalid: {0}")]
    Invalid(BuilderError),
    /// The lifted state lowers to a DIFFERENT query.
    #[error(
        "this query cannot be represented in the builder without changing it, so it was not \
         loaded — edit it as raw AQL instead"
    )]
    RoundTrip,
}

/// Lift stored AQL into a builder state, or refuse it.
///
/// Success is round-trip-proven: `lower(from_aql(aql))` re-parses to exactly the
/// tree `aql` parses to, so opening the query in the builder and saving it
/// unchanged rewrites the same query.
///
/// # Errors
/// A [`LiftError`] naming the property of `aql` the builder cannot express.
pub fn from_aql(aql: &str) -> Result<BuilderQuery, LiftError> {
    // Parameters first: it is the most common reason a real stored query does
    // not fit, and the message points at the surface that DOES bind them.
    if !crate::aql_text::placeholders(aql).is_empty() {
        return Err(LiftError::Parameterised);
    }
    let parsed =
        openehr_query::parser::parse_str(aql).map_err(|e| LiftError::NotAql(e.to_string()))?;
    let lifted = lift_query(&parsed)?;
    let relowered = crate::builder::lower::to_aql(&lifted).map_err(LiftError::Invalid)?;
    let reparsed = openehr_query::parser::parse_str(&relowered)
        .map_err(|e| LiftError::NotAql(e.to_string()))?;
    if reparsed == parsed {
        Ok(lifted)
    } else {
        Err(LiftError::RoundTrip)
    }
}

// ── query level ──────────────────────────────────────────────────────────────

fn lift_query(query: &SelectQuery) -> Result<BuilderQuery, LiftError> {
    if query.select.top.is_some() {
        return Err(LiftError::UnsupportedTop);
    }
    check_from(&query.from)?;
    let (shape, columns) = lift_select(&query.select)?;
    let limit = match &query.limit {
        None => None,
        Some(Limit {
            offset: Some(_), ..
        }) => return Err(LiftError::UnsupportedOffset),
        Some(Limit { limit, .. }) => {
            // `TryFromIntError` carries no detail beyond "out of range", and the
            // rejected value is what the reader needs — it rides the variant.
            Some(
                u32::try_from(*limit)
                    .ok()
                    .ok_or(LiftError::LimitOutOfRange(*limit))?,
            )
        }
    };
    let order_by = query
        .order_by
        .iter()
        .map(lift_order)
        .collect::<Result<Vec<_>, LiftError>>()?;
    let (template_id, criteria) = lift_where(query.where_.as_ref())?;
    Ok(BuilderQuery {
        template_id,
        shape,
        columns,
        criteria,
        order_by,
        limit,
    })
}

/// The builder's `FROM` is fixed; anything else is out of the envelope.
fn check_from(from: &ContainsExpr) -> Result<(), LiftError> {
    let ContainsExpr::Contained {
        operand,
        contains: Some(constraint),
    } = from
    else {
        return Err(LiftError::UnsupportedFrom);
    };
    if constraint.negated || !is_plain_class(operand, "EHR", EHR_VAR) {
        return Err(LiftError::UnsupportedFrom);
    }
    let ContainsExpr::Contained {
        operand: composition,
        contains: None,
    } = &constraint.expr
    else {
        return Err(LiftError::UnsupportedFrom);
    };
    if is_plain_class(composition, "COMPOSITION", COMP_VAR) {
        Ok(())
    } else {
        Err(LiftError::UnsupportedFrom)
    }
}

fn is_plain_class(operand: &ClassExprOperand, rm_type: &str, variable: &str) -> bool {
    matches!(
        operand,
        ClassExprOperand::Class { rm_type: rm, variable: Some(var), predicate: None }
            if rm == rm_type && var == variable
    )
}

/// Recognize one of the four output shapes, and for a data-value projection its
/// columns.
fn lift_select(select: &SelectClause) -> Result<(QueryShape, Vec<SelectedColumn>), LiftError> {
    let columns = select.columns.as_slice();
    // The cohort shape is the ONLY DISTINCT query the builder emits.
    if select.distinct {
        if let [only] = columns
            && only.alias.as_deref() == Some(EHR_ID_ALIAS)
            && matches!(&only.column, ColumnExpr::Path(path)
                if relative_path(path, EHR_VAR).as_deref() == Some(EHR_ID_PATH))
        {
            return Ok((QueryShape::Ehrs, Vec::new()));
        }
        return Err(LiftError::UnsupportedSelect);
    }
    if let [only] = columns
        && only.alias.is_none()
    {
        if matches!(
            &only.column,
            ColumnExpr::Aggregate(AggregateCall::Count {
                distinct: false,
                path: None
            })
        ) {
            return Ok((QueryShape::Count, Vec::new()));
        }
        if matches!(&only.column, ColumnExpr::Path(IdentifiedPath { root, predicate: None, path: None, .. })
            if root == COMP_VAR)
        {
            return Ok((QueryShape::Compositions, Vec::new()));
        }
    }
    // Otherwise: a data-value projection, every column a composition-relative
    // path with an optional alias.
    let mut projected = Vec::with_capacity(columns.len());
    for column in columns {
        let ColumnExpr::Path(path) = &column.column else {
            return Err(LiftError::UnsupportedSelect);
        };
        let aql_path = relative_path(path, COMP_VAR).ok_or(LiftError::UnsupportedSelect)?;
        projected.push(SelectedColumn {
            aql_path,
            alias: column.alias.clone().unwrap_or_default(),
        });
    }
    if projected.is_empty() {
        return Err(LiftError::UnsupportedSelect);
    }
    Ok((QueryShape::DataValues, projected))
}

fn lift_order(term: &OrderByExpr) -> Result<OrderRule, LiftError> {
    let descending = match term.order {
        Some(SortOrder::Descending) => true,
        Some(SortOrder::Ascending) => false,
        // The builder always writes the direction; an implicit one would come
        // back as `ASC` and change the query text.
        None => return Err(LiftError::UnsupportedOrderBy(order_text(term))),
    };
    let aql_path = relative_path(&term.path, COMP_VAR)
        .ok_or_else(|| LiftError::UnsupportedOrderBy(order_text(term)))?;
    Ok(OrderRule {
        aql_path,
        descending,
    })
}

// ── WHERE ────────────────────────────────────────────────────────────────────

/// Split the `WHERE` clause into the template restriction (the leading
/// conjunct `lower` always emits first) and the criterion tree.
fn lift_where(where_: Option<&WhereExpr>) -> Result<(String, Option<CriterionNode>), LiftError> {
    let Some(where_) = where_ else {
        return Ok((String::new(), None));
    };
    let mut conjuncts: Vec<&WhereExpr> = Vec::new();
    flatten_and(where_, &mut conjuncts);
    let mut template_id = String::new();
    let mut rest = conjuncts.as_slice();
    if let Some((first, tail)) = conjuncts.split_first()
        && let Some(id) = template_restriction(first)
    {
        template_id = id;
        rest = tail;
    }
    if rest.is_empty() {
        return Ok((template_id, None));
    }
    Ok((template_id, Some(lift_conjunction(rest)?)))
}

/// `c/archetype_details/template_id/value='id'` — the template restriction.
fn template_restriction(expr: &WhereExpr) -> Option<String> {
    let WhereExpr::Identified(
        IdentifiedExpr::Compare {
            lhs: CompareOperand::Path(path),
            op: CompOp::Eq,
            rhs: Terminal::Primitive(Primitive::String(id)),
        },
        _,
    ) = expr
    else {
        return None;
    };
    (relative_path(path, COMP_VAR).as_deref() == Some(TEMPLATE_PATH)).then(|| id.clone())
}

/// Lift one boolean sub-expression into a criterion node.
fn lift_node(expr: &WhereExpr) -> Result<CriterionNode, LiftError> {
    match expr {
        WhereExpr::Not(inner) => match lift_node(inner)? {
            CriterionNode::Leaf(mut criterion) if !criterion.negated => {
                criterion.negated = true;
                Ok(CriterionNode::Leaf(criterion))
            }
            CriterionNode::Group {
                op,
                negated: false,
                children,
            } => Ok(CriterionNode::Group {
                op,
                negated: true,
                children,
            }),
            // A doubly negated node: the builder holds one NOT per node.
            _ => Err(LiftError::UnsupportedCondition(fragment_text(expr))),
        },
        WhereExpr::Or(..) => {
            let mut disjuncts: Vec<&WhereExpr> = Vec::new();
            flatten_or(expr, &mut disjuncts);
            let children = disjuncts
                .into_iter()
                .map(lift_node)
                .collect::<Result<Vec<_>, LiftError>>()?;
            Ok(CriterionNode::Group {
                op: BoolOp::Or,
                negated: false,
                children,
            })
        }
        WhereExpr::And(..) => {
            let mut conjuncts: Vec<&WhereExpr> = Vec::new();
            flatten_and(expr, &mut conjuncts);
            lift_conjunction(&conjuncts)
        }
        WhereExpr::Identified(..) => lift_conjunction(&[expr]),
    }
}

/// Lift an `AND` chain: a run of consecutive conditions on the same path is ONE
/// criterion (a quantity range is three `AND`ed fragments), so the walk is
/// greedy — each step consumes as many leading conjuncts as one criterion
/// covers.
fn lift_conjunction(conjuncts: &[&WhereExpr]) -> Result<CriterionNode, LiftError> {
    let mut children: Vec<CriterionNode> = Vec::new();
    let mut index = 0_usize;
    while let Some(rest) = conjuncts.get(index..) {
        let Some(head) = rest.first().copied() else {
            break;
        };
        if matches!(head, WhereExpr::Identified(..)) {
            let (criterion, consumed) = lift_criterion(rest)?;
            children.push(CriterionNode::Leaf(criterion));
            index = index.saturating_add(consumed);
        } else {
            children.push(lift_node(head)?);
            index = index.saturating_add(1);
        }
    }
    if children.len() == 1
        && let Some(only) = children.pop()
    {
        return Ok(only);
    }
    if children.is_empty() {
        return Err(LiftError::UnsupportedCondition(String::new()));
    }
    Ok(CriterionNode::Group {
        op: BoolOp::And,
        negated: false,
        children,
    })
}

fn flatten_and<'a>(expr: &'a WhereExpr, out: &mut Vec<&'a WhereExpr>) {
    if let WhereExpr::And(left, right) = expr {
        flatten_and(left, out);
        flatten_and(right, out);
    } else {
        out.push(expr);
    }
}

fn flatten_or<'a>(expr: &'a WhereExpr, out: &mut Vec<&'a WhereExpr>) {
    if let WhereExpr::Or(left, right) = expr {
        flatten_or(left, out);
        flatten_or(right, out);
    } else {
        out.push(expr);
    }
}

// ── criterion recognition ────────────────────────────────────────────────────

/// Which DV attribute a condition constrains — the suffix `lower` appends to a
/// leaf's `aql_path` for each [`CriterionKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// `…/magnitude` (`DV_QUANTITY` and `DV_COUNT`).
    Magnitude,
    /// `…/numerator` (`DV_PROPORTION`).
    Numerator,
    /// `…/units` (`DV_QUANTITY`).
    Units,
    /// `…/value` (text, date/time, ordinal, boolean).
    Value,
    /// `…/defining_code/code_string` (`DV_CODED_TEXT`).
    CodeString,
    /// `…/defining_code/terminology_id/value` (`DV_CODED_TEXT`).
    TerminologyId,
}

/// The DV-attribute suffixes, LONGEST FIRST so `…/defining_code/…/value` is
/// never mistaken for a plain `…/value`.
const SLOTS: [(&str, Slot); 6] = [
    ("/defining_code/terminology_id/value", Slot::TerminologyId),
    ("/defining_code/code_string", Slot::CodeString),
    ("/magnitude", Slot::Magnitude),
    ("/numerator", Slot::Numerator),
    ("/units", Slot::Units),
    ("/value", Slot::Value),
];

/// One recognized `WHERE` condition, reduced to the leaf path it constrains and
/// the constraint itself.
enum Atom {
    /// `EXISTS c/<path>`.
    Exists {
        /// The leaf path.
        base: String,
    },
    /// `c/<base><slot> <op> <primitive>`.
    Compare {
        /// The leaf path.
        base: String,
        /// Which DV attribute.
        slot: Slot,
        /// The comparison.
        op: CompOp,
        /// The compared literal.
        value: Primitive,
    },
    /// `c/<base>/value LIKE '<pattern>'`.
    Like {
        /// The leaf path.
        base: String,
        /// The pattern.
        pattern: String,
    },
    /// `c/<base>/defining_code/code_string MATCHES {'a', 'b'}`.
    Codes {
        /// The leaf path.
        base: String,
        /// The accepted codes.
        codes: Vec<String>,
    },
    /// `c/<base>/value MATCHES {1, 2}`.
    Ordinals {
        /// The leaf path.
        base: String,
        /// The accepted ordinals.
        values: Vec<i64>,
    },
}

/// Recognize the criterion starting at `conjuncts[0]`, returning it and how many
/// conjuncts it consumed.
fn lift_criterion(conjuncts: &[&WhereExpr]) -> Result<(Criterion, usize), LiftError> {
    let head = conjuncts
        .first()
        .copied()
        .ok_or_else(|| LiftError::UnsupportedCondition(String::new()))?;
    match classify(head)? {
        Atom::Exists { base } => Ok((leaf(base, CriterionKind::Exists), 1)),
        Atom::Like { base, pattern } => Ok((leaf(base, CriterionKind::TextLike { pattern }), 1)),
        Atom::Ordinals { base, values } => Ok((leaf(base, CriterionKind::OrdinalIn { values }), 1)),
        Atom::Codes { base, codes } => {
            // An optional terminology-id equality follows the code set.
            let terminology = next_string(conjuncts, 1, &base, Slot::TerminologyId, CompOp::Eq);
            let consumed = if terminology.is_some() { 2 } else { 1 };
            Ok((
                leaf(
                    base,
                    CriterionKind::CodedIn {
                        codes,
                        terminology: terminology.unwrap_or_default(),
                    },
                ),
                consumed,
            ))
        }
        Atom::Compare {
            base,
            slot,
            op,
            value,
        } => lift_compare(conjuncts, base, slot, op, &value),
    }
}

/// Recognize a comparison-rooted criterion, absorbing the range/units
/// conjuncts `lower` emits after it.
fn lift_compare(
    conjuncts: &[&WhereExpr],
    base: String,
    slot: Slot,
    op: CompOp,
    value: &Primitive,
) -> Result<(Criterion, usize), LiftError> {
    let unsupported = || {
        conjuncts
            .first()
            .copied()
            .map_or_else(String::new, fragment_text)
    };
    match (slot, op, value) {
        // DV_QUANTITY / DV_PROPORTION: a real-valued range (`lower` compares a
        // quantity magnitude and a proportion numerator as REALS), optionally
        // with units.
        (Slot::Magnitude | Slot::Numerator, CompOp::Ge | CompOp::Le, Primitive::Real(bound)) => {
            Ok(lift_real_range(conjuncts, base, slot, op, *bound))
        }
        // DV_COUNT: the same range on an INTEGER magnitude — which is exactly
        // what tells a count apart from a quantity.
        (Slot::Magnitude, CompOp::Ge | CompOp::Le, Primitive::Integer(bound)) => {
            Ok(lift_count_range(conjuncts, base, op, *bound))
        }
        // DV_QUANTITY constrained only by its units.
        (Slot::Units, CompOp::Eq, Primitive::String(units)) => Ok((
            leaf(
                base,
                CriterionKind::QuantityRange {
                    min: None,
                    max: None,
                    units: units.clone(),
                },
            ),
            1,
        )),
        // DV_TEXT exact match.
        (Slot::Value, CompOp::Eq, Primitive::String(text)) => Ok((
            leaf(base, CriterionKind::TextEquals { text: text.clone() }),
            1,
        )),
        // DV_BOOLEAN.
        (Slot::Value, CompOp::Eq, Primitive::Boolean(value)) => {
            Ok((leaf(base, CriterionKind::BooleanIs { value: *value }), 1))
        }
        // DV_DATE_TIME / DV_DATE / DV_TIME: an ISO-8601 text range.
        (Slot::Value, CompOp::Ge, Primitive::String(from)) => {
            let mut consumed = 1;
            let to = next_string(conjuncts, consumed, &base, Slot::Value, CompOp::Le);
            if to.is_some() {
                consumed = consumed.saturating_add(1);
            }
            Ok((
                leaf(
                    base,
                    CriterionKind::DateTimeRange {
                        from: from.clone(),
                        to: to.unwrap_or_default(),
                    },
                ),
                consumed,
            ))
        }
        (Slot::Value, CompOp::Le, Primitive::String(to)) => Ok((
            leaf(
                base,
                CriterionKind::DateTimeRange {
                    from: String::new(),
                    to: to.clone(),
                },
            ),
            1,
        )),
        _ => Err(LiftError::UnsupportedCondition(unsupported())),
    }
}

/// A real-valued range on a quantity magnitude or a proportion numerator.
/// `lower` writes the bounds and the units in a fixed order — min, then max,
/// then units — so the greedy absorb reads them in that order.
fn lift_real_range(
    conjuncts: &[&WhereExpr],
    base: String,
    slot: Slot,
    op: CompOp,
    bound: f64,
) -> (Criterion, usize) {
    let mut min = None;
    let mut max = None;
    let mut consumed = 1_usize;
    if op == CompOp::Ge {
        min = Some(bound);
        if let Some(upper) = next_real(conjuncts, consumed, &base, slot, CompOp::Le) {
            max = Some(upper);
            consumed = consumed.saturating_add(1);
        }
    } else {
        max = Some(bound);
    }
    let kind = if slot == Slot::Numerator {
        CriterionKind::ProportionNumeratorRange { min, max }
    } else {
        let units = next_string(conjuncts, consumed, &base, Slot::Units, CompOp::Eq);
        if units.is_some() {
            consumed = consumed.saturating_add(1);
        }
        CriterionKind::QuantityRange {
            min,
            max,
            units: units.unwrap_or_default(),
        }
    };
    (leaf(base, kind), consumed)
}

/// An integer range on a count magnitude (no units — `DV_COUNT` has none).
fn lift_count_range(
    conjuncts: &[&WhereExpr],
    base: String,
    op: CompOp,
    bound: i64,
) -> (Criterion, usize) {
    let mut min = None;
    let mut max = None;
    let mut consumed = 1_usize;
    if op == CompOp::Ge {
        min = Some(bound);
        if let Some(upper) = next_integer(conjuncts, consumed, &base, Slot::Magnitude, CompOp::Le) {
            max = Some(upper);
            consumed = consumed.saturating_add(1);
        }
    } else {
        max = Some(bound);
    }
    (leaf(base, CriterionKind::CountRange { min, max }), consumed)
}

/// The value list as code strings, or `None` when it is empty or carries a
/// non-string member.
fn code_list(items: &[ValueListItem]) -> Option<Vec<String>> {
    let mut codes = Vec::with_capacity(items.len());
    for item in items {
        let ValueListItem::Primitive(Primitive::String(code)) = item else {
            return None;
        };
        codes.push(code.clone());
    }
    (!codes.is_empty()).then_some(codes)
}

/// The value list as ordinals, or `None` when it is empty or carries a
/// non-integer member.
fn ordinal_list(items: &[ValueListItem]) -> Option<Vec<i64>> {
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let ValueListItem::Primitive(Primitive::Integer(ordinal)) = item else {
            return None;
        };
        values.push(*ordinal);
    }
    (!values.is_empty()).then_some(values)
}

/// Reduce one `WHERE` leaf to an [`Atom`], or refuse it.
fn classify(expr: &WhereExpr) -> Result<Atom, LiftError> {
    let WhereExpr::Identified(condition, _) = expr else {
        return Err(LiftError::UnsupportedCondition(fragment_text(expr)));
    };
    let refuse = || LiftError::UnsupportedCondition(fragment_text(expr));
    match condition {
        IdentifiedExpr::Exists(path) => {
            let base = relative_path(path, COMP_VAR).ok_or_else(refuse)?;
            Ok(Atom::Exists { base })
        }
        IdentifiedExpr::Like {
            path,
            operand: LikeOperand::String(pattern),
        } => {
            let (base, slot) = split_slot(path).ok_or_else(refuse)?;
            if slot != Slot::Value {
                return Err(refuse());
            }
            Ok(Atom::Like {
                base,
                pattern: pattern.clone(),
            })
        }
        IdentifiedExpr::Matches {
            path,
            operand: MatchesOperand::ValueList(items),
        } => {
            let (base, slot) = split_slot(path).ok_or_else(refuse)?;
            match slot {
                Slot::CodeString => Ok(Atom::Codes {
                    base,
                    codes: code_list(items).ok_or_else(refuse)?,
                }),
                Slot::Value => Ok(Atom::Ordinals {
                    base,
                    values: ordinal_list(items).ok_or_else(refuse)?,
                }),
                _ => Err(refuse()),
            }
        }
        IdentifiedExpr::Compare {
            lhs: CompareOperand::Path(path),
            op,
            rhs: Terminal::Primitive(value),
        } => {
            let (base, slot) = split_slot(path).ok_or_else(refuse)?;
            Ok(Atom::Compare {
                base,
                slot,
                op: *op,
                value: value.clone(),
            })
        }
        _ => Err(refuse()),
    }
}

/// The conjunct at `index`, when it is `<base><slot> <op> '<string>'`.
fn next_string(
    conjuncts: &[&WhereExpr],
    index: usize,
    base: &str,
    slot: Slot,
    op: CompOp,
) -> Option<String> {
    match next_compare(conjuncts, index, base, slot, op)? {
        Primitive::String(text) => Some(text),
        _ => None,
    }
}

/// The conjunct at `index`, when it is `<base><slot> <op> <real>`.
fn next_real(
    conjuncts: &[&WhereExpr],
    index: usize,
    base: &str,
    slot: Slot,
    op: CompOp,
) -> Option<f64> {
    match next_compare(conjuncts, index, base, slot, op)? {
        Primitive::Real(value) => Some(value),
        _ => None,
    }
}

/// The conjunct at `index`, when it is `<base><slot> <op> <integer>`.
fn next_integer(
    conjuncts: &[&WhereExpr],
    index: usize,
    base: &str,
    slot: Slot,
    op: CompOp,
) -> Option<i64> {
    match next_compare(conjuncts, index, base, slot, op)? {
        Primitive::Integer(value) => Some(value),
        _ => None,
    }
}

/// The literal the conjunct at `index` compares, when that conjunct constrains
/// exactly `base` + `slot` with `op`.
fn next_compare(
    conjuncts: &[&WhereExpr],
    index: usize,
    base: &str,
    slot: Slot,
    op: CompOp,
) -> Option<Primitive> {
    let candidate = conjuncts.get(index).copied()?;
    match classify(candidate).ok()? {
        Atom::Compare {
            base: found_base,
            slot: found_slot,
            op: found_op,
            value,
        } if found_base == base && found_slot == slot && found_op == op => Some(value),
        _ => None,
    }
}

fn leaf(aql_path: String, kind: CriterionKind) -> Criterion {
    Criterion {
        aql_path,
        negated: false,
        kind,
    }
}

// ── paths & text ─────────────────────────────────────────────────────────────

/// A composition/EHR-relative path: the path text as written, minus the root
/// variable, `/`-prefixed — the form the Web Template's `aqlPath` (and hence
/// [`crate::builder::catalog::CatalogNode::aql_path`]) uses, so a lifted
/// criterion keys straight into the builder's catalog metadata.
///
/// `None` for a path rooted elsewhere, one carrying a root predicate, or a bare
/// variable.
fn relative_path(path: &IdentifiedPath, variable: &str) -> Option<String> {
    if path.root != variable || path.predicate.is_some() {
        return None;
    }
    path.path
        .as_ref()
        .map(|object_path| format!("/{object_path}"))
}

/// Split a composition-relative path into the leaf it targets and the DV
/// attribute it constrains.
fn split_slot(path: &IdentifiedPath) -> Option<(String, Slot)> {
    let relative = relative_path(path, COMP_VAR)?;
    for (suffix, slot) in SLOTS {
        if let Some(base) = relative.strip_suffix(suffix)
            && !base.is_empty()
        {
            return Some((base.to_owned(), slot));
        }
    }
    None
}

/// Render a `WHERE` fragment as AQL text for a refusal message, by printing a
/// probe query through the canonical printer and keeping its `WHERE` tail.
fn fragment_text(expr: &WhereExpr) -> String {
    let probe = SelectQuery {
        select: SelectClause {
            distinct: false,
            top: None,
            columns: vec![SelectExpr {
                column: ColumnExpr::Path(IdentifiedPath::new(COMP_VAR.to_owned(), None, None)),
                alias: None,
            }],
        },
        from: ContainsExpr::Contained {
            operand: ClassExprOperand::Class {
                rm_type: "EHR".to_owned(),
                variable: Some(EHR_VAR.to_owned()),
                predicate: None,
            },
            contains: None,
        },
        where_: Some(expr.clone()),
        order_by: Vec::new(),
        limit: None,
    };
    let text = openehr_query::printer::to_aql(&probe);
    let tail = text.split_once(" WHERE ").map(|(_, tail)| tail.to_owned());
    tail.unwrap_or(text)
}

/// Render one `ORDER BY` term as AQL text for a refusal message.
fn order_text(term: &OrderByExpr) -> String {
    let direction = match term.order {
        Some(SortOrder::Ascending) => " ASC",
        Some(SortOrder::Descending) => " DESC",
        None => "",
    };
    format!(
        "{}{}{direction}",
        term.path.root,
        term.path
            .path
            .as_ref()
            .map(|object_path| format!("/{object_path}"))
            .unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use crate::builder::lift::{LiftError, from_aql};
    use crate::builder::lower::to_aql;
    use crate::builder::model::{
        BoolOp, BuilderQuery, Criterion, CriterionKind, CriterionNode, OrderRule, QueryShape,
        SelectedColumn,
    };

    const TEMP_PATH: &str = "/content[openEHR-EHR-OBSERVATION.body_temperature.v2]/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value";

    fn leaf(kind: CriterionKind) -> CriterionNode {
        CriterionNode::Leaf(Criterion {
            aql_path: TEMP_PATH.to_owned(),
            negated: false,
            kind,
        })
    }

    /// The round-trip law: every builder-produced query lifts back to a state
    /// that lowers to the SAME AQL text.
    #[track_caller]
    fn round_trips(query: &BuilderQuery) {
        let aql = to_aql(query).expect("the fixture lowers");
        let lifted = from_aql(&aql).unwrap_or_else(|e| panic!("lift of `{aql}` refused: {e}"));
        let relowered = to_aql(&lifted).expect("the lifted state lowers");
        assert_eq!(relowered, aql, "AQL drift through lift");
        assert_eq!(&lifted, query, "builder state drift through lift");
    }

    #[test]
    fn composition_shape_with_a_quantity_range_round_trips() {
        let mut query = BuilderQuery::new("vitals.v1".to_owned());
        query.criteria = Some(leaf(CriterionKind::QuantityRange {
            min: Some(36.0),
            max: Some(38.5),
            units: "°C".to_owned(),
        }));
        round_trips(&query);
    }

    #[test]
    fn every_criterion_kind_round_trips() {
        for kind in [
            CriterionKind::QuantityRange {
                min: Some(1.5),
                max: None,
                units: String::new(),
            },
            CriterionKind::QuantityRange {
                min: None,
                max: Some(9.0),
                units: "mm[Hg]".to_owned(),
            },
            CriterionKind::QuantityRange {
                min: None,
                max: None,
                units: "cm".to_owned(),
            },
            CriterionKind::CountRange {
                min: Some(1),
                max: Some(4),
            },
            CriterionKind::CountRange {
                min: None,
                max: Some(7),
            },
            CriterionKind::ProportionNumeratorRange {
                min: Some(0.25),
                max: Some(0.75),
            },
            CriterionKind::CodedIn {
                codes: vec!["at0037".to_owned(), "at0038".to_owned()],
                terminology: "local".to_owned(),
            },
            CriterionKind::CodedIn {
                codes: vec!["at0037".to_owned()],
                terminology: String::new(),
            },
            CriterionKind::TextEquals {
                text: "O'Neil \\ Sons".to_owned(),
            },
            CriterionKind::TextLike {
                pattern: "*fever*".to_owned(),
            },
            CriterionKind::DateTimeRange {
                from: "2026-01-01T00:00:00Z".to_owned(),
                to: "2026-12-31T23:59:59Z".to_owned(),
            },
            CriterionKind::DateTimeRange {
                from: String::new(),
                to: "2026-12-31".to_owned(),
            },
            CriterionKind::OrdinalIn {
                values: vec![1, 2, 3],
            },
            CriterionKind::BooleanIs { value: false },
            CriterionKind::Exists,
        ] {
            let mut query = BuilderQuery::new("vitals.v1".to_owned());
            query.criteria = Some(leaf(kind.clone()));
            round_trips(&query);
            // …and the same criterion negated.
            let mut negated = BuilderQuery::new(String::new());
            negated.criteria = Some(CriterionNode::Leaf(Criterion {
                aql_path: TEMP_PATH.to_owned(),
                negated: true,
                kind,
            }));
            round_trips(&negated);
        }
    }

    #[test]
    fn every_output_shape_round_trips() {
        for shape in [
            QueryShape::Compositions,
            QueryShape::Count,
            QueryShape::Ehrs,
        ] {
            let mut query = BuilderQuery::new("vitals.v1".to_owned());
            query.shape = shape;
            round_trips(&query);
            query.limit = None;
            round_trips(&query);
        }
        let mut projection = BuilderQuery::new(String::new());
        projection.shape = QueryShape::DataValues;
        projection.columns = vec![
            SelectedColumn {
                aql_path: format!("{TEMP_PATH}/magnitude"),
                alias: "temperature".to_owned(),
            },
            SelectedColumn {
                aql_path: "/context/start_time/value".to_owned(),
                alias: String::new(),
            },
        ];
        projection.order_by = vec![
            OrderRule {
                aql_path: "/context/start_time/value".to_owned(),
                descending: true,
            },
            OrderRule {
                aql_path: format!("{TEMP_PATH}/magnitude"),
                descending: false,
            },
        ];
        round_trips(&projection);
    }

    #[test]
    fn nested_groups_and_negation_round_trip() {
        let mut query = BuilderQuery::new(String::new());
        query.shape = QueryShape::Count;
        query.limit = None;
        query.criteria = Some(CriterionNode::Group {
            op: BoolOp::And,
            negated: false,
            children: vec![
                leaf(CriterionKind::CodedIn {
                    codes: vec!["at0037".to_owned(), "at0038".to_owned()],
                    terminology: "local".to_owned(),
                }),
                CriterionNode::Group {
                    op: BoolOp::Or,
                    negated: true,
                    children: vec![
                        leaf(CriterionKind::BooleanIs { value: true }),
                        leaf(CriterionKind::Exists),
                        CriterionNode::Group {
                            op: BoolOp::And,
                            negated: false,
                            children: vec![
                                leaf(CriterionKind::QuantityRange {
                                    min: Some(1.0),
                                    max: Some(2.0),
                                    units: "cm".to_owned(),
                                }),
                                leaf(CriterionKind::TextEquals {
                                    text: "x".to_owned(),
                                }),
                            ],
                        },
                    ],
                },
            ],
        });
        round_trips(&query);
    }

    #[test]
    fn a_hand_written_query_in_the_envelope_lifts() {
        let aql = "SELECT c FROM EHR e CONTAINS COMPOSITION c \
                   WHERE c/archetype_details/template_id/value='vitals.v1' LIMIT 25";
        let lifted = from_aql(aql).expect("in-envelope query lifts");
        assert_eq!(lifted.template_id, "vitals.v1");
        assert_eq!(lifted.shape, QueryShape::Compositions);
        assert_eq!(lifted.limit, Some(25));
        assert!(lifted.criteria.is_none());
    }

    #[test]
    fn a_criterion_path_lifts_with_the_catalog_leading_slash() {
        // The Web Template's `aqlPath` is `/`-rooted, and the builder's leaf
        // metadata is keyed by it — so the lifted path must carry the slash.
        let aql = "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE EXISTS c/context/start_time";
        let lifted = from_aql(aql).expect("lifts");
        let Some(CriterionNode::Leaf(criterion)) = lifted.criteria else {
            panic!("expected one leaf criterion");
        };
        assert_eq!(criterion.aql_path, "/context/start_time");
    }

    #[test]
    fn a_parameterised_query_is_refused_by_name() {
        let aql = "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE c/name/value=$name";
        assert_eq!(from_aql(aql).unwrap_err(), LiftError::Parameterised);
    }

    #[test]
    fn out_of_envelope_from_clauses_are_refused() {
        for aql in [
            // A different RM class.
            "SELECT c FROM COMPOSITION c",
            // A deeper CONTAINS chain.
            "SELECT c FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o",
            // A predicate on the EHR.
            "SELECT c FROM EHR e[ehr_id/value='x'] CONTAINS COMPOSITION c",
            // An archetype predicate on the COMPOSITION.
            "SELECT c FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1]",
            // NOT CONTAINS.
            "SELECT c FROM EHR e NOT CONTAINS COMPOSITION c",
            // Other variable names.
            "SELECT x FROM EHR y CONTAINS COMPOSITION x",
        ] {
            assert_eq!(
                from_aql(aql).unwrap_err(),
                LiftError::UnsupportedFrom,
                "must refuse `{aql}`"
            );
        }
    }

    #[test]
    fn out_of_envelope_select_lists_are_refused() {
        for aql in [
            // An aggregate the builder has no shape for.
            "SELECT MAX(c/context/start_time/value) FROM EHR e CONTAINS COMPOSITION c",
            // COUNT over a path.
            "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c",
            // A literal column.
            "SELECT 1 FROM EHR e CONTAINS COMPOSITION c",
            // DISTINCT on something other than the cohort column.
            "SELECT DISTINCT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
            // The cohort column WITHOUT distinct (the builder always emits it).
            "SELECT e/ehr_id/value AS ehr_id FROM EHR e CONTAINS COMPOSITION c",
            // A projection column rooted at the EHR.
            "SELECT e/ehr_id/value FROM EHR e CONTAINS COMPOSITION c",
            // An aliased whole composition.
            "SELECT c AS whole FROM EHR e CONTAINS COMPOSITION c",
        ] {
            assert_eq!(
                from_aql(aql).unwrap_err(),
                LiftError::UnsupportedSelect,
                "must refuse `{aql}`"
            );
        }
    }

    #[test]
    fn out_of_envelope_windows_are_refused() {
        assert_eq!(
            from_aql("SELECT TOP 5 c FROM EHR e CONTAINS COMPOSITION c").unwrap_err(),
            LiftError::UnsupportedTop
        );
        assert_eq!(
            from_aql("SELECT c FROM EHR e CONTAINS COMPOSITION c LIMIT 10 OFFSET 5").unwrap_err(),
            LiftError::UnsupportedOffset
        );
        assert_eq!(
            from_aql("SELECT c FROM EHR e CONTAINS COMPOSITION c LIMIT 99999999999").unwrap_err(),
            LiftError::LimitOutOfRange(99_999_999_999)
        );
    }

    #[test]
    fn an_implicit_sort_direction_is_refused() {
        // The builder writes ASC/DESC explicitly, so an implicit direction
        // would come back as text the stored query does not have.
        assert!(matches!(
            from_aql(
                "SELECT c FROM EHR e CONTAINS COMPOSITION c ORDER BY c/context/start_time/value"
            )
            .unwrap_err(),
            LiftError::UnsupportedOrderBy(_)
        ));
        // An order term rooted outside the COMPOSITION.
        assert!(matches!(
            from_aql("SELECT c FROM EHR e CONTAINS COMPOSITION c ORDER BY e/ehr_id/value ASC")
                .unwrap_err(),
            LiftError::UnsupportedOrderBy(_)
        ));
    }

    #[test]
    fn out_of_envelope_conditions_are_refused_naming_the_fragment() {
        for (aql, fragment) in [
            // An operator the criterion catalog has no widget for.
            (
                "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE c/context/x/value>1.0",
                "c/context/x/value>1.0",
            ),
            // A path with no DV-attribute suffix the catalog knows.
            (
                "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE c/context/x/defining_code='a'",
                "c/context/x/defining_code='a'",
            ),
            // A path-to-path comparison.
            (
                "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE c/a/value=c/b/value",
                "c/a/value=c/b/value",
            ),
            // A MATCHES set mixing types.
            (
                "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE c/a/value MATCHES {1, 'two'}",
                "c/a/value MATCHES {1, 'two'}",
            ),
        ] {
            assert_eq!(
                from_aql(aql).unwrap_err(),
                LiftError::UnsupportedCondition(fragment.to_owned()),
                "must refuse `{aql}`"
            );
        }
    }

    #[test]
    fn a_double_negation_is_refused() {
        assert!(matches!(
            from_aql("SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE NOT NOT EXISTS c/a")
                .unwrap_err(),
            LiftError::UnsupportedCondition(_)
        ));
    }

    #[test]
    fn invalid_aql_is_refused_with_the_parser_diagnostic() {
        assert!(matches!(
            from_aql("this is not a query").unwrap_err(),
            LiftError::NotAql(_)
        ));
    }

    #[test]
    fn same_path_ranges_that_do_not_merge_stay_separate_criteria() {
        // A quantity min beside an INTEGER max: the two are different criterion
        // kinds, so they lift as two conditions — which still re-lowers to the
        // identical text, so the round-trip law holds.
        let aql = "SELECT c FROM EHR e CONTAINS COMPOSITION c \
                   WHERE c/a[at0001]/value/magnitude>=36.0 AND c/a[at0001]/value/magnitude<=38";
        let lifted = from_aql(aql).expect("lifts as two criteria");
        assert_eq!(to_aql(&lifted).expect("lowers"), aql);
        let Some(CriterionNode::Group { children, .. }) = lifted.criteria else {
            panic!("expected an AND group of two criteria");
        };
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn a_template_restriction_alone_carries_no_criteria() {
        let query = BuilderQuery::new("vitals.v1".to_owned());
        round_trips(&query);
        let lifted = from_aql(&to_aql(&query).expect("lowers")).expect("lifts");
        assert!(lifted.criteria.is_none());
    }
}
