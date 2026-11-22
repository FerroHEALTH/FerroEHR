// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FROM item of a node read inside the object of the node that contains
//! it (`super::from`).

use sea_query::{Alias, Asterisk, Expr, Query};

use crate::db::iden::Node;

use super::expr::hot;
use super::from::strict_descendant;

/// The `LATERAL` FROM item of the content node `alias` contained in the node
/// `parent` on a query scoped to EHRs its caller named, or on a flat
/// statement whose WHERE clause anchors a leaf no SELECT column binds: the
/// strict descendants of `parent`, behind the `OFFSET 0` fence.
///
/// The fence holds the join order whatever the statistics: the planner reads
/// the named EHRs' objects first and each object's own nodes after them
/// (PostgreSQL 18 §7.2.1.5 LATERAL Subqueries). As a plain FROM item the
/// contained node could lead the join instead, through the archetype index
/// over the whole population, whenever the archetype's rows fell out of the
/// most-common-value list of the archetype statistics and were estimated at
/// one row; the storage bench's wide projection over one EHR then read ten
/// times its pages in the rounds that sampled so (#4487). With such an anchor
/// the anchor is the entry, and a contained node estimated at one row was
/// rescanned with the WHERE clause's `EXISTS` once per anchor version, 32.7 M
/// buffers for 500 rows (#6263). No openEHR spec governs the join mechanics:
/// our own design.
///
/// `conds`, over the alias `<alias>_s`, keep only the rows the source admits.
/// Each is stated as `(…) IS TRUE`, which holds exactly when it does and
/// which no index matches, so the item still reads the parent's object
/// through `node_hot_pkey` and returns only the rows it keeps.
pub(super) fn contained_rows(
    alias: &str,
    parent: &str,
    conds: Vec<Expr>,
) -> sea_query::FunctionCall {
    let inner = format!("{alias}_s");
    let mut rows = Query::select();
    rows.column((Alias::new(inner.as_str()), Asterisk))
        .from_as(Node::Table, Alias::new(inner.as_str()))
        .and_where(hot(&inner, Node::Tier))
        .and_where(strict_descendant(&inner, parent));
    for cond in conds {
        rows.and_where(Expr::cust_with_exprs("($1) IS TRUE", [cond]));
    }
    rows.offset(0);
    sea_query::Func::cust(Alias::new("LATERAL")).arg(Expr::from(rows))
}
