// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR scope and the population gate of a lowered query: every
//! versioned-object root and every `EHR` source restricted to the EHRs the
//! query may read (the whole-EHR restriction, the explicit `ehr_ids` scope,
//! the per-EHR access decision, the ABAC patient scope), and the
//! `EHR_STATUS.is_queryable` population gate (RM ehr
//! `org.openehr.rm.ehr.ehr_status.adoc`). No openEHR spec governs the join
//! mechanics: our own design.

use sea_query::extension::postgres::PgFunc;
use sea_query::{Alias, Expr, ExprTrait as _, JoinType, Query};
use uuid::Uuid;

use crate::db::iden::{Ehr, Node};

use super::expr::col;
use super::from::research_objection_clear;
use super::{Builder, EhrAccessScope, subject};

impl Builder<'_> {
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
            for predicate in self.gates.on_ehr(&alias) {
                self.q.and_where(predicate);
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
        // The per-EHR access decision (RM ehr `ehr_access.adoc` §`EHR_ACCESS`
        // Class) and the EHDS Art. 10 opt-out bind every VO root and EHR source
        // the way the explicit scope does, so no query shape reaches either.
        let accessed = roots
            .iter()
            .map(|root| col(root, Node::EhrId))
            .chain(ehrs.iter().map(|alias| col(alias, Ehr::Id)));
        for cond in accessed
            .flat_map(|t| [self.access_cond(t.clone()), self.gates.on_ehr_id(t)])
            .flatten()
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
    pub(super) fn root_scope_conds(&self, target: Expr) -> Vec<Expr> {
        [
            self.ehr_ids_cond(target.clone()),
            self.access_cond(target.clone()),
            self.gates.on_ehr_id(target.clone()),
            self.subject_cond(target),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// The population gate of [`Self::gate_vo_root`] for the node `root`, as
    /// one `EXISTS` a left binding can carry; `None` on a query aimed at
    /// named EHRs ([`super::SqlCtx::population_gated`]).
    pub(super) fn population_cond(&mut self, root: &str) -> Option<Expr> {
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
    /// The per-object predicate (`head::bind_head`) already removes every
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
}
