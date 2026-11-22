// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The anchors of a WHERE clause: the entry an index gives a filter on a
//! value.
//!
//! A path binding is a correlated subquery the planner evaluates per outer
//! row, so a filter on a value read every node of the archetype and tested
//! the value last (#4437, #6022). An anchor is the condition a top-level
//! conjunct implies, stated as a FROM item the planner can start from: the
//! versions some node row of which, under the leaf step's own predicate,
//! carries a value the conjunct keeps. Five leaves have one:
//!
//! - a text leaf of an `ELEMENT` value under `=` or `matches` over a list of
//!   strings, by its search text (`ext.node_search_text`,
//!   `idx_node_hot_value_search`), and on the `value` member a text in plain
//!   decimal form also by the number of an ordinal or a scale
//!   (`ext.node_number`, `idx_node_hot_value_number`, #6090);
//! - the code of an `ELEMENT` value under `=` or `matches` over a list of
//!   strings, the `defining_code` of a coded text or of an ordinal's symbol,
//!   by `ext.node_code` (`idx_node_hot_value_code`);
//! - the `time` of an event and the `origin` of a history under `=`, `<`,
//!   `<=`, `>` or `>=`, by `node.event_time`, which the node insert fills
//!   with the value the comparison reads (`idx_node_hot_event_time`);
//! - the `magnitude` of an `ELEMENT` value under `<`, `<=`, `>` or `>=`, by
//!   `ext.node_magnitude` (`idx_node_hot_value_magnitude`);
//! - the `relationships/target/id/value` of a party root under `=` or
//!   `matches` over a list of strings, in a demographic query that reads the
//!   trunk head, by the open rows of `party_relationship_target`
//!   (`uq_party_relationship_target_open`), which hold the party each trunk
//!   head's relationships name (#6090).
//!
//! The conjunct itself stays in the statement and each anchor reads the
//! value the conjunct reads, so an anchor removes work and no row. A conjunct
//! under `OR` or `NOT` implies nothing on its own and gets none.
//!
//! A `matches` over a value list holds when the leaf equals one of its
//! members (QUERY master03 §matches), so its anchor is the equality anchor
//! with the members as a list, and only when every member could anchor an
//! equality on its own. An equality keeps few rows, so its anchor always
//! applies. A range may keep
//! most of the store, and how its anchor enters depends on the statement
//! ([`RangeEntry`]). The comparisons of one leaf bind one node row, so they
//! share one anchor: a window `>= $from AND < $to` reads the rows inside it.
//!
//! A FROM item rather than an `EXISTS`: PostgreSQL pulls an `EXISTS` up into
//! a semi-join it orders freely only while the FROM list stays within
//! `join_collapse_limit`, eight items, which a projection of a few path
//! steps exceeds, and joins it last beyond that (PostgreSQL 18, "Controlling
//! the Planner with Explicit JOIN Clauses"). No openEHR spec governs access
//! paths: our own design.

use sea_query::{Alias, Expr, ExprTrait, Query, SelectStatement};

use openehr_query::lexer::CompOp;

use crate::aql::error::AqlError;
use crate::aql::ir::{
    self, Bind, Coercion, FragmentStep, LeafPath, Operand, ParamValue, PathTarget, QueryDomain,
    TypedLit,
};
use crate::db::iden::{Node, RelationshipTarget, VersionRow, VoHead};

use super::Builder;
use super::expr::{binoper, call, col, hot};

/// The RM type of the one structure node that carries a `value`, as a SQL
/// LITERAL: the value indexes are partial on it, and a partial index serves a
/// statement only when the statement proves its predicate, which a bound
/// parameter in a generic plan cannot (PostgreSQL 18, "Partial Indexes").
const ELEMENT_LITERAL: &str = "'ELEMENT'";

/// How the anchor of a range comparison enters a statement.
///
/// No openEHR spec governs access paths: our own design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum RangeEntry {
    /// The flat shape: the anchor is one more FROM item, the first row of
    /// each version the range keeps, which the planner places by its
    /// estimate of the range.
    ///
    /// The first-row form has no `DISTINCT`, so the planner pulls it up
    /// into a node row it can probe per version through `node_hot_pkey`.
    /// The distinct versions it replaced were an item no join could probe:
    /// on statistics older than the rows they missed, the planner rescanned
    /// it per outer row, 7 M head probes for 1,470 OBSERVATIONs (#6243).
    /// An anchor whose leaf the SELECT already binds is tied to that row
    /// instead; otherwise the check for an earlier row joins the statement,
    /// correlated to the leaf's source row, so it runs for the rows the
    /// containment keeps and not for every row the range keeps (#6249).
    #[default]
    Free,
    /// A population statement that streams toward its LIMIT: one anchor
    /// leads the spine as the first row of each version the range keeps, so
    /// the statement reads the index until the LIMIT and never the whole
    /// range.
    Leading,
    /// A statement that reads the versions of the EHRs it names: those
    /// versions are its entry, and a range over the store is none.
    Off,
}

/// What a range comparison reads of a node row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ranged {
    /// `event_time` of an event row.
    EventTime,
    /// `event_time` of a history row, its origin.
    Origin,
    /// `ext.node_magnitude` of an `ELEMENT` row.
    Magnitude,
}

impl Ranged {
    /// The RM types of the rows that carry the value, as SQL literals.
    const fn literals(self) -> &'static str {
        match self {
            Self::EventTime => "'INTERVAL_EVENT', 'POINT_EVENT'",
            Self::Origin => "'HISTORY'",
            Self::Magnitude => ELEMENT_LITERAL,
        }
    }

    /// Whether a step that binds `types` binds only rows that carry the
    /// value, or, for a magnitude, can bind an `ELEMENT`.
    fn binds(self, types: &ir::TypeSet) -> bool {
        let only = |timed: &[&str]| {
            !types.is_empty() && types.names().iter().all(|t| timed.contains(&t.as_str()))
        };
        match self {
            Self::EventTime => only(&["INTERVAL_EVENT", "POINT_EVENT"]),
            Self::Origin => only(&["HISTORY"]),
            Self::Magnitude => types.contains("ELEMENT"),
        }
    }

    /// The value of the row `node`.
    fn read(self, node: &str) -> Expr {
        match self {
            Self::EventTime | Self::Origin => col(node, Node::EventTime),
            // NOTE: crate::storage::shared::UNSHARED, `value` is never shared, so the row's own
            // data is the whole value and the expression stays the index's (#6229).
            Self::Magnitude => call(
                "node_magnitude",
                vec![
                    col(node, Node::ValueType),
                    col(node, Node::ValueNum),
                    col(node, Node::Data),
                ],
            ),
        }
    }

    /// The coercion of the comparison.
    const fn coercion(self) -> Coercion {
        match self {
            Self::EventTime | Self::Origin => Coercion::Temporal,
            Self::Magnitude => Coercion::Magnitude,
        }
    }
}

/// A top-level conjunct of the WHERE clause an index serves
/// ([`Builder::anchorable`]).
enum Anchorable<'q> {
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

/// The range comparisons of one leaf: what they read and each operator with
/// its bound.
struct Range<'a> {
    leaf: &'a LeafPath,
    ranged: Ranged,
    bounds: Vec<(CompOp, Expr)>,
}

impl Builder<'_> {
    /// The anchors of the WHERE clause, in the order the statement takes
    /// them: the equalities, then the ranges [`Builder::range_entry`] admits.
    ///
    /// # Errors
    /// [`AqlError`] from the leaf steps' predicate lowering and from a bound
    /// the comparison itself refuses.
    pub(super) fn value_anchors(&mut self) -> Result<Vec<ValueAnchor>, AqlError> {
        let ir = self.ir;
        let Some(filter) = ir.filter.as_ref() else {
            return Ok(Vec::new());
        };
        let mut anchors = Vec::new();
        let mut ranges: Vec<Range<'_>> = Vec::new();
        for conjunct in self.anchorable(filter) {
            match conjunct {
                Anchorable::Equality { leaf, texts } => {
                    anchors.extend(self.equality_anchor(leaf, &texts)?);
                }
                Anchorable::Range {
                    leaf,
                    ranged,
                    bounds,
                } => {
                    let bounds = bounds
                        .into_iter()
                        .map(|(op, bound)| Ok((op, self.operand_value(bound, ranged.coercion())?)))
                        .collect::<Result<Vec<_>, AqlError>>()?;
                    ranges.push(Range {
                        leaf,
                        ranged,
                        bounds,
                    });
                }
            }
        }
        // A leading anchor is the one entry of a streaming statement, which an
        // equality already gives.
        let leading = self.range_entry == RangeEntry::Leading;
        if leading && !anchors.is_empty() {
            return Ok(anchors);
        }
        for range in ranges {
            let anchor = self.range_anchor(&range, leading)?;
            let found = anchor.is_some();
            anchors.extend(anchor);
            if leading && found {
                break;
            }
        }
        Ok(anchors)
    }

    /// Whether the WHERE clause has a conjunct a value index serves, which
    /// [`Builder::value_anchors`] makes an anchor of.
    #[cfg(test)]
    fn takes_anchor(&self) -> bool {
        self.ir
            .filter
            .as_ref()
            .is_some_and(|filter| !self.anchorable(filter).is_empty())
    }

    /// Whether the WHERE clause anchors a leaf no SELECT column binds: the
    /// flat FROM then reads each contained node inside its object
    /// (`from::Builder::emit_rm`), so a node estimated at one row is never
    /// rescanned with the clause's `EXISTS` per anchor version (#6263). An
    /// anchor on a bound leaf is tied to the bound row (#6249), and the
    /// contained node keeps its archetype-index entry.
    pub(super) fn anchors_an_unbound_leaf(&self) -> bool {
        self.ir.filter.as_ref().is_some_and(|filter| {
            self.anchorable(filter).iter().any(|conjunct| {
                let (Anchorable::Equality { leaf, .. } | Anchorable::Range { leaf, .. }) = conjunct;
                !self.select_binds(leaf)
            })
        })
    }

    /// Whether a SELECT column binds the node `leaf` reads, as
    /// `select::Builder::bind_select_paths` binds every step prefix of a
    /// projected path: the leaf's source itself, or a projected path of the
    /// same source through the leaf's steps.
    fn select_binds(&self, leaf: &LeafPath) -> bool {
        if leaf.anchor.is_empty() {
            return true;
        }
        let mut binds = false;
        for column in &self.ir.select {
            if matches!(column.value, ir::SelectValue::Aggregate { .. }) {
                continue;
            }
            column.value.for_each_path(&mut |target| {
                if let PathTarget::Data(other) = target {
                    binds |= other.source == leaf.source && other.anchor.starts_with(&leaf.anchor);
                }
            });
        }
        binds
    }

    /// The top-level conjuncts of `filter` an index serves, in the order the
    /// statement takes them, with the comparisons of one leaf's ranges
    /// together: the one walk [`Builder::value_anchors`] builds its anchors
    /// from and [`Builder::takes_anchor`] decides by.
    fn anchorable<'q>(&self, filter: &'q ir::Expr) -> Vec<Anchorable<'q>> {
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

    /// The string a bound value compares as, a string literal or a string
    /// parameter, or `None` for any other value: a `Raw` leaf compares with a
    /// number by its number (`Builder::value_test`), which no index holds.
    fn anchor_text(&self, bind: &Bind) -> Option<String> {
        match bind {
            Bind::Literal(TypedLit::String(text)) => Some(text.clone()),
            Bind::Param(name) => match self.params.get(name) {
                Some(ParamValue::Str(text)) => Some(text.clone()),
                _ => None,
            },
            Bind::Literal(_) => None,
        }
    }

    /// The anchor of the equality of `leaf` with one of `texts`: the
    /// relationship target's ([`Self::target_anchor`]), or the value's.
    ///
    /// # Errors
    /// [`AqlError`] from the leaf step's predicate lowering.
    fn equality_anchor(
        &mut self,
        leaf: &LeafPath,
        texts: &[String],
    ) -> Result<Option<ValueAnchor>, AqlError> {
        match self.target_anchor(leaf, texts) {
            Some(anchor) => Ok(Some(anchor)),
            None => self.value_anchor(leaf, texts),
        }
    }

    /// The anchor of the equality of the relationship target
    /// `relationships/target/id/value` of a party root with one of `texts`:
    /// the parties whose trunk head names, among its relationships, one of
    /// the parties the texts name, read from the open rows of
    /// `party_relationship_target`; `None` for any other leaf, a text that
    /// names no party, or a scope other than `LATEST_VERSION`.
    ///
    /// The table holds the target a reference names as
    /// [`crate::party_binding::derive::container`] reads it (its root before
    /// any `::`, case-folded, a hyphenated UUID), so a text the comparison
    /// keeps names the same party and the parties it yields hold every party
    /// the comparison keeps, and more. Its open rows name what each trunk head
    /// names, so the anchor holds only for a query that reads the trunk head;
    /// under `ALL_VERSIONS` or a version predicate a party's older versions
    /// would be lost, and the comparison has no entry.
    fn target_anchor(&mut self, leaf: &LeafPath, texts: &[String]) -> Option<ValueAnchor> {
        if self.ir.domain != QueryDomain::Demographic || !leaf.anchor.is_empty() {
            return None;
        }
        let names: Vec<&str> = leaf.fragment.iter().map(|s| s.name.as_str()).collect();
        if names != ["relationships", "target", "id", "value"] {
            return None;
        }
        let mut targets: Vec<uuid::Uuid> = texts
            .iter()
            .map(|text| crate::party_binding::derive::container(text))
            .collect::<Option<_>>()?;
        targets.sort_unstable();
        targets.dedup();
        let version = self.version_alias_of(leaf.source.0)?;
        if !self.reads_trunk_head(&version) {
            return None;
        }
        let key = self.node_key_of(&version);
        let alias = format!("ra{}", self.next_ctr());
        let (rows, head) = (format!("{alias}_t"), format!("{alias}_h"));
        // The head is read per row by its key, so the entry probes one head a party
        // and never scans the heads (PostgreSQL 18, "Scalar Subqueries").
        let mut content_key = Query::select();
        content_key
            .column((Alias::new(head.as_str()), VoHead::TrunkHeadContentKey))
            .from_as(VoHead::Table, Alias::new(head.as_str()))
            .and_where(col(&head, VoHead::VoId).eq(col(&rows, RelationshipTarget::SourceVoId)));
        let mut select = Query::select();
        select
            .distinct()
            .expr_as(col(&rows, RelationshipTarget::SourceVoId), Node::VoId)
            .expr_as(
                Expr::SubQuery(None, Box::new(content_key.into())),
                Node::SysVersion,
            )
            .from_as(RelationshipTarget::Table, Alias::new(rows.as_str()))
            .and_where(match targets.as_slice() {
                [one] => col(&rows, RelationshipTarget::TargetPartyId).eq(Expr::val(*one)),
                _ => col(&rows, RelationshipTarget::TargetPartyId)
                    .is_in(targets.iter().map(|target| Expr::val(*target))),
            })
            .and_where(col(&rows, RelationshipTarget::ValidTo).is_null());
        Some(ValueAnchor {
            alias,
            version,
            key,
            tie: None,
            select,
        })
    }

    /// The anchor of the equality of `leaf` with one of `texts`, or `None`
    /// when no value index holds the leaf ([`text_leaf`]). On the `value`
    /// member a text in plain decimal form also reads the number of an
    /// ordinal or a scale ([`plain_decimal`]).
    ///
    /// # Errors
    /// [`AqlError`] from the leaf step's predicate lowering.
    fn value_anchor(
        &mut self,
        leaf: &LeafPath,
        texts: &[String],
    ) -> Result<Option<ValueAnchor>, AqlError> {
        let kind = text_leaf(&leaf.fragment);
        let (function, column) = match kind {
            Some(TextLeaf::Code) => ("node_code", Node::ValueCode),
            Some(TextLeaf::Search | TextLeaf::Value) => ("node_search_text", Node::ValueText),
            None => return Ok(None),
        };
        // NOTE: our own design (#6090), a DV_ORDINAL's or DV_SCALE's `value` is a number
        // that equals a text only when the text is its plain decimal rendering.
        let numbers: Vec<&str> = if kind == Some(TextLeaf::Value) {
            texts
                .iter()
                .map(String::as_str)
                .filter(|text| plain_decimal(text))
                .collect()
        } else {
            Vec::new()
        };
        let Some((types, predicate)) = leaf_step(self.ir, leaf) else {
            return Ok(None);
        };
        if !types.contains("ELEMENT") {
            return Ok(None);
        }
        self.row_anchor(leaf, predicate, false, |node| {
            // NOTE: crate::storage::shared::UNSHARED, `value` is never shared, so the row's own
            // data is the whole value and the expression stays the index's (#6229).
            let read = |function: &str, column: Node| {
                call(
                    function,
                    vec![
                        col(node, Node::ValueType),
                        col(node, column),
                        col(node, Node::Data),
                    ],
                )
            };
            let search = read(function, column);
            let mut test = match texts {
                [text] => search.eq(Expr::val(text.as_str())),
                _ => search.is_in(texts.iter().map(|text| Expr::val(text.as_str()))),
            };
            if !numbers.is_empty() {
                let number = read("node_number", Node::ValueNum);
                let as_numeric =
                    |text: &str| Expr::cust_with_exprs("CAST($1 AS numeric)", [Expr::val(text)]);
                test = test.or(match numbers.as_slice() {
                    [one] => number.eq(as_numeric(one)),
                    _ => number.is_in(numbers.iter().map(|text| as_numeric(text))),
                });
            }
            (
                col(node, Node::RmType).eq(Expr::cust(ELEMENT_LITERAL)),
                test,
            )
        })
    }

    /// The anchor of the range comparisons of one leaf, or `None` when the
    /// leaf's step can bind a row that does not carry the value.
    ///
    /// `first` asks for the first row of each version the range keeps
    /// instead of the distinct versions ([`RangeEntry::Leading`]); the flat
    /// shape always takes it ([`RangeEntry::Free`]).
    ///
    /// # Errors
    /// [`AqlError`] from the leaf step's predicate lowering.
    fn range_anchor(
        &mut self,
        range: &Range<'_>,
        first: bool,
    ) -> Result<Option<ValueAnchor>, AqlError> {
        let Some((types, predicate)) = leaf_step(self.ir, range.leaf) else {
            return Ok(None);
        };
        if !range.ranged.binds(types) {
            return Ok(None);
        }
        let ranged = range.ranged;
        let first = first || !self.streaming;
        self.row_anchor(range.leaf, predicate, first, |node| {
            let tests = range
                .bounds
                .iter()
                .map(|(op, bound)| ranged.read(node).binary(binoper(*op), bound.clone()))
                .reduce(ExprTrait::and)
                .unwrap_or_else(|| Expr::val(true));
            (
                Expr::cust_with_exprs(
                    format!("$1 IN ({})", ranged.literals()),
                    [col(node, Node::RmType)],
                ),
                tests,
            )
        })
    }

    /// The versions some node row of which satisfies, under `predicate`, the
    /// two conditions `of` states of a row, its RM type and the test of its
    /// value, as an anchor of `leaf`'s versioned object; `None` when the
    /// leaf's source has no version row.
    ///
    /// With `first` the item is the first such row of each version, found
    /// by the absence of an earlier one: a scan toward a LIMIT reads it row
    /// by row, and the planner pulls it up into a node row it can enter from
    /// the value index or probe per version ([`RangeEntry::Free`]).
    /// Without it, the distinct versions.
    ///
    /// # Errors
    /// [`AqlError`] from the predicate lowering.
    fn row_anchor(
        &mut self,
        leaf: &LeafPath,
        predicate: Option<&ir::NodeConstraint>,
        first: bool,
        of: impl Fn(&str) -> (Expr, Expr),
    ) -> Result<Option<ValueAnchor>, AqlError> {
        let Some(version) = self.version_alias_of(leaf.source.0) else {
            return Ok(None);
        };
        let key = self.node_key_of(&version);
        let alias = format!("va{}", self.next_ctr());
        let node = format!("{alias}_n");
        let mut select = self.anchor_rows(&node, predicate, &of, false)?;
        select
            .column((Alias::new(node.as_str()), Node::VoId))
            .column((Alias::new(node.as_str()), Node::SysVersion));
        let flat = first && !self.streaming;
        // A leaf the SELECT already binds is read off that one row, so the
        // flat item is tied to it: one item row a statement row, no probe.
        let bound = if flat {
            let (start, rest) = self.walk_start(leaf)?;
            rest.is_empty().then_some(start)
        } else {
            None
        };
        let mut tie = None;
        if let Some(start) = &bound {
            select.column((Alias::new(node.as_str()), Node::Num));
            tie = Some(
                [Node::VoId, Node::SysVersion, Node::Num]
                    .into_iter()
                    .map(|column| col(&alias, column).eq(col(start, column)))
                    .reduce(ExprTrait::and)
                    .unwrap_or_else(|| Expr::val(true)),
            );
        } else if first {
            // In the flat shape the probe joins the statement beside the
            // item, correlated to the leaf's source row too, so it runs only
            // for the rows the containment keeps.
            let row = if flat { alias.as_str() } else { node.as_str() };
            let earlier = format!("{alias}_e");
            let mut before = self.anchor_rows(&earlier, predicate, &of, true)?;
            before
                .expr(Expr::val(1))
                .and_where(col(&earlier, Node::VoId).eq(col(row, Node::VoId)))
                .and_where(col(&earlier, Node::SysVersion).eq(col(row, Node::SysVersion)))
                .and_where(col(&earlier, Node::Num).lt(col(row, Node::Num)))
                // The fence keeps the probe correlated: without it the planner
                // makes it an anti-join over every row the range keeps.
                .offset(0);
            if flat {
                if let Some(source) = self.node_alias.get(&leaf.source.0) {
                    before.and_where(col(source, Node::VoId).eq(col(row, Node::VoId)));
                }
                select.column((Alias::new(node.as_str()), Node::Num));
                tie = Some(Expr::exists(before).not());
            } else {
                select.and_where(Expr::exists(before).not());
            }
        } else {
            select.distinct();
        }
        Ok(Some(ValueAnchor {
            alias,
            version,
            key,
            tie,
            select,
        }))
    }

    /// The hot node rows `node` that satisfy `of` under `predicate`, with no
    /// column selected yet.
    ///
    /// `probe` states each condition but the tier as `(…) IS TRUE`, which
    /// holds exactly when the condition does and which no index matches, so
    /// the earlier-row probe of a version reads that version's rows by
    /// `node_hot_pkey` whatever the statistics say. On statistics that held
    /// none of the rows, the probe entered through the value index instead,
    /// once per candidate row (#6243).
    fn anchor_rows(
        &self,
        node: &str,
        predicate: Option<&ir::NodeConstraint>,
        of: &impl Fn(&str) -> (Expr, Expr),
        probe: bool,
    ) -> Result<SelectStatement, AqlError> {
        let fence = |cond: Expr| {
            if probe {
                Expr::cust_with_exprs("($1) IS TRUE", [cond])
            } else {
                cond
            }
        };
        let mut select = Query::select();
        select.from_as(Node::Table, Alias::new(node));
        select.and_where(hot(node, Node::Tier));
        let (of_type, test) = of(node);
        select.and_where(fence(of_type));
        if let Some(predicate) = predicate {
            for cond in self.node_constraint_conds(node, predicate)? {
                select.and_where(fence(cond));
            }
        }
        select.and_where(fence(test));
        Ok(select)
    }
}

/// The RM types and the predicate of the node step `leaf`'s fragment is read
/// from: its last structure step, or its source.
fn leaf_step<'a>(
    ir: &'a ir::QueryIr,
    leaf: &'a LeafPath,
) -> Option<(&'a ir::TypeSet, Option<&'a ir::NodeConstraint>)> {
    match leaf.anchor.last() {
        Some(step) => Some((&step.node_types, step.predicate.as_ref())),
        None => match ir.sources.get(leaf.source.0) {
            Some(ir::Source::Rm(source)) => Some((&source.rm_type, None)),
            _ => None,
        },
    }
}

/// What a comparison of `leaf` under `op` and `coercion` reads of a node row
/// an index holds, or `None` for any other comparison.
fn ranged(leaf: &LeafPath, op: CompOp, coercion: Coercion) -> Option<Ranged> {
    if op == CompOp::Ne || leaf.fragment.iter().any(|step| step.predicate.is_some()) {
        return None;
    }
    let names: Vec<&str> = leaf.fragment.iter().map(|s| s.name.as_str()).collect();
    match (coercion, names.as_slice()) {
        (Coercion::Temporal, ["time", "value"]) => Some(Ranged::EventTime),
        (Coercion::Temporal, ["origin", "value"]) => Some(Ranged::Origin),
        (Coercion::Magnitude, ["value", "magnitude"]) if op != CompOp::Eq => {
            Some(Ranged::Magnitude)
        }
        _ => None,
    }
}

/// One anchor ([`Builder::value_anchors`]): the versions some node row of
/// which carries a value, as a FROM item linked to the version row of the
/// leaf's versioned object.
pub(super) struct ValueAnchor {
    /// The item's alias.
    pub(super) alias: String,
    /// The `version` alias the item links to.
    pub(super) version: String,
    /// The column the item's `sys_version` joins: the node key of the version
    /// row under its scope (`head::node_key`, #4944).
    key: Expr,
    /// What makes a flat item one row a statement row: its row is the row
    /// the SELECT binds the leaf to, or no earlier row of its version is one
    /// the item would also keep, the first row of each version.
    tie: Option<Expr>,
    /// The item: `SELECT DISTINCT vo_id, sys_version FROM node …`, or the
    /// first row of each version for a range.
    pub(super) select: SelectStatement,
}

impl ValueAnchor {
    /// The join condition between the item and its version row.
    pub(super) fn link(&self) -> Expr {
        let link = col(&self.alias, Node::VoId)
            .eq(col(&self.version, VersionRow::VoId))
            .and(col(&self.alias, Node::SysVersion).eq(self.key.clone()));
        match &self.tie {
            Some(tie) => link.and(tie.clone()),
            None => link,
        }
    }
}

/// Which value index an equality of a fragment path compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextLeaf {
    /// The code `ext.node_code` yields: the `defining_code` of a coded text,
    /// or of the symbol of an ordinal or a scale.
    Code,
    /// The search text `ext.node_search_text` yields: a `DV_IDENTIFIER` id,
    /// a `DV_QUANTITY`'s units or a symbol's value.
    Search,
    /// The `value` member, which is the search text of every value but a
    /// `DV_ORDINAL` or a `DV_SCALE`, whose `value` is the number
    /// `ext.node_number` yields.
    Value,
}

/// The value index an equality of the fragment path `steps` compares
/// against, for every `ELEMENT` row that can satisfy it; `None` for a path
/// no value index holds.
fn text_leaf(steps: &[FragmentStep]) -> Option<TextLeaf> {
    if steps.iter().any(|step| step.predicate.is_some()) {
        return None;
    }
    let names: Vec<&str> = steps.iter().map(|step| step.name.as_str()).collect();
    match names.as_slice() {
        ["value", "defining_code", "code_string"]
        | ["value", "symbol", "defining_code", "code_string"] => Some(TextLeaf::Code),
        ["value", "id" | "units"] | ["value", "symbol", "value"] => Some(TextLeaf::Search),
        ["value", "value"] => Some(TextLeaf::Value),
        _ => None,
    }
}

/// Whether `text` is how a number renders as text, `-?digits[.digits]`: a
/// `numeric` and a JSON number both render in plain notation (PostgreSQL 18,
/// "Numeric Types"), so a text of any other form equals no number.
fn plain_decimal(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, None), |(whole, fraction)| {
            (whole, Some(fraction))
        });
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    digits(whole) && fraction.is_none_or(digits)
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
        builder.anchors_an_unbound_leaf()
    }

    /// An anchor on a leaf only the WHERE clause reads fences the contained
    /// nodes; one on a leaf a SELECT column binds, its source's own value, or
    /// no anchor at all leaves them their archetype-index entry (#6263).
    #[test]
    fn only_an_anchor_on_a_where_only_leaf_fences_the_contained_nodes() {
        let from = "FROM EHR e CONTAINS COMPOSITION c \
                    CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.timed.v1]";
        let leaf = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/magnitude";
        let events = "o/data[at0001]/events[at0002]";
        for (aql, fenced) in [
            (
                format!("SELECT c/uid/value {from} WHERE {leaf} >= 140"),
                true,
            ),
            (
                format!("SELECT c/uid/value, {leaf} {from} WHERE {leaf} >= 140"),
                false,
            ),
            (
                format!(
                    "SELECT c/uid/value, {events}/data[at0003]/items[at0004]/value/units {from} WHERE {leaf} >= 140"
                ),
                false,
            ),
            (
                format!("SELECT {events}/time/value {from} WHERE {leaf} >= 140"),
                true,
            ),
            (
                format!("SELECT count({leaf}) {from} WHERE {leaf} >= 140"),
                true,
            ),
            (format!("SELECT c/uid/value {from} WHERE {leaf} = 7"), false),
            (format!("SELECT c/uid/value {from}"), false),
            (
                "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS ELEMENT s[at0004] \
                 WHERE s/value/magnitude > 7"
                    .to_owned(),
                false,
            ),
        ] {
            assert_eq!(fences(&aql), fenced, "{aql}");
        }
    }

    /// The walk that decides whether the flat FROM reads its contained nodes
    /// inside their objects finds exactly the conjuncts the anchor builder
    /// makes anchors of (#6263).
    #[test]
    fn the_walk_and_the_anchor_builder_agree() {
        let observation = "FROM EHR e CONTAINS COMPOSITION c \
                           CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.timed.v1]";
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
            let aql = format!("SELECT c/uid/value {observation} WHERE {filter}");
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
