// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lowering [`crate::builder::model::BuilderQuery`] into the `openehr_query`
//! AST.
//!
//! Paths are parsed by the real AQL grammar (a probe `SELECT` through
//! `openehr_query::parser`), constraints become typed `WHERE` fragments, and
//! the result renders via `openehr_query::printer::to_aql` — invalid input is
//! a typed [`BuilderError`], never a malformed query string.

use openehr_query::ast::{
    ClassExprOperand, ColumnExpr, CompareOperand, ContainsConstraint, ContainsExpr, IdentifiedExpr,
    IdentifiedPath, Limit, MatchesOperand, OrderByExpr, Primitive, SelectClause, SelectExpr,
    SelectQuery, SortOrder, Terminal, ValueListItem, WhereExpr,
};
use openehr_query::lexer::CompOp;

use crate::builder::model::{
    BoolOp, BuilderQuery, Criterion, CriterionKind, CriterionNode, QueryShape,
};

/// Everything that can be wrong with a builder state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, serde::Serialize, serde::Deserialize)]
pub enum BuilderError {
    /// A criterion group has no children.
    #[error("a criterion group is empty — add a condition or remove the group")]
    EmptyGroup,
    /// Data-value shape without any selected column.
    #[error("select at least one column for a data-value query")]
    NoColumns,
    /// A path failed to parse under the AQL grammar.
    #[error("path `{path}` is not valid AQL: {detail}")]
    InvalidPath {
        /// The offending path.
        path: String,
        /// Parser diagnostic.
        detail: String,
    },
    /// A coded criterion with no codes.
    #[error("a coded-text condition needs at least one code")]
    EmptyCodeList,
    /// An ordinal criterion with no values.
    #[error("an ordinal condition needs at least one value")]
    EmptyOrdinalList,
    /// A range with neither bound.
    #[error("a range condition needs at least one bound")]
    EmptyRange,
    /// A column alias that is not a valid AQL identifier.
    #[error("alias `{0}` is not a valid AQL identifier")]
    InvalidAlias(String),
}

/// The variable bound to the EHR.
const EHR_VAR: &str = "e";
/// The variable bound to the COMPOSITION — all builder paths are relative
/// to it (the Web Template's `aqlPath` is COMPOSITION-rooted).
const COMP_VAR: &str = "c";

/// Lower a builder state to the AQL AST.
///
/// # Errors
/// A [`BuilderError`] naming the first invalid part of the state.
pub fn lower(query: &BuilderQuery) -> Result<SelectQuery, BuilderError> {
    let columns = select_columns(query)?;
    let where_ = where_clause(query)?;
    let order_by = query
        .order_by
        .iter()
        .map(|rule| {
            Ok(OrderByExpr {
                path: parse_path(COMP_VAR, &rule.aql_path)?,
                order: Some(if rule.descending {
                    SortOrder::Descending
                } else {
                    SortOrder::Ascending
                }),
            })
        })
        .collect::<Result<Vec<_>, BuilderError>>()?;

    Ok(SelectQuery {
        select: SelectClause {
            // A cohort query de-duplicates: several matching compositions in
            // one EHR must yield that EHR once.
            distinct: query.shape == QueryShape::Ehrs,
            top: None,
            columns,
        },
        from: ContainsExpr::Contained {
            operand: ClassExprOperand::Class {
                rm_type: "EHR".to_owned(),
                variable: Some(EHR_VAR.to_owned()),
                predicate: None,
            },
            contains: Some(Box::new(ContainsConstraint {
                negated: false,
                expr: ContainsExpr::Contained {
                    operand: ClassExprOperand::Class {
                        rm_type: "COMPOSITION".to_owned(),
                        variable: Some(COMP_VAR.to_owned()),
                        predicate: None,
                    },
                    contains: None,
                },
            })),
        },
        where_,
        order_by,
        limit: query.limit.map(|n| Limit {
            limit: i64::from(n),
            offset: None,
        }),
    })
}

/// Lower and render in one step.
///
/// # Errors
/// See [`lower`].
pub fn to_aql(query: &BuilderQuery) -> Result<String, BuilderError> {
    Ok(openehr_query::printer::to_aql(&lower(query)?))
}

fn select_columns(query: &BuilderQuery) -> Result<Vec<SelectExpr>, BuilderError> {
    match query.shape {
        QueryShape::Compositions => Ok(vec![SelectExpr {
            column: ColumnExpr::Path(IdentifiedPath::new(COMP_VAR.to_owned(), None, None)),
            alias: None,
        }]),
        QueryShape::Count => Ok(vec![SelectExpr {
            column: ColumnExpr::Aggregate(openehr_query::ast::AggregateCall::Count {
                distinct: false,
                path: None,
            }),
            alias: None,
        }]),
        QueryShape::Ehrs => Ok(vec![SelectExpr {
            column: ColumnExpr::Path(parse_path(EHR_VAR, "ehr_id/value")?),
            alias: Some("ehr_id".to_owned()),
        }]),
        QueryShape::DataValues => {
            if query.columns.is_empty() {
                return Err(BuilderError::NoColumns);
            }
            query
                .columns
                .iter()
                .map(|col| {
                    let alias = if col.alias.is_empty() {
                        None
                    } else if is_identifier(&col.alias) {
                        Some(col.alias.clone())
                    } else {
                        return Err(BuilderError::InvalidAlias(col.alias.clone()));
                    };
                    Ok(SelectExpr {
                        column: ColumnExpr::Path(parse_path(COMP_VAR, &col.aql_path)?),
                        alias,
                    })
                })
                .collect()
        }
    }
}

fn where_clause(query: &BuilderQuery) -> Result<Option<WhereExpr>, BuilderError> {
    let mut conjuncts: Vec<WhereExpr> = Vec::new();
    if !query.template_id.is_empty() {
        conjuncts.push(compare_str(
            "archetype_details/template_id/value",
            CompOp::Eq,
            &query.template_id,
        )?);
    }
    if let Some(tree) = &query.criteria {
        conjuncts.push(criterion_node(tree)?);
    }
    Ok(conjuncts
        .into_iter()
        .reduce(|a, b| WhereExpr::And(Box::new(a), Box::new(b))))
}

fn criterion_node(node: &CriterionNode) -> Result<WhereExpr, BuilderError> {
    match node {
        CriterionNode::Leaf(criterion) => {
            let expr = leaf(criterion)?;
            Ok(if criterion.negated {
                WhereExpr::Not(Box::new(expr))
            } else {
                expr
            })
        }
        CriterionNode::Group {
            op,
            negated,
            children,
        } => {
            if children.is_empty() {
                return Err(BuilderError::EmptyGroup);
            }
            let lowered = children
                .iter()
                .map(criterion_node)
                .collect::<Result<Vec<_>, _>>()?;
            let joined = lowered
                .into_iter()
                .reduce(|a, b| match op {
                    BoolOp::And => WhereExpr::And(Box::new(a), Box::new(b)),
                    BoolOp::Or => WhereExpr::Or(Box::new(a), Box::new(b)),
                })
                // `reduce` yields `None` only for an empty iterator, which the
                // emptiness check above has already rejected; the typed error
                // keeps that impossible branch panic-free.
                .ok_or(BuilderError::EmptyGroup)?;
            Ok(if *negated {
                WhereExpr::Not(Box::new(joined))
            } else {
                joined
            })
        }
    }
}

/// Join range parts with `AND`; no parts at all is a typed error.
fn all_of(parts: Vec<WhereExpr>) -> Result<WhereExpr, BuilderError> {
    parts
        .into_iter()
        .reduce(|a, b| WhereExpr::And(Box::new(a), Box::new(b)))
        .ok_or(BuilderError::EmptyRange)
}

fn leaf(criterion: &Criterion) -> Result<WhereExpr, BuilderError> {
    let at = |suffix: &str| format!("{}/{suffix}", criterion.aql_path.trim_matches('/'));
    match &criterion.kind {
        CriterionKind::QuantityRange { min, max, units } => {
            let mut parts = real_bounds(&at("magnitude"), *min, *max)?;
            if !units.is_empty() {
                parts.push(compare_str(&at("units"), CompOp::Eq, units)?);
            }
            all_of(parts)
        }
        CriterionKind::CodedIn { codes, terminology } => {
            coded_in(&at("defining_code"), codes, terminology)
        }
        CriterionKind::TextEquals { text } => compare_str(&at("value"), CompOp::Eq, text),
        CriterionKind::TextLike { pattern } => Ok(WhereExpr::identified(IdentifiedExpr::Like {
            path: parse_path(COMP_VAR, &at("value"))?,
            operand: openehr_query::ast::LikeOperand::String(pattern.clone()),
        })),
        CriterionKind::DateTimeRange { from, to } => all_of(text_bounds(&at("value"), from, to)?),
        CriterionKind::CountRange { min, max } => all_of(int_bounds(&at("magnitude"), *min, *max)?),
        CriterionKind::OrdinalIn { values } => ordinal_in(&at("value"), values),
        CriterionKind::BooleanIs { value } => Ok(WhereExpr::identified(IdentifiedExpr::Compare {
            lhs: CompareOperand::Path(parse_path(COMP_VAR, &at("value"))?),
            op: CompOp::Eq,
            rhs: Terminal::Primitive(Primitive::Boolean(*value)),
        })),
        CriterionKind::ProportionNumeratorRange { min, max } => {
            all_of(real_bounds(&at("numerator"), *min, *max)?)
        }
        CriterionKind::Exists => Ok(WhereExpr::identified(IdentifiedExpr::Exists(parse_path(
            COMP_VAR,
            &criterion.aql_path,
        )?))),
    }
}

/// The present bounds of a real-valued range, as comparisons on `path`.
fn real_bounds(
    path: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> Result<Vec<WhereExpr>, BuilderError> {
    let mut parts = Vec::new();
    if let Some(min) = min {
        parts.push(compare_real(path, CompOp::Ge, min)?);
    }
    if let Some(max) = max {
        parts.push(compare_real(path, CompOp::Le, max)?);
    }
    Ok(parts)
}

/// The present bounds of an integer-valued range, as comparisons on `path`.
fn int_bounds(
    path: &str,
    min: Option<i64>,
    max: Option<i64>,
) -> Result<Vec<WhereExpr>, BuilderError> {
    let mut parts = Vec::new();
    if let Some(min) = min {
        parts.push(compare_int(path, CompOp::Ge, min)?);
    }
    if let Some(max) = max {
        parts.push(compare_int(path, CompOp::Le, max)?);
    }
    Ok(parts)
}

/// The present bounds of a lexical (ISO-8601 date/time) range, as comparisons
/// on `path`. An empty string is an absent bound.
fn text_bounds(path: &str, from: &str, to: &str) -> Result<Vec<WhereExpr>, BuilderError> {
    let mut parts = Vec::new();
    if !from.is_empty() {
        parts.push(compare_str(path, CompOp::Ge, from)?);
    }
    if !to.is_empty() {
        parts.push(compare_str(path, CompOp::Le, to)?);
    }
    Ok(parts)
}

/// `<path> MATCHES {values}` over an ordinal criterion's selected values.
///
/// # Errors
/// [`BuilderError::EmptyOrdinalList`] when the criterion selects nothing.
fn ordinal_in(path: &str, values: &[i64]) -> Result<WhereExpr, BuilderError> {
    if values.is_empty() {
        return Err(BuilderError::EmptyOrdinalList);
    }
    Ok(WhereExpr::identified(IdentifiedExpr::Matches {
        path: parse_path(COMP_VAR, path)?,
        operand: MatchesOperand::ValueList(
            values
                .iter()
                .map(|v| ValueListItem::Primitive(Primitive::Integer(*v)))
                .collect(),
        ),
    }))
}

/// `<defining_code>/code_string MATCHES {codes}` plus an optional
/// terminology-id equality.
fn coded_in(
    defining_code: &str,
    codes: &[String],
    terminology: &str,
) -> Result<WhereExpr, BuilderError> {
    if codes.is_empty() {
        return Err(BuilderError::EmptyCodeList);
    }
    let matches = WhereExpr::identified(IdentifiedExpr::Matches {
        path: parse_path(COMP_VAR, &format!("{defining_code}/code_string"))?,
        operand: MatchesOperand::ValueList(
            codes
                .iter()
                .map(|code| ValueListItem::Primitive(Primitive::String(code.clone())))
                .collect(),
        ),
    });
    if terminology.is_empty() {
        Ok(matches)
    } else {
        Ok(WhereExpr::And(
            Box::new(matches),
            Box::new(compare_str(
                &format!("{defining_code}/terminology_id/value"),
                CompOp::Eq,
                terminology,
            )?),
        ))
    }
}

fn compare_str(relative: &str, op: CompOp, value: &str) -> Result<WhereExpr, BuilderError> {
    Ok(WhereExpr::identified(IdentifiedExpr::Compare {
        lhs: CompareOperand::Path(parse_path(COMP_VAR, relative)?),
        op,
        rhs: Terminal::Primitive(Primitive::String(value.to_owned())),
    }))
}

fn compare_real(path: &str, op: CompOp, value: f64) -> Result<WhereExpr, BuilderError> {
    Ok(WhereExpr::identified(IdentifiedExpr::Compare {
        lhs: CompareOperand::Path(parse_path(COMP_VAR, path)?),
        op,
        rhs: Terminal::Primitive(Primitive::Real(value)),
    }))
}

fn compare_int(path: &str, op: CompOp, value: i64) -> Result<WhereExpr, BuilderError> {
    Ok(WhereExpr::identified(IdentifiedExpr::Compare {
        lhs: CompareOperand::Path(parse_path(COMP_VAR, path)?),
        op,
        rhs: Terminal::Primitive(Primitive::Integer(value)),
    }))
}

/// Parse `var/relative-path` into an [`IdentifiedPath`] using the real AQL
/// grammar (probe `SELECT`), so path validity is the parser's judgement.
fn parse_path(var: &str, relative: &str) -> Result<IdentifiedPath, BuilderError> {
    parse_path_rooted(&format!("{var}/{}", relative.trim_matches('/')))
}

/// Parse an already-rooted path text (`c/a/b[at0001]/c`).
fn parse_path_rooted(rooted: &str) -> Result<IdentifiedPath, BuilderError> {
    let probe = format!("SELECT {rooted} FROM EHR {EHR_VAR}");
    let parsed =
        openehr_query::parser::parse_str(&probe).map_err(|e| BuilderError::InvalidPath {
            path: rooted.to_owned(),
            detail: e.to_string(),
        })?;
    match parsed.select.columns.into_iter().next() {
        Some(SelectExpr {
            column: ColumnExpr::Path(path),
            ..
        }) => Ok(path),
        _ => Err(BuilderError::InvalidPath {
            path: rooted.to_owned(),
            detail: "did not parse as a data path".to_owned(),
        }),
    }
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::{BuilderError, lower, to_aql};
    use crate::builder::model::{
        BoolOp, BuilderQuery, Criterion, CriterionKind, CriterionNode, OrderRule, QueryShape,
        SelectedColumn,
    };

    const TEMP_PATH: &str = "content[openEHR-EHR-OBSERVATION.body_temperature.v2]/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value";

    fn leaf(kind: CriterionKind) -> CriterionNode {
        CriterionNode::Leaf(Criterion {
            aql_path: TEMP_PATH.to_owned(),
            negated: false,
            kind,
        })
    }

    #[test]
    fn composition_query_with_quantity_range_renders() {
        let mut q = BuilderQuery::new("vitals.v1".to_owned());
        q.criteria = Some(leaf(CriterionKind::QuantityRange {
            min: Some(36.0),
            max: Some(38.5),
            units: "°C".to_owned(),
        }));
        let aql = to_aql(&q).unwrap();
        // The range's three conjuncts are the RIGHT operand of the top-level
        // `AND`, and AQL resolves `AND` left-associatively, so the group keeps
        // its parentheses: without them the text would re-parse as a
        // differently-shaped (if logically equivalent) query.
        assert_eq!(
            aql,
            format!(
                "SELECT c FROM EHR e CONTAINS COMPOSITION c \
                 WHERE c/archetype_details/template_id/value='vitals.v1' \
                 AND (c/{TEMP_PATH}/magnitude>=36.0 \
                 AND c/{TEMP_PATH}/magnitude<=38.5 \
                 AND c/{TEMP_PATH}/units='°C') LIMIT 50"
            )
        );
        // And the emitted text does not merely parse — it parses back to the
        // very query that was printed.
        assert_eq!(
            openehr_query::parser::parse_str(&aql).unwrap(),
            lower(&q).unwrap()
        );
    }

    #[test]
    fn datavalue_shape_selects_aliased_columns_and_orders() {
        let mut q = BuilderQuery::new(String::new());
        q.shape = QueryShape::DataValues;
        q.columns = vec![SelectedColumn {
            aql_path: format!("{TEMP_PATH}/magnitude"),
            alias: "temperature".to_owned(),
        }];
        q.order_by = vec![OrderRule {
            aql_path: "context/start_time/value".to_owned(),
            descending: true,
        }];
        q.limit = None;
        let aql = to_aql(&q).unwrap();
        assert_eq!(
            aql,
            format!(
                "SELECT c/{TEMP_PATH}/magnitude AS temperature \
                 FROM EHR e CONTAINS COMPOSITION c \
                 ORDER BY c/context/start_time/value DESC"
            )
        );
        openehr_query::parser::parse_str(&aql).unwrap();
    }

    #[test]
    fn ehrs_shape_selects_distinct_ehr_ids() {
        let mut q = BuilderQuery::new("vitals.v1".to_owned());
        q.shape = QueryShape::Ehrs;
        q.limit = None;
        let aql = to_aql(&q).unwrap();
        assert_eq!(
            aql,
            "SELECT DISTINCT e/ehr_id/value AS ehr_id FROM EHR e CONTAINS COMPOSITION c WHERE c/archetype_details/template_id/value='vitals.v1'"
        );
        // The printed cohort query must survive the real grammar.
        openehr_query::parser::parse_str(&aql).unwrap();
    }

    #[test]
    fn nested_groups_negation_and_coded_sets_render_with_parens() {
        let mut q = BuilderQuery::new(String::new());
        q.shape = QueryShape::Count;
        q.limit = None;
        q.criteria = Some(CriterionNode::Group {
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
                    ],
                },
            ],
        });
        let aql = to_aql(&q).unwrap();
        assert_eq!(
            aql,
            format!(
                "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c \
                 WHERE c/{TEMP_PATH}/defining_code/code_string MATCHES {{'at0037', 'at0038'}} \
                 AND c/{TEMP_PATH}/defining_code/terminology_id/value='local' \
                 AND NOT (c/{TEMP_PATH}/value=true OR EXISTS c/{TEMP_PATH})"
            )
        );
        openehr_query::parser::parse_str(&aql).unwrap();
    }

    #[test]
    fn validation_errors_are_typed() {
        let mut q = BuilderQuery::new(String::new());
        q.shape = QueryShape::DataValues;
        assert_eq!(to_aql(&q).unwrap_err(), BuilderError::NoColumns);

        let mut q = BuilderQuery::new(String::new());
        q.criteria = Some(CriterionNode::Group {
            op: BoolOp::And,
            negated: false,
            children: vec![],
        });
        assert_eq!(to_aql(&q).unwrap_err(), BuilderError::EmptyGroup);

        let mut q = BuilderQuery::new(String::new());
        q.criteria = Some(leaf(CriterionKind::QuantityRange {
            min: None,
            max: None,
            units: String::new(),
        }));
        assert_eq!(to_aql(&q).unwrap_err(), BuilderError::EmptyRange);

        let mut q = BuilderQuery::new(String::new());
        q.criteria = Some(CriterionNode::Leaf(Criterion {
            aql_path: "not a ]] path".to_owned(),
            negated: false,
            kind: CriterionKind::Exists,
        }));
        assert!(matches!(
            to_aql(&q).unwrap_err(),
            BuilderError::InvalidPath { .. }
        ));

        // O'Neil-style quoting survives escaping and stays parseable.
        let q = BuilderQuery::new("O'Neil template".to_owned());
        let aql = to_aql(&q).unwrap();
        openehr_query::parser::parse_str(&aql).unwrap();
    }
}
