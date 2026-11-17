// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two statements of the ITI-81 retrieval, the total and the page, read
//! so neither costs more as the log grows than the records it serves.
//!
//! `audit.audit_record` is the union of the system chain (`audit_event`) and
//! the indexed entries of the access log (`access_log_record` joined to its
//! shared parts). A union member is planned on its own, without the order or
//! the limit of the query over the union, so a page read through the view
//! sorted the whole access log (#5646). Each chain therefore gives its
//! own first records in the search's order, and the page is taken from their
//! union. No openEHR spec governs the store: our own design; the search
//! follows IHE `RESTful` ATNA (ITI-81).

use sea_query::{
    Alias, Asterisk, Cond, Expr, ExprTrait, Order, PostgresQueryBuilder, Query, SelectStatement,
    UnionType,
};
use sea_query_sqlx::{SqlxBinder as _, SqlxValues};

use super::{AuditSearchFilter, RECORD_COLUMNS};
use crate::system_log::search::TokenMatch;

/// The schema of the audit store.
const AUDIT: &str = "audit";
/// The relation every record is read from.
const RECORD: &str = "audit_record";
/// The ITI-81 index of the access log, the view of the entries' index
/// columns, which holds every column a search criterion names (#5784).
const ACCESS_LOG_SEARCH: &str = "access_log_search";

/// Build the ITI-81 retrieval's two statements, the count and the page, from a
/// filter and the token ids its criteria on the document resolved to, with no
/// database involved.
///
/// Extracted from [`super::AuditStore::search`] so the SQL is reachable by a
/// test. This is the server's second runtime dynamic-SQL site after the AQL
/// engine, and the property that matters is the same one: every IDENTIFIER
/// here (the schema, the tables, every column, the sort direction) is a
/// literal in this module, while every caller-supplied value goes through
/// `Expr::val` and arrives as a bound parameter. `limit`/`offset` are clamped
/// integers, never text.
///
/// The guard `scripts/checks/sql-string-building.sh` catches string-built SQL
/// but cannot see an `Alias::new` argument, which is why the closed set is
/// pinned by a test rather than only by the guard.
pub(super) fn build_search_statements(
    filter: &AuditSearchFilter,
    resolved: &[Option<TokenMatch>],
) -> (String, SqlxValues, String, SqlxValues) {
    let condition = crate::system_log::search::condition::search_condition(filter, resolved);
    // One record past the page, which tells the caller whether a next page exists.
    let count = u64::try_from(page_size(filter))
        .unwrap_or(50)
        .saturating_add(1);
    let offset = u64::try_from(Ord::max(filter.offset, 0)).unwrap_or(0);
    let (count_sql, count_values) = total(&condition).build_sqlx(PostgresQueryBuilder);
    let (sql, values) = page(&condition, count, offset).build_sqlx(PostgresQueryBuilder);
    (count_sql, count_values, sql, values)
}

/// The records a page serves: `_count`, between 1 and 1000.
pub(super) fn page_size(filter: &AuditSearchFilter) -> usize {
    usize::try_from(filter.count.clamp(1, 1000)).unwrap_or(50)
}

/// The number of records `condition` selects: those of the system chain,
/// plus the indexed entries of the access log, read from their index columns
/// alone (written by `audit.access_log_chain_move` and emptied when the entry
/// becomes a tombstone).
fn total(condition: &Cond) -> SelectStatement {
    let system = count_of(
        Query::select()
            .expr(Expr::col(Asterisk).count())
            .from((Alias::new(AUDIT), Alias::new(RECORD)))
            .cond_where(on_chain(condition, "system"))
            .to_owned(),
    );
    let access_log = count_of(
        Query::select()
            .expr(Expr::col(Asterisk).count())
            .from((Alias::new(AUDIT), Alias::new(ACCESS_LOG_SEARCH)))
            .cond_where(condition.clone())
            .to_owned(),
    );
    Query::select().expr(system.add(access_log)).to_owned()
}

/// A scalar subquery of `count`.
fn count_of(count: SelectStatement) -> Expr {
    Expr::SubQuery(None, Box::new(count.into()))
}

/// The page: the `count` records after the first `offset` of those
/// `condition` selects, newest first.
///
/// The order is total (a position is unique within its chain), so the page
/// lies within the first `offset + count` records of each chain, and each
/// chain reads only those.
fn page(condition: &Cond, count: u64, offset: u64) -> SelectStatement {
    let first = offset.saturating_add(count);
    let mut system = Query::select();
    system
        .columns(RECORD_COLUMNS.map(Alias::new))
        .from((Alias::new(AUDIT), Alias::new(RECORD)))
        .cond_where(on_chain(condition, "system"));
    within_chain(&mut system, None).limit(first);
    let mut chains = Query::select();
    chains
        .column(Asterisk)
        .from_subquery(system, Alias::new("system_page"))
        .union(UnionType::All, access_log_page(condition, first));
    newest_first_of(
        Query::select()
            .columns(RECORD_COLUMNS.map(Alias::new))
            .from_subquery(chains, Alias::new("chains")),
    )
    .limit(count)
    .offset(offset)
    .to_owned()
}

/// The first `first` access-log records `condition` selects, newest first.
///
/// The candidates are read from `access_log_search` through the entries'
/// time index: its first `first` rows by `recorded_at`, and every row tied
/// with the last of them, since the order's next key, `stored_at`, is not in
/// the view. Each candidate's record is read from the view by its position.
fn access_log_page(condition: &Cond, first: u64) -> SelectStatement {
    let search = || (Alias::new(AUDIT), Alias::new(ACCESS_LOG_SEARCH));
    let newest = |limit: u64, offset: u64| {
        Query::select()
            .from(search())
            .cond_where(condition.clone())
            .order_by(Alias::new("recorded_at"), Order::Desc)
            .limit(limit)
            .offset(offset)
            .to_owned()
    };
    let last_instant = newest(1, first.saturating_sub(1))
        .column(Alias::new("recorded_at"))
        .to_owned();
    let mut candidates = Query::select();
    candidates
        .column(Alias::new("chain_seq"))
        .from(search())
        .cond_where(condition.clone())
        .and_where(Expr::col(Alias::new("recorded_at")).eq(count_of(last_instant)))
        .union(
            UnionType::Distinct,
            newest(first, 0).column(Alias::new("chain_seq")).to_owned(),
        );
    let candidate = Alias::new("access_log_candidate");
    let record = Alias::new("access_log_page");
    // NOTE: our own design (#5646), `OFFSET 0` keeps the read lateral, so each candidate's
    // record is found by its position rather than by a read of the whole log.
    let by_position = Query::select()
        .columns(RECORD_COLUMNS.map(Alias::new))
        .from((Alias::new(AUDIT), Alias::new(RECORD)))
        .and_where(Expr::col(Alias::new("chain")).eq("access_log"))
        .and_where(
            Expr::col(Alias::new("chain_seq"))
                .eq(Expr::col((candidate.clone(), Alias::new("chain_seq")))),
        )
        .offset(0)
        .to_owned();
    let mut page = Query::select();
    page.columns(RECORD_COLUMNS.map(|column| (record.clone(), Alias::new(column))))
        .from_subquery(candidates, candidate)
        .join_lateral(
            sea_query::JoinType::InnerJoin,
            by_position,
            record.clone(),
            Expr::val(true),
        );
    within_chain(&mut page, Some(&record)).limit(first);
    page
}

/// `condition` on the records of `chain`.
fn on_chain(condition: &Cond, chain: &str) -> Cond {
    condition
        .clone()
        .add(Expr::col(Alias::new("chain")).eq(chain))
}

/// `select` ordered as the search orders the records of one chain, its
/// columns those of `relation`.
fn within_chain<'a>(
    select: &'a mut SelectStatement,
    relation: Option<&Alias>,
) -> &'a mut SelectStatement {
    for column in ["recorded_at", "stored_at", "chain_seq"] {
        match relation {
            Some(relation) => select.order_by((relation.clone(), Alias::new(column)), Order::Desc),
            None => select.order_by(Alias::new(column), Order::Desc),
        };
    }
    select
}

/// `select` ordered as the search orders its records, newest first.
pub(super) fn newest_first_of(select: &mut SelectStatement) -> &mut SelectStatement {
    select
        .order_by(Alias::new("recorded_at"), Order::Desc)
        .order_by(Alias::new("stored_at"), Order::Desc)
        .order_by(Alias::new("chain"), Order::Desc)
        .order_by(Alias::new("chain_seq"), Order::Desc)
}
