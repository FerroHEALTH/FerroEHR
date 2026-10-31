// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FROM-clause + containment lowering (QUERY master03 §Class expressions,
//! §Containment, §VERSION sources) and the population / scope gates.
//!
//! No openEHR spec governs the join mechanics — this is our own design: the FROM containment tree becomes a **cross
//! join of table aliases + typed WHERE conditions** (the planner folds
//! cross-join+filter into joins). Each RM source that roots a versioned object
//! gets a `node` + `version` (+ `audit`) alias; content sources contained
//! within it share the `version` and join on the STRICT nested-set interval
//! (`a.num < num AND num <= a.num_cap`, same `(vo_id, sys_version)`): QUERY
//! master03 §Containment makes the left operand "the parent object" of the
//! right, and no node is its own parent. `EHR` sources join VO roots via
//! `ehr_id`; `VERSION` sources share the contained VO's `version`. `AND`
//! containment is a join; `OR` and `NOT CONTAINS` are disjunctive / anti-join
//! correlated `EXISTS` filters (QUERY master03 §Containment — boolean
//! `AND`/`OR`, `NOT`).
//!
//! A variable declared inside an `OR` branch and read by an identified path is
//! additionally LEFT-bound under the shared anchor ([`Builder::left_bind`]):
//! one row per matching node, a `NULL` binding where its branch did not match,
//! so a row never carries a value from a branch that did not hold. A branch
//! that is its own versioned object (under an `EHR`, or referenced from a
//! `FOLDER`) binds its node together with its version row, in the scope the
//! same object would have outside the `OR` ([`Builder::bind_versioned_branch`]).
//! QUERY master03 §Variables lets any FROM variable be read and is silent on the
//! row shape of a disjunction, so that shape is our own design.

use sea_query::extension::postgres::PgFunc;
use sea_query::{Alias, Asterisk, Expr, ExprTrait as _, JoinType, Query, SelectStatement};
use uuid::Uuid;

use crate::aql::error::{AqlError, AqlFeatureError, SqlError};
use crate::aql::ir::{
    Coercion, Contained, ContainsTree, EhrField, EhrPredicate, Link, PathTarget, QueryIr, RmSource,
    Source, VersionField, VersionScope,
};
use crate::db::iden::{CommitAudit, Ehr, Node, VersionRow, VoHead};

use super::expr::{cast, col, hot, type_cond};
use super::value::version_field_expr;
use super::{Builder, EhrAccessScope, VoGroup, subject};

/// The correlation anchor for an `OR` / `NOT CONTAINS` `EXISTS` subquery.
enum ExistsAnchor {
    /// Interval-anchored inside a node subtree of a shared versioned object:
    /// a strict descendant of the parent ([`strict_descendant`]).
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
    /// sys_version)`, strict interval join. The child shares the parent's
    /// version group.
    Structural,
    /// `FOLDER CONTAINS <versioned object>`: reference resolution through
    /// `FOLDER.items` `OBJECT_REF`s, transitively over the folder subtree (RM
    /// common master05 — folders hold *references* to versioned objects; RM
    /// ehr master04 §Folders). The child opens its own version group.
    FolderItems,
    /// `FOLDER CONTAINS FOLDER`: the union of the by-value `folders` nesting
    /// (a strict descendant interval, as every structural edge) and an items-referenced `VERSIONED_FOLDER`'s folder
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
        _ => Err(AqlFeatureError::UnsupportedContainment(
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
fn bind_head(
    q: &mut SelectStatement,
    version_alias: &str,
    latest: bool,
    gates: super::gates::Gates,
) -> String {
    let h = format!("{version_alias}_head");
    q.from_as(VoHead::Table, Alias::new(h.as_str()));
    q.and_where(col(&h, VoHead::VoId).eq(col(version_alias, VersionRow::VoId)));
    q.and_where(col(&h, VoHead::RestrictedAt).is_null());
    for predicate in gates.on_head(&h) {
        q.and_where(predicate);
    }
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
/// child row is a strict nested-set descendant of the parent row inside the
/// same versioned folder tree.
fn folder_by_value_child(parent_node: &str, child_node: &str) -> Expr {
    strict_descendant(child_node, parent_node)
}

/// Whether `child` is a STRICT nested-set descendant of `parent` within one
/// stored version: same `(vo_id, sys_version)`, `parent.num < num <=
/// parent.num_cap`.
///
/// QUERY master03 §Containment: the left-hand operand "is the parent object of
/// the right-hand operand". We read that as transitive over the subtree (a
/// descendant at any depth is contained), and since a node is not its own
/// parent, the interval excludes the node itself.
pub(super) fn strict_descendant(child: &str, parent: &str) -> Expr {
    col(child, Node::VoId)
        .eq(col(parent, Node::VoId))
        .and(col(child, Node::SysVersion).eq(col(parent, Node::SysVersion)))
        .and(col(child, Node::Num).gt(col(parent, Node::Num)))
        .and(col(child, Node::Num).lte(col(parent, Node::NumCap)))
}

/// Whether `child` is the node an attribute step `attribute` reaches from
/// `parent`: a strict descendant whose structure parent IS `parent` and whose
/// stored path continues `parent`'s by exactly that attribute (the codec
/// writes `<parent path><attribute>[<index>].`).
///
/// BASE `architecture_overview/master11-paths.adoc` §Basic Syntax: a path
/// "select\[s\] the object which is the value of the final attribute name in the
/// path, when going from some starting point in the tree and following
/// attribute names given in the path", so each step is one attribute of the
/// previous object, never the object itself and never a deeper descendant or
/// a sibling attribute of the same type (`OBSERVATION.data` versus
/// `OBSERVATION.state`). Attribute names reach here from the RM model, so the
/// escaped regular expression is a plain literal match.
pub(super) fn attribute_child(child: &str, parent: &str, attribute: &str) -> Expr {
    let pattern = format!("^{}[0-9]*\\.$", regex::escape(attribute));
    strict_descendant(child, parent)
        .and(col(child, Node::ParentNum).eq(col(parent, Node::Num)))
        .and(Expr::cust_with_exprs(
            "substr($1, length($2) + 1) ~ $3",
            [
                col(child, Node::Path),
                col(parent, Node::Path),
                Expr::val(pattern),
            ],
        ))
}

/// How a left binding ([`Builder::left_bind`]) reaches its nodes from the
/// parent.
#[derive(Debug, Clone, Copy)]
pub(super) enum BoundEdge<'a> {
    /// One attribute step of an identified path ([`attribute_child`]).
    Attribute(&'a str),
    /// A `CONTAINS` operand anywhere in the parent's subtree
    /// ([`strict_descendant`]).
    Descendant,
}

/// The sources some identified path of the query reads.
fn referenced_sources(ir: &QueryIr) -> std::collections::BTreeSet<usize> {
    let mut out = std::collections::BTreeSet::new();
    ir.for_each_path(&mut |t| {
        out.insert(match t {
            PathTarget::Data(leaf) | PathTarget::EhrStatus(leaf) => leaf.source.0,
            PathTarget::Version { source, .. } | PathTarget::Ehr { source, .. } => source.0,
        });
    });
    out
}

/// The parent an `OR` branch binds under ([`Builder::bind_branch`]).
#[derive(Debug, Clone)]
enum BranchParent {
    /// A node of a versioned-object group (a shared COMPOSITION, a FOLDER).
    Group(VoGroup),
    /// The `ehr` alias of an EHR source: the branch operand is a node of any
    /// versioned object of that EHR.
    Ehr(String),
}

impl BranchParent {
    /// The `EXISTS` anchor the containment filters of the branch correlate to.
    fn exists_anchor(&self) -> ExistsAnchor {
        match self {
            Self::Group(g) => ExistsAnchor::Vo(g.node.clone(), g.types.clone()),
            Self::Ehr(e) => ExistsAnchor::Ehr(e.clone()),
        }
    }
}

/// The relation a left binding ([`Builder::left_bind_rows`]) draws its rows
/// from.
#[derive(Debug, Clone, Copy)]
pub(super) enum BoundTable {
    /// `node` rows, in document order.
    Node,
    /// `version` rows (a binding of at most one row per outer row).
    Version,
}

/// The variable of the first source in `tree` (nested containment included)
/// that `referenced` names, if any.
fn first_referenced_var(
    ir: &QueryIr,
    tree: &ContainsTree,
    referenced: &std::collections::BTreeSet<usize>,
) -> Option<String> {
    match tree {
        ContainsTree::And(a, b) | ContainsTree::Or(a, b) => first_referenced_var(ir, a, referenced)
            .or_else(|| first_referenced_var(ir, b, referenced)),
        ContainsTree::Operand { source, contained } => {
            let own = referenced
                .contains(&source.0)
                .then(|| ir.sources.get(source.0).and_then(Source::var))
                .flatten()
                .map(str::to_owned);
            own.or_else(|| {
                contained
                    .as_deref()
                    .and_then(|c| first_referenced_var(ir, &c.tree, referenced))
            })
        }
    }
}

/// Refuses a query reading a variable declared under `NOT CONTAINS`.
///
/// QUERY master03 §Containment: `NOT CONTAINS` expresses "the absence of any
/// containment relationship between a parent and potential child items", so a
/// variable declared there names an object no matching row holds and no path
/// through it has a value.
///
/// # Errors
/// [`AqlFeatureError::NegatedContainmentVariable`] naming the variable.
pub(super) fn refuse_negated_reads(ir: &QueryIr) -> Result<(), AqlError> {
    fn walk(
        ir: &QueryIr,
        tree: &ContainsTree,
        referenced: &std::collections::BTreeSet<usize>,
    ) -> Result<(), AqlError> {
        match tree {
            ContainsTree::And(a, b) | ContainsTree::Or(a, b) => {
                walk(ir, a, referenced)?;
                walk(ir, b, referenced)
            }
            ContainsTree::Operand { contained, .. } => match contained.as_deref() {
                Some(Contained {
                    link: Link::NotContains,
                    tree,
                }) => match first_referenced_var(ir, tree, referenced) {
                    Some(var) => Err(AqlFeatureError::NegatedContainmentVariable(var).into()),
                    None => Ok(()),
                },
                Some(Contained { tree, .. }) => walk(ir, tree, referenced),
                None => Ok(()),
            },
        }
    }
    walk(ir, &ir.contains, &referenced_sources(ir))
}

impl Builder<'_> {
    // ── FROM / containment ────────────────────────────────────────────────────

    pub(super) fn build_from(&mut self, tree: &ContainsTree) -> Result<(), AqlError> {
        self.walk(tree, None, None)?;
        Ok(())
    }

    /// The single-source node conditions of an RM source (type, archetype,
    /// name, standard predicates) — the one implementation the flat walk,
    /// the `EXISTS` builder, and the streaming laterals all share.
    pub(super) fn rm_conds(&self, alias: &str, r: &RmSource) -> Result<Vec<Expr>, AqlError> {
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
    /// branch lowers to a correlated `EXISTS` over the shared anchor (an
    /// enclosing versioned object, else the EHR), combined with `OR`. A branch
    /// variable an identified path reads is then left-bound under the anchor
    /// ([`Self::bind_branch`], the row shape the module documentation states);
    /// a branch nobody reads stays a filter.
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] for a top-level `OR` with no enclosing object
    /// (there is no row scope to correlate to); plus the branch lowerings.
    fn walk_or(
        &mut self,
        a: &ContainsTree,
        b: &ContainsTree,
        ehr: Option<&str>,
        vo: Option<&VoGroup>,
    ) -> Result<Option<VoGroup>, AqlError> {
        let parent = match (vo, ehr) {
            (Some(g), _) => BranchParent::Group(g.clone()),
            (None, Some(e)) => BranchParent::Ehr(e.to_owned()),
            (None, None) => {
                return Err(SqlError::Unsupported(
                    "OR at the top of the FROM tree without a containing object".to_owned(),
                )
                .into());
            }
        };
        let anchor = parent.exists_anchor();
        let left = self.contained_exists(&anchor, a)?;
        let right = self.contained_exists(&anchor, b)?;
        self.q.and_where(left.or(right));
        let referenced = referenced_sources(self.ir);
        self.bind_branch(&parent, a, &[], &referenced, ehr)?;
        self.bind_branch(&parent, b, &[], &referenced, ehr)?;
        Ok(None)
    }

    /// Left-binds every operand of the `OR` branch `tree` whose subtree holds
    /// a variable an identified path reads, under the branch's `parent`.
    ///
    /// A bound operand yields one row per node that matches it, its own nested
    /// containment and `siblings` (the other halves of an enclosing `AND`
    /// inside the branch, correlated to the parent), and one `NULL` row where
    /// none does: a value never comes from a branch that did not hold. Its
    /// node alias then serves paths exactly as a joined variable's does. An
    /// operand that is a node inside the parent's own tree binds through
    /// [`Self::left_bind`]; one that is a versioned object of its own (any
    /// operand under an EHR, a `FOLDER` reference) through
    /// [`Self::bind_versioned_branch`]. `ehr` is the enclosing EHR alias, if
    /// any.
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] for an operand that is no RM class, plus the
    /// edge classification and the condition lowerings.
    fn bind_branch(
        &mut self,
        parent: &BranchParent,
        tree: &ContainsTree,
        siblings: &[Expr],
        referenced: &std::collections::BTreeSet<usize>,
        ehr: Option<&str>,
    ) -> Result<(), AqlError> {
        if first_referenced_var(self.ir, tree, referenced).is_none() {
            return Ok(());
        }
        let anchor = parent.exists_anchor();
        match tree {
            ContainsTree::Or(a, b) => {
                self.bind_branch(parent, a, siblings, referenced, ehr)?;
                self.bind_branch(parent, b, siblings, referenced, ehr)
            }
            ContainsTree::And(a, b) => {
                let needs_b = self.contained_exists(&anchor, b)?;
                let needs_a = self.contained_exists(&anchor, a)?;
                let mut for_a = siblings.to_vec();
                for_a.push(needs_b);
                let mut for_b = siblings.to_vec();
                for_b.push(needs_a);
                self.bind_branch(parent, a, &for_a, referenced, ehr)?;
                self.bind_branch(parent, b, &for_b, referenced, ehr)
            }
            ContainsTree::Operand { source, contained } => {
                let sid = source.0;
                let Some(Source::Rm(r)) = self.ir.sources.get(sid).cloned() else {
                    return Err(SqlError::Unsupported(
                        "OR/NOT CONTAINS of a non-structure operand".to_owned(),
                    )
                    .into());
                };
                let contained = contained.as_deref();
                let group = match parent {
                    BranchParent::Group(g)
                        if classify_edge(&g.types, &r.rm_type)? == EdgeKind::Structural =>
                    {
                        let siblings = siblings.to_vec();
                        let alias =
                            self.left_bind(&g.node, BoundEdge::Descendant, |b, inner| {
                                let mut conds = b.rm_conds(inner, &r)?;
                                conds.extend(siblings);
                                conds.extend(b.nested_condition(inner, &r, contained)?);
                                Ok(conds)
                            })?;
                        self.node_alias.insert(sid, alias.clone());
                        VoGroup {
                            node: alias,
                            vo: g.vo.clone(),
                            types: r.rm_type.clone(),
                        }
                    }
                    BranchParent::Group(_) | BranchParent::Ehr(_) => {
                        self.bind_versioned_branch(&anchor, sid, &r, siblings, contained, ehr)?
                    }
                };
                if let Some(Contained {
                    link: Link::Contains,
                    tree: nested,
                }) = contained
                {
                    self.bind_branch(&BranchParent::Group(group), nested, &[], referenced, ehr)?;
                }
                Ok(())
            }
        }
    }

    /// The condition an operand's own nested `[NOT] CONTAINS` puts on its node
    /// `inner`, if it carries one.
    ///
    /// # Errors
    /// The containment lowerings of [`Self::contained_exists`].
    fn nested_condition(
        &mut self,
        inner: &str,
        r: &RmSource,
        contained: Option<&Contained>,
    ) -> Result<Option<Expr>, AqlError> {
        let Some(c) = contained else {
            return Ok(None);
        };
        let nested = self.contained_exists(
            &ExistsAnchor::Vo(inner.to_owned(), r.rm_type.clone()),
            &c.tree,
        )?;
        Ok(Some(match c.link {
            Link::Contains => nested,
            Link::NotContains => nested.not(),
        }))
    }

    /// Left-binds an `OR` branch operand `sid` that is a node of a versioned
    /// object of its own, and returns the group its nested containment binds
    /// under.
    ///
    /// The node binding admits exactly the nodes the branch's containment
    /// filter admits under `anchor` ([`Self::anchor_correlation`]): the
    /// object's version in the operand's scope, `LATEST_VERSION` being the
    /// trunk head, and no restricted object. A logically deleted version has
    /// no node rows (RM common master06 §Logical Deletion: its data is Void),
    /// so it binds nothing, as on the non-`OR` path. Under an EHR the node's
    /// `ehr_id` is that EHR, whose own gates then cover it, as they cover a
    /// joined root; otherwise the binding carries the root gates of
    /// [`Self::apply_ehr_scope`] and [`Self::apply_population_gate`] itself.
    /// The node's version row is then bound one-to-one beside it, so the
    /// server-assigned `uid` synthesizes as for a joined root (RM common
    /// master06 §Version Identification), and the pair is reported to the
    /// access log and the ABAC scope collection. A row where the branch did
    /// not match carries `NULL` in both bindings.
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] for a version predicate on the operand, plus
    /// the condition lowerings.
    fn bind_versioned_branch(
        &mut self,
        anchor: &ExistsAnchor,
        sid: usize,
        r: &RmSource,
        siblings: &[Expr],
        contained: Option<&Contained>,
        ehr: Option<&str>,
    ) -> Result<VoGroup, AqlError> {
        let siblings = siblings.to_vec();
        let node = self.left_bind_rows(BoundTable::Node, |b, inner| {
            let mut on = vec![hot(inner, Node::Tier)];
            if let Some(e) = ehr {
                on.push(col(inner, Node::EhrId).eq(col(e, Ehr::Id)));
            }
            on.extend(b.rm_conds(inner, r)?);
            let mut scope = Query::select();
            scope.expr(Expr::val(1));
            b.anchor_correlation(&mut scope, anchor, inner, &r.rm_type, &r.scope)?;
            on.push(Expr::exists(scope));
            on.extend(siblings);
            on.extend(b.nested_condition(inner, r, contained)?);
            if ehr.is_none() {
                on.extend(b.root_scope_conds(col(inner, Node::EhrId)));
                on.extend(b.population_cond(inner));
            }
            Ok(on)
        })?;
        let vo = self.left_bind_rows(BoundTable::Version, |_, inner| {
            Ok(vec![
                hot(inner, VersionRow::Tier),
                col(inner, VersionRow::VoId).eq(col(&node, Node::VoId)),
                col(inner, VersionRow::SysVersion).eq(col(&node, Node::SysVersion)),
            ])
        })?;
        self.node_alias.insert(sid, node.clone());
        self.vo_alias.insert(sid, vo.clone());
        self.branch_roots.push((node.clone(), vo.clone()));
        Ok(VoGroup {
            node,
            vo,
            types: r.rm_type.clone(),
        })
    }

    /// Left-binds the nodes reached from `parent` over `edge` that also meet
    /// `conds`, and returns the alias whose columns are those of `node`.
    ///
    /// The binding yields one row per matching node and a
    /// single all-`NULL` row when none matches, so it multiplies rows without
    /// ever removing one. `conds` builds its conditions over the inner node
    /// alias it is handed. On the streaming shape the binding is a
    /// `LEFT JOIN LATERAL` behind the `OFFSET 0` fence; on the flat shape, whose
    /// comma-separated FROM list a join could not reference, it is a `LATERAL`
    /// subquery item of that list over a one-row left join, behind the same
    /// fence (PostgreSQL 18 §7.2.1.5 LATERAL Subqueries). No openEHR spec
    /// governs the join mechanics — our own design.
    ///
    /// # Errors
    /// Whatever `conds` returns.
    pub(super) fn left_bind(
        &mut self,
        parent: &str,
        edge: BoundEdge<'_>,
        conds: impl FnOnce(&mut Self, &str) -> Result<Vec<Expr>, AqlError>,
    ) -> Result<String, AqlError> {
        self.left_bind_rows(BoundTable::Node, |b, inner| {
            let edge = match edge {
                BoundEdge::Attribute(attribute) => attribute_child(inner, parent, attribute),
                BoundEdge::Descendant => strict_descendant(inner, parent),
            };
            let mut on = vec![hot(inner, Node::Tier), edge];
            on.extend(conds(b, inner)?);
            Ok(on)
        })
    }

    /// Left-binds the rows of `table` that meet every condition `conds` builds
    /// over the inner alias it is handed, and returns the alias whose columns
    /// are those of `table`.
    ///
    /// One row per admitted row, and a single all-`NULL` row when none is
    /// admitted, in the two shapes [`Self::left_bind`] describes.
    ///
    /// # Errors
    /// Whatever `conds` returns.
    fn left_bind_rows(
        &mut self,
        table: BoundTable,
        conds: impl FnOnce(&mut Self, &str) -> Result<Vec<Expr>, AqlError>,
    ) -> Result<String, AqlError> {
        let alias = format!("b{}", self.next_ctr());
        let inner = format!("{alias}_n");
        let on = conds(self, &inner)?
            .into_iter()
            .reduce(sea_query::ExprTrait::and)
            .unwrap_or_else(|| Expr::val(true));
        if self.streaming {
            let mut sub = Query::select();
            sub.column((Alias::new(inner.as_str()), Asterisk))
                .and_where(on)
                .offset(0);
            match table {
                BoundTable::Node => sub.from_as(Node::Table, Alias::new(inner.as_str())),
                BoundTable::Version => sub.from_as(VersionRow::Table, Alias::new(inner.as_str())),
            };
            self.q.join_lateral(
                JoinType::LeftJoin,
                sub,
                Alias::new(alias.as_str()),
                Expr::val(true),
            );
        } else {
            self.q.from_function(
                binding_rows(table, &alias, &inner, on),
                Alias::new(alias.as_str()),
            );
        }
        Ok(alias)
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
                // NOTE: QUERY master03 §Containment: `NOT CONTAINS` is "the absence of any
                // containment relationship between a parent and potential child items", the EHR included.
                let anchor = match (vo, ehr) {
                    (Some(parent), _) => ExistsAnchor::Vo(parent.node, parent.types),
                    (None, Some(e)) => ExistsAnchor::Ehr(e.to_owned()),
                    (None, None) => {
                        return Err(SqlError::Unsupported(
                            "NOT CONTAINS without a containing object".to_owned(),
                        )
                        .into());
                    }
                };
                // the exclusion is the negation of "parent contains
                // <tree>" — the same disjunction-capable `EXISTS` builder,
                // negated. This generalises NOT CONTAINS to compound (AND/OR)
                // and further-nested operands (QUERY master03 §Containment,
                // §NOT).
                let inner = self.contained_exists(&anchor, &c.tree)?;
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
        // The edge kind is classified from the RM relationship of BOTH
        // operands' types: the child alone does not determine the join shape.
        let edge = match vo {
            Some(parent) => Some(classify_edge(&parent.types, &r.rm_type)?),
            None => None,
        };
        let structural = match (edge, vo) {
            (Some(EdgeKind::Structural), Some(parent)) => Some(parent),
            _ => None,
        };
        match structural {
            Some(parent) if self.ctx.names_ehrs() => {
                self.q.from_function(
                    contained_rows(&node, &parent.node),
                    Alias::new(node.as_str()),
                );
            }
            _ => {
                self.q.from_as(Node::Table, Alias::new(node.as_str()));
            }
        }
        self.q.and_where(hot(&node, Node::Tier));
        self.node_alias.insert(sid, node.clone());
        for cond in self.rm_conds(&node, r)? {
            self.q.and_where(cond);
        }

        if let Some(parent) = structural {
            self.q.and_where(strict_descendant(&node, &parent.node));
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
            // A class that occurs only as its object's root is the `num = 0`
            // row, so the node probe is one primary-key row, not a scan of the
            // whole version's nodes filtered by type (#4089).
            if is_root_only(&r.rm_type) {
                self.q.and_where(col(&node, Node::Num).eq(0));
            }
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
                        sub.and_where(strict_descendant(alias, parent));
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
                                bind_head(sub, &voa, true, self.gates);
                            }
                            VersionScope::All => {
                                bind_head(sub, &voa, false, self.gates);
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
                                bind_head(sub, &voa, true, self.gates);
                            }
                            VersionScope::All => {
                                bind_head(sub, &voa, false, self.gates);
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
                        bind_head(sub, &voa, true, self.gates);
                    }
                    VersionScope::All => {
                        bind_head(sub, &voa, false, self.gates);
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

    /// Restricts every versioned-object root and every EHR source to the EHRs
    /// the query may read: the whole-EHR restriction, the explicit `ehr_ids`
    /// scope, the per-EHR access decision and the ABAC patient scope.
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
            // NOTE: EHDS Art. 8 (`docs/law/eu/ehds/text.html`), a whole-EHR access
            // restriction withholds the EHR itself while the request enforces it.
            if self.gates.access_restricted {
                self.q
                    .and_where(col(&alias, Ehr::AccessRestrictedAt).is_null());
            }
        }
        // Multi-EHR scoping restricts every VO root and every EHR source. A
        // bare `FROM EHR e` (no CONTAINS) has no VO root — the scope
        // must bind the EHR source itself, or a scoped query would run
        // over the whole population (ITS-REST query Request.md: `ehr_id`
        // "used to execute the query within a single EHR context").
        let roots = self.group_roots.clone();
        let ehrs = self.ehr_alias.values().cloned().collect::<Vec<_>>();
        let scoped = roots
            .iter()
            .map(|root| col(root, Node::EhrId))
            .chain(ehrs.iter().map(|alias| col(alias, Ehr::Id)));
        for cond in scoped
            .filter_map(|target| self.ehr_ids_cond(target))
            .collect::<Vec<_>>()
        {
            self.q.and_where(cond);
        }
        // The per-EHR access decision binds every VO root and EHR source the
        // same way the explicit scope does, so no query shape reaches a
        // withheld EHR (RM ehr `ehr_access.adoc` §`EHR_ACCESS` Class).
        let accessed = roots
            .iter()
            .map(|root| col(root, Node::EhrId))
            .chain(ehrs.iter().map(|alias| col(alias, Ehr::Id)));
        for cond in accessed
            .filter_map(|target| self.access_cond(target))
            .collect::<Vec<_>>()
        {
            self.q.and_where(cond);
        }
        // The ABAC patient scope binds the same targets: a query with no
        // versioned-object root (`FROM EHR e`, an `ehr_status` path, an `OR`
        // under the EHR) still reads the `ehr` row itself.
        let subject = roots
            .iter()
            .map(|root| self.subject_cond(col(root, Node::EhrId)))
            .chain(ehrs.iter().map(|alias| self.ehr_subject_cond(alias)));
        for cond in subject.flatten().collect::<Vec<_>>() {
            self.q.and_where(cond);
        }
    }

    /// The explicit `ehr_ids` scope on the EHR id `target`, when the query is
    /// scoped (`ehr_ids: List<UUID>`; the single-`ehr_id` REST case is the
    /// one-element set, and an empty set leaves the population gate in charge).
    ///
    /// The set is ONE array bind, `ehr_id = ANY($ids)`, so a cohort of any size
    /// stays inside the 65535-parameter wire limit (`PostgreSQL` 18
    /// §Frontend/Backend Protocol, Bind, whose parameter count is an `int16`,
    /// <https://www.postgresql.org/docs/18/protocol-message-formats.html>).
    fn ehr_ids_cond(&self, target: Expr) -> Option<Expr> {
        if self.ctx.ehr_ids.is_empty() {
            return None;
        }
        let ids: Vec<Uuid> = self.ctx.ehr_ids.iter().map(|id| id.0).collect();
        Some(target.eq(Expr::from(PgFunc::any(Expr::val(ids)))))
    }

    /// The caller's per-EHR access decision on the EHR id `target`, when it
    /// restricts anything (RM ehr `ehr_access.adoc` §`EHR_ACCESS` Class).
    fn access_cond(&self, target: Expr) -> Option<Expr> {
        match &self.ctx.ehr_access {
            EhrAccessScope::Unrestricted => None,
            EhrAccessScope::Excluding(ids) => {
                let ids: Vec<Uuid> = ids.iter().map(|id| id.0).collect();
                Some(target.ne(Expr::from(PgFunc::all(Expr::val(ids)))))
            }
            EhrAccessScope::Only(ids) => {
                let ids: Vec<Uuid> = ids.iter().map(|id| id.0).collect();
                Some(target.eq(Expr::from(PgFunc::any(Expr::val(ids)))))
            }
        }
    }

    /// The ABAC patient scope on the EHR id `target` of a versioned-object
    /// root, when one is set: `target` is the id of an `ehr` row
    /// [`subject::matches`] admits.
    fn subject_cond(&self, target: Expr) -> Option<Expr> {
        // NOTE: the ABAC patient scope restricts every VO root and EHR source to
        // the caller's patient EHRs whatever the projection (no openEHR spec
        // governs this — our own extension).
        let scope = self.ctx.subject_scope.as_ref()?;
        let mut sub = Query::select();
        sub.expr(col(subject::SUBJECT_ROW, Ehr::Id))
            .from_as(Ehr::Table, Alias::new(subject::SUBJECT_ROW))
            .and_where(subject::matches(subject::SUBJECT_ROW, scope));
        Some(target.in_subquery(sub))
    }

    /// The ABAC patient scope on the `ehr` row bound as `alias`, when one is
    /// set: [`subject::matches`] on that row itself.
    fn ehr_subject_cond(&self, alias: &str) -> Option<Expr> {
        let scope = self.ctx.subject_scope.as_ref()?;
        Some(subject::matches(alias, scope))
    }

    /// Every [`Self::apply_ehr_scope`] condition a versioned-object root puts
    /// on its EHR id `target`, for a root no bound EHR alias covers.
    fn root_scope_conds(&self, target: Expr) -> Vec<Expr> {
        [
            self.ehr_ids_cond(target.clone()),
            self.access_cond(target.clone()),
            self.subject_cond(target),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// The population gate of [`Self::gate_vo_root`] for the node `root`, as
    /// one `EXISTS` a left binding can carry; `None` on a query aimed at
    /// named EHRs ([`super::SqlCtx::population_gated`]).
    fn population_cond(&mut self, root: &str) -> Option<Expr> {
        if !self.ctx.population_gated() {
            return None;
        }
        let alias = format!("qg{}", self.next_ctr());
        let mut sub = Query::select();
        sub.expr(Expr::val(1))
            .from_as(Ehr::Table, Alias::new(alias.as_str()))
            .and_where(col(&alias, Ehr::Id).eq(col(root, Node::EhrId)))
            .and_where(col(&alias, Ehr::IsQueryable).eq(true))
            .and_where(col(&alias, Ehr::RestrictedAt).is_null())
            .and_where(research_objection_clear(&alias));
        Some(Expr::exists(sub))
    }

    /// The query-population gate. RM `EHR_STATUS.is_queryable`
    /// (`org.openehr.rm.ehr.ehr_status.adoc`): "True if this EHR should be
    /// included in population queries". Every EHR root of a population query,
    /// bare `EHR` sources (`ehr.id`) and VO roots (`node.ehr_id`) alike, is
    /// restricted to that set: an unscoped query and a population subset
    /// ([`super::EhrScopeKind::PopulationSubset`]). A query aimed at EHRs its caller
    /// named ([`super::EhrScopeKind::Named`]) is not gated.
    pub(super) fn apply_population_gate(&mut self) {
        if !self.ctx.population_gated() {
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
    /// every current `EHR_STATUS` root. A full-population
    /// query runs over every EHR whose status has `is_queryable` set. No openEHR
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
        self.ehr_status_vo.insert(ehr_sid, vo);
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
        for predicate in self.gates.on_head(&h) {
            cond = cond.and(predicate);
        }
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

    pub(super) fn push_scope(&mut self, voa: &str, scope: &VersionScope) -> Result<(), AqlError> {
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
                let rhs = cast(Expr::val(self.bind_value(&p.value)?), "text");
                let lhs = version_field_expr(voa, || self.ensure_audit(voa), p.field);
                self.q
                    .and_where(lhs.binary(super::expr::binoper(p.op), rhs));
            }
        }
        Ok(())
    }

    pub(super) fn push_ehr_predicate(
        &mut self,
        alias: &str,
        p: &EhrPredicate,
    ) -> Result<(), AqlError> {
        let cond = match p.field {
            EhrField::EhrId => match self.ehr_id_value(&p.value)? {
                Some(raw) => super::predicate::ehr_id_compare(alias, p.op, &raw),
                None => cast(col(alias, Ehr::Id), "text").binary(
                    super::expr::binoper(p.op),
                    super::value::coerce_rhs(self.bind_value(&p.value)?, Coercion::Text)?,
                ),
            },
            EhrField::TimeCreated => col(alias, Ehr::TimeCreated).binary(
                super::expr::binoper(p.op),
                cast(Expr::val(self.bind_value(&p.value)?), "timestamptz"),
            ),
            EhrField::SystemId | EhrField::Whole => {
                return Err(SqlError::Unsupported(
                    "EHR system_id / whole-EHR predicate".to_owned(),
                )
                .into());
            }
        };
        self.q.and_where(cond);
        Ok(())
    }
}

/// The VO-root RM types the store versions independently (RM common master06
/// versioned objects), decided from the storage domain itself
/// ([`crate::versioning::Kind`], the `version.kind` discriminants) rather
/// than a restated list — a kind added there is a VO root here.
///
/// The demographic kinds are included: a served demographic root opens its own
/// version group exactly as a clinical root does, and one nested under another
/// class is refused before any SQL is built
/// ([`crate::aql::error::AqlFeatureError::DemographicRootPlacement`]).
pub(super) fn is_vo_root_type(t: &str) -> bool {
    crate::versioning::Kind::from_type(t).is_some()
}

/// Whether every class in `types` occurs only as the root (`num = 0`) of its
/// versioned object.
///
/// The EHR holds its `EHR_STATUS`, `EHR_ACCESS` and `COMPOSITION`s as separate
/// versioned objects (RM ehr master04 §The Parts of the EHR), and a
/// `COMPOSITION` holds only `CONTENT_ITEM`s (RM ehr master05), so none of the
/// three nests in a tree; a `FOLDER` nests in `FOLDER.folders` and is left out.
/// Storing the root at `num = 0` is our own design (`crate::storage::codec`).
pub(super) fn is_root_only(types: &crate::aql::ir::TypeSet) -> bool {
    !types.is_empty()
        && types
            .names()
            .iter()
            .all(|t| matches!(t.as_str(), "COMPOSITION" | "EHR_STATUS" | "EHR_ACCESS"))
}

/// The `LATERAL` FROM item of a left binding on a comma-separated FROM list:
/// a subquery over a one-row left join, so the binding yields one row per node
/// `on` admits and one all-`NULL` row when it admits none, with the `node`
/// columns. `inner` is the node alias `on` is written over.
pub(super) fn left_binding_rows(alias: &str, inner: &str, on: Expr) -> sea_query::FunctionCall {
    binding_rows(BoundTable::Node, alias, inner, on)
}

/// The `LATERAL` FROM item of the content node `alias` contained in the node
/// `parent` on a query scoped to EHRs its caller named: the strict
/// descendants of `parent`, behind the `OFFSET 0` fence.
///
/// The fence holds the join order whatever the statistics: the planner reads
/// the named EHRs' objects first and each object's own nodes after them
/// (PostgreSQL 18 §7.2.1.5 LATERAL Subqueries). As a plain FROM item the
/// contained node could lead the join instead, through the archetype index
/// over the whole population, whenever the archetype's rows fell out of the
/// most-common-value list of the archetype statistics and were estimated at
/// one row; the storage bench's wide projection over one EHR then read ten
/// times its pages in the rounds that sampled so (#4487). No openEHR spec
/// governs the join mechanics: our own design.
fn contained_rows(alias: &str, parent: &str) -> sea_query::FunctionCall {
    let inner = format!("{alias}_s");
    let mut rows = Query::select();
    rows.column((Alias::new(inner.as_str()), Asterisk))
        .from_as(Node::Table, Alias::new(inner.as_str()))
        .and_where(hot(&inner, Node::Tier))
        .and_where(strict_descendant(&inner, parent))
        .offset(0);
    sea_query::Func::cust(Alias::new("LATERAL")).arg(Expr::from(rows))
}

/// The `LATERAL` FROM item of a left binding over `table`, in the shape
/// [`left_binding_rows`] describes; `node` rows come in document order.
///
/// The planner estimates the rows of a subquery from its own plan, about one
/// per probe of a node's children, where a set-returning function over an
/// array it cannot see into is estimated at ten rows: a chain of such
/// bindings multiplied the estimate tenfold per step, which chose the join
/// order and switched on JIT compilation from the inflated total cost
/// (PostgreSQL 18 §30.2 When to JIT?, #4430, #4431). sea-query builds a
/// `LATERAL` subquery only as a join, which a comma-separated item cannot be,
/// so the item is written as a call of `LATERAL` on the parenthesized
/// subquery, which PostgreSQL's grammar reads as `LATERAL select_with_parens`.
fn binding_rows(table: BoundTable, alias: &str, inner: &str, on: Expr) -> sea_query::FunctionCall {
    let mut one = Query::select();
    one.expr(Expr::val(1));
    let mut rows = Query::select();
    rows.column((Alias::new(inner), Asterisk))
        .from_subquery(one, Alias::new(format!("{alias}_one")));
    match table {
        BoundTable::Node => rows
            .join_as(JoinType::LeftJoin, Node::Table, Alias::new(inner), on)
            .order_by_expr(col(inner, Node::Num), sea_query::Order::Asc),
        BoundTable::Version => {
            rows.join_as(JoinType::LeftJoin, VersionRow::Table, Alias::new(inner), on)
        }
    };
    rows.offset(0);
    sea_query::Func::cust(Alias::new("LATERAL")).arg(Expr::from(rows))
}
