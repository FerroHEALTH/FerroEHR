// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `WHERE` condition of an ITI-81 retrieval: each criterion on its column,
//! or on the search tokens and record columns it resolved to.

use crate::system_log::search::TokenMatch;
use crate::system_log::store::{AuditCriterion, AuditSearchFilter};

/// The `WHERE` condition for the ITI-81 filter: the date window, and each
/// criterion on its column or, with the token ids it resolved to in
/// `resolved` (one entry per criterion), on `search_keys`, the keys of the
/// records' token index (`audit.search_keys`).
///
/// Every column name is a literal here; every caller-supplied value goes through
/// `Expr::val` and therefore binds as a parameter rather than entering the SQL
/// text. That split is the property `sql_injection`-style tests pin.
pub(crate) fn search_condition(
    filter: &AuditSearchFilter,
    resolved: &[Option<TokenMatch>],
) -> sea_query::Cond {
    use sea_query::{Alias, Expr, ExprTrait};

    let mut cond = sea_query::Cond::all();
    // Timestamps bind as RFC 3339 text cast to timestamptz: the sea-query
    // binder's with-jiff is unimplemented upstream (see the workspace manifest
    // note), so the value crosses as text.
    if let Some(from) = filter.from {
        cond = cond.add(
            Expr::col(Alias::new("recorded_at"))
                .gte(Expr::val(from.to_string()).cast_as("timestamptz")),
        );
    }
    if let Some(until) = filter.until {
        let bound = Expr::val(until.to_string()).cast_as("timestamptz");
        cond = cond.add(if filter.until_inclusive {
            Expr::col(Alias::new("recorded_at")).lte(bound)
        } else {
            Expr::col(Alias::new("recorded_at")).lt(bound)
        });
    }
    for (index, criterion) in filter.criteria.iter().enumerate() {
        let condition = match resolved.get(index).and_then(Option::as_ref) {
            Some(found) => token_condition(found),
            None => criterion_condition(criterion),
        };
        cond = cond.add(condition);
    }
    cond
}

/// The column condition of one criterion: any of its values matches. A
/// criterion on the document has no column, so it matches nothing here; the
/// search resolves it to token ids instead.
fn criterion_condition(criterion: &AuditCriterion) -> sea_query::Cond {
    use sea_query::{Alias, Expr, ExprTrait};

    let column_in = |column: &'static str, values: &[String]| {
        values.iter().fold(sea_query::Cond::any(), |any, value| {
            any.add(Expr::col(Alias::new(column)).eq(value.clone()))
        })
    };
    match criterion {
        AuditCriterion::Patient(values) => column_in("patient_id", values),
        AuditCriterion::Agent(values) => column_in("principal", values),
        // NOTE: our own design (#6202): resource_id is keyed by its hash in the token index, which
        // a long stored query name fits, so the key narrows through the index and the text decides.
        AuditCriterion::Entity(values) => {
            values.iter().fold(sea_query::Cond::any(), |any, value| {
                any.add(
                    sea_query::Cond::all()
                        .add(Expr::cust_with_values(
                            crate::system_log::search::IdentifierColumn::ResourceId.hashed(),
                            [value.clone()],
                        ))
                        .add(Expr::col(Alias::new("resource_id")).eq(value.clone())),
                )
            })
        }
        AuditCriterion::Action(values) => column_in("action", values),
        AuditCriterion::Outcome(values) => {
            values.iter().fold(sea_query::Cond::any(), |any, value| {
                any.add(Expr::col(Alias::new("outcome")).eq(*value))
            })
        }
        AuditCriterion::Address(_) | AuditCriterion::Element(..) => {
            sea_query::Cond::any().add(Expr::cust("false"))
        }
    }
}

/// The condition of a criterion on the document: the record carries one of
/// the token ids, or meets one of the column matches.
fn token_condition(found: &TokenMatch) -> sea_query::Cond {
    use sea_query::{Alias, Expr, ExprTrait};

    let ids_of = |ids: &[i32]| {
        let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
        format!("{{{}}}", ids.join(","))
    };
    // NOTE: our own design (#6373), token ids are `integer` keys and column keys lie above them.
    let carries = |ids: &[i32]| {
        sea_query::Cond::all().add(Expr::cust_with_values(
            "search_keys && CAST($1 AS bigint[])",
            [ids_of(ids)],
        ))
    };
    let mut any = sea_query::Cond::any();
    if !found.ids.is_empty() {
        any = any.add(carries(&found.ids));
    }
    for column in &found.columns {
        let all = match &column.value {
            // NOTE: our own design (#6202), with a value the sets are tested on the row, so the
            // index reads the value's key alone rather than every record of every set.
            Some(value) => sea_query::Cond::all()
                .add(Expr::cust_with_values(
                    "search_tokens && CAST($1 AS integer[])",
                    [ids_of(&column.sets)],
                ))
                .add(Expr::cust_with_values(
                    column.column.hashed(),
                    [value.clone()],
                ))
                .add(Expr::col(Alias::new(column.column.column())).eq(value.clone())),
            None => carries(&column.sets),
        };
        any = any.add(all);
    }
    any
}
