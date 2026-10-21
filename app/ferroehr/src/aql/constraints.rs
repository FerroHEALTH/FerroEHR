// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The positive archetype and template constraints of a bound query, for the
//! access record's category classification (EHDS Annex II 3.2(c)).
//!
//! A query answer that carries no object of its own — leaf columns of a
//! `DISTINCT` or aggregate projection, or zero rows — is classified from what
//! the query asked for. Positive means asserted: an operand reachable through
//! `CONTAINS` (never through `NOT CONTAINS`), an archetype predicate on such an
//! operand or on a path rooted at one, and a `template_id` equality or
//! `MATCHES` outside any `NOT`.
//!
//! **No openEHR spec governs this — our own design/extension.** The grammar
//! read here is QUERY `master03-syntax.adoc` §Containment and §Node predicate.

use std::collections::BTreeSet;

use openehr_query::lexer::CompOp;

use super::ir::{
    ArchetypeConstraint, Bind, ContainsTree, Expr, LeafPath, Link, NodeConstraint, Operand,
    ParamValue, Params, PathTarget, QueryIr, SelectValue, Source, StdPredicate, TypedLit,
};
use crate::system_log::categories::{QueryConstraints, ResourceKind};

/// The fragment path that addresses a root's committed template id
/// (`archetype_details/template_id/value`, RM common `ARCHETYPED.template_id`).
const TEMPLATE_PATH: [&str; 3] = ["archetype_details", "template_id", "value"];

/// The RM classes that hold no priority-category data.
const NO_CATEGORY_TYPES: [(&str, ResourceKind); 3] = [
    ("EHR_STATUS", ResourceKind::EhrStatus),
    ("EHR_ACCESS", ResourceKind::EhrAccess),
    ("FOLDER", ResourceKind::Folder),
];

/// The positive constraints of `ir` with `params` bound.
#[must_use]
pub fn positive_constraints(ir: &QueryIr, params: &Params) -> QueryConstraints {
    let mut positive = BTreeSet::new();
    collect_positive(&ir.contains, &mut positive);
    let mut walk = Walk {
        params,
        positive: &positive,
        out: QueryConstraints::default(),
        bound: BTreeSet::new(),
    };
    let mut content = BTreeSet::new();
    for id in &positive {
        match ir.sources.get(*id) {
            Some(Source::Ehr(_)) => {
                walk.out.resource_kinds.insert(ResourceKind::Ehr);
            }
            Some(Source::Rm(rm)) => {
                let kinds: Vec<Option<ResourceKind>> = rm
                    .rm_type
                    .names()
                    .iter()
                    .map(|name| {
                        NO_CATEGORY_TYPES
                            .iter()
                            .find(|(rm_name, _)| rm_name == name)
                            .map(|(_, kind)| kind.clone())
                    })
                    .collect();
                if kinds.iter().all(Option::is_some) && !kinds.is_empty() {
                    walk.out.resource_kinds.extend(kinds.into_iter().flatten());
                } else {
                    walk.out.reaches_content = true;
                    content.insert(*id);
                }
                let mut bound = false;
                if let Some(archetype) = &rm.archetype {
                    bound |= walk.archetype(archetype);
                }
                for predicate in &rm.standard {
                    bound |= walk.standard(predicate);
                }
                if bound {
                    walk.bound.insert(*id);
                }
            }
            Some(Source::Version(_)) | None => {}
        }
    }
    for column in &ir.select {
        match &column.value {
            SelectValue::Path(target)
            | SelectValue::Aggregate {
                arg: Some(target), ..
            } => walk.target(target),
            SelectValue::Function { args, .. } => args.iter().for_each(|a| walk.operand(a)),
            SelectValue::Aggregate { arg: None, .. } | SelectValue::Literal(_) => {}
        }
    }
    for key in &ir.order_by {
        walk.target(&key.path);
    }
    if let Some(filter) = &ir.filter {
        walk.expr(filter, false);
    }
    walk.out.every_root_bound = content.is_subset(&walk.bound);
    walk.out
}

/// Collects the operands reachable through positive containment.
fn collect_positive(tree: &ContainsTree, out: &mut BTreeSet<usize>) {
    match tree {
        ContainsTree::Operand { source, contained } => {
            out.insert(source.0);
            if let Some(contained) = contained
                && contained.link == Link::Contains
            {
                collect_positive(&contained.tree, out);
            }
        }
        ContainsTree::And(a, b) | ContainsTree::Or(a, b) => {
            collect_positive(a, out);
            collect_positive(b, out);
        }
    }
}

/// The walk state: the bound parameters, the positive operands, the
/// constraints found so far and the operands an id binds.
struct Walk<'a> {
    params: &'a Params,
    positive: &'a BTreeSet<usize>,
    out: QueryConstraints,
    bound: BTreeSet<usize>,
}

impl Walk<'_> {
    /// The string a bind resolves to, when it is one.
    fn bound(&self, bind: &Bind) -> Option<String> {
        match bind {
            Bind::Literal(TypedLit::String(value)) => Some(value.clone()),
            Bind::Param(name) => match self.params.get(name) {
                Some(ParamValue::Str(value)) => Some(value.clone()),
                _ => None,
            },
            Bind::Literal(_) => None,
        }
    }

    /// Records the archetype id `constraint` names, returning whether it named
    /// one.
    fn archetype(&mut self, constraint: &ArchetypeConstraint) -> bool {
        let id = match constraint {
            ArchetypeConstraint::Archetype(id) => Some(id.clone()),
            ArchetypeConstraint::Param(name) => self.bound(&Bind::Param(name.clone())),
            ArchetypeConstraint::NodeCode(_) => None,
        };
        // NOTE: a node code is an at-code, not an archetype id; only a full HRID constrains content.
        if let Some(id) = id.filter(|id| id.parse::<openehr_base::prelude::ArchetypeId>().is_ok()) {
            self.out.archetype_ids.insert(id);
            return true;
        }
        false
    }

    fn node(&mut self, constraint: &NodeConstraint) {
        if let Some(archetype) = &constraint.archetype {
            self.archetype(archetype);
        }
        for predicate in &constraint.standard {
            self.standard(predicate);
        }
    }

    /// Records the template id `predicate` names, returning whether it named
    /// one.
    fn standard(&mut self, predicate: &StdPredicate) -> bool {
        if predicate.op == CompOp::Eq
            && predicate.path.iter().map(String::as_str).eq(TEMPLATE_PATH)
            && let Some(template) = self.bound(&predicate.value)
        {
            self.out.template_ids.insert(template);
            return true;
        }
        false
    }

    /// The archetype predicates on a path rooted at a positive operand.
    fn leaf(&mut self, leaf: &LeafPath) {
        if !self.positive.contains(&leaf.source.0) {
            return;
        }
        if let Some(root) = &leaf.root_predicate {
            self.node(root);
        }
        for step in &leaf.anchor {
            if let Some(predicate) = &step.predicate {
                self.node(predicate);
            }
        }
    }

    fn target(&mut self, target: &PathTarget) {
        if let PathTarget::Data(leaf) = target {
            self.leaf(leaf);
        }
    }

    fn operand(&mut self, operand: &Operand) {
        match operand {
            Operand::Path(target) => self.target(target),
            Operand::Function { args, .. } => args.iter().for_each(|a| self.operand(a)),
            Operand::Literal(_) | Operand::Param(_) => {}
        }
    }

    /// Whether `target` addresses a positive root's template id.
    fn is_template_path(&self, target: &PathTarget) -> bool {
        matches!(target, PathTarget::Data(leaf)
            if leaf.anchor.is_empty()
                && self.positive.contains(&leaf.source.0)
                && leaf.fragment.iter().map(|s| s.name.as_str()).eq(TEMPLATE_PATH))
    }

    /// Marks the operand `target` is rooted at as bound to a template id.
    fn bind_root(&mut self, target: &PathTarget) {
        if let PathTarget::Data(leaf) = target {
            self.bound.insert(leaf.source.0);
        }
    }

    fn operand_bind(operand: &Operand) -> Option<Bind> {
        match operand {
            Operand::Literal(literal) => Some(Bind::Literal(literal.clone())),
            Operand::Param(name) => Some(Bind::Param(name.clone())),
            Operand::Path(_) | Operand::Function { .. } => None,
        }
    }

    fn expr(&mut self, expr: &Expr, negated: bool) {
        match expr {
            Expr::Not(inner) => self.expr(inner, !negated),
            Expr::And(a, b) | Expr::Or(a, b) => {
                self.expr(a, negated);
                self.expr(b, negated);
            }
            _ if negated => {}
            Expr::Compare { lhs, op, rhs, .. } => {
                self.operand(lhs);
                self.operand(rhs);
                if *op != CompOp::Eq {
                    return;
                }
                let value = match (lhs, rhs) {
                    (Operand::Path(path), other) | (other, Operand::Path(path))
                        if self.is_template_path(path) =>
                    {
                        Self::operand_bind(other).map(|bind| (path, bind))
                    }
                    _ => None,
                };
                if let Some((path, template)) =
                    value.and_then(|(path, bind)| Some((path, self.bound(&bind)?)))
                {
                    self.out.template_ids.insert(template);
                    self.bind_root(path);
                }
            }
            Expr::Matches { path, values, .. } => {
                self.target(path);
                if self.is_template_path(path) {
                    let templates: Vec<String> =
                        values.iter().filter_map(|v| self.bound(v)).collect();
                    if !templates.is_empty() {
                        self.bind_root(path);
                    }
                    self.out.template_ids.extend(templates);
                }
            }
            Expr::Exists(path) | Expr::Like { path, .. } => self.target(path),
            Expr::Const(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::profile::SpecProfile;

    fn constraints(aql: &str, params: &Params) -> QueryConstraints {
        let query = openehr_query::parser::parse_str(aql).expect("parse");
        let ir = super::super::plan(&query, params, SpecProfile::Development).expect("plan");
        positive_constraints(&ir, params)
    }

    #[test]
    fn a_not_contains_operand_never_counts() {
        let c = constraints(
            "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.report.v1] \
             NOT CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.imaging_exam_result.v0]",
            &Params::new(),
        );
        assert_eq!(
            c.archetype_ids,
            BTreeSet::from(["openEHR-EHR-COMPOSITION.report.v1".to_owned()])
        );
        assert!(c.reaches_content);
        assert!(
            c.every_root_bound,
            "the one positive content operand is bound"
        );
    }

    #[test]
    fn template_equalities_count_and_negated_ones_do_not() {
        let c = constraints(
            "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
             WHERE c/archetype_details/template_id/value = 'Lab Report' \
             AND NOT c/archetype_details/template_id/value = 'Imaging Report'",
            &Params::new(),
        );
        assert_eq!(c.template_ids, BTreeSet::from(["Lab Report".to_owned()]));
        assert!(
            c.every_root_bound,
            "the template equality binds the composition"
        );
    }

    #[test]
    fn an_operand_no_id_binds_leaves_the_query_unbound() {
        let c = constraints(
            "SELECT o/data FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.report.v1] \
             CONTAINS OBSERVATION o",
            &Params::new(),
        );
        assert!(c.reaches_content);
        assert!(!c.every_root_bound, "the observation is bound to no id");
    }

    #[test]
    fn parameters_and_path_predicates_are_bound() {
        let params = Params::new().with(
            "arch",
            ParamValue::Str("openEHR-EHR-COMPOSITION.encounter.v1".to_owned()),
        );
        let c = constraints(
            "SELECT c/content[openEHR-EHR-OBSERVATION.blood_pressure.v2]/name/value \
             FROM EHR e CONTAINS COMPOSITION c[$arch]",
            &params,
        );
        assert_eq!(
            c.archetype_ids,
            BTreeSet::from([
                "openEHR-EHR-COMPOSITION.encounter.v1".to_owned(),
                "openEHR-EHR-OBSERVATION.blood_pressure.v2".to_owned(),
            ])
        );
    }

    #[test]
    fn an_ehr_only_query_reaches_no_content() {
        let c = constraints("SELECT e/ehr_id/value FROM EHR e", &Params::new());
        assert!(!c.reaches_content);
        assert_eq!(c.resource_kinds, BTreeSet::from([ResourceKind::Ehr]));
    }

    #[test]
    fn an_ehr_access_query_reaches_no_content() {
        let c = constraints("SELECT a FROM EHR e CONTAINS EHR_ACCESS a", &Params::new());
        assert!(!c.reaches_content);
        assert_eq!(
            c.resource_kinds,
            BTreeSet::from([ResourceKind::Ehr, ResourceKind::EhrAccess])
        );
    }
}
