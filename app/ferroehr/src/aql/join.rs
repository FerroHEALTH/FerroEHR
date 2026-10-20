// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic-to-EHR join: a query rooted at a demographic class that
//! contains the `EHR` its party is the subject of, split into one query per
//! storage domain.
//!
//! No openEHR spec governs this: QUERY master03 §FROM gives semantics only to
//! the clinical repository, so the experimental demographic extension and its
//! join are our own design. The form served is
//!
//! ```text
//! FROM PERSON p CONTAINS EHR e [CONTAINS <clinical classes>]
//! FROM PERSON p CONTAINS (<party classes> AND EHR e [CONTAINS <clinical classes>])
//! ```
//!
//! with `ORGANISATION` or `ROLE` as the root too. The party, the linkage map
//! and the clinical content each live behind their own pool and role, so no
//! statement can answer the query; [`split`] cuts it into a party query (the
//! root, its party classes, the `WHERE` conjuncts and columns that read them)
//! and a clinical query (the `EHR` and what it contains, likewise), each an
//! ordinary single-domain AQL statement the engine plans and runs unchanged.
//! The caller joins the two answers on the linkage map: a row is a party row
//! and a clinical row of an EHR that party is the subject of, so a party with
//! no EHR and an EHR with no party answer nothing.
//!
//! The order keys are projected beside each side's columns and each side runs
//! ordered by its own keys, so the joined rows sort by the ranks the database's
//! own comparison assigned ([`JoinPlan::order`]). An aggregate, `DISTINCT`, and
//! any expression reading both sides would need the joined rows inside one
//! statement, and each is refused.

use std::collections::BTreeSet;

use openehr_query::ast::{
    ClassExprOperand, ColumnExpr, ContainsConstraint, ContainsExpr, IdentifiedPath, ObjectPath,
    OrderByExpr, PathPart, SelectClause, SelectExpr, SelectQuery, WhereExpr,
};
use openehr_query::visit::{Visit, walk_identified_path};

use super::error::{AnalysisError, AqlError, AqlFeatureError};
use super::lower::{SERVED_PARTY_ROOTS, lower_limit};

/// The variable the party query binds its root to when the query names none.
const PARTY_VAR: &str = "ferroehr_join_party";
/// The variable the clinical query binds its `EHR` to when the query names
/// none.
const EHR_VAR: &str = "ferroehr_join_ehr";

/// Which of the two queries a column or an order key is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The party query, run on the demographic pool.
    Party,
    /// The clinical query, run on the clinical pool.
    Clinical,
}

/// One column of the joined `RESULT_SET`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinColumn {
    /// The query the value is read from.
    pub side: Side,
    /// The column's index in that query's `RESULT_SET`.
    pub index: usize,
    /// The `RESULT_SET` column name (the `AS` alias, else `#{index}`).
    pub name: String,
    /// The `RESULT_SET` column path: the query's own path text.
    pub path: Option<String>,
}

/// One order key of the joined rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinOrderKey {
    /// The query the key is projected in.
    pub side: Side,
    /// The key's position among that query's own order keys, which is also
    /// its position in that query's `ORDER BY`.
    pub rank: usize,
}

/// A demographic-to-EHR query cut into its two single-domain halves.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinPlan {
    /// The party query: its columns, then the root's `uid/value`, then its
    /// order keys, ordered by those keys.
    pub party: SelectQuery,
    /// The clinical query: its columns, then the `EHR`'s `ehr_id/value`, then
    /// its order keys, ordered by those keys.
    pub clinical: SelectQuery,
    /// The joined `RESULT_SET` columns, in `SELECT` order.
    pub columns: Vec<JoinColumn>,
    /// The column of the party query that holds the root's `uid/value`.
    pub party_uid: usize,
    /// The column of the clinical query that holds the `EHR`'s
    /// `ehr_id/value`.
    pub ehr_id: usize,
    /// The order keys, in `ORDER BY` order.
    pub order: Vec<JoinOrderKey>,
    /// The row limit (`LIMIT`, or the deprecated `TOP`).
    pub limit: Option<i64>,
    /// Whether [`Self::limit`] came from `TOP`.
    pub limit_is_top: bool,
    /// The row offset (`OFFSET`).
    pub offset: Option<i64>,
}

/// Cuts a demographic-to-EHR query into its party and clinical halves, or
/// answers `None` for any query of another shape (which the planner then
/// lowers, or refuses, as one statement).
///
/// # Errors
/// [`AqlFeatureError::DemographicRootPlacement`] when the root contains more
/// than one `EHR`, [`AqlFeatureError::JoinedAggregate`] for an aggregate
/// column, [`AqlFeatureError::JoinedDistinct`] for `SELECT DISTINCT`, and
/// [`AqlFeatureError::JoinedCrossDomainExpression`] for a column, a `WHERE`
/// condition or an order key reading both sides, a variable bound on both
/// sides as [`AnalysisError::DuplicateVariable`], and the `LIMIT`/`TOP`
/// refusals of a single-domain query.
pub fn split(query: &SelectQuery) -> Result<Option<JoinPlan>, AqlError> {
    let Some(cut) = cut_from(&query.from)? else {
        return Ok(None);
    };
    if query.select.distinct {
        return Err(AqlFeatureError::JoinedDistinct.into());
    }
    let mut party_vars = BTreeSet::new();
    bound_variables(&cut.party_from, &mut party_vars);
    let mut clinical_vars = BTreeSet::new();
    bound_variables(&cut.clinical_from, &mut clinical_vars);
    // Each half's planner checks uniqueness within the half; a name bound in
    // both is the duplicate it cannot see (QUERY master03 §Variables/Syntax).
    if let Some(shared) = party_vars.intersection(&clinical_vars).next() {
        return Err(AnalysisError::DuplicateVariable(shared.clone()).into());
    }
    let (limit, offset, limit_is_top) = lower_limit(query)?;
    let sides = Sides {
        party: &party_vars,
        clinical: &clinical_vars,
    };
    let mut halves = [Half::new(cut.party_from), Half::new(cut.clinical_from)];
    let columns = distribute_columns(query, &sides, &mut halves)?;
    distribute_conditions(query, &sides, &mut halves)?;
    let [party, clinical] = &mut halves;
    let party_uid = party.push_column(SelectExpr {
        column: ColumnExpr::Path(attribute_path(&cut.party_var, &["uid", "value"])),
        alias: None,
    });
    let ehr_id = clinical.push_column(SelectExpr {
        column: ColumnExpr::Path(attribute_path(&cut.ehr_var, &["ehr_id", "value"])),
        alias: None,
    });
    let order = distribute_order(query, &sides, &mut halves)?;
    let [party, clinical] = halves;
    Ok(Some(JoinPlan {
        party: party.finish(),
        clinical: clinical.finish(),
        columns,
        party_uid,
        ehr_id,
        order,
        limit,
        limit_is_top,
        offset,
    }))
}

/// The `FROM` clause of a joined query, cut into its halves.
struct Cut {
    /// The root with its party classes.
    party_from: ContainsExpr,
    /// The `EHR` with what it contains.
    clinical_from: ContainsExpr,
    /// The root's variable, bound when the query named none.
    party_var: String,
    /// The `EHR`'s variable, bound when the query named none.
    ehr_var: String,
}

/// Cuts `from` into its halves when it is a served demographic root whose
/// top-level containment holds an `EHR` operand, or answers `None`.
///
/// # Errors
/// [`AqlFeatureError::DemographicRootPlacement`] when the root contains more
/// than one `EHR`.
fn cut_from(from: &ContainsExpr) -> Result<Option<Cut>, AqlError> {
    let ContainsExpr::Contained {
        operand:
            ClassExprOperand::Class {
                rm_type,
                variable,
                predicate,
            },
        contains: Some(constraint),
    } = from
    else {
        return Ok(None);
    };
    if !SERVED_PARTY_ROOTS.contains(&rm_type.as_str()) || constraint.negated {
        return Ok(None);
    }
    let mut leaves = Vec::new();
    flatten_and(&constraint.expr, &mut leaves);
    let (ehr, party_leaves): (Vec<&ContainsExpr>, Vec<&ContainsExpr>) =
        leaves.into_iter().partition(|leaf| is_ehr_operand(leaf));
    let ehr = match ehr.as_slice() {
        [] => return Ok(None),
        [one] => *one,
        _ => return Err(AqlFeatureError::DemographicRootPlacement("EHR".to_owned()).into()),
    };
    let party_var = variable.clone().unwrap_or_else(|| PARTY_VAR.to_owned());
    let party_from = ContainsExpr::Contained {
        operand: ClassExprOperand::Class {
            rm_type: rm_type.clone(),
            variable: Some(party_var.clone()),
            predicate: predicate.clone(),
        },
        contains: and_all(party_leaves.iter().map(|leaf| (*leaf).clone())).map(|expr| {
            Box::new(ContainsConstraint {
                negated: false,
                expr,
            })
        }),
    };
    let (clinical_from, ehr_var) = with_variable(ehr);
    Ok(Some(Cut {
        party_from,
        clinical_from,
        party_var,
        ehr_var,
    }))
}

/// The half `side` names in `halves` (party first).
fn half_of(halves: &mut [Half; 2], side: Side) -> &mut Half {
    let [party, clinical] = halves;
    match side {
        Side::Party => party,
        Side::Clinical => clinical,
    }
}

/// Places each `SELECT` column in the half whose variables it reads.
///
/// # Errors
/// [`AqlFeatureError::JoinedAggregate`] for an aggregate and
/// [`AqlFeatureError::JoinedCrossDomainExpression`] for a column reading both
/// halves.
fn distribute_columns(
    query: &SelectQuery,
    sides: &Sides<'_>,
    halves: &mut [Half; 2],
) -> Result<Vec<JoinColumn>, AqlError> {
    let mut columns = Vec::with_capacity(query.select.columns.len());
    for (i, column) in query.select.columns.iter().enumerate() {
        if matches!(column.column, ColumnExpr::Aggregate(_)) {
            return Err(AqlFeatureError::JoinedAggregate.into());
        }
        let side = sides.of(&roots_of_column(column), || describe_column(column))?;
        let index = half_of(halves, side).push_column(column.clone());
        columns.push(JoinColumn {
            side,
            index,
            name: column.alias.clone().unwrap_or_else(|| format!("#{i}")),
            path: match &column.column {
                ColumnExpr::Path(p) => Some(p.column_path_text()),
                _ => None,
            },
        });
    }
    Ok(columns)
}

/// Places each top-level `WHERE` conjunct in the half whose variables it
/// reads.
///
/// # Errors
/// [`AqlFeatureError::JoinedCrossDomainExpression`] for a conjunct reading
/// both halves.
fn distribute_conditions(
    query: &SelectQuery,
    sides: &Sides<'_>,
    halves: &mut [Half; 2],
) -> Result<(), AqlError> {
    let Some(filter) = &query.where_ else {
        return Ok(());
    };
    let mut conjuncts = Vec::new();
    flatten_where(filter, &mut conjuncts);
    for conjunct in conjuncts {
        let mut roots = Roots::default();
        roots.visit_where_expr(conjunct);
        let side = sides.of(&roots.0, || "a WHERE condition".to_owned())?;
        half_of(halves, side).push_condition(conjunct.clone());
    }
    Ok(())
}

/// Places each order key in the half whose variable it reads.
///
/// # Errors
/// [`AqlFeatureError::JoinedCrossDomainExpression`] for a key reading both
/// halves.
fn distribute_order(
    query: &SelectQuery,
    sides: &Sides<'_>,
    halves: &mut [Half; 2],
) -> Result<Vec<JoinOrderKey>, AqlError> {
    let mut order = Vec::with_capacity(query.order_by.len());
    for key in &query.order_by {
        let mut roots = Roots::default();
        roots.visit_identified_path(&key.path);
        let side = sides.of(&roots.0, || format!("ORDER BY {}", key.path.root))?;
        let rank = half_of(halves, side).push_order(key);
        order.push(JoinOrderKey { side, rank });
    }
    Ok(order)
}

/// The variables each half binds, case-folded (QUERY master03 §Variables:
/// names are not case-sensitive).
struct Sides<'a> {
    party: &'a BTreeSet<String>,
    clinical: &'a BTreeSet<String>,
}

impl Sides<'_> {
    /// The half every root in `roots` belongs to; an expression reading no
    /// variable (a literal, `CURRENT_DATE()`) is evaluated by the party query.
    ///
    /// A root neither half binds is left to the party half, whose planner
    /// refuses it as an unknown variable.
    ///
    /// # Errors
    /// [`AqlFeatureError::JoinedCrossDomainExpression`] when `roots` names a
    /// variable of each half.
    fn of(
        &self,
        roots: &BTreeSet<String>,
        describe: impl FnOnce() -> String,
    ) -> Result<Side, AqlError> {
        let clinical = roots.iter().any(|r| self.clinical.contains(r));
        let party = roots
            .iter()
            .any(|r| self.party.contains(r) || !self.clinical.contains(r));
        match (party, clinical) {
            (true, true) => Err(AqlFeatureError::JoinedCrossDomainExpression(describe()).into()),
            (false, true) => Ok(Side::Clinical),
            _ => Ok(Side::Party),
        }
    }
}

/// One half under construction.
struct Half {
    from: ContainsExpr,
    columns: Vec<SelectExpr>,
    conditions: Vec<WhereExpr>,
    order: Vec<OrderByExpr>,
    /// The order-key columns, appended after every other column.
    keys: Vec<SelectExpr>,
}

impl Half {
    fn new(from: ContainsExpr) -> Self {
        Self {
            from,
            columns: Vec::new(),
            conditions: Vec::new(),
            order: Vec::new(),
            keys: Vec::new(),
        }
    }

    /// Appends a column and returns its index.
    fn push_column(&mut self, column: SelectExpr) -> usize {
        self.columns.push(column);
        self.columns.len() - 1
    }

    fn push_condition(&mut self, condition: WhereExpr) {
        self.conditions.push(condition);
    }

    /// Appends an order key and returns its rank among this half's keys.
    /// [`Self::finish`] places the key columns after every other column, in
    /// rank order.
    fn push_order(&mut self, key: &OrderByExpr) -> usize {
        self.order.push(key.clone());
        self.keys.push(SelectExpr {
            column: ColumnExpr::Path(key.path.clone()),
            alias: None,
        });
        self.order.len() - 1
    }

    fn finish(self) -> SelectQuery {
        let mut columns = self.columns;
        columns.extend(self.keys);
        SelectQuery {
            select: SelectClause {
                distinct: false,
                top: None,
                columns,
            },
            from: self.from,
            where_: and_where(self.conditions),
            order_by: self.order,
            limit: None,
        }
    }
}

impl JoinPlan {
    /// The query of `side`.
    #[must_use]
    pub fn half(&self, side: Side) -> &SelectQuery {
        match side {
            Side::Party => &self.party,
            Side::Clinical => &self.clinical,
        }
    }

    /// The `RESULT_SET` column index of the `rank`-th order key of the query
    /// of `side`: the key columns follow every other column of that query.
    #[must_use]
    pub fn key_column(&self, side: Side, rank: usize) -> usize {
        let half = self.half(side);
        half.select.columns.len() - half.order_by.len() + rank
    }
}

/// Every operand of the top-level `AND` chain of `expr`.
fn flatten_and<'a>(expr: &'a ContainsExpr, out: &mut Vec<&'a ContainsExpr>) {
    match expr {
        ContainsExpr::And(a, b) => {
            flatten_and(a, out);
            flatten_and(b, out);
        }
        other => out.push(other),
    }
}

/// Every conjunct of the top-level `AND` chain of `expr`.
fn flatten_where<'a>(expr: &'a WhereExpr, out: &mut Vec<&'a WhereExpr>) {
    match expr {
        WhereExpr::And(a, b) => {
            flatten_where(a, out);
            flatten_where(b, out);
        }
        other => out.push(other),
    }
}

/// `exprs` joined by `AND`, or `None` when empty.
fn and_all(exprs: impl Iterator<Item = ContainsExpr>) -> Option<ContainsExpr> {
    exprs.reduce(|a, b| ContainsExpr::And(Box::new(a), Box::new(b)))
}

/// `conditions` joined by `AND`, or `None` when empty.
fn and_where(conditions: Vec<WhereExpr>) -> Option<WhereExpr> {
    conditions
        .into_iter()
        .reduce(|a, b| WhereExpr::And(Box::new(a), Box::new(b)))
}

/// Whether `expr` is an `EHR` class operand, with or without containment.
fn is_ehr_operand(expr: &ContainsExpr) -> bool {
    matches!(
        expr,
        ContainsExpr::Contained {
            operand: ClassExprOperand::Class { rm_type, .. },
            ..
        } if rm_type == "EHR"
    )
}

/// The `EHR` operand with a variable bound, and that variable.
fn with_variable(ehr: &ContainsExpr) -> (ContainsExpr, String) {
    let mut ehr = ehr.clone();
    let mut bound = EHR_VAR.to_owned();
    if let ContainsExpr::Contained {
        operand: ClassExprOperand::Class { variable, .. },
        ..
    } = &mut ehr
    {
        match variable {
            Some(v) => bound.clone_from(v),
            None => *variable = Some(bound.clone()),
        }
    }
    (ehr, bound)
}

/// Every variable bound in `expr`, case-folded.
fn bound_variables(expr: &ContainsExpr, out: &mut BTreeSet<String>) {
    match expr {
        ContainsExpr::And(a, b) | ContainsExpr::Or(a, b) => {
            bound_variables(a, out);
            bound_variables(b, out);
        }
        ContainsExpr::Contained { operand, contains } => {
            let variable = match operand {
                ClassExprOperand::Class { variable, .. }
                | ClassExprOperand::Version { variable, .. } => variable,
            };
            if let Some(v) = variable {
                out.insert(v.to_ascii_lowercase());
            }
            if let Some(c) = contains {
                bound_variables(&c.expr, out);
            }
        }
    }
}

/// `var/<attributes…>` as an identified path.
fn attribute_path(var: &str, attributes: &[&str]) -> IdentifiedPath {
    IdentifiedPath::new(
        var.to_owned(),
        None,
        Some(ObjectPath {
            parts: attributes
                .iter()
                .map(|a| PathPart {
                    name: (*a).to_owned(),
                    predicate: None,
                })
                .collect(),
        }),
    )
}

/// The variables a `SELECT` column reads.
fn roots_of_column(column: &SelectExpr) -> BTreeSet<String> {
    let mut roots = Roots::default();
    roots.visit_select_expr(column);
    roots.0
}

/// A column as a refusal names it: its alias, else its path text.
fn describe_column(column: &SelectExpr) -> String {
    match (&column.alias, &column.column) {
        (Some(alias), _) => format!("SELECT column {alias}"),
        (None, ColumnExpr::Path(p)) => format!("SELECT {}{}", p.root, p.column_path_text()),
        (None, _) => "a SELECT column".to_owned(),
    }
}

/// The case-folded root variables of every identified path visited.
#[derive(Default)]
struct Roots(BTreeSet<String>);

impl<'ast> Visit<'ast> for Roots {
    fn visit_identified_path(&mut self, node: &'ast IdentifiedPath) {
        self.0.insert(node.root.to_ascii_lowercase());
        walk_identified_path(self, node);
    }
}
