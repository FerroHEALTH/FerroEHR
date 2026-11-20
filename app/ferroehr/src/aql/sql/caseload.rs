// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caseload a query request narrows the query to, as a condition of its
//! one SQL statement ([`crate::access_eval::caseload`]).

use sea_query::{Alias, Expr, ExprTrait as _, Query};

use super::access::{CARE_PROBE, census_text, probe_values};
use super::expr::col;
use crate::access_eval::caseload::Caseload;
use crate::db::iden::Ehr;

/// The alias of the `ehr` row a census caseload probe reads.
const CASELOAD: &str = "access_caseload_ehr";

/// Whether the EHR `target` is in the request's `caseload`: its subject
/// among the census, or a row of the care projection that reaches one of the
/// path's principals in one of the roles, by the same liveness and provider
/// as the care condition, on no self-declared relationship and on none a
/// principal of the caller wrote. No openEHR spec governs this: our own
/// design over the draft access control, Queries and the Caseload (#6056).
pub(super) fn in_caseload(caseload: &Caseload, target: Expr) -> Expr {
    match caseload {
        Caseload::Census {
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
                        Expr::val(census_text(subjects)),
                        col(CASELOAD, Ehr::SubjectNamespace),
                        col(CASELOAD, Ehr::SubjectId),
                        Expr::val(namespace.to_string()),
                    ],
                ));
            Expr::exists(probe)
        }
        Caseload::Projected {
            care,
            roles,
            principals: held,
        } => {
            let (terminologies, codes): (Vec<String>, Vec<String>) = roles
                .iter()
                .map(|role| {
                    (
                        role.terminology.clone().unwrap_or_default(),
                        role.code.clone(),
                    )
                })
                .unzip();
            let [target, principals, day, grace, issuers, ids, roles] =
                probe_values(care, target, Expr::val(roles.is_empty()));
            Expr::cust_with_exprs(
                format!(
                    "EXISTS ({CARE_PROBE} \
                     AND NOT access_care_row.self_declared \
                     AND (access_care_row.writer_id IS NULL \
                          OR access_care_row.writer_id <> ALL($8)) \
                     AND ($7 OR EXISTS (SELECT 1 FROM unnest(CAST($9 AS text[]), \
                          CAST($10 AS text[])) AS caseload_role (terminology, code) \
                          WHERE caseload_role.code = access_care_row.role_code \
                          AND caseload_role.terminology IN ('', access_care_row.role_terminology))))"
                ),
                [
                    target,
                    principals,
                    day,
                    grace,
                    issuers,
                    ids,
                    roles,
                    Expr::val(held.to_vec()),
                    Expr::val(terminologies),
                    Expr::val(codes),
                ],
            )
        }
    }
}
