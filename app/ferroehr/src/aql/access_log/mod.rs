// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL over the access log (#5131): `ACCESS_LOG_ENTRY` as a top-level class
//! expression, outside every `EHR` containment.
//!
//! AQL class expressions name an RM class (QUERY `master03-syntax.adoc` §Class
//! expressions), and the draft access log defines `ACCESS_LOG_ENTRY` as one,
//! so `SELECT l/accessor/party/external_ref/id/value FROM ACCESS_LOG_ENTRY l`
//! is ordinary AQL. The log is kept apart from the EHR content (BASE
//! `architecture_overview/master07-security.adoc` §Security Policy), so the
//! class stands alone in its `FROM`: it never appears inside an `EHR`
//! containment and contains no class, and no query returns log entries and
//! clinical data in one result. Who may query it, and what they see, is the
//! log's own access control, never a clinical query right.
//!
//! The query reads the stored entries: `WHERE`, `ORDER BY`, `LIMIT` and the
//! REST paging run in the statement [`sql`] builds, and each `SELECT` column is
//! read out of the view of the entry its reader is served. No openEHR spec
//! governs AQL over an access log: our own design, proposed upstream with the
//! draft model.

pub mod path;
pub mod sql;

use openehr_query::ast::{
    ClassExprOperand, ColumnExpr, CompareOperand, ContainsExpr, IdentifiedExpr, LikeOperand,
    MatchesOperand, OrderByExpr, Primitive, SelectQuery, SortOrder, Terminal, ValueListItem,
    WhereExpr,
};
use openehr_query::lexer::CompOp;
use thiserror::Error;

use super::analyze::param_name;
use super::error::AqlError;
use super::ir::{ParamValue, Params};
use super::lower::lower_limit;
use path::{ACCESS_LOG_ENTRY, LogPath, resolve};

/// The ways an AQL query over the access log is refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LogQueryError {
    /// `ACCESS_LOG_ENTRY` inside an `EHR` containment, or an `EHR` inside it.
    #[error(
        "ACCESS_LOG_ENTRY never appears inside an EHR containment and never contains an EHR: \
         the access log stays outside the EHR, so no query returns log entries and clinical \
         data in one result (FerroEHR access-log extension; QUERY §FROM)"
    )]
    InsideEhr,
    /// `ACCESS_LOG_ENTRY` beside, under or over any other class.
    #[error(
        "ACCESS_LOG_ENTRY is a top-level class expression only: it is the one class of its \
         FROM clause and contains no other class (FerroEHR access-log extension; QUERY §FROM)"
    )]
    NotAlone,
    /// A predicate on the class expression.
    #[error(
        "ACCESS_LOG_ENTRY takes no predicate in FROM; constrain the entries in WHERE \
         (FerroEHR access-log extension; QUERY §FROM)"
    )]
    ClassPredicate,
    /// A path over a variable FROM does not bind.
    #[error("unknown variable `{0}` (not bound in the FROM clause)")]
    UnknownVariable(String),
    /// A step no class the path reaches declares.
    #[error("attribute `{attribute}` is not defined on `{class}` (draft access log model)")]
    UnknownAttribute {
        /// The step.
        attribute: String,
        /// The class it was looked up on.
        class: String,
    },
    /// A predicate on a step of a path.
    #[error(
        "`{0}`: a query over the access log takes no path predicate; constrain the entries in \
         WHERE (FerroEHR access-log extension; QUERY §Identified paths)"
    )]
    PathPredicate(String),
    /// The draft model table lacks a class the entry's attributes name.
    #[error("the draft access log model is incomplete")]
    NoModel,
    /// A construct a query over the access log does not take.
    #[error("a query over the access log takes no {0} (FerroEHR access-log extension)")]
    Unsupported(&'static str),
    /// A comparison of a path that reaches an object.
    #[error("`{0}` reaches an object, which compares with nothing; compare a primitive under it")]
    NotComparable(String),
    /// A value of another type than the path's.
    #[error("`{path}` compares with {expected} only")]
    TypeMismatch {
        /// The path.
        path: String,
        /// The type it compares with.
        expected: &'static str,
    },
    /// An `ORDER BY` key that reaches a list.
    #[error("`{0}` reaches a list of values, which orders nothing; order by a single value")]
    MultipleOrder(String),
    /// A `$parameter` the request does not bind.
    #[error("parameter `${0}` is referenced but not supplied in query_parameters")]
    UnboundParameter(String),
}

/// A value a condition compares with.
#[derive(Debug, Clone, PartialEq)]
pub enum LogValue {
    /// A literal of the query.
    Literal(ParamValue),
    /// A `$parameter`, by name.
    Parameter(String),
}

/// A `WHERE` condition over the entries.
#[derive(Debug, Clone, PartialEq)]
pub enum LogCondition {
    /// A condition semantic analysis resolved to a constant.
    Constant(bool),
    /// `NOT`.
    Not(Box<LogCondition>),
    /// `AND`.
    And(Box<LogCondition>, Box<LogCondition>),
    /// `OR`.
    Or(Box<LogCondition>, Box<LogCondition>),
    /// `EXISTS path`.
    Exists(LogPath),
    /// `path op value`.
    Compare {
        /// The path.
        path: LogPath,
        /// The operator.
        op: CompOp,
        /// The value.
        value: LogValue,
    },
    /// `path LIKE pattern`.
    Like {
        /// The path.
        path: LogPath,
        /// The AQL pattern.
        pattern: LogValue,
    },
    /// `path matches {values}`.
    Matches {
        /// The path.
        path: LogPath,
        /// The values, any of which matches.
        values: Vec<LogValue>,
    },
}

/// A `SELECT` column.
#[derive(Debug, Clone, PartialEq)]
pub struct LogColumn {
    /// The `AS` alias, else `#{index}`.
    pub name: String,
    /// What the column reads.
    pub value: LogCell,
}

/// What a column reads.
#[derive(Debug, Clone, PartialEq)]
pub enum LogCell {
    /// A path of the entry.
    Path(LogPath),
    /// A literal, the same in every row.
    Literal(Primitive),
}

/// A planned AQL query over the access log.
#[derive(Debug, Clone, PartialEq)]
pub struct LogPlan {
    /// The `SELECT` columns, in order.
    pub columns: Vec<LogColumn>,
    /// The `WHERE` condition.
    pub filter: Option<LogCondition>,
    /// The `ORDER BY` keys, each descending when `true`.
    pub order: Vec<(LogPath, bool)>,
    /// The AQL `LIMIT` or `TOP`.
    pub limit: Option<i64>,
    /// The AQL `OFFSET`.
    pub offset: Option<i64>,
    /// Whether [`Self::limit`] came from the deprecated `TOP`.
    pub limit_is_top: bool,
    /// Every `$parameter` the query names.
    pub params: Vec<String>,
}

impl LogPlan {
    /// `Ok` when `params` binds every `$parameter` the query names.
    ///
    /// # Errors
    /// [`LogQueryError::UnboundParameter`] naming the first it does not.
    pub fn check_params(&self, params: &Params) -> Result<(), AqlError> {
        match self.params.iter().find(|name| params.get(name).is_none()) {
            Some(name) => Err(feature(LogQueryError::UnboundParameter(name.clone()))),
            None => Ok(()),
        }
    }
}

/// The plan of `query` when it reads the access log, `None` when it names no
/// `ACCESS_LOG_ENTRY`.
///
/// # Errors
/// [`AqlError::Feature`] carrying the [`LogQueryError`] that refuses it: a
/// placement of the class other than alone at the top of `FROM`, an unknown
/// path, or a construct the log query does not take.
pub fn plan(query: &SelectQuery) -> Result<Option<LogPlan>, AqlError> {
    let mut classes = Vec::new();
    classes_of(&query.from, &mut classes);
    if !classes.contains(&ACCESS_LOG_ENTRY) {
        return Ok(None);
    }
    let variable = alone(&query.from, &classes)?;
    if query.select.distinct {
        return Err(feature(LogQueryError::Unsupported("SELECT DISTINCT")));
    }
    let mut planner = Planner {
        variable,
        params: Vec::new(),
    };
    let columns = query
        .select
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| planner.column(index, &column.column, column.alias.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    let filter = query
        .where_
        .as_ref()
        .map(|condition| planner.condition(condition))
        .transpose()?;
    let order = query
        .order_by
        .iter()
        .map(|key| planner.order(key))
        .collect::<Result<Vec<_>, _>>()?;
    let (limit, offset, limit_is_top) = lower_limit(query)?;
    Ok(Some(LogPlan {
        columns,
        filter,
        order,
        limit,
        offset,
        limit_is_top,
        params: planner.params,
    }))
}

/// `error` as the planner error it surfaces as.
fn feature(error: LogQueryError) -> AqlError {
    AqlError::Feature(error.into())
}

/// Every class operand `from` names, `VERSION` as `"VERSION"`.
fn classes_of<'q>(from: &'q ContainsExpr, into: &mut Vec<&'q str>) {
    match from {
        ContainsExpr::Contained { operand, contains } => {
            into.push(match operand {
                ClassExprOperand::Class { rm_type, .. } => rm_type.as_str(),
                ClassExprOperand::Version { .. } => "VERSION",
            });
            if let Some(contained) = contains {
                classes_of(&contained.expr, into);
            }
        }
        ContainsExpr::And(left, right) | ContainsExpr::Or(left, right) => {
            classes_of(left, into);
            classes_of(right, into);
        }
    }
}

/// The variable of `ACCESS_LOG_ENTRY` when it stands alone in `from`, whose
/// classes are `classes`.
///
/// # Errors
/// [`LogQueryError::InsideEhr`] when `from` also names `EHR`,
/// [`LogQueryError::NotAlone`] for any other company, and
/// [`LogQueryError::ClassPredicate`] for a predicate on the class.
fn alone(from: &ContainsExpr, classes: &[&str]) -> Result<String, AqlError> {
    if classes.contains(&"EHR") {
        return Err(feature(LogQueryError::InsideEhr));
    }
    let ContainsExpr::Contained {
        operand:
            ClassExprOperand::Class {
                variable,
                predicate,
                ..
            },
        contains: None,
    } = from
    else {
        return Err(feature(LogQueryError::NotAlone));
    };
    if predicate.is_some() {
        return Err(feature(LogQueryError::ClassPredicate));
    }
    // NOTE: QUERY §FROM, a class expression may leave its variable out; no path can then
    // name it, so only literal columns can be selected.
    Ok(variable.clone().unwrap_or_default())
}

/// The lowering state: the class variable and the parameters met.
struct Planner {
    /// The variable `ACCESS_LOG_ENTRY` is bound to.
    variable: String,
    /// Every `$parameter` met, each once.
    params: Vec<String>,
}

impl Planner {
    /// The `SELECT` column `column` at `index`.
    fn column(
        &mut self,
        index: usize,
        column: &ColumnExpr,
        alias: Option<&String>,
    ) -> Result<LogColumn, AqlError> {
        let value = match column {
            ColumnExpr::Path(path) => LogCell::Path(self.path(path)?),
            ColumnExpr::Primitive(primitive) => LogCell::Literal(primitive.clone()),
            ColumnExpr::Aggregate(_) => {
                return Err(feature(LogQueryError::Unsupported("aggregate function")));
            }
            ColumnExpr::Function(_) => {
                return Err(feature(LogQueryError::Unsupported("function call")));
            }
        };
        Ok(LogColumn {
            name: alias.cloned().unwrap_or_else(|| format!("#{index}")),
            value,
        })
    }

    /// `path`, typed.
    fn path(&self, path: &openehr_query::ast::IdentifiedPath) -> Result<LogPath, AqlError> {
        resolve(path, &self.variable).map_err(feature)
    }

    /// The `WHERE` condition `condition`.
    fn condition(&mut self, condition: &WhereExpr) -> Result<LogCondition, AqlError> {
        Ok(match condition {
            WhereExpr::Not(inner) => LogCondition::Not(Box::new(self.condition(inner)?)),
            WhereExpr::And(left, right) => LogCondition::And(
                Box::new(self.condition(left)?),
                Box::new(self.condition(right)?),
            ),
            WhereExpr::Or(left, right) => LogCondition::Or(
                Box::new(self.condition(left)?),
                Box::new(self.condition(right)?),
            ),
            WhereExpr::Identified(leaf, _span) => self.leaf(leaf)?,
        })
    }

    /// One `WHERE` leaf.
    fn leaf(&mut self, leaf: &IdentifiedExpr) -> Result<LogCondition, AqlError> {
        Ok(match leaf {
            IdentifiedExpr::Resolved(constant) => LogCondition::Constant(*constant),
            IdentifiedExpr::Exists(path) => LogCondition::Exists(self.path(path)?),
            IdentifiedExpr::Compare { lhs, op, rhs } => {
                let CompareOperand::Path(path) = lhs else {
                    return Err(feature(LogQueryError::Unsupported("function call")));
                };
                let path = self.comparable(path)?;
                let value = match rhs {
                    Terminal::Primitive(primitive) => literal(primitive)?,
                    Terminal::Parameter(name) => self.parameter(name),
                    Terminal::Path(_) => {
                        return Err(feature(LogQueryError::Unsupported(
                            "comparison of two paths",
                        )));
                    }
                    Terminal::Function(_) => {
                        return Err(feature(LogQueryError::Unsupported("function call")));
                    }
                };
                LogCondition::Compare {
                    path,
                    op: *op,
                    value,
                }
            }
            IdentifiedExpr::Like { path, operand } => LogCondition::Like {
                path: self.comparable(path)?,
                pattern: match operand {
                    LikeOperand::String(pattern) => {
                        LogValue::Literal(ParamValue::Str(pattern.clone()))
                    }
                    LikeOperand::Parameter(name) => self.parameter(name),
                },
            },
            IdentifiedExpr::Matches { path, operand } => {
                let path = self.comparable(path)?;
                let MatchesOperand::ValueList(items) = operand else {
                    return Err(feature(LogQueryError::Unsupported(
                        "matches operand other than a value list",
                    )));
                };
                let values = items
                    .iter()
                    .map(|item| match item {
                        ValueListItem::Primitive(primitive) => literal(primitive),
                        ValueListItem::Parameter(name) => Ok(self.parameter(name)),
                        ValueListItem::Terminology(_) => {
                            Err(feature(LogQueryError::Unsupported("TERMINOLOGY()")))
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                LogCondition::Matches { path, values }
            }
        })
    }

    /// `path`, typed, when it reaches a primitive a condition can test.
    fn comparable(&self, path: &openehr_query::ast::IdentifiedPath) -> Result<LogPath, AqlError> {
        let path = self.path(path)?;
        if path.leaf == path::Leaf::Object {
            return Err(feature(LogQueryError::NotComparable(path.text)));
        }
        Ok(path)
    }

    /// The `$parameter` `raw`, recorded.
    fn parameter(&mut self, raw: &str) -> LogValue {
        let name = param_name(raw);
        if !self.params.contains(&name) {
            self.params.push(name.clone());
        }
        LogValue::Parameter(name)
    }

    /// The `ORDER BY` key `key`.
    fn order(&self, key: &OrderByExpr) -> Result<(LogPath, bool), AqlError> {
        let path = self.comparable(&key.path)?;
        if path.multiple {
            return Err(feature(LogQueryError::MultipleOrder(path.text)));
        }
        Ok((path, key.order == Some(SortOrder::Descending)))
    }
}

/// The literal `primitive` as a value to compare with.
fn literal(primitive: &Primitive) -> Result<LogValue, AqlError> {
    Ok(LogValue::Literal(match primitive {
        Primitive::String(text) => ParamValue::Str(text.clone()),
        Primitive::Integer(number) => ParamValue::Int(*number),
        Primitive::Real(number) => ParamValue::Real(*number),
        Primitive::Boolean(flag) => ParamValue::Bool(*flag),
        Primitive::Null => {
            return Err(feature(LogQueryError::Unsupported(
                "comparison with NULL; test presence with EXISTS",
            )));
        }
    }))
}

#[cfg(test)]
mod tests {
    use openehr_query::parser::parse_str;

    use super::*;
    use crate::aql::error::AqlFeatureError;

    fn planned(aql: &str) -> Result<Option<LogPlan>, AqlError> {
        plan(&parse_str(aql).expect("parses"))
    }

    fn refusal(aql: &str) -> LogQueryError {
        match planned(aql) {
            Err(AqlError::Feature(AqlFeatureError::AccessLog(error))) => error,
            other => panic!("{aql}: expected a log refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_clinical_query_is_no_log_query() {
        let plan = planned("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c").expect("plans");
        assert_eq!(plan, None);
    }

    #[test]
    fn the_log_class_stands_alone() {
        assert_eq!(
            refusal("SELECT l FROM EHR e CONTAINS ACCESS_LOG_ENTRY l"),
            LogQueryError::InsideEhr
        );
        assert_eq!(
            refusal("SELECT l FROM ACCESS_LOG_ENTRY l CONTAINS EHR e"),
            LogQueryError::InsideEhr
        );
        assert_eq!(
            refusal("SELECT l FROM ACCESS_LOG_ENTRY l AND EHR e"),
            LogQueryError::InsideEhr
        );
        assert_eq!(
            refusal("SELECT l FROM ACCESS_LOG_ENTRY l CONTAINS COMPOSITION c"),
            LogQueryError::NotAlone
        );
        assert_eq!(
            refusal("SELECT l FROM PERSON p AND ACCESS_LOG_ENTRY l"),
            LogQueryError::NotAlone
        );
    }

    #[test]
    fn a_log_query_plans_its_clauses() {
        let plan = planned(
            "SELECT l/uid/value AS uid, l/items/version_id/value FROM ACCESS_LOG_ENTRY l \
             WHERE l/ehr_id/value = $ehr AND l/time_recorded/value >= '2026-11-01T00:00:00Z' \
             AND NOT EXISTS l/query ORDER BY l/time_recorded/value DESC LIMIT 10 OFFSET 5",
        )
        .expect("plans")
        .expect("a log query");
        assert_eq!(plan.columns.len(), 2);
        assert_eq!(plan.columns[0].name, "uid");
        assert_eq!(plan.columns[1].name, "#1");
        assert_eq!(plan.params, vec!["ehr".to_owned()]);
        assert_eq!((plan.limit, plan.offset), (Some(10), Some(5)));
        assert!(plan.order[0].1, "descending");
        assert!(matches!(plan.filter, Some(LogCondition::And(..))));
        assert!(plan.check_params(&Params::default()).is_err());
    }

    #[test]
    fn constructs_outside_the_log_query_are_refused() {
        assert!(matches!(
            refusal("SELECT COUNT(*) FROM ACCESS_LOG_ENTRY l"),
            LogQueryError::Unsupported(_)
        ));
        assert!(matches!(
            refusal("SELECT DISTINCT l/action/value FROM ACCESS_LOG_ENTRY l"),
            LogQueryError::Unsupported(_)
        ));
        assert!(matches!(
            refusal("SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/accessor = 'x'"),
            LogQueryError::NotComparable(_)
        ));
        assert!(matches!(
            refusal("SELECT l FROM ACCESS_LOG_ENTRY l ORDER BY l/items/rm_type"),
            LogQueryError::MultipleOrder(_)
        ));
        assert_eq!(
            refusal("SELECT l FROM ACCESS_LOG_ENTRY l[uid/value='x']"),
            LogQueryError::ClassPredicate
        );
    }
}
