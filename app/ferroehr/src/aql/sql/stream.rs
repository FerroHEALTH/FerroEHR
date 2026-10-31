// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The streaming FROM shape for a LIMIT-bearing, unordered query: the
//! eligible linear containment chain ([`streaming_plan`]) and its lowering
//! onto the version spine ([`Builder::build_from_streaming`]), with every
//! node source a `LATERAL` off that spine. No openEHR spec governs the join
//! mechanics: our own design.

use sea_query::{Alias, Asterisk, Expr, ExprTrait as _, JoinType, Query};

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::{
    AggFunc, Contained, ContainsTree, LeafPath, Link, PathTarget, QueryIr, RmSource, SelectValue,
    Source,
};
use crate::db::iden::{Ehr, Node, VersionRow};

use super::Builder;
use super::expr::{col, hot, hot_unaliased};
use super::from::{is_vo_root_type, strict_descendant};

/// The streaming-shape plan: the linear containment chain eligible for the
/// LIMIT-streaming FROM (see [`Builder::build_from_streaming`]).
#[derive(Debug)]
pub(super) struct StreamPlan {
    /// The enclosing `EHR` source, when present.
    ehr: Option<usize>,
    /// The single versioned-object-root RM source.
    root: usize,
    /// Contained content sources as `(source id, parent source id)`,
    /// pre-order.
    contained: Vec<(usize, usize)>,
}

impl StreamPlan {
    /// Whether the query only counts the root objects of this chain: every
    /// SELECT column is `COUNT(*)` or a plain `COUNT` of the whole root, and
    /// nothing is contained below the root. Such a count reads the version
    /// spine alone (QUERY master03 §COUNT), which the flat shape cannot
    /// skip.
    pub(super) fn counts_roots(&self, ir: &QueryIr) -> bool {
        self.contained.is_empty()
            && !ir.distinct
            && !ir.select.is_empty()
            && ir.select.iter().all(|column| match &column.value {
                SelectValue::Aggregate {
                    func: AggFunc::Count,
                    arg: None,
                    ..
                } => true,
                SelectValue::Aggregate {
                    func: AggFunc::Count,
                    arg: Some(target),
                    distinct: false,
                } => is_whole_root_object(target, self.root),
                _ => false,
            })
    }
}

/// Extract the streaming-eligible containment chain, or `None` when the
/// tree needs the general flat shape: `OR`/`NOT CONTAINS` branches,
/// `VERSION` sources, a bare `EHR` with no versioned-object root, nested
/// versioned-object roots (a second version group), or an untyped root.
pub(super) fn streaming_plan(ir: &QueryIr) -> Option<StreamPlan> {
    let (ehr, root_tree) = match &ir.contains {
        ContainsTree::Operand { source, contained } => match ir.sources.get(source.0) {
            Some(Source::Ehr(_)) => match contained.as_deref() {
                Some(Contained {
                    link: Link::Contains,
                    tree,
                }) => (Some(source.0), tree),
                _ => return None,
            },
            Some(Source::Rm(_)) => (None, &ir.contains),
            Some(Source::Version(_)) | None => return None,
        },
        _ => return None,
    };
    let ContainsTree::Operand { source, contained } = root_tree else {
        return None;
    };
    let root = source.0;
    let Some(Source::Rm(r)) = ir.sources.get(root) else {
        return None;
    };
    if r.rm_type.is_empty() || !r.rm_type.names().iter().all(|t| is_vo_root_type(t)) {
        return None;
    }
    // A FOLDER root is ineligible: the streaming shape binds the root at
    // `num = 0`, but `EHR CONTAINS FOLDER` matches every folder node
    // (containment is transitive — QUERY master03 §Containment), which only
    // the flat shape's whole-table scan serves.
    if r.rm_type.names().iter().any(|t| t == "FOLDER") {
        return None;
    }
    let mut plan = StreamPlan {
        ehr,
        root,
        contained: Vec::new(),
    };
    if let Some(c) = contained.as_deref() {
        if !matches!(c.link, Link::Contains) {
            return None;
        }
        if !collect_contained(ir, &c.tree, root, &mut plan.contained) {
            return None;
        }
    }
    Some(plan)
}

/// Collect the contained content sources of a streaming chain (pre-order),
/// or `false` when the subtree is ineligible.
fn collect_contained(
    ir: &QueryIr,
    tree: &ContainsTree,
    parent: usize,
    out: &mut Vec<(usize, usize)>,
) -> bool {
    match tree {
        ContainsTree::And(a, b) => {
            collect_contained(ir, a, parent, out) && collect_contained(ir, b, parent, out)
        }
        ContainsTree::Or(..) => false,
        ContainsTree::Operand { source, contained } => {
            let sid = source.0;
            let Some(Source::Rm(r)) = ir.sources.get(sid) else {
                return false;
            };
            // A nested versioned-object root would open its own version
            // group — the flat shape owns that case.
            if r.rm_type.names().iter().any(|t| is_vo_root_type(t)) {
                return false;
            }
            out.push((sid, parent));
            match contained.as_deref() {
                None => true,
                Some(Contained {
                    link: Link::Contains,
                    tree,
                }) => collect_contained(ir, tree, sid, out),
                Some(_) => false,
            }
        }
    }
}

/// Whether an identified path must read the streaming ROOT source's node row.
/// The one root-sourced shape the version spine serves without the node row is
/// the direct `uid/value` synthesis (`version_uid_expr`: no structure hop, no
/// predicates — the `OBJECT_VERSION_ID` is composed from `version` columns per
/// RM common master06 §Version Identification). Everything else — promoted
/// columns, fragment reads, whole-object projection (empty fragment), any
/// predicate along the path — reads `node`. The bare-`uid` object projection is
/// conservatively counted as needing the node row (its non-projection uses fall
/// through to the fragment).
fn leaf_needs_root_node(leaf: &LeafPath, root: usize) -> bool {
    if leaf.source.0 != root {
        return false;
    }
    if leaf.anchor.is_empty()
        && leaf.root_predicate.is_none()
        && leaf.fragment.iter().all(|s| s.predicate.is_none())
    {
        let names: Vec<&str> = leaf.fragment.iter().map(|s| s.name.as_str()).collect();
        if names.as_slice() == ["uid", "value"] {
            return false;
        }
    }
    true
}

fn path_needs_root_node(target: &PathTarget, root: usize) -> bool {
    match target {
        PathTarget::Data(leaf) => leaf_needs_root_node(leaf, root),
        // Version metadata reads `version`; EHR fields read `ehr`; an
        // `e/ehr_status/...` path joins its OWN EHR_STATUS root node (its
        // leaf is rooted at the EHR source, never the streaming root).
        PathTarget::Version { .. } | PathTarget::Ehr { .. } | PathTarget::EhrStatus(_) => false,
    }
}

/// Whether `target` is the whole streaming root object itself (`c` in
/// `COUNT(c)`): no structure hop, no fragment, no predicate.
pub(super) fn is_whole_root_object(target: &PathTarget, root: usize) -> bool {
    matches!(target, PathTarget::Data(leaf)
        if leaf.source.0 == root
            && leaf.anchor.is_empty()
            && leaf.root_predicate.is_none()
            && leaf.is_whole_object())
}

/// Whether ANY identified path in the query (SELECT, WHERE, ORDER BY) reads
/// the streaming root's node row — the projection/predicate half of the
/// dead-root-lateral test (`build_from_streaming`).
///
/// A plain `COUNT` of the whole root object reads no node: it asks only
/// whether the object is there (QUERY master03 §COUNT), which the spine row
/// of a version that is not logically deleted already answers.
fn root_node_referenced(ir: &QueryIr, root: usize) -> bool {
    let mut needed = false;
    for column in &ir.select {
        match &column.value {
            SelectValue::Aggregate {
                func: AggFunc::Count,
                arg: Some(target),
                distinct: false,
            } if is_whole_root_object(target, root) => {}
            value => value.for_each_path(&mut |t| needed |= path_needs_root_node(t, root)),
        }
    }
    if let Some(filter) = &ir.filter {
        filter.for_each_path(&mut |t| needed |= path_needs_root_node(t, root));
    }
    for key in &ir.order_by {
        needed |= path_needs_root_node(&key.path, root);
    }
    needed
}

impl Builder<'_> {
    /// Emit the STREAMING FROM shape for a LIMIT-bearing, unordered query:
    /// the version spine (`version`) is the single FROM item and every
    /// node source hangs off it as a `LATERAL` subquery (`OFFSET 0` as the
    /// planner's documented pull-up fence), so `PostgreSQL` walks current
    /// versions lazily, probes each through the nested-set indexes, and
    /// stops at the LIMIT — instead of materializing an archetype-anchor
    /// bitmap over the whole corpus first (the measured 1M-entry/1 s
    /// failure on non-selective anchors).
    ///
    /// Semantics are identical to the flat shape: each `LATERAL` returns
    /// ALL matching nodes per outer row, so row-per-binding multiplicity
    /// is preserved (QUERY master03 §DISTINCT — duplicates exist by
    /// default); a logically deleted version, which the flat shape drops for
    /// having no node rows, is dropped here too ([`Self::stream_root`]); and
    /// QUERY master03 §LIMIT makes WHICH rows return explicitly
    /// non-deterministic without `ORDER BY`, so the changed visit order is
    /// conformant. No openEHR spec governs the join mechanics — the shape
    /// selection is our own design.
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] when the plan names a source of the wrong
    /// kind (a planner defect), plus the per-source condition lowerings.
    pub(super) fn build_from_streaming(&mut self, plan: &StreamPlan) -> Result<(), AqlError> {
        self.streaming = true;
        let Some(Source::Rm(root)) = self.ir.sources.get(plan.root).cloned() else {
            return Err(SqlError::Unsupported(
                "streaming plan root is not an RM source".to_owned(),
            )
            .into());
        };

        // The version spine: the one FROM item everything joins onto.
        let v = format!("v{}", plan.root);
        self.vo_alias.insert(plan.root, v.clone());
        self.stream_spine(&v)?;
        self.q.and_where(hot(&v, VersionRow::Tier));
        let kinds: Vec<String> = root.rm_type.names().to_vec();
        self.q.and_where(col(&v, VersionRow::Kind).is_in(kinds));
        self.push_scope(&v, &root.scope)?;

        if let Some(esid) = plan.ehr {
            let e = format!("e{esid}");
            self.q.join_as(
                JoinType::Join,
                Ehr::Table,
                Alias::new(e.as_str()),
                col(&e, Ehr::Id).eq(col(&v, VersionRow::EhrId)),
            );
            let Some(Source::Ehr(src)) = self.ir.sources.get(esid).cloned() else {
                return Err(SqlError::Unsupported(
                    "streaming plan EHR is not an EHR source".to_owned(),
                )
                .into());
            };
            for p in &src.predicates {
                self.push_ehr_predicate(&e, p)?;
            }
            self.ehr_alias.insert(esid, e);
        }

        let group_root = self.stream_root(plan, &root, &v)?;
        self.group_roots.push(group_root.clone());
        self.group_vos.push(v.clone());
        if plan.ehr.is_some() {
            self.roots_linked_to_ehr.insert(group_root);
        }

        // Contained content sources: one LATERAL per source, interval-bound
        // into its parent's subtree. A first-level source under a dead root
        // (no node alias) binds directly on the spine's version instead.
        for (sid, parent) in &plan.contained {
            let parent_alias = self.node_alias.get(parent).cloned();
            if parent_alias.is_none() && *parent != plan.root {
                return Err(SqlError::Unsupported(
                    "streaming plan parent without an alias".to_owned(),
                )
                .into());
            }
            let Some(Source::Rm(r)) = self.ir.sources.get(*sid).cloned() else {
                return Err(SqlError::Unsupported(
                    "streaming plan content is not an RM source".to_owned(),
                )
                .into());
            };
            let alias = format!("n{sid}");
            let inner = format!("{alias}_s");
            let mut sub = Query::select();
            sub.column((Alias::new(inner.as_str()), Asterisk))
                .from_as(Node::Table, Alias::new(inner.as_str()))
                .offset(0);
            sub.and_where(hot(&inner, Node::Tier));
            if let Some(p) = &parent_alias {
                sub.and_where(strict_descendant(&inner, p));
            } else {
                // Dead-root binding: the root's subtree is the whole
                // versioned object, so strict containment in it is version
                // membership minus the root row itself (`num = 0`).
                sub.and_where(col(&inner, Node::VoId).eq(col(&v, VersionRow::VoId)))
                    .and_where(col(&inner, Node::SysVersion).eq(col(&v, VersionRow::SysVersion)))
                    .and_where(col(&inner, Node::Num).gt(0));
            }
            self.q.join_lateral(
                JoinType::Join,
                sub,
                Alias::new(alias.as_str()),
                Expr::val(true),
            );
            for cond in self.rm_conds(&alias, &r)? {
                self.q.and_where(cond);
            }
            self.node_alias.insert(*sid, alias);
        }
        Ok(())
    }

    /// Emits the version spine `v`, the one FROM item everything joins onto,
    /// behind the value anchors of the WHERE clause when it has any
    /// ([`Builder::value_anchors`]): an anchor leads the chain, so the spine
    /// is probed by the anchor's versions even when the chain is too long for
    /// the planner to reorder it.
    ///
    /// # Errors
    /// [`AqlError`] from the anchors' predicate lowering.
    fn stream_spine(&mut self, v: &str) -> Result<(), AqlError> {
        let mut anchors = self.value_anchors()?.into_iter();
        match anchors.next() {
            Some(first) => {
                let link = first.link();
                self.q
                    .from_subquery(first.select, Alias::new(first.alias.as_str()));
                self.q
                    .join_as(JoinType::Join, VersionRow::Table, Alias::new(v), link);
            }
            None => {
                self.q.from_as(VersionRow::Table, Alias::new(v));
            }
        }
        for anchor in anchors {
            let link = anchor.link();
            self.q.join_subquery(
                JoinType::Join,
                anchor.select,
                Alias::new(anchor.alias.as_str()),
                link,
            );
        }
        Ok(())
    }

    /// Emits the streaming root's node lateral, or elides it, and returns the
    /// alias the scope/gate machinery group-tracks.
    ///
    /// The lateral is DEAD WEIGHT when (a) the root source carries no
    /// node-level condition beyond its RM type — which the spine's `kind`
    /// filter already pins: a versioned object's `num = 0` row has
    /// `rm_type = kind` by construction — and (b) no identified path reads the
    /// root's node row ([`root_node_referenced`]). It is then one wasted
    /// `pk_node` probe per spine row, and the root's subtree interval spans the
    /// whole versioned object, so first-level containment reduces to sharing
    /// the version.
    ///
    /// The probe's other effect is kept without it: a version with no root
    /// node contributes no row. Exactly the logically deleted versions have
    /// none — a deletion commits a version whose data is Void (RM common
    /// master06 §Logical Deletion) with lifecycle state `523`, and every other
    /// commit decomposes a body into at least its root row — so the elided
    /// shape filters the spine on `lifecycle_state <> '523'`, and both shapes
    /// answer the same rows. No openEHR spec governs the join mechanics — our
    /// own design.
    ///
    /// Returns the node alias, or the spine when elided (it carries the
    /// `ehr_id` those gates filter on).
    ///
    /// # Errors
    /// [`AqlError`] from the root's condition lowerings.
    fn stream_root(
        &mut self,
        plan: &StreamPlan,
        root: &RmSource,
        v: &str,
    ) -> Result<String, AqlError> {
        let root_node_needed = root.archetype.is_some()
            || root.name.is_some()
            || !root.standard.is_empty()
            || root_node_referenced(self.ir, plan.root);
        if !root_node_needed {
            self.q.and_where(
                col(v, VersionRow::LifecycleState)
                    .ne(Expr::val(crate::versioning::lifecycle::state::DELETED)),
            );
            return Ok(v.to_owned());
        }
        let root_alias = format!("n{}", plan.root);
        let mut sub = Query::select();
        sub.column(Asterisk)
            .from(Node::Table)
            .and_where(hot_unaliased())
            .and_where(Expr::col(Node::VoId).eq(col(v, VersionRow::VoId)))
            .and_where(Expr::col(Node::SysVersion).eq(col(v, VersionRow::SysVersion)))
            .and_where(Expr::col(Node::Num).eq(Expr::val(0)))
            .offset(0);
        self.q.join_lateral(
            JoinType::Join,
            sub,
            Alias::new(root_alias.as_str()),
            Expr::val(true),
        );
        for cond in self.rm_conds(&root_alias, root)? {
            self.q.and_where(cond);
        }
        self.node_alias.insert(plan.root, root_alias.clone());
        Ok(root_alias)
    }
}
