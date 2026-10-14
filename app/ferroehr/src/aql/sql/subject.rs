// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The subject-identity rule of the ABAC patient scope, as SQL.
//!
//! A patient claim carries a subject id and no namespace, while an EHR subject
//! is the pair (`subject_id`, `subject_namespace`) (RM ehr master04 §EHR Status:
//! `subject.external_ref` is a `PARTY_REF` with its own `namespace`). An id
//! held under one namespace names that subject; an id held under two or more
//! names none of them. The query scope ([`super::from`]), the scope collection
//! and the direct-route patient gate all decide through [`shared`], so the
//! three agree. No openEHR spec governs the claim — our own extension.

use sea_query::extension::postgres::PgFunc;
use sea_query::{Alias, Expr, ExprTrait as _, PostgresQueryBuilder, Query};
use sea_query_sqlx::{SqlxBinder as _, SqlxValues};
use uuid::Uuid;

use super::expr::col;
use crate::db::iden::Ehr;

/// The `ehr` alias every other holder of a subject id is bound to.
const SUBJECT_OTHER: &str = "subject_other";

/// The `ehr` alias the subject lookup reads its rows from.
pub(super) const SUBJECT_ROW: &str = "subject_row";

/// The column the subject lookup reports [`shared`] under.
pub const SHARED_COLUMN: &str = "subject_shared";

/// Whether another EHR holds the subject id of the `ehr` row bound as `alias`
/// under a different namespace (a NULL namespace counts as different from any
/// other).
pub(super) fn shared(alias: &str) -> Expr {
    let mut other = Query::select();
    other
        .expr(Expr::val(1))
        .from_as(Ehr::Table, Alias::new(SUBJECT_OTHER))
        .and_where(col(SUBJECT_OTHER, Ehr::SubjectId).eq(col(alias, Ehr::SubjectId)))
        .and_where(col(SUBJECT_OTHER, Ehr::Id).ne(col(alias, Ehr::Id)))
        .and_where(
            col(SUBJECT_OTHER, Ehr::SubjectNamespace)
                .ne(col(alias, Ehr::SubjectNamespace))
                .or(col(SUBJECT_OTHER, Ehr::SubjectNamespace).is_null())
                .or(col(alias, Ehr::SubjectNamespace).is_null()),
        );
    Expr::exists(other)
}

/// Whether the `ehr` row bound as `alias` is the one subject the patient claim
/// `subject` names: its id is `subject` and no other namespace holds that id.
pub(super) fn matches(alias: &str, subject: &str) -> Expr {
    col(alias, Ehr::SubjectId)
        .eq(Expr::val(subject))
        .and(shared(alias).not())
}

/// The subject lookup over the EHRs `ids`: one row per existing EHR with its
/// `id`, `subject_id`, `subject_namespace` and the [`SHARED_COLUMN`] flag.
///
/// The ids are one array bind, so a set of any size stays inside the
/// 65535-parameter wire limit (`PostgreSQL` 18 §Frontend/Backend Protocol, Bind,
/// <https://www.postgresql.org/docs/18/protocol-message-formats.html>).
#[must_use]
pub fn lookup(ids: &[Uuid]) -> (String, SqlxValues) {
    let mut q = Query::select();
    q.expr(col(SUBJECT_ROW, Ehr::Id))
        .expr(col(SUBJECT_ROW, Ehr::SubjectId))
        .expr(col(SUBJECT_ROW, Ehr::SubjectNamespace))
        .expr_as(shared(SUBJECT_ROW), Alias::new(SHARED_COLUMN))
        .from_as(Ehr::Table, Alias::new(SUBJECT_ROW))
        .and_where(col(SUBJECT_ROW, Ehr::Id).eq(Expr::from(PgFunc::any(Expr::val(ids.to_vec())))));
    q.build_sqlx(PostgresQueryBuilder)
}
