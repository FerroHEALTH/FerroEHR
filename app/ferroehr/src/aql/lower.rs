// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AST → typed IR lowering (our own engine). Covers the full accepted feature
//! envelope; every rejected
//! construct maps to a precise [`AqlFeatureError`].
//!
//! The FROM clause is lowered first (building the [`Source`] list, the
//! [`ContainsTree`], and the variable [`Bindings`]); SELECT / WHERE / ORDER BY
//! are then analysed against those bindings. Parameter presence is validated by
//! [`super::plan`] after the whole IR is built.

use openehr_query::ast::{
    AggregateCall, ClassExprOperand, ColumnExpr, CompareOperand, ContainsExpr, FunctionCall,
    IdentifiedExpr, LikeOperand, MatchesOperand, OrderByExpr, SelectQuery, SortOrder, StatFunc,
    Terminal, TopDirection, ValueListItem, VersionPredicate, WhereExpr,
};
use openehr_rm::v1_2::model;

use super::analyze::{
    Binding, BindingKind, Bindings, analyze_path, param_name, resolve_ehr_predicate,
    resolve_node_predicate, resolve_version_predicate, typed_lit,
};
use super::error::{AnalysisError, AqlError, AqlFeatureError};
use super::ir::{
    AggFunc, Bind, Coercion, Contained, ContainsTree, EhrPredicate, EhrSource, Expr, LikePattern,
    Link, Operand, OrderKey, PathTarget, QueryIr, RmSource, ScalarFn, SelectColumn, SelectValue,
    Source, SourceId, TypeSet, TypedLit, VersionScope, VersionSource,
};

/// Lowers one parsed [`SelectQuery`] into a [`QueryIr`] (without parameter-
/// presence validation, which [`super::plan`] performs).
pub(crate) fn lower(
    query: &SelectQuery,
    profile: crate::config::profile::SpecProfile,
) -> Result<QueryIr, AqlError> {
    let mut planner = Planner::for_profile(profile);
    let contains = planner.lower_from(&query.from, None)?;

    let select = query
        .select
        .columns
        .iter()
        .map(|c| planner.lower_select_column(c))
        .collect::<Result<Vec<_>, _>>()?;
    refuse_mixed_aggregates(&select)?;

    let filter = query
        .where_
        .as_ref()
        .map(|w| planner.lower_where(w))
        .transpose()?;

    let order_by = query
        .order_by
        .iter()
        .map(|o| planner.lower_order_by(o))
        .collect::<Result<Vec<_>, _>>()?;

    let (limit, offset, limit_is_top) = lower_limit(query)?;

    Ok(QueryIr {
        sources: planner.sources,
        contains,
        filter,
        select,
        order_by,
        distinct: query.select.distinct,
        limit,
        limit_is_top,
        offset,
        params: Vec::new(), // filled by `plan`
    })
}

/// Refuse a SELECT that mixes an aggregate with a plain projection.
///
/// Literals may sit beside aggregates (they are constant per row set); an
/// identified path or a scalar function beside an aggregate has no defined
/// value without grouping, which AQL 1.1 does not define.
fn refuse_mixed_aggregates(select: &[SelectColumn]) -> Result<(), AqlError> {
    let has_aggregate = select
        .iter()
        .any(|c| matches!(c.value, SelectValue::Aggregate { .. }));
    let has_plain = select
        .iter()
        .any(|c| matches!(c.value, SelectValue::Path(_) | SelectValue::Function { .. }));
    if has_aggregate && has_plain {
        return Err(AqlFeatureError::MixedAggregateProjection.into());
    }
    Ok(())
}

/// The FROM-clause lowering state: the growing source list and variable
/// bindings.
#[derive(Debug, Default)]
struct Planner {
    sources: Vec<Source>,
    bindings: Bindings,
}

impl Planner {
    fn for_profile(profile: crate::config::profile::SpecProfile) -> Self {
        let mut p = Self::default();
        p.bindings.profile = profile;
        p
    }
}

impl Planner {
    fn next_id(&self) -> SourceId {
        SourceId(self.sources.len())
    }

    // ── FROM ────────────────────────────────────────────────────────────────

    fn lower_from(
        &mut self,
        expr: &ContainsExpr,
        inherited: Option<&VersionScope>,
    ) -> Result<ContainsTree, AqlError> {
        match expr {
            ContainsExpr::And(a, b) => Ok(ContainsTree::And(
                Box::new(self.lower_from(a, inherited)?),
                Box::new(self.lower_from(b, inherited)?),
            )),
            ContainsExpr::Or(a, b) => Ok(ContainsTree::Or(
                Box::new(self.lower_from(a, inherited)?),
                Box::new(self.lower_from(b, inherited)?),
            )),
            ContainsExpr::Contained { operand, contains } => {
                let (source, child_scope) = self.lower_operand(operand, inherited)?;
                let contained = contains
                    .as_ref()
                    .map(|cc| -> Result<_, AqlError> {
                        let link = if cc.negated {
                            Link::NotContains
                        } else {
                            Link::Contains
                        };
                        let tree = self.lower_from(&cc.expr, child_scope.as_ref())?;
                        Ok(Box::new(Contained { link, tree }))
                    })
                    .transpose()?;
                Ok(ContainsTree::Operand { source, contained })
            }
        }
    }

    /// Lower one class/version operand, returning its [`SourceId`] and the
    /// version scope to propagate to its contained children.
    fn lower_operand(
        &mut self,
        operand: &ClassExprOperand,
        inherited: Option<&VersionScope>,
    ) -> Result<(SourceId, Option<VersionScope>), AqlError> {
        match operand {
            ClassExprOperand::Class {
                rm_type,
                variable,
                predicate,
            } => {
                let id = self.next_id();
                if rm_type == "EHR" {
                    self.lower_ehr_operand(id, variable.as_deref(), predicate.as_ref())?;
                } else {
                    self.lower_rm_operand(
                        id,
                        rm_type,
                        variable.as_deref(),
                        predicate.as_ref(),
                        inherited,
                    )?;
                }
                Ok((id, inherited.cloned()))
            }
            ClassExprOperand::Version {
                variable,
                predicate,
            } => {
                let id = self.next_id();
                let scope = match predicate {
                    None | Some(VersionPredicate::Latest) => VersionScope::Latest,
                    Some(VersionPredicate::All) => VersionScope::All,
                    Some(VersionPredicate::Standard(sp)) => {
                        VersionScope::Predicate(resolve_version_predicate(sp)?)
                    }
                };
                self.sources.push(Source::Version(VersionSource {
                    id,
                    var: variable.clone(),
                    scope: scope.clone(),
                }));
                if let Some(v) = variable {
                    self.bind(v, id, BindingKind::Version)?;
                }
                Ok((id, Some(scope)))
            }
        }
    }

    /// Registers an `EHR` operand as an EHR source and binds its variable.
    ///
    /// # Errors
    /// The EHR-predicate lowering rejections, and a duplicate variable
    /// binding.
    fn lower_ehr_operand(
        &mut self,
        id: SourceId,
        variable: Option<&str>,
        predicate: Option<&openehr_query::ast::PathPredicate>,
    ) -> Result<(), AqlError> {
        let predicates = predicate
            .map(Self::lower_ehr_predicate)
            .transpose()?
            .into_iter()
            .flatten()
            .collect();
        self.sources.push(Source::Ehr(EhrSource {
            id,
            var: variable.map(str::to_owned),
            predicates,
        }));
        if let Some(v) = variable {
            self.bind(v, id, BindingKind::Ehr)?;
        }
        Ok(())
    }

    /// Registers an RM structure/VO-class operand as an RM source and binds
    /// its variable.
    ///
    /// The class must be addressable in the node store: at least one concrete
    /// descendant is a structure root.
    ///
    /// # Errors
    /// [`AnalysisError::UnknownClass`] for a class the RM model does not
    /// declare, [`AnalysisError::ClassNotInProfile`] for one outside the
    /// active `spec_profile`'s generation set,
    /// [`AqlFeatureError::UnsupportedSourceClass`] for a class no concrete
    /// descendant of which is a structure root, plus the node-predicate
    /// rejections and a duplicate variable binding.
    fn lower_rm_operand(
        &mut self,
        id: SourceId,
        rm_type: &str,
        variable: Option<&str>,
        predicate: Option<&openehr_query::ast::PathPredicate>,
        inherited: Option<&VersionScope>,
    ) -> Result<(), AqlError> {
        let class =
            model::class(rm_type).ok_or_else(|| AnalysisError::UnknownClass(rm_type.to_owned()))?;
        if !crate::aql::analyze::profile_defines_class(self.bindings.profile, rm_type) {
            return Err(AnalysisError::ClassNotInProfile {
                class: rm_type.to_owned(),
                profile: self.bindings.profile.as_str(),
                generation: self.bindings.profile.rm().spec_version(),
            }
            .into());
        }
        let concrete = TypeSet::new(class.descendants.iter().map(|s| (*s).to_owned()).collect());
        if !concrete.names().iter().any(|t| model::is_structure_root(t)) {
            return Err(AqlFeatureError::UnsupportedSourceClass(rm_type.to_owned()).into());
        }
        let constraint = predicate
            .map(resolve_node_predicate)
            .transpose()?
            .unwrap_or_default();
        let scope = inherited.cloned().unwrap_or(VersionScope::Latest);
        self.sources.push(Source::Rm(RmSource {
            id,
            var: variable.map(str::to_owned),
            rm_type: concrete.clone(),
            archetype: constraint.archetype,
            name: constraint.name,
            standard: constraint.standard,
            scope,
        }));
        if let Some(v) = variable {
            self.bind(v, id, BindingKind::Rm(concrete))?;
        }
        Ok(())
    }

    /// Resolve an EHR class predicate into standard EHR-field predicates. EHR
    /// predicates are standard predicates (`ehr_id/value=$id`); a node-predicate
    /// `AND`-tree of them is also accepted.
    fn lower_ehr_predicate(
        pred: &openehr_query::ast::PathPredicate,
    ) -> Result<Vec<EhrPredicate>, AqlError> {
        use openehr_query::ast::{NodePredicate, PathPredicate};
        match pred {
            PathPredicate::Standard(sp) => {
                let (field, op, value) = resolve_ehr_predicate(sp)?;
                Ok(vec![EhrPredicate { field, op, value }])
            }
            PathPredicate::Node(np) => match np.as_ref() {
                NodePredicate::Standard(sp) => {
                    let (field, op, value) = resolve_ehr_predicate(sp)?;
                    Ok(vec![EhrPredicate { field, op, value }])
                }
                NodePredicate::And(a, b) => {
                    let mut out = Self::lower_ehr_predicate(&PathPredicate::Node(a.clone()))?;
                    out.extend(Self::lower_ehr_predicate(&PathPredicate::Node(b.clone()))?);
                    Ok(out)
                }
                _ => Err(
                    AnalysisError::TypeMismatch("unsupported EHR predicate form".to_owned()).into(),
                ),
            },
            PathPredicate::Archetype(_) => Err(AnalysisError::TypeMismatch(
                "EHR does not take an archetype predicate".to_owned(),
            )
            .into()),
        }
    }

    /// Bind a FROM variable, rejecting a name already declared by another
    /// class expression (variable names must be unique within an AQL
    /// statement — QUERY master03 §Variables/Syntax; names are
    /// case-insensitive, so the uniqueness check folds case).
    fn bind(&mut self, name: &str, source: SourceId, kind: BindingKind) -> Result<(), AqlError> {
        if self.bindings.contains(name) {
            return Err(AnalysisError::DuplicateVariable(name.to_owned()).into());
        }
        self.bindings.insert(name, Binding { source, kind });
        Ok(())
    }

    // ── SELECT ────────────────────────────────────────────────────────────────

    fn lower_select_column(
        &self,
        col: &openehr_query::ast::SelectExpr,
    ) -> Result<SelectColumn, AqlError> {
        let value = match &col.column {
            ColumnExpr::Path(p) => SelectValue::Path(analyze_path(p, &self.bindings)?),
            ColumnExpr::Primitive(p) => SelectValue::Literal(typed_lit(p)),
            ColumnExpr::Aggregate(agg) => self.lower_aggregate(agg)?,
            ColumnExpr::Function(f) => {
                let (func, args) = self.lower_function(f)?;
                SelectValue::Function { func, args }
            }
        };
        // The RESULT_SET column `path` echoes the query's own path text
        // (ITS-REST 1.1.0 RESULT_SET; the CNF query goldens compare it verbatim).
        let path = match &col.column {
            ColumnExpr::Path(p) => Some(p.column_path_text()),
            _ => None,
        };
        Ok(SelectColumn {
            value,
            alias: col.alias.clone(),
            path,
        })
    }

    fn lower_aggregate(&self, agg: &AggregateCall) -> Result<SelectValue, AqlError> {
        match agg {
            AggregateCall::Count { distinct, path } => Ok(SelectValue::Aggregate {
                func: AggFunc::Count,
                arg: path
                    .as_ref()
                    .map(|p| analyze_path(p, &self.bindings))
                    .transpose()?,
                distinct: *distinct,
            }),
            AggregateCall::Stat { func, path } => {
                let target = analyze_path(path, &self.bindings)?;
                // SUM/AVG accept Integer/Real input only (QUERY master03
                // §Functions/SUM, AVG) — a textual/temporal/boolean leaf is a
                // typed reject, never a silent NULL coercion.
                if matches!(func, StatFunc::Sum | StatFunc::Avg)
                    && let Some(got) = non_numeric_leaf(&target)
                {
                    return Err(AnalysisError::AggregateInputType {
                        func: if matches!(func, StatFunc::Sum) {
                            "SUM"
                        } else {
                            "AVG"
                        },
                        got,
                    }
                    .into());
                }
                Ok(SelectValue::Aggregate {
                    func: match func {
                        StatFunc::Min => AggFunc::Min,
                        StatFunc::Max => AggFunc::Max,
                        StatFunc::Sum => AggFunc::Sum,
                        StatFunc::Avg => AggFunc::Avg,
                    },
                    arg: Some(target),
                    distinct: false,
                })
            }
        }
    }

    // ── WHERE ───────────────────────────────────────────────────────────────

    fn lower_where(&self, expr: &WhereExpr) -> Result<Expr, AqlError> {
        match expr {
            WhereExpr::Identified(ie, _) => self.lower_identified(ie),
            WhereExpr::Not(w) => Ok(Expr::Not(Box::new(self.lower_where(w)?))),
            WhereExpr::And(a, b) => Ok(Expr::And(
                Box::new(self.lower_where(a)?),
                Box::new(self.lower_where(b)?),
            )),
            WhereExpr::Or(a, b) => Ok(Expr::Or(
                Box::new(self.lower_where(a)?),
                Box::new(self.lower_where(b)?),
            )),
        }
    }

    fn lower_identified(&self, ie: &IdentifiedExpr) -> Result<Expr, AqlError> {
        match ie {
            IdentifiedExpr::Resolved(b) => Ok(Expr::Const(*b)),
            IdentifiedExpr::Exists(path) => Ok(Expr::Exists(analyze_path(path, &self.bindings)?)),
            IdentifiedExpr::Compare { lhs, op, rhs } => {
                let lhs = self.lower_compare_operand(lhs)?;
                let mut rhs = self.lower_terminal(rhs)?;
                let coercion = comparison_coercion(&lhs, &rhs);
                let mut lhs = lhs;
                if coercion == Coercion::Temporal {
                    retype_temporal(&mut lhs);
                    retype_temporal(&mut rhs);
                }
                Ok(Expr::Compare {
                    lhs,
                    op: *op,
                    rhs,
                    coercion,
                })
            }
            IdentifiedExpr::Like { path, operand } => Ok(Expr::Like {
                path: analyze_path(path, &self.bindings)?,
                pattern: match operand {
                    LikeOperand::String(s) => LikePattern::Literal(s.clone()),
                    LikeOperand::Parameter(p) => LikePattern::Param(param_name(p)),
                },
            }),
            IdentifiedExpr::Matches { path, operand } => {
                let target = analyze_path(path, &self.bindings)?;
                let coercion = target_coercion(&target);
                let values = match operand {
                    MatchesOperand::ValueList(items) => items
                        .iter()
                        .map(bind_from_value_item)
                        .collect::<Result<Vec<_>, _>>()?,
                    MatchesOperand::Terminology(_) => {
                        return Err(AqlFeatureError::MatchesTerminology.into());
                    }
                    MatchesOperand::Uri(_) => return Err(AqlFeatureError::MatchesUri.into()),
                };
                Ok(Expr::Matches {
                    path: target,
                    values,
                    coercion,
                })
            }
        }
    }

    fn lower_compare_operand(&self, op: &CompareOperand) -> Result<Operand, AqlError> {
        match op {
            CompareOperand::Path(p) => Ok(Operand::Path(analyze_path(p, &self.bindings)?)),
            CompareOperand::Function(f) => {
                let (func, args) = self.lower_function(f)?;
                Ok(Operand::Function { func, args })
            }
        }
    }

    fn lower_terminal(&self, t: &Terminal) -> Result<Operand, AqlError> {
        match t {
            Terminal::Primitive(p) => Ok(Operand::Literal(typed_lit(p))),
            Terminal::Parameter(p) => Ok(Operand::Param(param_name(p))),
            Terminal::Path(p) => Ok(Operand::Path(analyze_path(p, &self.bindings)?)),
            Terminal::Function(f) => {
                let (func, args) = self.lower_function(f)?;
                Ok(Operand::Function { func, args })
            }
        }
    }

    fn lower_function(&self, call: &FunctionCall) -> Result<(ScalarFn, Vec<Operand>), AqlError> {
        match call {
            FunctionCall::Terminology(_) => Err(AqlFeatureError::TerminologyFunction.into()),
            FunctionCall::Named { name, args } => {
                let func = scalar_fn(name)
                    .ok_or_else(|| AqlFeatureError::UnsupportedFunction(name.clone()))?;
                check_function_arity(func, args.len())?;
                let args = args
                    .iter()
                    .map(|a| self.lower_terminal(a))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((func, args))
            }
        }
    }

    // ── ORDER BY ──────────────────────────────────────────────────────────────

    fn lower_order_by(&self, o: &OrderByExpr) -> Result<OrderKey, AqlError> {
        Ok(OrderKey {
            path: analyze_path(&o.path, &self.bindings)?,
            ascending: !matches!(o.order, Some(SortOrder::Descending)),
        })
    }
}

/// LIMIT / TOP resolution: `TOP n` maps to `LIMIT n`; combining the two is
/// rejected (QUERY §Query structure/LIMIT).
fn lower_limit(query: &SelectQuery) -> Result<(Option<i64>, Option<i64>, bool), AqlError> {
    let (limit, offset, limit_is_top) = match (&query.select.top, &query.limit) {
        (Some(_), Some(_)) => return Err(AqlFeatureError::TopWithLimit.into()),
        (Some(top), None) => {
            // `TOP n [FORWARD]` maps to `LIMIT n`; the deprecated BACKWARD
            // direction is a typed reject carrying the spec's own rewrite
            // guidance (QUERY §SELECT/TOP deprecation note) — never a silent
            // first-n answer.
            if top.direction == Some(TopDirection::Backward) {
                return Err(AqlFeatureError::TopBackward(top.count).into());
            }
            (Some(top.count), None, true)
        }
        (None, Some(limit)) => (Some(limit.limit), limit.offset, false),
        (None, None) => (None, None, false),
    };
    // `row_count` minimum is 1; `offset` minimum is 0 (QUERY master03
    // §LIMIT/Syntax).
    if let Some(l) = limit
        && l < 1
    {
        return Err(AnalysisError::PagingBounds {
            clause: "LIMIT",
            value: l,
            minimum: 1,
        }
        .into());
    }
    if let Some(o) = offset
        && o < 0
    {
        return Err(AnalysisError::PagingBounds {
            clause: "OFFSET",
            value: o,
            minimum: 0,
        }
        .into());
    }
    Ok((limit, offset, limit_is_top))
}

/// The human label of a leaf type SUM/AVG cannot aggregate, or `None` when
/// the target is numerically aggregable.
fn non_numeric_leaf(target: &PathTarget) -> Option<&'static str> {
    let coercion = match target {
        PathTarget::Data(leaf) | PathTarget::EhrStatus(leaf) => leaf.coercion,
        PathTarget::Version { .. } | PathTarget::Ehr { .. } => return Some("version/EHR metadata"),
    };
    match coercion {
        Coercion::Magnitude | Coercion::Raw => None,
        Coercion::Text => Some("a textual value"),
        Coercion::Temporal => Some("a temporal value"),
        Coercion::Boolean => Some("a boolean value"),
    }
}

/// Scalar-function arity (QUERY master03 §Functions): reject a call whose
/// argument count is outside the declared signature.
fn check_function_arity(func: ScalarFn, got: usize) -> Result<(), AqlError> {
    let (name, expected, ok): (&'static str, &'static str, bool) = match func {
        ScalarFn::Length => ("LENGTH", "1", got == 1),
        ScalarFn::Substring => ("SUBSTRING", "2 or 3", got == 2 || got == 3),
        ScalarFn::Position => ("POSITION", "2", got == 2),
        ScalarFn::StrContains => ("CONTAINS", "2", got == 2),
        ScalarFn::Concat => ("CONCAT", "at least 1", got >= 1),
        ScalarFn::ConcatWs => ("CONCAT_WS", "at least 2", got >= 2),
        ScalarFn::Abs => ("ABS", "1", got == 1),
        ScalarFn::Ceil => ("CEIL", "1", got == 1),
        ScalarFn::Floor => ("FLOOR", "1", got == 1),
        ScalarFn::Round => ("ROUND", "1 or 2", got == 1 || got == 2),
        ScalarFn::Mod => ("MOD", "2", got == 2),
        ScalarFn::CurrentDate => ("CURRENT_DATE", "0", got == 0),
        ScalarFn::CurrentTime => ("CURRENT_TIME", "0", got == 0),
        ScalarFn::CurrentDateTime => ("CURRENT_DATE_TIME", "0", got == 0),
        ScalarFn::Now => ("NOW", "0", got == 0),
        ScalarFn::CurrentTimezone => ("CURRENT_TIMEZONE", "0", got == 0),
    };
    if ok {
        Ok(())
    } else {
        Err(AnalysisError::FunctionArity {
            func: name,
            expected,
            got,
        }
        .into())
    }
}

fn scalar_fn(name: &str) -> Option<ScalarFn> {
    match name.to_ascii_lowercase().as_str() {
        "length" => Some(ScalarFn::Length),
        "substring" => Some(ScalarFn::Substring),
        "position" => Some(ScalarFn::Position),
        "concat" => Some(ScalarFn::Concat),
        "concat_ws" => Some(ScalarFn::ConcatWs),
        "abs" => Some(ScalarFn::Abs),
        "ceil" => Some(ScalarFn::Ceil),
        "floor" => Some(ScalarFn::Floor),
        "round" => Some(ScalarFn::Round),
        "mod" => Some(ScalarFn::Mod),
        "current_date" => Some(ScalarFn::CurrentDate),
        "current_time" => Some(ScalarFn::CurrentTime),
        "current_date_time" => Some(ScalarFn::CurrentDateTime),
        "now" => Some(ScalarFn::Now),
        "current_timezone" => Some(ScalarFn::CurrentTimezone),
        "contains" => Some(ScalarFn::StrContains),
        _ => None,
    }
}

fn bind_from_value_item(item: &ValueListItem) -> Result<Bind, AqlError> {
    match item {
        ValueListItem::Primitive(p) => Ok(Bind::Literal(typed_lit(p))),
        ValueListItem::Parameter(p) => Ok(Bind::Param(param_name(p))),
        ValueListItem::Terminology(_) => Err(AqlFeatureError::MatchesTerminology.into()),
    }
}

/// The coercion for a comparison: the data-leaf operand's coercion drives it
/// (left preferred, then right); non-path/scalar pairs fall back to
/// [`Coercion::Raw`].
fn comparison_coercion(lhs: &Operand, rhs: &Operand) -> Coercion {
    if let Operand::Path(t) = lhs {
        return target_coercion(t);
    }
    if let Operand::Path(t) = rhs {
        return target_coercion(t);
    }
    Coercion::Raw
}

fn target_coercion(t: &PathTarget) -> Coercion {
    match t {
        PathTarget::Data(l) | PathTarget::EhrStatus(l) => l.coercion,
        PathTarget::Version { field, .. } => {
            if *field == super::ir::VersionField::TimeCommitted {
                Coercion::Temporal
            } else {
                Coercion::Text
            }
        }
        PathTarget::Ehr { field, .. } => {
            if *field == super::ir::EhrField::TimeCreated {
                Coercion::Temporal
            } else {
                Coercion::Text
            }
        }
    }
}

/// Retype a string literal operand as a temporal when the comparison context is
/// temporal (QUERY §Built-in Types/Dates and Times: a quoted value is typed
/// from its identified-path context, not the literal).
fn retype_temporal(op: &mut Operand) {
    if let Operand::Literal(TypedLit::String(s)) = op {
        *op = Operand::Literal(TypedLit::Temporal(std::mem::take(s)));
    }
}
