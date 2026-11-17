// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access condition of a query pinned to one EHR in its SQL statement.
//!
//! The service judged the EHR before the statement
//! ([`crate::access_eval::condition::pinned`]), so each versioned object
//! carries only the part of the verdict that reads the object, and the EHR
//! itself a comparison with its id: no probe of the settings row per row,
//! and no class condition for the planner to plan. One uncorrelated check,
//! which PostgreSQL runs once, holds the verdict to the revision it was
//! judged on. No openEHR spec governs this: our own design (#5746).

use sea_query::{Expr, ExprTrait as _};

use super::access::item_pred;
use super::expr::col;
use crate::access_eval::condition::pinned::PinnedEhr;
use crate::access_eval::scope::Pred;
use crate::db::iden::VoHead;

/// The predicate on the head row `h` of a versioned object: of the pinned
/// EHR, and admitted by its verdict.
pub(super) fn on_head(pinned: &PinnedEhr, h: &str) -> Expr {
    if pinned.admitted == Pred::False {
        return Expr::val(false);
    }
    let of_ehr = col(h, VoHead::EhrId).eq(Expr::val(pinned.ehr));
    if pinned.admitted == Pred::True {
        return of_ehr;
    }
    of_ehr.and(item_pred(&pinned.admitted, h))
}

/// The predicate on the EHR id `target`: the pinned EHR, when the caller may
/// read some of it.
pub(super) fn on_ehr_id(pinned: &PinnedEhr, target: &Expr) -> Expr {
    if pinned.admitted == Pred::False {
        return Expr::val(false);
    }
    target.clone().eq(Expr::val(pinned.ehr))
}

/// The revision of the pinned EHR's settings row and subject in force is the
/// one its verdict was judged on: a scalar subquery with no outer reference,
/// which PostgreSQL evaluates once per execution, reading the revision from
/// the view `access_revision_of` inside the statement, and which fails the
/// statement with SQLSTATE `ZA001` through `access_stale_copy()` when the
/// revision moved (`clinical/0052_access_revision_view.sql`), so a stale
/// verdict answers no row, count or page (#5794, #5888). A missing EHR reads
/// as the revision of nulls, as the store writes it.
pub(super) fn in_force(pinned: &PinnedEhr) -> Expr {
    Expr::cust_with_exprs(
        "(SELECT CASE WHEN coalesce(CAST((SELECT ar.revision FROM access_revision_of ar \
         WHERE ar.ehr_id = $1) AS jsonb), '[null, null, null]'::jsonb) \
         IS NOT DISTINCT FROM CAST($2 AS jsonb) THEN true ELSE access_stale_copy() END)",
        [Expr::val(pinned.ehr), Expr::val(pinned.revision_json())],
    )
}
