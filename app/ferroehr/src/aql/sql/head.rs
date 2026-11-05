// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How a version row reaches its object's head row and its node rows.
//!
//! A version whose content equals its predecessor's reads its predecessor's
//! node rows (`version.content_key`, #4415), so one node row belongs to every
//! version of an identical-content chain. Under `ALL_VERSIONS` and at an
//! instant every version the content key matches can be an answer row, and
//! the node rows join the version row on that key. Under `LATEST_VERSION` one
//! version answers, the head's, so the node rows join the head row on its own
//! copy of the key (`vo_head.trunk_head_content_key`, #4944): one head row
//! per object, and the version row is reached through the head's primary key.
//!
//! The node's, the head's and the version's `vo_id` are still one
//! equivalence class, so the planner may probe the version by `vo_id` alone
//! and reach a shared row through every version of its chain before the head
//! keeps one. On statistics taken before a chain grew it priced that probe at
//! one row and took it; the migration that adds the key declares the
//! partitions' `vo_id` distinct count, so the probe is priced at two rows and
//! the exact path through the head always costs less. No openEHR spec governs
//! storage access paths: our own design.

use sea_query::{Alias, Expr, ExprTrait as _, JoinType, Query, SelectStatement};

use crate::aql::error::AqlError;
use crate::aql::ir::{Source, VersionField, VersionScope};
use crate::db::iden::{VersionRow, VoHead};

use super::Builder;
use super::expr::{binoper, cast, col, hot, version_field_expr};
use super::value::coerce_rhs;

/// Where [`bind_head`] puts the head row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeadPlacement {
    /// A FROM item with its predicates in the WHERE clause: the flat shape
    /// and every subquery.
    From,
    /// A `JOIN` after the version row: the streaming shape.
    Join,
    /// Already joined ahead of the version row ([`Builder::stream_spine`]):
    /// only the predicates are added.
    Joined,
}

/// The alias of the head row [`bind_head`] binds for `version_alias`.
pub(super) fn head_alias(version_alias: &str) -> String {
    format!("{version_alias}_head")
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
pub(super) fn bind_head(
    q: &mut SelectStatement,
    version_alias: &str,
    latest: bool,
    gates: super::gates::Gates,
    placement: HeadPlacement,
) -> String {
    let h = head_alias(version_alias);
    let mut cond = col(&h, VoHead::VoId)
        .eq(col(version_alias, VersionRow::VoId))
        .and(col(&h, VoHead::RestrictedAt).is_null());
    for predicate in gates.on_head(&h) {
        cond = cond.and(predicate);
    }
    if latest {
        cond = cond.and(
            col(version_alias, VersionRow::SysVersion).eq(col(&h, VoHead::TrunkHeadSysVersion)),
        );
    }
    match placement {
        HeadPlacement::Join => {
            q.join_as(JoinType::Join, VoHead::Table, Alias::new(h.as_str()), cond);
        }
        HeadPlacement::From => {
            q.from_as(VoHead::Table, Alias::new(h.as_str()));
            q.and_where(cond);
        }
        HeadPlacement::Joined => {
            q.and_where(cond);
        }
    }
    h
}

/// The node rows a version row `voa` reads: its own, or the rows of the
/// version `content_sys_version` names when its content equalled its
/// predecessor's at commit, the stored `version.content_key` (#4415). A plain
/// column the planner joins and estimates on; a node-to-node join stays on the
/// node key, both rows lying in one tree.
pub(super) fn content_key(voa: &str) -> Expr {
    col(voa, VersionRow::ContentKey)
}

/// The column a node row's `sys_version` joins for the version row `voa`
/// under `scope`: the head's content key under `LATEST_VERSION`, whose head
/// [`bind_head`] binds beside `voa`, and the version's own [`content_key`]
/// otherwise.
pub(super) fn node_key(voa: &str, scope: &VersionScope) -> Expr {
    match scope {
        VersionScope::Latest => col(&head_alias(voa), VoHead::TrunkHeadContentKey),
        VersionScope::All | VersionScope::Predicate(_) => content_key(voa),
    }
}

/// The version row `voa` is the trunk head of the object `vo_id` names, read
/// from its head row by primary key, under `LATEST_VERSION`; `None` under any
/// other scope, where every version sharing the tree is an answer row.
pub(super) fn trunk_head_version(scope: &VersionScope, voa: &str, vo_id: Expr) -> Option<Expr> {
    if !matches!(scope, VersionScope::Latest) {
        return None;
    }
    let h = head_alias(&format!("{voa}_pin"));
    let mut sub = Query::select();
    sub.column((Alias::new(h.as_str()), VoHead::TrunkHeadSysVersion))
        .from_as(VoHead::Table, Alias::new(h.as_str()))
        .and_where(col(&h, VoHead::VoId).eq(vo_id));
    Some(col(voa, VersionRow::SysVersion).eq(Expr::SubQuery(None, Box::new(sub.into()))))
}

/// Version-at-time: the version row is the TRUNK version in force at `at`.
///
/// Validity is derived rather than stored: version *i* is valid over
/// `[committed_at_i, committed_at_i+1)`, so the row in force at an instant is
/// the trunk row with the greatest `committed_at` at or before it (RM common
/// master06 §Committal and Audits gives `committed_at` its meaning; the
/// derivation is our own storage design).
pub(super) fn trunk_at_instant(version_alias: &str, at: Expr) -> Expr {
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

impl Builder<'_> {
    /// [`node_key`] of the version row `voa` under the scope of the source
    /// that opened it.
    pub(super) fn node_key_of(&self, voa: &str) -> Expr {
        let scope = self.vo_alias.iter().find_map(|(sid, alias)| {
            match (alias == voa, self.ir.sources.get(*sid)) {
                (true, Some(Source::Rm(r))) => Some(&r.scope),
                _ => None,
            }
        });
        // NOTE: every version alias is opened by an RM source (`vo_alias`); the content-key
        // join is the right answer under every scope, so an alias without one keeps it.
        scope.map_or_else(|| content_key(voa), |scope| node_key(voa, scope))
    }

    /// Binds the head row of the version row `voa` and the predicates its
    /// VERSION scope selects.
    pub(super) fn push_scope(&mut self, voa: &str, scope: &VersionScope) -> Result<(), AqlError> {
        let placement = match (self.streaming, self.joined_heads.contains(&head_alias(voa))) {
            (_, true) => HeadPlacement::Joined,
            (true, false) => HeadPlacement::Join,
            (false, false) => HeadPlacement::From,
        };
        match scope {
            VersionScope::Latest => {
                // LATEST_VERSION = the latest TRUNK version (RM common master06
                // latest_trunk_version; open branch tips coexist and are not
                // "the latest version" of the container).
                bind_head(&mut self.q, voa, true, self.gates, placement);
            }
            VersionScope::All => {
                bind_head(&mut self.q, voa, false, self.gates, placement);
            }
            VersionScope::Predicate(p) if p.field == VersionField::TimeCommitted => {
                bind_head(&mut self.q, voa, false, self.gates, placement);
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
                bind_head(&mut self.q, voa, false, self.gates, placement);
                let rhs = coerce_rhs(self.bind_value(&p.value)?, p.field.coercion())?;
                let lhs = version_field_expr(voa, || self.ensure_audit(voa), p.field);
                self.q.and_where(lhs.binary(binoper(p.op), rhs));
            }
        }
        Ok(())
    }
}
