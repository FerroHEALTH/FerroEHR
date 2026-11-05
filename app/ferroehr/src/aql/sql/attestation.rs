// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ORIGINAL_VERSION.attestations` (RM common master06 §Attestation): the reads
//! of a `v/attestations[...]/...` path off the `vo_attestation` rows of the
//! version row `v` binds.
//!
//! Each read is a correlated subquery over the attestations of that one
//! version, at committal and after it, as the version read serves them. The
//! predicate on `attestations` is a `jsonb_path_exists` filter on the stored
//! canonical `ATTESTATION` and the residual path a jsonpath into it, every
//! compared value bound in the `vars` jsonb. The rows carry `tier = 'hot'` as a
//! literal, so the cold partition is pruned at plan time, and the
//! `(vo_id, sys_version)` probe is the one `idx_vo_attestation_hot_version`
//! serves. No openEHR spec governs the storage or the lowering: our own
//! design.

use sea_query::{Alias, Expr, ExprTrait as _, Order, Query};

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::LeafPath;
use crate::db::iden::{VersionRow, VoAttestation};

use super::expr::{call, cast, col, hot, jsonb_path};
use super::jsonpath::jsonpath_vars;
use super::value::{ItemWalk, coerce_value, fragment_items};
use super::{Builder, ValueMode};

impl Builder<'_> {
    /// The walk over every attestation of the version `leaf` is rooted at
    /// that satisfies its predicate, with the value `leaf` reads off one of
    /// them, and the alias of the attestation row.
    ///
    /// A residual path that crosses a list (`items`) yields one item per
    /// element through the set-returning `jsonb_path_query`.
    ///
    /// # Errors
    /// [`AqlError`] when the VERSION variable is bound to no version row, a
    /// parameter is unbound, or a compared value is no scalar.
    #[expect(
        clippy::disallowed_types,
        reason = "the jsonpath vars object is SQL/JSON wire material (the bound \
                  third argument of jsonb_path_exists), not an RM value — no \
                  generated type models it"
    )]
    pub(super) fn attestation_walk(
        &mut self,
        leaf: &LeafPath,
        mode: ValueMode,
    ) -> Result<(ItemWalk, String), AqlError> {
        let voa = self
            .version_vo
            .get(&leaf.source.0)
            .cloned()
            .ok_or_else(|| {
                SqlError::Unsupported("an attestation path without a bound version".to_owned())
            })?;
        let alias = format!("x{}", self.next_ctr());
        let mut sub = Query::select();
        sub.from_as(VoAttestation::Table, Alias::new(alias.as_str()));
        sub.and_where(hot(&alias, VoAttestation::Tier));
        sub.and_where(col(&alias, VoAttestation::VoId).eq(col(&voa, VersionRow::VoId)));
        sub.and_where(col(&alias, VoAttestation::SysVersion).eq(col(&voa, VersionRow::SysVersion)));
        let data = col(&alias, VoAttestation::Data);
        if let Some(pred) = &leaf.root_predicate {
            let mut vars = serde_json::Map::new();
            let conds = self.standard_conds(&pred.standard, &mut vars)?;
            if !conds.is_empty() {
                let jp = format!("$ ? ({})", conds.join(" && "));
                let mut args = vec![data.clone(), cast(Expr::val(jp), "jsonpath")];
                args.extend(jsonpath_vars(vars));
                sub.and_where(call("jsonb_path_exists", args));
            }
        }
        let base = match self.fragment_path(leaf)? {
            None => data,
            Some((jp, vars)) if leaf.fragment_multi_valued() => {
                fragment_items(&mut sub, data, &jp, vars, self.next_ctr())
            }
            Some((jp, vars)) => jsonb_path(data, &jp, vars),
        };
        let walk = ItemWalk {
            sub,
            item: coerce_value(base, mode, leaf),
            bindings: Vec::new(),
        };
        Ok((walk, alias))
    }

    /// The value an attestation path reads in one row.
    ///
    /// Without `last`, a projection reads every matching attestation as ONE
    /// jsonb array cell, `NULL` when there is none
    /// (`ORIGINAL_VERSION.Attestations_valid` admits no empty list), and a
    /// scalar context (ORDER BY, a function argument, a comparison with
    /// another path) reads the first. With `last` (the `LAST(...)`
    /// extension) every context reads the path off the most recent matching
    /// attestation, `NULL` where that one lacks it.
    ///
    /// The order is the one the version read lists them in: those on the
    /// version at committal first, then by commit instant.
    ///
    /// # Errors
    /// [`AqlError`] from [`Self::attestation_walk`].
    pub(super) fn attestation_value(
        &mut self,
        leaf: &LeafPath,
        last: bool,
        mode: ValueMode,
    ) -> Result<Expr, AqlError> {
        let (walk, alias) = self.attestation_walk(leaf, mode)?;
        let mut sub = walk.sub;
        let at_committal = col(&alias, VoAttestation::AtCommittal);
        let time_committed = col(&alias, VoAttestation::TimeCommitted);
        let id = col(&alias, VoAttestation::Id);
        if last {
            sub.expr(walk.item);
            sub.order_by_expr(at_committal, Order::Asc);
            sub.order_by_expr(time_committed, Order::Desc);
            sub.order_by_expr(id, Order::Desc);
            sub.limit(1);
            return Ok(Expr::from(sub));
        }
        sub.and_where(walk.item.clone().is_not_null());
        // NOTE: QUERY master03 §Identified Paths is silent on projecting a
        // list-valued path; as for a list inside a node, every match is ONE cell.
        if matches!(mode, ValueMode::Projection) {
            sub.expr(Expr::cust_with_exprs(
                "jsonb_agg($1 ORDER BY $2 DESC, $3, $4)",
                [walk.item, at_committal, time_committed, id],
            ));
        } else {
            sub.expr(walk.item);
            sub.order_by_expr(at_committal, Order::Desc);
            sub.order_by_expr(time_committed, Order::Asc);
            sub.order_by_expr(id, Order::Asc);
            sub.limit(1);
        }
        Ok(Expr::from(sub))
    }
}
