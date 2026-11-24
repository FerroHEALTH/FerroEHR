// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caseload a query request narrows the query to, as a condition of its
//! one SQL statement ([`crate::access_eval::caseload`]). No openEHR spec
//! governs this: our own design over the draft access control, Queries and
//! the Caseload (#6056).

use sea_query::{Alias, Expr, ExprTrait, Query};

use super::care::{census_text_of, projected_need};
use super::expr::col;
use crate::access_eval::caseload::Reach;
use crate::db::iden::Ehr;

/// The alias of the `ehr` row a census caseload probe reads.
const CASELOAD: &str = "access_caseload_ehr";

/// Whether the EHR `target` is in the request's `caseload`: its subject
/// among the census, or a row of the care projection that meets one of the
/// caseload's needs, by the same liveness, provider and writer rule as the
/// care condition.
pub(super) fn in_caseload(caseload: &Reach, target: Expr) -> Expr {
    match caseload {
        Reach::Census {
            namespace,
            subjects,
        } => {
            let mut probe = Query::select();
            probe
                .expr(Expr::val(1))
                .from_as(Ehr::Table, Alias::new(CASELOAD))
                .and_where(col(CASELOAD, Ehr::Id).eq(target))
                .and_where(Expr::cust_with_exprs(
                    "CAST($1 AS jsonb) ? ($2 || chr(31) || \
                     CASE WHEN $2 = $4 THEN lower($3) ELSE $3 END)",
                    [
                        Expr::val(census_text_of(subjects)),
                        col(CASELOAD, Ehr::SubjectNamespace),
                        col(CASELOAD, Ehr::SubjectId),
                        Expr::val(namespace.to_string()),
                    ],
                ));
            Expr::exists(probe)
        }
        Reach::Projected { care, needs } => needs
            .iter()
            .map(|need| projected_need(care, need, target.clone(), false))
            .reduce(ExprTrait::or)
            .unwrap_or_else(|| Expr::val(false)),
    }
}
