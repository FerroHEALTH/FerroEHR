// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit search statements of each search shape a store has run, built
//! once per shape and bound to each search's values: the ITI-81 page
//! (#6362), and the person's own trail and the EHRs it names (#6467).

use std::sync::{Arc, PoisonError};

use sea_query_sqlx::SqlxValues;

use super::search_page::{self, CarriedRead, Numbers, build, carried_of, criterion_kind};
use super::{AuditCriterion, AuditPage, AuditSearchFilter, AuditStore};
use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::search::TokenMatch;

impl AuditStore {
    /// The RESTful-ATNA ITI-81 retrieval: the FHIR `AuditEvent` documents of
    /// the matching records, newest first, rendered from their columns,
    /// whether a match follows the page, and the total match count when
    /// [`AuditSearchFilter::total`] asks for it. The filter is the supported
    /// ITI-81 search-parameter subset ([`AuditSearchFilter`]).
    ///
    /// A criterion on a column is applied to that column. A criterion on an
    /// element of the document (`address` and the token parameters) first
    /// catches the access log's search index up ([`Self::catch_up`]), then keeps
    /// the records whose `search_keys` hold a stored token it names. Only the page is rendered.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a query fails, the rendering errors of a
    /// record that cannot be read back, and [`AuditError::IndexBehind`] when
    /// the index does not catch up within its wait.
    pub async fn search(&self, filter: &AuditSearchFilter) -> Result<AuditPage, AuditError> {
        // NOTE: our own design (#6322), an access-log entry's search tokens are rendered by the
        // frontier round, so a token search first moves the index past every entry before it.
        if filter.names_tokens() {
            self.catch_up().await?;
        }
        let Some(resolved) =
            crate::system_log::search::resolve_criteria(&self.pool, filter).await?
        else {
            return Ok(AuditPage::empty(filter));
        };
        let mut conn = acquire(&self.pool).await?;
        let cursor = self.cursor.bound(&mut conn).await?;
        let Statements {
            template,
            values: [count_values, values],
        } = self
            .page_shapes
            .statements(Kind::Page, filter, &resolved, cursor);
        // NOTE: our own design (#6005), a shape's text is a named statement that PostgreSQL may
        // plan once per connection (plan_cache_mode auto); any other text goes unnamed.
        let shaped = search_page::shaped(filter);
        let total = if filter.total {
            let total: i64 =
                sqlx::query_scalar_with(sqlx::AssertSqlSafe(template.sql(0)), count_values)
                    .persistent(shaped)
                    .fetch_one(&mut *conn)
                    .await?;
            Some(total)
        } else {
            None
        };
        let mut rows = sqlx::query_with(sqlx::AssertSqlSafe(template.sql(1)), values)
            .persistent(shaped)
            .fetch_all(&mut *conn)
            .await?;
        drop(conn);
        self.cursor.saw(&rows)?;
        filter.carried.observe(&rows)?;
        let more = rows.len() > search_page::page_size(filter);
        rows.truncate(search_page::page_size(filter));
        let documents = self.page_documents(&rows).await?;
        Ok(AuditPage {
            total,
            documents,
            more,
        })
    }

    /// The single `kind` statement of a search for `filter`, its token
    /// criteria resolved to `resolved`, with its values.
    pub(crate) fn shaped(
        &self,
        kind: Kind,
        filter: &AuditSearchFilter,
        resolved: &[Option<TokenMatch>],
    ) -> (Arc<Template>, SqlxValues) {
        let Statements {
            template,
            values: [values, _],
        } = self.page_shapes.statements(kind, filter, resolved, 0);
        (template, values)
    }
}

/// The most search shapes a store keeps the statements of; a search of
/// another shape builds its statements.
const SHAPES_KEPT: usize = 64;

/// The audit search statements of each search shape a store has run: the
/// text depends on the shape alone (which parameters are present, how many
/// values each has, what a token criterion resolved to, the date bounds,
/// what the page carries), so it is built once per shape and each search
/// binds its own values. No openEHR spec governs this: our own design.
#[derive(Debug, Default)]
pub(crate) struct PageShapes {
    templates: std::sync::RwLock<std::collections::BTreeMap<String, Arc<Template>>>,
}

/// The statements a search runs: the ITI-81 count and page (#6362), the
/// person's own trail, or the EHRs a search names (#6467).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    Page,
    Trail,
    Ehrs,
}

impl Kind {
    /// The statements of this kind for a search, with their values.
    fn build(
        self,
        filter: &AuditSearchFilter,
        resolved: &[Option<TokenMatch>],
        cursor: i64,
        numbers: Numbers,
        carried: &CarriedRead,
    ) -> Vec<(String, SqlxValues)> {
        match self {
            Self::Page => {
                let (count_sql, count_values, sql, values) =
                    build(filter, resolved, cursor, numbers, carried);
                vec![(count_sql, count_values), (sql, values)]
            }
            Self::Trail => vec![crate::system_log::withheld::trail_statement(
                filter, resolved,
            )],
            Self::Ehrs => vec![crate::system_log::withheld::ehrs_statement(
                filter, resolved,
            )],
        }
    }
}

/// The statements of one shape, and where each of their parameters comes
/// from.
#[derive(Debug)]
pub(crate) struct Template {
    slots: usize,
    statements: Vec<(String, Vec<Bind>)>,
}

/// The source of one parameter of a [`Template`]: a value of the search, by
/// its place among the search's values, or a value the shape fixes.
#[derive(Debug)]
enum Bind {
    Slot(usize),
    Fixed(sea_query::Value),
}

/// The statements of one search and their values, in the order its
/// [`Kind`] builds them; a kind of one statement leaves the second empty.
#[derive(Debug)]
pub(crate) struct Statements {
    /// The statements' text.
    pub(crate) template: Arc<Template>,
    /// Each statement's values.
    pub(crate) values: [SqlxValues; 2],
}

impl Template {
    /// The text of statement `at`, empty past the last.
    pub(crate) fn sql(&self, at: usize) -> &str {
        self.statements.get(at).map_or("", |(sql, _)| sql.as_str())
    }
}

impl PageShapes {
    /// The `kind` statements of a search for `filter`, its token criteria
    /// resolved to `resolved`, bound to the index cursor `cursor`: from the
    /// shape's template when this store has one, else built, and kept while
    /// fewer than [`SHAPES_KEPT`] shapes are.
    pub(crate) fn statements(
        &self,
        kind: Kind,
        filter: &AuditSearchFilter,
        resolved: &[Option<TokenMatch>],
        cursor: i64,
    ) -> Statements {
        let numbers = Numbers::of(filter);
        let carried = carried_of(filter);
        let key = shape_key(kind, filter, resolved, &carried);
        let held = self
            .templates
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        let template = held.unwrap_or_else(|| {
            let template = Arc::new(template_of(kind, filter, resolved, &carried));
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
            && let Some(values) = bound_all(&template, &slots)
        {
            return Statements { template, values };
        }
        // NOTE: our own design (#6362), a search whose values its shape's template does not
        // place builds its own statements.
        let mut values = [empty(), empty()];
        let mut statements = Vec::new();
        for (slot, (sql, bound)) in values
            .iter_mut()
            .zip(kind.build(filter, resolved, cursor, numbers, &carried))
        {
            *slot = bound;
            statements.push((sql, Vec::new()));
        }
        Statements {
            template: Arc::new(Template {
                slots: 0,
                statements,
            }),
            values,
        }
    }
}

/// No values.
fn empty() -> SqlxValues {
    SqlxValues(sea_query::Values(Vec::new()))
}

/// The values each statement of `template` takes from `slots`; `None` when a
/// bind names a value `slots` does not hold.
fn bound_all(template: &Template, slots: &[sea_query::Value]) -> Option<[SqlxValues; 2]> {
    let mut values = [empty(), empty()];
    for (slot, (_, binds)) in values.iter_mut().zip(&template.statements) {
        *slot = bound(binds, slots)?;
    }
    Some(values)
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
    kind: Kind,
    filter: &AuditSearchFilter,
    resolved: &[Option<TokenMatch>],
    carried: &CarriedRead,
) -> String {
    use std::fmt::Write as _;
    let mut key = String::with_capacity(32);
    let _ = write!(
        key,
        "{}{}{}{}{}",
        match kind {
            Kind::Page => 'P',
            Kind::Trail => 'T',
            Kind::Ehrs => 'E',
        },
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
    kind: Kind,
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
        statements: kind
            .build(&probe, &probe_resolved, cursor, numbers, &probe_carried)
            .into_iter()
            .map(|(sql, values)| (sql, binds_of(values)))
            .collect(),
    }
}
