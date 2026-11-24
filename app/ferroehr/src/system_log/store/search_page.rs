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
use sqlx::Row as _;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, PoisonError};

use super::{AuditCriterion, AuditElement, AuditSearchFilter, RECORD_COLUMNS};
use crate::system_log::search::TokenMatch;

/// The schema of the audit store.
const AUDIT: &str = "audit";
/// The relation every record is read from.
const RECORD: &str = "audit_record";

/// The select item of the index cursor a page's first record carries.
const CURSOR_SQL_ITEM: &str =
    "(SELECT x.last_id FROM audit.access_log_index_state x WHERE x.singleton)";

/// The column a page's first record carries the index cursor in.
const CURSOR: &str = "index_cursor";

/// The newest index cursor an ITI-81 page carried, which the next page is
/// read as of; unknown until the first page reads it.
#[derive(Debug, Clone)]
pub(crate) struct IndexCursor(Arc<AtomicI64>);

impl Default for IndexCursor {
    fn default() -> Self {
        Self(Arc::new(AtomicI64::new(-1)))
    }
}

impl IndexCursor {
    /// The cursor a page binds: the newest one a page carried, or, before
    /// any did, the one read on `conn`.
    ///
    /// # Errors
    /// [`sqlx::Error`] when that read fails.
    pub(super) async fn bound(&self, conn: &mut sqlx::PgConnection) -> Result<i64, sqlx::Error> {
        // NOTE: our own design (#6373), the cursor only moves forward, so a bound at or below it
        // is exact; a cursor another replica moved only widens the open window this one reads.
        let held = self.0.load(Ordering::Relaxed);
        if held >= 0 {
            return Ok(held);
        }
        let read: i64 = sqlx::query_scalar(crate::access_log::index::CURSOR)
            .fetch_one(conn)
            .await?;
        self.0.fetch_max(read, Ordering::Relaxed);
        Ok(read)
    }

    /// Keeps the cursor the first record of `rows` carried.
    ///
    /// # Errors
    /// [`sqlx::Error`] when the carried cursor does not decode.
    pub(super) fn saw(&self, rows: &[sqlx::postgres::PgRow]) -> Result<(), sqlx::Error> {
        if let Some(row) = rows.first()
            && let Some(cursor) = row.try_get::<Option<i64>, _>(CURSOR)?
        {
            self.0.fetch_max(cursor, Ordering::Relaxed);
        }
        Ok(())
    }
}

/// The alias of the access log's ITI-81 index in a statement.
const ACCESS_LOG_CANDIDATES: &str = "access_log_search";

/// The ITI-81 index of the access log as of the index cursor `cursor`, which
/// holds every column a search criterion names (#5784, #6373).
fn access_log_search(cursor: i64) -> sea_query::FunctionCall {
    sea_query::Func::cust(Alias::new("audit.access_log_search")).arg(Expr::val(cursor))
}
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
    cursor: i64,
) -> (String, SqlxValues, String, SqlxValues) {
    build(
        filter,
        resolved,
        cursor,
        Numbers::of(filter),
        &carried_of(filter),
    )
}

/// The page's counts: the records past the offset it reads, one past the
/// page, which tells the caller whether a next page exists; the offset; and
/// the records it serves whole.
#[derive(Debug, Clone, Copy)]
struct Numbers {
    count: u64,
    offset: u64,
    served: u64,
}

impl Numbers {
    /// The counts `filter` asks for.
    fn of(filter: &AuditSearchFilter) -> Self {
        let served = u64::try_from(page_size(filter)).unwrap_or(50);
        Self {
            count: served.saturating_add(1),
            offset: u64::try_from(Ord::max(filter.offset, 0)).unwrap_or(0),
            served,
        }
    }
}

/// What the page carries beside its records: nothing, the policy revisions,
/// or those and the party store's read through the door the value names.
#[derive(Debug, Clone, PartialEq)]
enum CarriedRead {
    Nothing,
    Policy,
    Party(Option<String>),
}

/// What `filter`'s page carries.
fn carried_of(filter: &AuditSearchFilter) -> CarriedRead {
    if !filter.carried.policy {
        return CarriedRead::Nothing;
    }
    if filter.carried.reads_party() {
        let door = crate::party_binding::request::door();
        CarriedRead::Party(door.as_deref().map(|held| held.text().to_owned()))
    } else {
        CarriedRead::Policy
    }
}

/// The two statements of [`build_search_statements`] from the page's counts
/// and what it carries.
fn build(
    filter: &AuditSearchFilter,
    resolved: &[Option<TokenMatch>],
    cursor: i64,
    numbers: Numbers,
    carried: &CarriedRead,
) -> (String, SqlxValues, String, SqlxValues) {
    let condition = crate::system_log::search::condition::search_condition(filter, resolved);
    let (count_sql, count_values) = total(&condition, cursor).build_sqlx(PostgresQueryBuilder);
    let mut page = page(
        &condition,
        numbers.count,
        numbers.offset,
        numbers.served,
        cursor,
    );
    // NOTE: our own design (#6320), the revisions ride on the page's first record alone, which
    // the confirmation reads, so the other records carry a NULL over the wire.
    let first = |carried: Expr| -> Expr {
        sea_query::CaseStatement::new()
            .case(
                Expr::col((Alias::new("page_key"), Alias::new("position"))).eq(1),
                carried,
            )
            .into()
    };
    page.expr_as(first(Expr::cust(CURSOR_SQL_ITEM)), Alias::new(CURSOR));
    match carried {
        CarriedRead::Nothing => {}
        CarriedRead::Policy => {
            page.expr_as(
                first(Expr::cust(super::carried::POLICY)),
                Alias::new(super::carried::CARRIED),
            );
        }
        CarriedRead::Party(door) => {
            page.expr_as(
                first(Expr::cust_with_values(
                    super::carried::PARTY,
                    [door.clone()],
                )),
                Alias::new(super::carried::CARRIED),
            );
        }
    }
    let (sql, values) = page.build_sqlx(PostgresQueryBuilder);
    (count_sql, count_values, sql, values)
}

/// The most search shapes a store keeps the statements of; a search of
/// another shape builds its statements.
const SHAPES_KEPT: usize = 64;

/// The ITI-81 statements of each search shape a store has run (#6362): the
/// text depends on the shape alone (which parameters are present, how many
/// values each has, what a token criterion resolved to, the date bounds,
/// what the page carries), so it is built once per shape and each search
/// binds its own values. No openEHR spec governs this: our own design.
#[derive(Debug, Default)]
pub(crate) struct PageShapes {
    templates: std::sync::RwLock<std::collections::BTreeMap<String, Arc<Template>>>,
}

/// The statements of one shape, and where each of their parameters comes
/// from.
#[derive(Debug)]
pub(super) struct Template {
    slots: usize,
    count_sql: String,
    count_binds: Vec<Bind>,
    sql: String,
    binds: Vec<Bind>,
}

/// The source of one parameter of a [`Template`]: a value of the search, by
/// its place among the search's values, or a value the shape fixes.
#[derive(Debug)]
enum Bind {
    Slot(usize),
    Fixed(sea_query::Value),
}

/// The two statements of one search and their values.
#[derive(Debug)]
pub(super) struct Statements {
    /// The statements' text.
    pub(super) template: Arc<Template>,
    /// The count statement's values.
    pub(super) count_values: SqlxValues,
    /// The page statement's values.
    pub(super) values: SqlxValues,
}

impl Template {
    /// The count statement.
    pub(super) fn count_sql(&self) -> &str {
        &self.count_sql
    }

    /// The page statement.
    pub(super) fn sql(&self) -> &str {
        &self.sql
    }
}

impl PageShapes {
    /// The statements of a search for `filter`, its token criteria resolved
    /// to `resolved`, bound to the index cursor `cursor`: from the shape's
    /// template when this store has one, else built, and kept while fewer
    /// than [`SHAPES_KEPT`] shapes are.
    pub(super) fn statements(
        &self,
        filter: &AuditSearchFilter,
        resolved: &[Option<TokenMatch>],
        cursor: i64,
    ) -> Statements {
        let numbers = Numbers::of(filter);
        let carried = carried_of(filter);
        let key = shape_key(filter, resolved, &carried);
        let held = self
            .templates
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        let template = held.unwrap_or_else(|| {
            let template = Arc::new(template_of(filter, resolved, &carried));
            let mut templates = self
                .templates
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            if templates.len() < SHAPES_KEPT {
                templates.insert(key, Arc::clone(&template));
            }
            template
        });
        let slots = slots(filter, resolved, cursor, numbers, &carried);
        if slots.len() == template.slots
            && let Some(count_values) = bound(&template.count_binds, &slots)
            && let Some(values) = bound(&template.binds, &slots)
        {
            return Statements {
                template,
                count_values,
                values,
            };
        }
        // NOTE: our own design (#6362), a search whose values its shape's template does not
        // place builds its own statements.
        let (count_sql, count_values, sql, values) =
            build(filter, resolved, cursor, numbers, &carried);
        Statements {
            template: Arc::new(Template {
                slots: 0,
                count_sql,
                count_binds: Vec::new(),
                sql,
                binds: Vec::new(),
            }),
            count_values,
            values,
        }
    }
}

/// The values `binds` take from `slots`; `None` when a bind names a value
/// `slots` does not hold.
fn bound(binds: &[Bind], slots: &[sea_query::Value]) -> Option<SqlxValues> {
    binds
        .iter()
        .map(|bind| match bind {
            Bind::Slot(at) => slots.get(*at).cloned(),
            Bind::Fixed(value) => Some(value.clone()),
        })
        .collect::<Option<Vec<_>>>()
        .map(|values| SqlxValues(sea_query::Values(values)))
}

/// The key of the shape of a search for `filter`, its token criteria
/// resolved to `resolved`, carrying `carried`.
fn shape_key(
    filter: &AuditSearchFilter,
    resolved: &[Option<TokenMatch>],
    carried: &CarriedRead,
) -> String {
    use std::fmt::Write as _;
    let mut key = String::with_capacity(32);
    let _ = write!(
        key,
        "{}{}{}{}",
        u8::from(filter.from.is_some()),
        u8::from(filter.until.is_some()),
        u8::from(filter.until_inclusive),
        match carried {
            CarriedRead::Nothing => 'n',
            CarriedRead::Policy => 'p',
            CarriedRead::Party(Some(_)) => 'd',
            CarriedRead::Party(None) => 'e',
        }
    );
    for (index, criterion) in filter.criteria.iter().enumerate() {
        let (kind, values) = criterion_kind(criterion);
        let _ = write!(key, "|{kind:?}{values}");
        if let Some(Some(found)) = resolved.get(index) {
            let _ = write!(key, ":{}", u8::from(!found.ids.is_empty()));
            for column in &found.columns {
                let _ = write!(
                    key,
                    ",{:?}{}",
                    column.column,
                    u8::from(column.value.is_some())
                );
            }
        }
    }
    key
}

/// The values of a search, in the order [`template_of`] gives their
/// sentinels: the cursor, the counts, the date bounds, the door, then each
/// criterion's values or what its token criterion resolved to.
fn slots(
    filter: &AuditSearchFilter,
    resolved: &[Option<TokenMatch>],
    cursor: i64,
    numbers: Numbers,
    carried: &CarriedRead,
) -> Vec<sea_query::Value> {
    use sea_query::Value;
    let mut slots = vec![
        Value::from(cursor),
        Value::from(numbers.count),
        Value::from(numbers.offset),
        Value::from(numbers.served),
        Value::from(numbers.offset.saturating_add(numbers.count)),
    ];
    if let Some(from) = filter.from {
        slots.push(Value::from(from.to_string()));
    }
    if let Some(until) = filter.until {
        slots.push(Value::from(until.to_string()));
    }
    if let CarriedRead::Party(door) = carried {
        slots.push(Value::from(door.clone()));
    }
    for (index, criterion) in filter.criteria.iter().enumerate() {
        match resolved.get(index).and_then(Option::as_ref) {
            Some(found) => {
                if !found.ids.is_empty() {
                    slots.push(Value::from(crate::system_log::search::condition::ids_text(
                        &found.ids,
                    )));
                }
                for column in &found.columns {
                    slots.push(Value::from(crate::system_log::search::condition::ids_text(
                        &column.sets,
                    )));
                    if let Some(value) = &column.value {
                        slots.push(Value::from(value.clone()));
                    }
                }
            }
            None => criterion_values(criterion, &mut slots),
        }
    }
    slots
}

/// The values of a criterion matched on its column.
fn criterion_values(criterion: &AuditCriterion, slots: &mut Vec<sea_query::Value>) {
    use sea_query::Value;
    match criterion {
        AuditCriterion::Patient(values)
        | AuditCriterion::Agent(values)
        | AuditCriterion::Entity(values)
        | AuditCriterion::Action(values) => {
            slots.extend(values.iter().map(|value| Value::from(value.clone())));
        }
        AuditCriterion::Outcome(values) => slots.extend(values.iter().map(|v| Value::from(*v))),
        AuditCriterion::Address(_) | AuditCriterion::Element(..) => {}
    }
}

/// The statements of the shape of a search for `filter`, built from a
/// search of that shape whose every value is a sentinel no statement fixes,
/// so each parameter is told apart as a value of the search or a value of
/// the shape.
fn template_of(
    filter: &AuditSearchFilter,
    resolved: &[Option<TokenMatch>],
    carried: &CarriedRead,
) -> Template {
    let mut probe = filter.clone();
    let mut text = 0_u32;
    let mut next_text = || {
        text += 1;
        format!("\u{1}sentinel {text}")
    };
    let mut instant = 0_i64;
    let mut next_instant = || {
        instant += 1;
        jiff::Timestamp::constant(instant, 0)
    };
    probe.from = filter.from.map(|_| next_instant());
    probe.until = filter.until.map(|_| next_instant());
    let mut outcome = i16::MIN;
    for criterion in &mut probe.criteria {
        match criterion {
            AuditCriterion::Patient(values)
            | AuditCriterion::Agent(values)
            | AuditCriterion::Entity(values)
            | AuditCriterion::Action(values) => {
                for value in values.iter_mut() {
                    *value = next_text();
                }
            }
            AuditCriterion::Outcome(values) => {
                for value in values.iter_mut() {
                    outcome += 1;
                    *value = outcome;
                }
            }
            AuditCriterion::Address(_) | AuditCriterion::Element(..) => {}
        }
    }
    let mut id = i32::MIN;
    let mut next_ids = || {
        id += 1;
        vec![id]
    };
    let probe_resolved: Vec<Option<TokenMatch>> = resolved
        .iter()
        .map(|found| {
            found.as_ref().map(|found| TokenMatch {
                ids: if found.ids.is_empty() {
                    Vec::new()
                } else {
                    next_ids()
                },
                columns: found
                    .columns
                    .iter()
                    .map(|column| crate::system_log::search::ColumnMatch {
                        column: column.column,
                        sets: next_ids(),
                        value: column.value.as_ref().map(|_| next_text()),
                    })
                    .collect(),
            })
        })
        .collect();
    let probe_carried = match carried {
        CarriedRead::Party(Some(_)) => CarriedRead::Party(Some(next_text())),
        other => other.clone(),
    };
    let numbers = Numbers {
        count: 3_000_000_000_000,
        offset: 5_000_000_000_000,
        served: 7_000_000_000_000,
    };
    let cursor = i64::MIN + 1;
    let slots = slots(&probe, &probe_resolved, cursor, numbers, &probe_carried);
    let (count_sql, count_values, sql, values) =
        build(&probe, &probe_resolved, cursor, numbers, &probe_carried);
    let binds_of = |values: SqlxValues| -> Vec<Bind> {
        values
            .0
            .0
            .into_iter()
            .map(|value| match slots.iter().position(|slot| *slot == value) {
                Some(at) => Bind::Slot(at),
                None => Bind::Fixed(value),
            })
            .collect()
    };
    Template {
        slots: slots.len(),
        count_sql,
        count_binds: binds_of(count_values),
        sql,
        binds: binds_of(values),
    }
}

/// The page statement [`super::AuditStore::search`] runs for `filter` bound to
/// the index cursor `cursor`, with its values, its token criteria resolved on
/// `pool`; `None` when a token criterion names no stored token.
///
/// # Errors
/// [`crate::system_log::AuditError`] when the token resolution fails.
pub async fn page_statement(
    pool: &sqlx::PgPool,
    filter: &AuditSearchFilter,
    cursor: i64,
) -> Result<Option<(String, SqlxValues)>, crate::system_log::AuditError> {
    let Some(resolved) = crate::system_log::search::resolve_criteria(pool, filter).await? else {
        return Ok(None);
    };
    let (_, _, sql, values) = build_search_statements(filter, &resolved, cursor);
    Ok(Some((sql, values)))
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
fn total(condition: &Cond, cursor: i64) -> SelectStatement {
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
            .from_function(access_log_search(cursor), Alias::new(ACCESS_LOG_CANDIDATES))
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
fn page(condition: &Cond, count: u64, offset: u64, served: u64, cursor: i64) -> SelectStatement {
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
        .union(UnionType::All, access_log_page(condition, first, cursor));
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
    // NOTE: our own design (#5965, #6320), `OFFSET 0` keeps the read lateral, one key read per
    // served record in its own chain's relation.
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
/// newest first, read from `access_log_search` through the entries' time
/// index: an entry is stored at the instant it records, so the search's
/// order is its instant and then its position.
fn access_log_page(condition: &Cond, first: u64, cursor: i64) -> SelectStatement {
    // NOTE: our own design (#5968), a page reads only the entries the chainer linked.
    let linked = Expr::col(Alias::new("chain_seq")).is_not_null();
    let candidates = Query::select()
        .column(Alias::new("chain_seq"))
        .column(Alias::new("recorded_at"))
        .from_function(access_log_search(cursor), Alias::new(ACCESS_LOG_CANDIDATES))
        .cond_where(condition.clone())
        .and_where(linked)
        .order_by(Alias::new("recorded_at"), Order::Desc)
        .order_by(Alias::new("chain_seq"), Order::Desc)
        .limit(first)
        .to_owned();
    let candidate = Alias::new("access_log_candidate");
    // NOTE: our own design (#6320), an entry is stored at the instant it records.
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
