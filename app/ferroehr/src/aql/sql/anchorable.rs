// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one walk over the WHERE clause that finds the conjuncts a value index
//! serves (`super::anchor`), shared by the anchor builder and the flat FROM,
//! which decides by it whether to read each contained node inside its object
//! (#6263). No openEHR spec governs access paths: our own design.

use openehr_query::lexer::CompOp;

use crate::aql::ir::{self, Bind, Coercion, LeafPath, Operand, PathTarget, QueryDomain};

use super::Builder;
use super::anchor::{RangeEntry, Ranged, leaf_step, ranged, text_leaf};

/// A top-level conjunct of the WHERE clause an index serves
/// ([`Builder::anchorable`]).
pub(super) enum Anchorable<'q> {
    /// An equality or a `matches` of a text leaf with one of `texts`.
    Equality {
        leaf: &'q LeafPath,
        texts: Vec<String>,
    },
    /// The range comparisons of one leaf, each operator with its bound.
    Range {
        leaf: &'q LeafPath,
        ranged: Ranged,
        bounds: Vec<(CompOp, &'q Operand)>,
    },
}

impl Builder<'_> {
    /// Whether the WHERE clause has a conjunct a value index serves, which
    /// [`Builder::value_anchors`] makes an anchor of: the flat FROM then reads
    /// each contained node inside its object (`from::Builder::emit_rm`).
    ///
    /// On statistics older than the rows they missed, the anchor and the
    /// contained node were both estimated at one row, and the planner nested
    /// the whole of one entry, unparameterised, inside the rows of the other:
    /// with the contained node read inside its object the anchor is the one
    /// entry from the population (#6243, #6263).
    pub(super) fn takes_anchor(&self) -> bool {
        self.ir
            .filter
            .as_ref()
            .is_some_and(|filter| !self.anchorable(filter).is_empty())
    }

    /// The top-level conjuncts of `filter` an index serves, in the order the
    /// statement takes them, with the comparisons of one leaf's ranges
    /// together: the one walk [`Builder::value_anchors`] builds its anchors
    /// from and [`Builder::takes_anchor`] decides by.
    pub(super) fn anchorable<'q>(&self, filter: &'q ir::Expr) -> Vec<Anchorable<'q>> {
        let mut found = Vec::new();
        let mut conjuncts = vec![filter];
        while let Some(expr) = conjuncts.pop() {
            match expr {
                ir::Expr::And(a, b) => {
                    conjuncts.push(b);
                    conjuncts.push(a);
                }
                // NOTE: a `Raw` (mixed-type) equality with a string compares as text,
                // the numeric branch being taken on a numeric literal alone
                // (`Builder::value_test`), which no string operand is.
                ir::Expr::Compare {
                    lhs,
                    op: CompOp::Eq,
                    rhs,
                    coercion: Coercion::Text | Coercion::Raw,
                } => {
                    let ((Operand::Path(PathTarget::Data(leaf)), bound)
                    | (bound, Operand::Path(PathTarget::Data(leaf)))) = (lhs, rhs)
                    else {
                        continue;
                    };
                    let bind = match bound {
                        Operand::Literal(literal) => Bind::Literal(literal.clone()),
                        Operand::Param(name) => Bind::Param(name.clone()),
                        Operand::Path(_) | Operand::Function { .. } => continue,
                    };
                    let Some(text) = self.anchor_text(&bind) else {
                        continue;
                    };
                    let texts = vec![text];
                    if self.equality_anchorable(leaf, &texts) {
                        found.push(Anchorable::Equality { leaf, texts });
                    }
                }
                // NOTE: AQL QUERY master03 §matches: a value list holds when the leaf
                // matches one of its items, so it reads as the equalities above.
                ir::Expr::Matches {
                    path: PathTarget::Data(leaf),
                    values,
                    coercion: Coercion::Text | Coercion::Raw,
                } => {
                    let Some(texts) = values
                        .iter()
                        .map(|bind| self.anchor_text(bind))
                        .collect::<Option<Vec<_>>>()
                    else {
                        continue;
                    };
                    if !texts.is_empty() && self.equality_anchorable(leaf, &texts) {
                        found.push(Anchorable::Equality { leaf, texts });
                    }
                }
                // NOTE: AQL QUERY master03 §WHERE (`identifiedExpr`): the grammar puts the
                // path on the left of a comparison and the terminal on its right.
                ir::Expr::Compare {
                    lhs: Operand::Path(PathTarget::Data(leaf)),
                    op,
                    rhs: bound @ (Operand::Literal(_) | Operand::Param(_)),
                    coercion,
                } if self.range_entry != RangeEntry::Off => {
                    let Some(reads) = ranged(leaf, *op, *coercion) else {
                        continue;
                    };
                    let leaf: &LeafPath = leaf;
                    if !leaf_step(self.ir, leaf).is_some_and(|(types, _)| reads.binds(types)) {
                        continue;
                    }
                    let same = found.iter_mut().find_map(|conjunct| match conjunct {
                        Anchorable::Range {
                            leaf: other,
                            bounds,
                            ..
                        } if *other == leaf => Some(bounds),
                        _ => None,
                    });
                    match same {
                        Some(bounds) => bounds.push((*op, bound)),
                        None => found.push(Anchorable::Range {
                            leaf,
                            ranged: reads,
                            bounds: vec![(*op, bound)],
                        }),
                    }
                }
                _ => {}
            }
        }
        found
    }

    /// Whether an equality of `leaf` with one of `texts` has an anchor: the
    /// relationship target's ([`Self::target_anchor`]) or the value's
    /// ([`Self::value_anchor`]).
    fn equality_anchorable(&self, leaf: &LeafPath, texts: &[String]) -> bool {
        let target = self.ir.domain == QueryDomain::Demographic
            && leaf.anchor.is_empty()
            && leaf.fragment.iter().map(|s| s.name.as_str()).eq([
                "relationships",
                "target",
                "id",
                "value",
            ])
            && texts
                .iter()
                .all(|text| crate::party_binding::derive::container(text).is_some())
            && matches!(
                self.ir.sources.get(leaf.source.0),
                Some(ir::Source::Rm(source)) if matches!(source.scope, ir::VersionScope::Latest)
            );
        let value = text_leaf(&leaf.fragment).is_some()
            && leaf_step(self.ir, leaf).is_some_and(|(types, _)| types.contains("ELEMENT"));
        target || value
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::aql::ir::Params;
    use crate::aql::lineage::ArchetypeLineage;
    use crate::aql::sql::{Builder, SqlCtx};

    /// A population context with no access condition.
    fn population() -> SqlCtx {
        SqlCtx {
            system_id: "anchor.test".to_owned(),
            ehr_ids: Vec::new(),
            ehr_scope_kind: crate::aql::sql::EhrScopeKind::Named,
            ehr_access: crate::access_eval::condition::AccessCondition::default(),
            limit: None,
            offset: None,
            archetype_lineage: Arc::new(ArchetypeLineage::default()),
            policy_parties: crate::aql::sql::readers::PolicyParties::Served,
            access_settings: crate::aql::sql::readers::EhrAccessReaders::Every,
            own_records: None,
            caseload: None,
        }
    }

    /// The conjuncts the shared walk finds in `aql` and the anchors the
    /// flat statement builds from them.
    fn walked_and_built(aql: &str) -> (usize, bool, usize) {
        let ast = openehr_query::parser::parse_str(aql).expect("the AQL parses");
        let params = Params::default();
        let ir = crate::aql::plan(
            &ast,
            &params,
            crate::config::profile::SpecProfile::default(),
        )
        .expect("the AQL plans");
        let ctx = population();
        let mut builder = Builder::new(&ir, &params, &ctx);
        builder.build_from(&ir.contains).expect("the FROM builds");
        let walked = ir
            .filter
            .as_ref()
            .map_or(0, |filter| builder.anchorable(filter).len());
        let takes = builder.takes_anchor();
        let built = builder.value_anchors().expect("the anchors build").len();
        (walked, takes, built)
    }

    /// Whether the flat FROM of `aql` reads its contained nodes inside their
    /// objects.
    fn fences(aql: &str) -> bool {
        let ast = openehr_query::parser::parse_str(aql).expect("the AQL parses");
        let params = Params::default();
        let ir = crate::aql::plan(
            &ast,
            &params,
            crate::config::profile::SpecProfile::default(),
        )
        .expect("the AQL plans");
        let ctx = population();
        let builder = Builder::new(&ir, &params, &ctx);
        builder.takes_anchor()
    }

    /// Every anchor fences the contained nodes, on a leaf only the WHERE
    /// clause reads, on one a SELECT column binds and on the source's own
    /// value; a statement with no anchor leaves them their archetype-index
    /// entry (#6243, #6263).
    #[test]
    fn every_anchor_fences_the_contained_nodes() {
        let leaf = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/magnitude";
        let units = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/units";
        let time = "o/data[at0001]/events[at0002]/time/value";
        let over = [leaf, " >= 140"].concat();
        let kpa = [units, " = 'kPa'"].concat();
        let count = ["count(", leaf, ")"].concat();
        let projected = ["c/uid/value, ", leaf].concat();
        let sibling = ["c/uid/value, ", units].concat();
        let equal = [leaf, " = 7"].concat();
        for (select, filter, fenced) in [
            ("c/uid/value", Some(over.as_str()), true),
            (projected.as_str(), Some(over.as_str()), true),
            (sibling.as_str(), Some(over.as_str()), true),
            (sibling.as_str(), Some(kpa.as_str()), true),
            (time, Some(over.as_str()), true),
            (count.as_str(), Some(over.as_str()), true),
            ("c/uid/value", Some(equal.as_str()), false),
            ("c/uid/value", None, false),
        ] {
            let aql = observation_query(select, filter);
            assert_eq!(fences(&aql), fenced, "{aql}");
        }
        let element = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
                       CONTAINS ELEMENT s[at0004] WHERE s/value/magnitude > 7";
        assert!(fences(element), "{element}");
    }

    /// `SELECT select FROM` the timed OBSERVATION, `WHERE filter` when given.
    fn observation_query(select: &str, filter: Option<&str>) -> String {
        let from = " FROM EHR e CONTAINS COMPOSITION c \
                    CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.timed.v1]";
        match filter {
            Some(filter) => ["SELECT ", select, from, " WHERE ", filter].concat(),
            None => ["SELECT ", select, from].concat(),
        }
    }

    /// The walk that decides whether the flat FROM reads its contained nodes
    /// inside their objects finds exactly the conjuncts the anchor builder
    /// makes anchors of (#6263).
    #[test]
    fn the_walk_and_the_anchor_builder_agree() {
        let items = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]";
        let time = "o/data[at0001]/events[at0002]/time/value";
        for (filter, anchors) in [
            (format!("{items}/value/magnitude >= 140"), 1),
            (
                format!("{items}/value/magnitude > 1 AND {items}/value/magnitude < 9"),
                1,
            ),
            (
                format!("{time} >= '2026-01-01' AND {items}/value/magnitude < 9"),
                2,
            ),
            (format!("{items}/value/units = 'kPa'"), 1),
            (
                format!("{items}/value/defining_code/code_string = 'at0008'"),
                1,
            ),
            (format!("{items}/value/value matches {{'a', 'b'}}"), 1),
            (format!("{items}/value/magnitude = 7"), 0),
            (format!("{items}/value/magnitude != 7"), 0),
            (
                format!("{items}/value/magnitude > 1 OR {items}/value/magnitude > 1"),
                0,
            ),
            (format!("NOT {items}/value/magnitude > 1"), 0),
            ("c/context/start_time/value >= '2026'".to_owned(), 0),
            (
                format!("{items}/value/magnitude > {items}/value/precision"),
                0,
            ),
        ] {
            let aql = observation_query("c/uid/value", Some(&filter));
            let (walked, takes, built) = walked_and_built(&aql);
            assert_eq!((walked, built), (anchors, anchors), "{filter}");
            assert_eq!(takes, anchors > 0, "{filter}");
        }
        for (aql, anchors) in [
            (
                "SELECT p/uid/value FROM PERSON p WHERE p/relationships/target/id/value \
                 = '0190d5a8-2f4e-7c52-9a1e-4f6b2c3d4e5f::ferroehr::1'",
                1,
            ),
            (
                "SELECT p/uid/value FROM PERSON p WHERE p/relationships/target/id/value \
                 = 'not a party'",
                0,
            ),
            (
                "SELECT p/uid/value FROM VERSION v[ALL_VERSIONS] CONTAINS PERSON p \
                 WHERE p/relationships/target/id/value \
                 = '0190d5a8-2f4e-7c52-9a1e-4f6b2c3d4e5f::ferroehr::1'",
                0,
            ),
        ] {
            let (walked, takes, built) = walked_and_built(aql);
            assert_eq!((walked, built), (anchors, anchors), "{aql}");
            assert_eq!(takes, anchors > 0, "{aql}");
        }
    }
}
