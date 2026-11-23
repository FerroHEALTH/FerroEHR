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
    UnionType, WindowStatement,
};
use sea_query_sqlx::{SqlxBinder as _, SqlxValues};

use super::{AuditCriterion, AuditElement, AuditSearchFilter, RECORD_COLUMNS};
use crate::system_log::search::TokenMatch;

/// The schema of the audit store.
const AUDIT: &str = "audit";
/// The relation every record is read from.
const RECORD: &str = "audit_record";
/// The ITI-81 index of the access log, the view of the entries' index
/// columns, which holds every column a search criterion names (#5784).
const ACCESS_LOG_SEARCH: &str = "access_log_search";
/// One access-log entry as `audit_record` holds it, read by its key (#6320).
const ACCESS_LOG_BY_KEY: &str = "access_log_by_key";
/// The ITI-81 index of the system chain, the view of the linked records'
/// search columns: a plain join the planner reads in the search's order
/// through the time index, where a member of `audit_record`'s union is
/// planned without that order (#6203).
const SYSTEM_SEARCH: &str = "system_search";

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
    let served = u64::try_from(page_size(filter)).unwrap_or(50);
    let mut page = page(&condition, count, offset, served);
    // NOTE: our own design (#6320), the revisions ride on the page's first record alone, which
    // the confirmation reads, so the other records carry a NULL over the wire.
    let first = |carried: &'static str| -> Expr {
        sea_query::CaseStatement::new()
            .case(
                Expr::col((Alias::new("page_key"), Alias::new("position"))).eq(1),
                Expr::cust(carried),
            )
            .into()
    };
    if filter.carried.policy {
        let carried = if filter.carried.reads_party() {
            super::carried::PARTY
        } else {
            super::carried::POLICY
        };
        page.expr_as(first(carried), Alias::new(super::carried::CARRIED));
    }
    let (sql, values) = page.build_sqlx(PostgresQueryBuilder);
    (count_sql, count_values, sql, values)
}

/// Whether the statements of `filter` are of a bounded shape set, which runs
/// as named statements: each criterion of its own kind, with one value.
///
/// Every value binds, so the text of such a filter is decided by its date
/// bounds and the kinds of its criteria in their order, a set finite in
/// size, and the connection's statement cache bounds what it keeps of it. A
/// criterion with several values, or a kind named twice, writes text of its
/// own, which goes unnamed. No openEHR spec governs this: our own design.
pub(super) fn shaped(filter: &AuditSearchFilter) -> bool {
    let mut kinds = Vec::with_capacity(filter.criteria.len());
    filter.criteria.iter().all(|criterion| {
        let (kind, values) = criterion_kind(criterion);
        let fresh = !kinds.contains(&kind);
        kinds.push(kind);
        fresh && values == 1
    })
}

/// The kind of `criterion`, the token parameter's element among them, and
/// the number of its values.
fn criterion_kind(criterion: &AuditCriterion) -> (CriterionKind, usize) {
    match criterion {
        AuditCriterion::Patient(values) => (CriterionKind::Patient, values.len()),
        AuditCriterion::Agent(values) => (CriterionKind::Agent, values.len()),
        AuditCriterion::Entity(values) => (CriterionKind::Entity, values.len()),
        AuditCriterion::Outcome(values) => (CriterionKind::Outcome, values.len()),
        AuditCriterion::Action(values) => (CriterionKind::Action, values.len()),
        AuditCriterion::Address(values) => (CriterionKind::Address, values.len()),
        AuditCriterion::Element(element, tokens) => {
            (CriterionKind::Element(*element), tokens.len())
        }
    }
}

/// What a criterion filters on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CriterionKind {
    Patient,
    Agent,
    Entity,
    Outcome,
    Action,
    Address,
    Element(AuditElement),
}

/// The records a page serves: `_count`, between 1 and 1000.
pub(super) fn page_size(filter: &AuditSearchFilter) -> usize {
    usize::try_from(filter.count.clamp(1, 1000)).unwrap_or(50)
}

/// The number of records `condition` selects: the system records and the
/// indexed entries of the access log no write holds, those a page can return
/// (#6000, #6203, #6242), each counted from its index columns alone.
fn total(condition: &Cond) -> SelectStatement {
    // NOTE: our own design (#6242), system_search holds no record a write still holds.
    let system = count_of(
        Query::select()
            .expr(Expr::col(Asterisk).count())
            .from((Alias::new(AUDIT), Alias::new(SYSTEM_SEARCH)))
            .cond_where(condition.clone())
            .to_owned(),
    );
    let indexed = || {
        Query::select()
            .expr(Expr::col(Asterisk).count())
            .from((Alias::new(AUDIT), Alias::new(ACCESS_LOG_SEARCH)))
            .cond_where(condition.clone())
            .to_owned()
    };
    // NOTE: our own design (#6242), access_log_search holds no entry a write still holds.
    let access_log = count_of(indexed());
    Query::select().expr(system.add(access_log)).to_owned()
}

/// A scalar subquery of `count`.
fn count_of(count: SelectStatement) -> Expr {
    Expr::SubQuery(None, Box::new(count.into()))
}

/// The columns that order the records and name each one: the keys a page
/// is taken by before any record is read whole.
const KEYS: [&str; 4] = ["chain", "chain_seq", "recorded_at", "stored_at"];

/// The shared part of an access-log entry, in `audit_record`.
const SHARED_PART: &str = "shared_part";

/// The digest that names an access-log entry's shared part, in
/// `audit_record`.
const SHARED_DIGEST: &str = "shared_digest";

/// A page record's shared part, carried only by the first record of the page
/// that names it, so a part many entries share crosses the wire once a page
/// (#6244); the server reuses it for the records after.
const FIRST_NAMING: &str = "CASE WHEN row_number() OVER (PARTITION BY \"page_record\".\"shared_digest\" \
     ORDER BY \"page_key\".\"position\") = 1 THEN \"page_record\".\"shared_part\" END";

/// The page: the `count` records after the first `offset` of those
/// `condition` selects, newest first, the first `served` of them read whole
/// and the rest by their keys alone.
///
/// The order is total (a position is unique within its chain), so the page
/// lies within the first `offset + count` records of each chain, and each
/// chain gives only the keys of those. The page is taken by the keys, and
/// only then is each record it serves read through `audit_record`, where an
/// access-log entry's document is rendered (#5965). A record past `served`,
/// the one that says another page follows, keeps its keys and no record.
fn page(condition: &Cond, count: u64, offset: u64, served: u64) -> SelectStatement {
    let first = offset.saturating_add(count);
    let mut system = Query::select();
    system
        .columns(KEYS.map(Alias::new))
        .from((Alias::new(AUDIT), Alias::new(SYSTEM_SEARCH)))
        .cond_where(condition.clone());
    within_chain(&mut system, None).limit(first);
    let mut chains = Query::select();
    chains
        .column(Asterisk)
        .from_subquery(system, Alias::new("system_page"))
        .union(UnionType::All, access_log_page(condition, first));
    let page_keys = newest_first_of(
        Query::select()
            .columns(KEYS.map(Alias::new))
            .from_subquery(chains, Alias::new("chains")),
    )
    .limit(count)
    .offset(offset)
    .to_owned();
    let mut in_order = WindowStatement::new();
    for column in ["recorded_at", "stored_at", "chain", "chain_seq"] {
        in_order.order_by(Alias::new(column), Order::Desc);
    }
    let keys = Query::select()
        .columns(KEYS.map(Alias::new))
        .expr_window_as(Expr::cust("row_number()"), in_order, Alias::new("position"))
        .from_subquery(page_keys, Alias::new("page_keys"))
        .to_owned();
    let key = Alias::new("page_key");
    let record = Alias::new("page_record");
    // NOTE: our own design (#5965, #6320), `OFFSET 0` keeps the read lateral, so each served
    // record is found by its position in its own chain's relation: an access-log entry by the
    // key of its row, never through the open window of `audit_record`'s union.
    let in_chain = |relation: &str, chain: &str| {
        Query::select()
            .columns(record_columns().map(Alias::new))
            .column(Alias::new(SHARED_DIGEST))
            .from((Alias::new(AUDIT), Alias::new(relation)))
            .and_where(Expr::col((key.clone(), Alias::new("chain"))).eq(chain))
            .and_where(
                Expr::col(Alias::new("chain_seq"))
                    .eq(Expr::col((key.clone(), Alias::new("chain_seq")))),
            )
            .and_where(Expr::col((key.clone(), Alias::new("position"))).lte(served))
            .to_owned()
    };
    let mut by_key = in_chain(RECORD, "system")
        .and_where(Expr::col(Alias::new("chain")).eq("system"))
        .to_owned();
    by_key
        .union(UnionType::All, in_chain(ACCESS_LOG_BY_KEY, "access_log"))
        .offset(0);
    let mut page = Query::select();
    page.columns(KEYS.map(|column| (key.clone(), Alias::new(column))))
        .columns(
            record_columns()
                .filter(|column| *column != SHARED_PART)
                .map(|column| (record.clone(), Alias::new(column))),
        )
        .column((record.clone(), Alias::new(SHARED_DIGEST)))
        .expr_as(Expr::cust(FIRST_NAMING), Alias::new(SHARED_PART))
        .from_subquery(keys, key.clone())
        .join_lateral(
            sea_query::JoinType::LeftJoin,
            by_key,
            record,
            Expr::val(true),
        );
    for column in ["recorded_at", "stored_at", "chain", "chain_seq"] {
        page.order_by((key.clone(), Alias::new(column)), Order::Desc);
    }
    page
}

/// The columns of a record past its keys.
fn record_columns() -> impl Iterator<Item = &'static str> {
    RECORD_COLUMNS
        .into_iter()
        .filter(|column| !KEYS.contains(column))
}

/// The keys of the first `first` access-log records `condition` selects,
/// newest first.
///
/// The candidates are read from `access_log_search` through the entries'
/// time index: its first `first` rows by `recorded_at`, and every row tied
/// with the last of them, since the order's next key, `stored_at`, is not in
/// the view. A candidate's keys are its index row's own (#6320), so no
/// record is read before the page is taken.
fn access_log_page(condition: &Cond, first: u64) -> SelectStatement {
    let search = || (Alias::new(AUDIT), Alias::new(ACCESS_LOG_SEARCH));
    // NOTE: our own design (#5968), a page reads only the entries the chainer linked.
    let linked = || Expr::col(Alias::new("chain_seq")).is_not_null();
    let newest = |limit: u64, offset: u64| {
        Query::select()
            .from(search())
            .cond_where(condition.clone())
            .and_where(linked())
            .order_by(Alias::new("recorded_at"), Order::Desc)
            .limit(limit)
            .offset(offset)
            .to_owned()
    };
    let last_instant = newest(1, first.saturating_sub(1))
        .column(Alias::new("recorded_at"))
        .to_owned();
    let keys = |mut select: SelectStatement| {
        select
            .column(Alias::new("chain_seq"))
            .column(Alias::new("recorded_at"))
            .to_owned()
    };
    let mut candidates = keys(
        Query::select()
            .from(search())
            .cond_where(condition.clone())
            .and_where(linked())
            .and_where(Expr::col(Alias::new("recorded_at")).eq(count_of(last_instant)))
            .to_owned(),
    );
    candidates.union(UnionType::Distinct, keys(newest(first, 0)));
    let candidate = Alias::new("access_log_candidate");
    // NOTE: our own design (#6320), an entry's keys are its search index row's: it is stored at
    // the instant it records, and its chain is the access log's, so no record is read for them.
    let mut page = Query::select();
    page.expr_as(Expr::cust("'access_log'::text"), Alias::new("chain"))
        .column((candidate.clone(), Alias::new("chain_seq")))
        .column((candidate.clone(), Alias::new("recorded_at")))
        .expr_as(
            Expr::col((candidate.clone(), Alias::new("recorded_at"))),
            Alias::new("stored_at"),
        )
        .from_subquery(candidates, candidate.clone());
    within_chain(&mut page, None).limit(first);
    page
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
