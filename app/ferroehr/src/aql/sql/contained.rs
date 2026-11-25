// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FROM item of a node read inside the object of the node that contains
//! it (`super::from`).

use sea_query::{Alias, Asterisk, Expr, Query};

use crate::db::iden::Node;

use super::expr::hot;
use super::from::strict_descendant;

/// The `LATERAL` FROM item of the content node `alias` contained in the node
/// `parent`, on a query scoped to named EHRs or a flat statement with a WHERE
/// anchor: the strict descendants of `parent`, behind the `OFFSET 0` fence.
///
/// The fence holds the join order: the named EHRs' objects first, each
/// object's nodes after (PostgreSQL 18 §7.2.1.5 LATERAL Subqueries; #4487,
/// #6243, #6263). `conds`, over the alias `<alias>_s`, are each stated as
/// `(…) IS TRUE`, which no index matches, so the item still reads through
/// `node_hot_pkey`. No openEHR spec governs the join mechanics: our own
/// design.
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
