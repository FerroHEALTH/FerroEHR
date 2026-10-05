// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FROM-clause + containment lowering (QUERY master03 §Class expressions,
//! §Containment, §VERSION sources) and the population / scope gates.
//!
//! No openEHR spec governs the join mechanics — this is our own design: the FROM containment tree becomes a **cross
//! join of table aliases + typed WHERE conditions** (the planner folds
//! cross-join+filter into joins). Each RM source that roots a versioned object
//! gets a `node` + `version` (+ `audit`) alias; content sources contained
//! within it share the `version` and interval-join
//! (`num BETWEEN a.num AND a.num_cap`, same `(vo_id, sys_version)`). `EHR`
//! sources join VO roots via `ehr_id`; `VERSION` sources share the contained
//! VO's `version`. `AND` containment is a join; `OR` and `NOT CONTAINS` are
//! disjunctive / anti-join correlated `EXISTS` filters (QUERY master03
//! §Containment — boolean `AND`/`OR`, `NOT`).

use sea_query::extension::postgres::PgFunc;
use sea_query::{Alias, Asterisk, Expr, ExprTrait as _, JoinType, Query, SelectStatement};
use uuid::Uuid;

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::{
    Contained, ContainsTree, EhrField, EhrPredicate, LeafPath, Link, Operand, PathTarget, QueryIr,
    RmSource, SelectValue, Source, VersionField, VersionScope,
};
use crate::db::iden::{CommitAudit, Ehr, Node, VersionRow, VoHead};

use super::expr::{cast, col, hot, hot_unaliased, type_cond};
use super::value::version_field_expr;
use super::{Builder, EhrAccessScope, VoGroup};

/// The correlation anchor for an `OR` / `NOT CONTAINS` `EXISTS` subquery.
enum ExistsAnchor {
    /// Interval-anchored inside a node subtree of a shared versioned object:
    /// `num BETWEEN parent.num AND parent.num_cap`, same `(vo_id, sys_version)`.
    /// Carries the anchor operand's resolved types so the edge classifier can
    /// decide the join shape.
    Vo(String, crate::aql::ir::TypeSet),
    /// Contained in an EHR as its own versioned object: `ehr_id` join + version
    /// scope.
    Ehr(String),
}

/// How a `CONTAINS` edge renders in SQL, decided from the RM relationship
/// between the parent and child operand types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeKind {
    /// Content inside the parent's own node subtree: same `(vo_id,
    /// sys_version)`, inclusive interval join. The child shares the parent's
    /// version group.
    Structural,
    /// `FOLDER CONTAINS <versioned object>`: reference resolution through
    /// `FOLDER.items` `OBJECT_REF`s, transitively over the folder subtree (RM
    /// common master05 — folders hold *references* to versioned objects; RM
    /// ehr master04 §Folders). The child opens its own version group.
    FolderItems,
    /// `FOLDER CONTAINS FOLDER`: the union of the by-value `folders` nesting
    /// (a STRICT descendant interval — the inclusive join would self-match
    /// every folder row) and an items-referenced `VERSIONED_FOLDER`'s folder
    /// rows (RM common master05: `items` references name versioned objects
    /// "logically in this folder", with no target-type restriction — the same
    /// ground as `FolderItems`, one reference hop). The child opens its own
    /// version group either way.
    FolderChild,
}

/// Classifies a `CONTAINS` edge, or refuses a pair the RM defines no
/// containment for.
fn classify_edge(
    parent: &crate::aql::ir::TypeSet,
    child: &crate::aql::ir::TypeSet,
) -> Result<EdgeKind, AqlError> {
    let child_is_vo_root = !child.is_empty() && child.names().iter().all(|t| is_vo_root_type(t));
    if !child_is_vo_root {
        return Ok(EdgeKind::Structural);
    }
    let parent_all_folder = !parent.is_empty() && parent.names().iter().all(|t| t == "FOLDER");
    let child_all_folder = child.names().iter().all(|t| t == "FOLDER");
    let child_no_folder = !child.names().iter().any(|t| t == "FOLDER");
    match (parent_all_folder, child_all_folder, child_no_folder) {
        (true, true, _) => Ok(EdgeKind::FolderChild),
        (true, _, true) => Ok(EdgeKind::FolderItems),
        _ => Err(crate::aql::error::AqlFeatureError::UnsupportedContainment(
            parent.names().join("|"),
            child.names().join("|"),
        )
        .into()),
    }
}

/// The `FOLDER.items` reference-resolution correlation: some folder
/// row in the parent's subtree (the parent row itself included — transitive
/// containment) carries an `items` `OBJECT_REF` whose uid root equals the
/// child's versioned-object id. The ref uid is compared as TEXT against the
/// child's `vo_id`, taking the segment before `::` (an `OBJECT_VERSION_ID`
/// still names its object root — the same resolution the directory commit
/// validator performs), so a malformed or foreign ref yields no match rather
/// than a cast error.
fn folder_items_exists(parent_node: &str, child_node: &str) -> Expr {
    let sf = format!("{parent_node}_items_sf");
    let mut sub = Query::select();
    sub.expr(Expr::val(1));
    sub.from_as(Node::Table, Alias::new(sf.as_str()));
    sub.and_where(hot(&sf, Node::Tier));
    sub.and_where(col(&sf, Node::VoId).eq(col(parent_node, Node::VoId)));
    sub.and_where(col(&sf, Node::SysVersion).eq(col(parent_node, Node::SysVersion)));
    sub.and_where(
        col(&sf, Node::Num).between(col(parent_node, Node::Num), col(parent_node, Node::NumCap)),
    );
    sub.and_where(col(&sf, Node::RmType).eq(Expr::val("FOLDER")));
    sub.and_where(Expr::cust_with_exprs(
        "EXISTS (SELECT 1 FROM jsonb_array_elements(COALESCE($1 -> 'items', '[]'::jsonb)) AS item \
         WHERE split_part(item.value #>> '{id,value}', '::', 1) = ($2)::text)",
        [col(&sf, Node::Data), col(child_node, Node::VoId)],
    ));
    Expr::exists(sub)
}

/// Bind the head row of the object `version_alias` belongs to into `q`, and
/// return its alias.
///
/// One join serves two predicates that used to be two correlated `EXISTS`
/// subqueries. The restriction gate: GDPR Art. 18(2) leaves storage as the
/// only processing a restricted object admits (`docs/law/eu/gdpr/text.html`),
/// and answering a query from its content is processing, so the mark is a
/// predicate on the head join rather than a post-filter — a restricted object
/// contributes no row to any result set, under `LATEST_VERSION` and
/// `ALL_VERSIONS` alike (`RESULT_SET` has no per-row diagnostic channel, QUERY
/// master01 §Result Set; a point read of the same object answers `403`). And,
/// when the caller asks for it, `LATEST_VERSION`: the version row is the
/// object's current TRUNK head, `vo_head.trunk_head_sys_version` IS
/// `latest_trunk_version` (RM common master06 §The 'Virtual Version Tree').
///
/// Written as a join on the head's primary key, the version row is reached
/// through `pk_version` with all three columns bound; the `EXISTS` form let the
/// planner reach it by `vo_id` alone through the at-time index and walk every
/// trunk version of the object per node row (#3453). The `branch_number = 0`
/// predicate is implied by the head and deliberately not written: it is what
/// made that partial index applicable. No openEHR spec governs restriction of
/// processing or storage access paths — our own design/extension.
fn bind_head(q: &mut SelectStatement, version_alias: &str, latest: bool) -> String {
    let h = format!("{version_alias}_head");
    q.from_as(VoHead::Table, Alias::new(h.as_str()));
    q.and_where(col(&h, VoHead::VoId).eq(col(version_alias, VersionRow::VoId)));
    q.and_where(col(&h, VoHead::RestrictedAt).is_null());
    if latest {
        q.and_where(
            col(version_alias, VersionRow::SysVersion).eq(col(&h, VoHead::TrunkHeadSysVersion)),
        );
    }
    h
}

/// The research objection is not standing on this `ehr` alias.
///
/// GDPR Art. 21(6) gives the subject a right to object to processing "for
/// scientific or historical research purposes or statistical purposes pursuant
/// to Article 89(1)" unless the controller has a public-interest ground
/// (`docs/law/eu/gdpr/text.html`), and a full-population query is the surface
/// a secondary-use consumer reads this repository through. So an objected EHR
/// leaves the population, while a query that names its `ehr_id` — a clinician
/// reading the record for care — is untouched, because the objection reaches
/// research processing and not the care record. The controller's recorded
/// override is the second column: while `research_objection_ground` is set,
/// the EHR is in the population again and the ground says on whose authority.
/// No openEHR spec governs the objection — our own design/extension.
fn research_objection_clear(ehr_alias: &str) -> Expr {
    col(ehr_alias, Ehr::ResearchObjectedAt).is_null().or(col(
        ehr_alias,
        Ehr::ResearchObjectionGround,
    )
    .is_not_null())
}

/// Version-at-time: the version row is the TRUNK version in force at `at`.
///
/// Validity is derived rather than stored: version *i* is valid over
/// `[committed_at_i, committed_at_i+1)`, so the row in force at an instant is
/// the trunk row with the greatest `committed_at` at or before it (RM common
/// master06 §Committal and Audits gives `committed_at` its meaning; the
/// derivation is our own storage design).
fn trunk_at_instant(version_alias: &str, at: Expr) -> Expr {
    let s = format!("{version_alias}_at");
    let mut sub = Query::select();
    sub.expr(Expr::val(1));
    sub.from_as(VersionRow::Table, Alias::new(s.as_str()));
    sub.and_where(hot(&s, VersionRow::Tier));
    sub.and_where(col(&s, VersionRow::VoId).eq(col(version_alias, VersionRow::VoId)));
    sub.and_where(col(&s, VersionRow::BranchNumber).eq(Expr::val(0)));
    sub.and_where(col(&s, VersionRow::CommittedAt).gt(col(version_alias, VersionRow::CommittedAt)));
    sub.and_where(col(&s, VersionRow::CommittedAt).lte(at.clone()));
    col(version_alias, VersionRow::CommittedAt)
        .lte(at)
        .and(Expr::exists(sub).not())
}

/// The by-value half of the `FOLDER CONTAINS FOLDER` union edge: the
/// child row is a STRICT nested-set descendant of the parent row inside the
/// same versioned folder tree (same `(vo_id, sys_version)`; strict, because
/// the inclusive interval would self-match every folder row).
fn folder_by_value_child(parent_node: &str, child_node: &str) -> Expr {
    col(child_node, Node::VoId)
        .eq(col(parent_node, Node::VoId))
        .and(col(child_node, Node::SysVersion).eq(col(parent_node, Node::SysVersion)))
        .and(col(child_node, Node::Num).gt(col(parent_node, Node::Num)))
        .and(col(child_node, Node::Num).lte(col(parent_node, Node::NumCap)))
}

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

fn operand_needs_root_node(operand: &Operand, root: usize) -> bool {
    match operand {
        Operand::Path(t) => path_needs_root_node(t, root),
        Operand::Function { args, .. } => args.iter().any(|a| operand_needs_root_node(a, root)),
        Operand::Literal(_) | Operand::Param(_) => false,
    }
}

fn filter_needs_root_node(expr: &crate::aql::ir::Expr, root: usize) -> bool {
    match expr {
        crate::aql::ir::Expr::Compare { lhs, rhs, .. } => {
            operand_needs_root_node(lhs, root) || operand_needs_root_node(rhs, root)
        }
        crate::aql::ir::Expr::Exists(t)
        | crate::aql::ir::Expr::Like { path: t, .. }
        | crate::aql::ir::Expr::Matches { path: t, .. } => path_needs_root_node(t, root),
        crate::aql::ir::Expr::Const(_) => false,
        crate::aql::ir::Expr::And(a, b) | crate::aql::ir::Expr::Or(a, b) => {
            filter_needs_root_node(a, root) || filter_needs_root_node(b, root)
        }
        crate::aql::ir::Expr::Not(a) => filter_needs_root_node(a, root),
    }
}

/// Whether ANY identified path in the query (SELECT, WHERE, ORDER BY) reads
/// the streaming root's node row — the projection/predicate half of the
/// dead-root-lateral test (`build_from_streaming`).
fn root_node_referenced(ir: &QueryIr, root: usize) -> bool {
    ir.select.iter().any(|c| match &c.value {
        SelectValue::Path(t) => path_needs_root_node(t, root),
        SelectValue::Aggregate { arg, .. } => {
            arg.as_ref().is_some_and(|t| path_needs_root_node(t, root))
        }
        SelectValue::Function { args, .. } => args.iter().any(|a| operand_needs_root_node(a, root)),
        SelectValue::Literal(_) => false,
    }) || ir
        .filter
        .as_ref()
        .is_some_and(|f| filter_needs_root_node(f, root))
        || ir
            .order_by
            .iter()
            .any(|k| path_needs_root_node(&k.path, root))
}

impl Builder<'_> {
    // ── FROM / containment ────────────────────────────────────────────────────

    pub(super) fn build_from(&mut self, tree: &ContainsTree) -> Result<(), AqlError> {
        self.walk(tree, None, None)?;
        Ok(())
    }

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
    /// default), and QUERY master03 §LIMIT makes WHICH rows return
    /// explicitly non-deterministic without `ORDER BY`, so the changed
    /// visit order is conformant. No openEHR spec governs the join
    /// mechanics — the shape selection is our own design.
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
        self.q.from_as(VersionRow::Table, Alias::new(v.as_str()));
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
        self.vo_alias.insert(plan.root, v.clone());
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
            let mut sub = Query::select();
            sub.column(Asterisk).from(Node::Table).offset(0);
            sub.and_where(hot_unaliased());
            if let Some(p) = &parent_alias {
                sub.and_where(Expr::col(Node::VoId).eq(col(p, Node::VoId)))
                    .and_where(Expr::col(Node::SysVersion).eq(col(p, Node::SysVersion)))
                    .and_where(
                        Expr::col(Node::Num).between(col(p, Node::Num), col(p, Node::NumCap)),
                    );
            } else {
                // Dead-root binding: the root's subtree is the whole
                // versioned object, so containment in it IS version
                // membership — same `(vo_id, sys_version)` as the spine.
                sub.and_where(Expr::col(Node::VoId).eq(col(&v, VersionRow::VoId)))
                    .and_where(Expr::col(Node::SysVersion).eq(col(&v, VersionRow::SysVersion)));
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

    /// Emit the streaming root's node lateral — or elide it. The lateral is
    /// DEAD WEIGHT when (a) the root source carries no node-level condition
    /// beyond its RM type — which the spine's `kind` filter already pins: a
    /// versioned object's `num = 0` row has `rm_type = kind` by construction —
    /// and (b) no identified path reads the root's node row
    /// ([`root_node_referenced`]). It is then one wasted `pk_node` probe per
    /// spine row: it yields exactly one row per version (every stored version
    /// has its root node), so dropping it preserves row multiplicity, and the
    /// root's subtree interval spans the whole versioned object, so
    /// first-level containment reduces to sharing the version. Measured as
    /// the post-streaming ladder's rung 1. No openEHR spec governs the join
    /// mechanics — our own design. Returns the alias the scope/gate machinery
    /// group-tracks (the node alias, or the spine when elided — it carries
    /// the `ehr_id` those gates filter on).
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

    /// The single-source node conditions of an RM source (type, archetype,
    /// name, standard predicates) — the one implementation the flat walk,
    /// the `EXISTS` builder, and the streaming laterals all share.
    fn rm_conds(&self, alias: &str, r: &RmSource) -> Result<Vec<Expr>, AqlError> {
        let mut out = Vec::new();
        if let Some(cond) = type_cond(alias, &r.rm_type) {
            out.push(cond);
        }
        if let Some(a) = &r.archetype {
            out.push(self.archetype_cond(alias, a)?);
        }
        if let Some(n) = &r.name {
            out.push(self.name_cond(alias, n)?);
        }
        for sp in &r.standard {
            out.push(self.std_cond(alias, sp)?);
        }
        Ok(out)
    }

    fn walk(
        &mut self,
        tree: &ContainsTree,
        ehr: Option<&str>,
        vo: Option<VoGroup>,
    ) -> Result<Option<VoGroup>, AqlError> {
        match tree {
            ContainsTree::And(a, b) => {
                let g = self.walk(a, ehr, vo.clone())?;
                self.walk(b, ehr, vo)?;
                Ok(g)
            }
            ContainsTree::Or(a, b) => self.walk_or(a, b, ehr, vo.as_ref()),
            ContainsTree::Operand { source, contained } => {
                self.walk_operand(source.0, contained.as_deref(), ehr, vo)
            }
        }
    }

    /// Lowers OR-containment (QUERY master03 §Containment — "Logical operators
    /// AND and OR").
    ///
    /// The enclosing object satisfies either branch's containment, so each
    /// branch lowers to a correlated `EXISTS` over the shared anchor, combined
    /// with `OR`. Variables bound only inside an `OR` branch are
    /// containment-filter-only — see the module NOTE.
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] for a top-level `OR` with no enclosing object
    /// (there is no row scope to correlate to), plus the branch lowerings.
    fn walk_or(
        &mut self,
        a: &ContainsTree,
        b: &ContainsTree,
        ehr: Option<&str>,
        vo: Option<&VoGroup>,
    ) -> Result<Option<VoGroup>, AqlError> {
        let anchor = match (vo, ehr) {
            (Some(g), _) => ExistsAnchor::Vo(g.node.clone(), g.types.clone()),
            (None, Some(e)) => ExistsAnchor::Ehr(e.to_owned()),
            (None, None) => {
                return Err(SqlError::Unsupported(
                    "OR at the top of the FROM tree without a containing object".to_owned(),
                )
                .into());
            }
        };
        let left = self.contained_exists(&anchor, a)?;
        let right = self.contained_exists(&anchor, b)?;
        self.q.and_where(left.or(right));
        Ok(None)
    }

    /// Lowers one containment operand: the source it names plus the edge it
    /// carries.
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] when the operand names an unknown source, or
    /// from the per-source lowerings.
    fn walk_operand(
        &mut self,
        sid: usize,
        contained: Option<&Contained>,
        ehr: Option<&str>,
        vo: Option<VoGroup>,
    ) -> Result<Option<VoGroup>, AqlError> {
        match self.ir.sources.get(sid) {
            Some(Source::Version(_)) => self.walk_version_source(sid, contained, ehr, vo),
            Some(Source::Ehr(e)) => {
                let alias = format!("e{sid}");
                self.q.from_as(Ehr::Table, Alias::new(alias.as_str()));
                self.ehr_alias.insert(sid, alias.clone());
                let preds = e.predicates.clone();
                for p in &preds {
                    self.push_ehr_predicate(&alias, p)?;
                }
                if let Some(c) = contained {
                    self.contained_edge(c, Some(&alias), None)?;
                }
                Ok(None)
            }
            Some(Source::Rm(r)) => {
                let r = r.clone();
                let group = self.emit_rm(sid, &r, ehr, vo.as_ref())?;
                if let Some(c) = contained {
                    self.contained_edge(c, ehr, Some(group.clone()))?;
                }
                Ok(Some(group))
            }
            None => Err(SqlError::Unsupported(
                "containment operand names an unknown source".to_owned(),
            )
            .into()),
        }
    }

    /// Lowers a VERSION operand: it opens no relation of its own and adopts
    /// the version group of the object it contains.
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] for `VERSION NOT CONTAINS`, plus the child
    /// lowering.
    fn walk_version_source(
        &mut self,
        sid: usize,
        contained: Option<&Contained>,
        ehr: Option<&str>,
        vo: Option<VoGroup>,
    ) -> Result<Option<VoGroup>, AqlError> {
        let child = match contained {
            Some(Contained {
                link: Link::Contains,
                tree,
            }) => self.walk(tree, ehr, vo)?,
            Some(_) => {
                return Err(SqlError::Unsupported("VERSION NOT CONTAINS".to_owned()).into());
            }
            None => None,
        };
        if let Some(g) = &child {
            self.version_vo.insert(sid, g.vo.clone());
        }
        Ok(child)
    }

    fn contained_edge(
        &mut self,
        c: &Contained,
        ehr: Option<&str>,
        vo: Option<VoGroup>,
    ) -> Result<(), AqlError> {
        match c.link {
            Link::Contains => {
                self.walk(&c.tree, ehr, vo)?;
                Ok(())
            }
            Link::NotContains => {
                let parent = vo.ok_or_else(|| {
                    SqlError::Unsupported("NOT CONTAINS without a containing object".to_owned())
                })?;
                // the exclusion is the negation of "parent contains
                // <tree>" — the same disjunction-capable `EXISTS` builder,
                // negated. This generalises NOT CONTAINS to compound (AND/OR)
                // and further-nested operands (QUERY master03 §Containment,
                // §NOT).
                let inner = self.contained_exists(
                    &ExistsAnchor::Vo(parent.node.clone(), parent.types.clone()),
                    &c.tree,
                )?;
                self.q.and_where(inner.not());
                Ok(())
            }
        }
    }

    fn emit_rm(
        &mut self,
        sid: usize,
        r: &RmSource,
        ehr: Option<&str>,
        vo: Option<&VoGroup>,
    ) -> Result<VoGroup, AqlError> {
        let node = format!("n{sid}");
        self.q.from_as(Node::Table, Alias::new(node.as_str()));
        self.q.and_where(hot(&node, Node::Tier));
        self.node_alias.insert(sid, node.clone());
        for cond in self.rm_conds(&node, r)? {
            self.q.and_where(cond);
        }

        // The edge kind is classified from the RM relationship of BOTH
        // operands' types: the child alone does not determine the join shape.
        let edge = match vo {
            Some(parent) => Some(classify_edge(&parent.types, &r.rm_type)?),
            None => None,
        };

        if let (Some(EdgeKind::Structural), Some(parent)) = (edge, vo) {
            self.q
                .and_where(col(&node, Node::VoId).eq(col(&parent.node, Node::VoId)));
            self.q
                .and_where(col(&node, Node::SysVersion).eq(col(&parent.node, Node::SysVersion)));
            self.q.and_where(col(&node, Node::Num).between(
                col(&parent.node, Node::Num),
                col(&parent.node, Node::NumCap),
            ));
            Ok(VoGroup {
                node,
                vo: parent.vo.clone(),
                types: r.rm_type.clone(),
            })
        } else {
            let voa = format!("v{sid}");
            self.q.from_as(VersionRow::Table, Alias::new(voa.as_str()));
            self.q.and_where(hot(&voa, VersionRow::Tier));
            self.q
                .and_where(col(&node, Node::VoId).eq(col(&voa, VersionRow::VoId)));
            self.q
                .and_where(col(&node, Node::SysVersion).eq(col(&voa, VersionRow::SysVersion)));
            // The audit join is summoned lazily (`ensure_audit`) by the
            // version-field expressions that actually read it — a query that
            // never projects/filters audit fields pays no join to the
            // ever-growing audit table.
            self.push_scope(&voa, &r.scope)?;
            self.group_roots.push(node.clone());
            self.group_vos.push(voa.clone());
            self.vo_alias.insert(sid, voa.clone());
            if let Some(e) = ehr {
                // The EHR link binds on the version SPINE only: one owning EHR
                // per versioned object (RM ehr master04 §EHR) makes the node
                // rows' `ehr_id` equal by construction, and the spine predicate
                // drives idx_version_hot_ehr with no per-node-row index.
                self.q
                    .and_where(col(&voa, VersionRow::EhrId).eq(col(e, Ehr::Id)));
                self.roots_linked_to_ehr.insert(node.clone());
            }
            // FolderItems: the child's own version group is open; the
            // reference edge itself is the correlated `items` lookup over the
            // parent folder's subtree (RM common master05 — folders hold
            // references to versioned objects).
            if let (Some(EdgeKind::FolderItems), Some(parent)) = (edge, vo) {
                self.q.and_where(folder_items_exists(&parent.node, &node));
            }
            // FolderChild is a union edge: a by-value strict descendant of the
            // parent's own tree, or any folder row of an items-referenced
            // VERSIONED_FOLDER. The child's version group serves both — a
            // by-value child's spine row IS the parent's.
            if let (Some(EdgeKind::FolderChild), Some(parent)) = (edge, vo) {
                self.q.and_where(
                    folder_by_value_child(&parent.node, &node)
                        .or(folder_items_exists(&parent.node, &node)),
                );
            }
            Ok(VoGroup {
                node,
                vo: voa,
                types: r.rm_type.clone(),
            })
        }
    }

    /// Build the correlated `EXISTS` boolean for "the `anchor` object contains
    /// `tree`" (QUERY master03 §Containment). Recurses through `AND`/`OR`
    /// subtrees (each a conjunction / disjunction of `EXISTS`) and nested
    /// `[NOT] CONTAINS` within an operand. Used by both OR-containment and
    /// the generalised NOT-CONTAINS anti-join.
    fn contained_exists(
        &mut self,
        anchor: &ExistsAnchor,
        tree: &ContainsTree,
    ) -> Result<Expr, AqlError> {
        match tree {
            ContainsTree::And(a, b) => Ok(self
                .contained_exists(anchor, a)?
                .and(self.contained_exists(anchor, b)?)),
            ContainsTree::Or(a, b) => Ok(self
                .contained_exists(anchor, a)?
                .or(self.contained_exists(anchor, b)?)),
            ContainsTree::Operand { source, contained } => {
                let Some(Source::Rm(r)) = self.ir.sources.get(source.0) else {
                    return Err(SqlError::Unsupported(
                        "OR/NOT CONTAINS of a non-structure operand".to_owned(),
                    )
                    .into());
                };
                let r = r.clone();
                let alias = format!("x{}", self.next_ctr());
                let mut sub = Query::select();
                sub.expr(Expr::val(1));
                sub.from_as(Node::Table, Alias::new(alias.as_str()));
                sub.and_where(hot(&alias, Node::Tier));
                self.anchor_correlation(&mut sub, anchor, &alias, &r.rm_type, &r.scope)?;
                for cond in self.rm_conds(&alias, &r)? {
                    sub.and_where(cond);
                }
                if let Some(c) = contained {
                    let inner = self.contained_exists(
                        &ExistsAnchor::Vo(alias.clone(), r.rm_type.clone()),
                        &c.tree,
                    )?;
                    match c.link {
                        Link::Contains => sub.and_where(inner),
                        Link::NotContains => sub.and_where(inner.not()),
                    };
                }
                Ok(Expr::exists(sub))
            }
        }
    }

    /// Correlate an `EXISTS` subquery's operand node (`alias`) to its anchor:
    /// the classified containment edge for a VO anchor (structural interval,
    /// strict folder-subtree interval, or the `FOLDER.items` reference
    /// resolution), or an `ehr_id` join plus version scope for a VO contained in
    /// an EHR.
    fn anchor_correlation(
        &mut self,
        sub: &mut SelectStatement,
        anchor: &ExistsAnchor,
        alias: &str,
        child_types: &crate::aql::ir::TypeSet,
        scope: &VersionScope,
    ) -> Result<(), AqlError> {
        match anchor {
            ExistsAnchor::Vo(parent, parent_types) => {
                match classify_edge(parent_types, child_types)? {
                    EdgeKind::Structural => {
                        sub.and_where(col(alias, Node::VoId).eq(col(parent, Node::VoId)));
                        sub.and_where(
                            col(alias, Node::SysVersion).eq(col(parent, Node::SysVersion)),
                        );
                        sub.and_where(
                            col(alias, Node::Num)
                                .between(col(parent, Node::Num), col(parent, Node::NumCap)),
                        );
                    }
                    EdgeKind::FolderChild => {
                        // The union edge: an items-referenced VERSIONED_FOLDER
                        // needs the child's own version spine + scope, which a
                        // by-value descendant satisfies too by sharing the
                        // parent's spine row.
                        let voa = format!("xv{}", self.next_ctr());
                        sub.from_as(VersionRow::Table, Alias::new(voa.as_str()));
                        sub.and_where(hot(&voa, VersionRow::Tier));
                        sub.and_where(col(alias, Node::VoId).eq(col(&voa, VersionRow::VoId)));
                        sub.and_where(
                            col(alias, Node::SysVersion).eq(col(&voa, VersionRow::SysVersion)),
                        );
                        match scope {
                            VersionScope::Latest => {
                                bind_head(sub, &voa, true);
                            }
                            VersionScope::All => {
                                bind_head(sub, &voa, false);
                            }
                            VersionScope::Predicate(_) => {
                                return Err(SqlError::Unsupported(
                                    "a version predicate on an OR/NOT-CONTAINS branch VO"
                                        .to_owned(),
                                )
                                .into());
                            }
                        }
                        sub.and_where(
                            folder_by_value_child(parent, alias)
                                .or(folder_items_exists(parent, alias)),
                        );
                    }
                    EdgeKind::FolderItems => {
                        // The child is its own versioned object: bind its
                        // version spine + scope (the Ehr arm's shape), then
                        // the reference edge over the anchor folder's subtree.
                        let voa = format!("xv{}", self.next_ctr());
                        sub.from_as(VersionRow::Table, Alias::new(voa.as_str()));
                        sub.and_where(hot(&voa, VersionRow::Tier));
                        sub.and_where(col(alias, Node::VoId).eq(col(&voa, VersionRow::VoId)));
                        sub.and_where(
                            col(alias, Node::SysVersion).eq(col(&voa, VersionRow::SysVersion)),
                        );
                        match scope {
                            VersionScope::Latest => {
                                bind_head(sub, &voa, true);
                            }
                            VersionScope::All => {
                                bind_head(sub, &voa, false);
                            }
                            VersionScope::Predicate(_) => {
                                return Err(SqlError::Unsupported(
                                    "a version predicate on an OR/NOT-CONTAINS branch VO"
                                        .to_owned(),
                                )
                                .into());
                            }
                        }
                        sub.and_where(folder_items_exists(parent, alias));
                    }
                }
                Ok(())
            }
            ExistsAnchor::Ehr(e) => {
                let voa = format!("xv{}", self.next_ctr());
                sub.from_as(VersionRow::Table, Alias::new(voa.as_str()));
                sub.and_where(hot(&voa, VersionRow::Tier));
                sub.and_where(col(alias, Node::VoId).eq(col(&voa, VersionRow::VoId)));
                sub.and_where(col(alias, Node::SysVersion).eq(col(&voa, VersionRow::SysVersion)));
                sub.and_where(col(alias, Node::EhrId).eq(col(e, Ehr::Id)));
                // Same version-spine mirror as the FROM path (see above):
                // lets the planner bound the EXISTS by the EHR instead of the
                // corpus. Identical semantics by construction.
                sub.and_where(col(&voa, VersionRow::EhrId).eq(col(e, Ehr::Id)));
                match scope {
                    VersionScope::Latest => {
                        bind_head(sub, &voa, true);
                    }
                    VersionScope::All => {
                        bind_head(sub, &voa, false);
                    }
                    VersionScope::Predicate(_) => {
                        return Err(SqlError::Unsupported(
                            "a version predicate on an OR/NOT-CONTAINS branch VO".to_owned(),
                        )
                        .into());
                    }
                }
                Ok(())
            }
        }
    }

    pub(super) fn apply_ehr_scope(&mut self) {
        // The whole-EHR restriction, at EVERY scope. The per-object predicate
        // covers every version spine (a whole-EHR restriction stamps each
        // `vo_head`), but a bare `FROM EHR e` has no spine, so the `ehr` row
        // carries it. Unlike the objection below, this is not a population-only
        // gate: GDPR Art. 18(2) leaves storage as the only processing a
        // restriction admits (`docs/law/eu/gdpr/text.html`), so naming the
        // `ehr_id` does not make the content answerable.
        for alias in self.ehr_alias.values().cloned().collect::<Vec<_>>() {
            self.q.and_where(col(&alias, Ehr::RestrictedAt).is_null());
        }
        // Multi-EHR scoping (`ehr_ids: List<UUID>`): restrict every VO root to
        // the id set with `ehr_id = ANY($ids)` — ONE array bind rather than one
        // bind per id, so a cohort of any size stays inside the 65535-parameter
        // wire limit (`PostgreSQL` 18 §Frontend/Backend Protocol, Bind, whose
        // parameter count is an `int16`,
        // <https://www.postgresql.org/docs/18/protocol-message-formats.html>).
        // The single-`ehr_id` REST case is the one-element set; empty = no
        // explicit scope (the population gate takes over).
        if !self.ctx.ehr_ids.is_empty() {
            let ids: Vec<Uuid> = self.ctx.ehr_ids.iter().map(|id| id.0).collect();
            for root in self.group_roots.clone() {
                self.q.and_where(
                    col(&root, Node::EhrId).eq(Expr::from(PgFunc::any(Expr::val(ids.clone())))),
                );
            }
            // A bare `FROM EHR e` (no CONTAINS) has no VO root — the scope
            // must bind the EHR source itself, or a scoped query would run
            // over the whole population (ITS-REST query Request.md: `ehr_id`
            // "used to execute the query within a single EHR context").
            for alias in self.ehr_alias.values().cloned().collect::<Vec<_>>() {
                self.q.and_where(
                    col(&alias, Ehr::Id).eq(Expr::from(PgFunc::any(Expr::val(ids.clone())))),
                );
            }
        }
        // The per-EHR access decision binds every VO root and EHR source the
        // same way the explicit scope does, so no query shape reaches a
        // withheld EHR (RM ehr `ehr_access.adoc` §`EHR_ACCESS` Class).
        let access = match &self.ctx.ehr_access {
            EhrAccessScope::Unrestricted => None,
            EhrAccessScope::Excluding(ids) => Some((false, ids)),
            EhrAccessScope::Only(ids) => Some((true, ids)),
        };
        if let Some((only, ids)) = access {
            let ids: Vec<Uuid> = ids.iter().map(|id| id.0).collect();
            let targets = self
                .group_roots
                .iter()
                .map(|root| col(root, Node::EhrId))
                .chain(self.ehr_alias.values().map(|alias| col(alias, Ehr::Id)))
                .collect::<Vec<_>>();
            for target in targets {
                let condition = if only {
                    target.eq(Expr::from(PgFunc::any(Expr::val(ids.clone()))))
                } else {
                    target.ne(Expr::from(PgFunc::all(Expr::val(ids.clone()))))
                };
                self.q.and_where(condition);
            }
        }
        // NOTE: the ABAC patient scope restricts every VO root to the caller's
        // patient EHRs, so out-of-scope rows are never fetched whatever the
        // projection (no openEHR spec governs this — our own extension).
        if let Some(subject) = self.ctx.subject_scope.clone() {
            for root in self.group_roots.clone() {
                let mut sub = Query::select();
                sub.column(Ehr::Id)
                    .from(Ehr::Table)
                    .and_where(Expr::col(Ehr::SubjectId).eq(Expr::val(subject.clone())));
                self.q.and_where(col(&root, Node::EhrId).in_subquery(sub));
            }
        }
    }

    /// The query-population gate (SM `I_QUERY_SERVICE.execute_ad_hoc_query` /
    /// `execute_stored_query`). When no explicit `ehr_ids` are supplied, the
    /// `ehr_ids` parameter doc mandates a "full population query [...] on all
    /// EHRs whose status has the `is_queryable` flag set to `True`"
    /// (`docs/specs/openehr/SM/docs/UML/classes/i_query_service.adoc`). We
    /// therefore restrict every EHR root — bare `EHR` sources (`ehr.id`) and VO
    /// roots (`node.ehr_id`) alike — to that set. A scoped query (a non-empty
    /// `ehr_ids` set) targets specific EHRs and is not gated.
    pub(super) fn apply_population_gate(&mut self) {
        if !self.ctx.ehr_ids.is_empty() {
            return;
        }
        // One gate per join-connected component: a VO root linked to
        // an EHR alias (`node.ehr_id = e.id`) is covered by that alias's gate —
        // gating both would filter the same EHR twice.
        for root in self.group_roots.clone() {
            if self.roots_linked_to_ehr.contains(&root) {
                continue;
            }
            self.gate_vo_root(&root);
        }
        // An EHR source already has its `ehr` row in the FROM — filter its
        // promoted `is_queryable` column directly.
        for alias in self.ehr_alias.values().cloned().collect::<Vec<_>>() {
            self.q.and_where(col(&alias, Ehr::IsQueryable).eq(true));
            self.q.and_where(research_objection_clear(&alias));
        }
    }

    /// The EHR-scoped restriction mark on a population query.
    ///
    /// The per-object predicate ([`bind_head`]) already removes every
    /// restricted object; this removes an EHR restricted as a whole, whose
    /// objects carry the mark too but whose `ehr` row is the cheaper place to
    /// decide it once the population gate has already joined it.
    fn gate_ehr_marks(&mut self, alias: &str) {
        self.q.and_where(col(alias, Ehr::RestrictedAt).is_null());
        self.q.and_where(research_objection_clear(alias));
    }

    /// Gate an unlinked versioned-object root (no bound EHR alias covers it):
    /// join the owning `ehr` row on the node's `ehr_id` and filter the promoted
    /// [`is_queryable`](crate::db::iden::Ehr::IsQueryable) column. The flag is a
    /// scalar attribute of the current `EHR_STATUS` (RM ehr master04 §EHR Status,
    /// `EHR_STATUS.is_queryable` 1..1 Boolean), promoted onto the `ehr` row and
    /// kept in lockstep by the status write path, so the gate is a boolean-column
    /// filter over a PK join instead of a per-query `EXISTS` that index-scanned
    /// every current `EHR_STATUS` root. SM `I_QUERY_SERVICE`: a
    /// full-population query runs over "all EHRs whose status has the
    /// `is_queryable` flag set to `True`" (`i_query_service.adoc`). No openEHR
    /// spec governs the join mechanics — our own storage design.
    fn gate_vo_root(&mut self, root: &str) {
        let alias = format!("qg{}", self.next_ctr());
        let link = col(&alias, Ehr::Id).eq(col(root, Node::EhrId));
        if self.streaming {
            self.q
                .join_as(JoinType::Join, Ehr::Table, Alias::new(alias.as_str()), link);
        } else {
            self.q.from_as(Ehr::Table, Alias::new(alias.as_str()));
            self.q.and_where(link);
        }
        self.q.and_where(col(&alias, Ehr::IsQueryable).eq(true));
        self.gate_ehr_marks(&alias);
    }

    /// Join the EHR's current `EHR_STATUS` versioned-object root node for an EHR
    /// source, once, and register it as that source's node so the whole-object /
    /// anchor-walk / fragment machinery resolves paths under `ehr_status`.
    ///
    /// `EHR` is not a `node` and `EHR_STATUS` is a *separate* VO (RM 1.2.0
    /// `EHR.ehr_status`), so this is an engine-level join on the store, not a
    /// node-tree walk: `version.ehr_id = ehr.id`, `kind = 'EHR_STATUS'`, the
    /// object's trunk head, root node (`num = 0`). Every
    /// EHR has exactly one current `EHR_STATUS`, so the inner join is 1:1. The
    /// population/`ehr_id` gates already constrain the `ehr` row, so the joined
    /// status inherits that scope transitively (no separate gating).
    pub(super) fn ensure_ehr_status_root(&mut self, ehr_sid: usize) -> Result<String, AqlError> {
        if let Some(a) = self.ehr_status_node.get(&ehr_sid) {
            return Ok(a.clone());
        }
        let ehr = self.ehr_alias.get(&ehr_sid).cloned().ok_or_else(|| {
            SqlError::Unsupported("ehr_status path on a non-EHR source".to_owned())
        })?;
        let vo = format!("esv{}", self.next_ctr());
        let node = format!("esn{}", self.next_ctr());
        let vo_link = col(&vo, VersionRow::EhrId).eq(col(&ehr, Ehr::Id));
        let node_link = col(&node, Node::VoId)
            .eq(col(&vo, VersionRow::VoId))
            .and(col(&node, Node::SysVersion).eq(col(&vo, VersionRow::SysVersion)));
        if self.streaming {
            self.q.join_as(
                JoinType::Join,
                VersionRow::Table,
                Alias::new(vo.as_str()),
                vo_link,
            );
            self.q.join_as(
                JoinType::Join,
                Node::Table,
                Alias::new(node.as_str()),
                node_link,
            );
        } else {
            self.q.from_as(VersionRow::Table, Alias::new(vo.as_str()));
            self.q.from_as(Node::Table, Alias::new(node.as_str()));
            self.q.and_where(vo_link);
            self.q.and_where(node_link);
        }
        self.q.and_where(hot(&vo, VersionRow::Tier));
        self.q.and_where(hot(&node, Node::Tier));
        self.q
            .and_where(col(&vo, VersionRow::Kind).eq(Expr::val("EHR_STATUS")));
        // Current = latest trunk (master06 latest_trunk_version).
        self.bind_head(&vo, true);
        self.q.and_where(col(&node, Node::Num).eq(Expr::val(0)));
        self.ehr_status_node.insert(ehr_sid, node.clone());
        // Register as the source node so `source_node`/`whole_object_alias`/
        // `data_leaf_expr` (which start from the leaf's source node) resolve the
        // EHR_STATUS root; the EHR source has no other node entry.
        self.node_alias.insert(ehr_sid, node.clone());
        Ok(node)
    }

    pub(super) fn ensure_audit(&mut self, voa: &str) -> String {
        if let Some(a) = self.audit_alias.get(voa) {
            return a.clone();
        }
        let alias = format!("a_{voa}");
        let cond = col(&alias, CommitAudit::Id).eq(col(voa, VersionRow::CommitAuditId));
        if self.streaming {
            self.q.join_as(
                JoinType::Join,
                CommitAudit::Table,
                Alias::new(alias.as_str()),
                cond,
            );
        } else {
            self.q
                .from_as(CommitAudit::Table, Alias::new(alias.as_str()));
            self.q.and_where(cond);
        }
        self.audit_alias.insert(voa.to_owned(), alias.clone());
        alias
    }

    /// Bind the object's head row for the version spine `voa` (the restriction
    /// gate, and `LATEST_VERSION` when `latest`): a `JOIN` on the streaming
    /// shape, a FROM item with its predicates otherwise. See [`bind_head`].
    fn bind_head(&mut self, voa: &str, latest: bool) -> String {
        let h = format!("{voa}_head");
        let mut cond = col(&h, VoHead::VoId)
            .eq(col(voa, VersionRow::VoId))
            .and(col(&h, VoHead::RestrictedAt).is_null());
        if latest {
            cond =
                cond.and(col(voa, VersionRow::SysVersion).eq(col(&h, VoHead::TrunkHeadSysVersion)));
        }
        if self.streaming {
            self.q
                .join_as(JoinType::Join, VoHead::Table, Alias::new(h.as_str()), cond);
        } else {
            self.q.from_as(VoHead::Table, Alias::new(h.as_str()));
            self.q.and_where(cond);
        }
        h
    }

    fn push_scope(&mut self, voa: &str, scope: &VersionScope) -> Result<(), AqlError> {
        match scope {
            VersionScope::Latest => {
                // LATEST_VERSION = the latest TRUNK version (RM common master06
                // latest_trunk_version; open branch tips coexist and are not
                // "the latest version" of the container).
                self.bind_head(voa, true);
            }
            VersionScope::All => {
                self.bind_head(voa, false);
            }
            VersionScope::Predicate(p) if p.field == VersionField::TimeCommitted => {
                self.bind_head(voa, false);
                // Version-at-time: the TRUNK version in force at the instant; a
                // branch version live at that instant coexists by design and
                // must not duplicate the row.
                let value = self.bind_value(&p.value)?;
                self.q
                    .and_where(col(voa, VersionRow::BranchNumber).eq(Expr::val(0)));
                self.q
                    .and_where(trunk_at_instant(voa, cast(Expr::val(value), "timestamptz")));
            }
            VersionScope::Predicate(p) => {
                self.bind_head(voa, false);
                let system_id = self.ctx.system_id.clone();
                let rhs = cast(Expr::val(self.bind_value(&p.value)?), "text");
                let lhs = version_field_expr(voa, || self.ensure_audit(voa), p.field, &system_id);
                self.q
                    .and_where(lhs.binary(super::expr::binoper(p.op), rhs));
            }
        }
        Ok(())
    }

    fn push_ehr_predicate(&mut self, alias: &str, p: &EhrPredicate) -> Result<(), AqlError> {
        let value = self.bind_value(&p.value)?;
        let (lhs, rhs) = match p.field {
            EhrField::EhrId => (col(alias, Ehr::Id), cast(Expr::val(value), "uuid")),
            EhrField::TimeCreated => (
                col(alias, Ehr::TimeCreated),
                cast(Expr::val(value), "timestamptz"),
            ),
            EhrField::SystemId | EhrField::Whole => {
                return Err(SqlError::Unsupported(
                    "EHR system_id / whole-EHR predicate".to_owned(),
                )
                .into());
            }
        };
        self.q
            .and_where(lhs.binary(super::expr::binoper(p.op), rhs));
        Ok(())
    }
}

/// The VO-root RM types the store versions independently (RM common master06
/// versioned objects), decided from the storage domain itself
/// ([`crate::versioning::Kind`], the `version.kind` discriminants) rather
/// than a restated list — a kind added there is a VO root here.
///
/// The EHR-scoped subset only: the demographic kinds (the five party roots and
/// `PARTY_RELATIONSHIP`) are versioned objects but are **unreachable** as an
/// AQL source, because a FROM class must have a concrete descendant the node
/// store addresses (`openehr_rm::v1_2::model::is_structure_root`, whose set excludes
/// the demographic LOCATABLE hierarchy) — `crate::aql::lower` refuses the rest
/// as [`crate::aql::error::AqlFeatureError::UnsupportedSourceClass`] before any
/// SQL is built. So the exclusion is that gate's consequence, stated here
/// explicitly, not a second opinion about what a versioned object is.
pub(super) fn is_vo_root_type(t: &str) -> bool {
    crate::versioning::Kind::from_type(t).is_some_and(|kind| !kind.is_demographic())
}
