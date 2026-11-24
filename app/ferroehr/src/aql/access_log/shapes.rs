// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The statement of each access-log query shape run (#6467), built once per
//! shape by [`super::sql::statement`] and bound to each run's values.
//!
//! A shape is the query text, the reader and the restrictions it reads under,
//! how many EHRs it is scoped to, whether it is paged, and the kind of each
//! parameter's value. Its statement is built twice, from two sets of values
//! no query fixes; a value that differs between the two is a value of the run,
//! found by its place among the run's values, and one that does not is a value
//! of the shape. A shape whose statement cannot be told apart so builds its
//! statement on every run. No openEHR spec governs this: our own design.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use sea_query::Value;
use sea_query_sqlx::SqlxValues;

use super::LogPlan;
use super::sql::{LogScope, statement};
use crate::access_log::stored_form::column_uuid;
use crate::aql::error::AqlError;
use crate::aql::ir::{ParamValue, Params};
use crate::aql::sql::expr::aql_like_to_sql;
use crate::ids::EhrId;

/// The most shapes whose statements are kept; a run of another shape builds
/// its statement.
const SHAPES_KEPT: usize = 64;

/// The statement of each shape run, or `None` for a shape that builds its
/// statement on every run.
static SHAPES: LazyLock<RwLock<BTreeMap<ShapeKey, Option<Arc<Template>>>>> =
    LazyLock::new(RwLock::default);

/// The key of a shape: what it reads under and its parameters' kinds, and
/// the query text.
type ShapeKey = (String, String);

/// The statement of one shape, and where each of its parameters comes from.
#[derive(Debug)]
struct Template {
    slots: usize,
    sql: String,
    binds: Vec<Bind>,
}

/// The source of one parameter of a [`Template`]: a value of the run, by its
/// place among the run's values, or a value the shape fixes.
#[derive(Debug)]
enum Bind {
    Slot(usize),
    Fixed(Value),
}

/// The statement an AQL query `aql`, planned as `plan`, runs for `scope`,
/// its values bound from `params`: from the shape's statement when one is
/// kept, else built.
///
/// # Errors
/// As [`statement`].
pub fn shaped_statement(
    aql: &str,
    plan: &LogPlan,
    params: &Params,
    scope: &LogScope<'_>,
) -> Result<(String, SqlxValues), AqlError> {
    let Some(values) = run_values(plan, params) else {
        return statement(plan, params, scope);
    };
    let key = shape_key(aql, scope, &values);
    let held = SHAPES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
        .cloned();
    let template = if let Some(template) = held {
        template
    } else {
        let template = template_of(plan, scope, &values).map(Arc::new);
        let mut shapes = SHAPES.write().unwrap_or_else(PoisonError::into_inner);
        if shapes.len() < SHAPES_KEPT {
            shapes.insert(key, template.clone());
        }
        template
    };
    let slots = slots(scope.ehrs, scope.limit, scope.offset, &values);
    if let Some(template) = template
        && slots.len() == template.slots
        && let Some(bound) = bound(&template.binds, &slots)
    {
        return Ok((template.sql.clone(), bound));
    }
    statement(plan, params, scope)
}

/// The value of each parameter `plan` names, in its order; `None` when one
/// is unbound, which [`statement`] refuses.
fn run_values(plan: &LogPlan, params: &Params) -> Option<Vec<ParamValue>> {
    plan.params
        .iter()
        .map(|name| params.get(name).cloned())
        .collect()
}

/// The key of the shape of a run of `aql` for `scope` with `values`.
fn shape_key(aql: &str, scope: &LogScope<'_>, values: &[ParamValue]) -> ShapeKey {
    use std::fmt::Write as _;
    let mut key = String::with_capacity(64);
    let _ = write!(
        key,
        "{:?}|{:?}|{}|{}{}|",
        scope.reader,
        scope.restrictions,
        scope.ehrs.len(),
        u8::from(scope.limit.is_some()),
        u8::from(scope.offset.is_some()),
    );
    for value in values {
        key.push(kind_of(value));
    }
    (key, aql.to_owned())
}

/// The kind of `value` a shape depends on: its type, and for a string
/// whether it reads as an instant or as a canonical EHR id.
fn kind_of(value: &ParamValue) -> char {
    match value {
        ParamValue::Int(_) => 'i',
        ParamValue::Real(_) => 'r',
        ParamValue::Bool(_) => 'b',
        ParamValue::Null => 'n',
        ParamValue::Str(text) if column_uuid(text).is_some() => 'u',
        ParamValue::Str(text) if text.parse::<jiff::Timestamp>().is_ok() => 't',
        ParamValue::Str(_) => 's',
    }
}

/// The values a run of `scope` with `values` may bind, in the order
/// [`template_of`] reads them: each EHR, the limit, the offset, then each
/// parameter's value in each form a statement binds it.
fn slots(
    ehrs: &[EhrId],
    limit: Option<i64>,
    offset: Option<i64>,
    values: &[ParamValue],
) -> Vec<Value> {
    let mut slots: Vec<Value> = ehrs.iter().map(|ehr| Value::from(ehr.0)).collect();
    for paged in [limit, offset].into_iter().flatten() {
        if let Ok(paged) = u64::try_from(paged) {
            slots.push(Value::from(paged));
        }
    }
    for value in values {
        match value {
            ParamValue::Int(number) => slots.push(Value::from(*number)),
            ParamValue::Real(number) => slots.push(Value::from(*number)),
            ParamValue::Bool(flag) => slots.push(Value::from(*flag)),
            ParamValue::Null => {}
            // NOTE: our own design (#6467), the text of an instant or an EHR id holds no
            // character a LIKE pattern reads, so only another text has a pattern form of its own.
            ParamValue::Str(text) => {
                slots.push(Value::from(text.clone()));
                match kind_of(value) {
                    'u' => slots.extend(column_uuid(text).map(Value::from)),
                    's' => slots.push(Value::from(aql_like_to_sql(text))),
                    _ => {}
                }
            }
        }
    }
    slots
}

/// The statement of the shape of a run for `scope` with `values`, or `None`
/// when two runs of the shape do not tell its values apart.
fn template_of(plan: &LogPlan, scope: &LogScope<'_>, values: &[ParamValue]) -> Option<Template> {
    let probe = |round: u64| -> Option<(String, Vec<Value>, Vec<Value>)> {
        let ehrs: Vec<EhrId> = (0..scope.ehrs.len())
            .map(|at| EhrId(sentinel_uuid(round, at)))
            .collect();
        let limit = scope.limit.map(|_| sentinel_number(round, 0));
        let offset = scope.offset.map(|_| sentinel_number(round, 1));
        let probed: Vec<ParamValue> = values
            .iter()
            .enumerate()
            .map(|(at, value)| sentinel(round, at, value))
            .collect();
        let mut params = Params::default();
        for (name, value) in plan.params.iter().zip(&probed) {
            params.insert(name.clone(), value.clone());
        }
        let probe_scope = LogScope {
            ehrs: &ehrs,
            limit,
            offset,
            ..*scope
        };
        let (sql, bound) = statement(plan, &params, &probe_scope).ok()?;
        Some((sql, bound.0.0, slots(&ehrs, limit, offset, &probed)))
    };
    let (sql, first, first_slots) = probe(0)?;
    let (second_sql, second, second_slots) = probe(1)?;
    if sql != second_sql || first.len() != second.len() {
        return None;
    }
    let binds = first
        .into_iter()
        .zip(second)
        .map(|(one, other)| {
            if one == other {
                return Some(Bind::Fixed(one));
            }
            let mut found = (0..first_slots.len()).filter(|at| {
                first_slots.get(*at) == Some(&one) && second_slots.get(*at) == Some(&other)
            });
            match (found.next(), found.next()) {
                (Some(at), None) => Some(Bind::Slot(at)),
                _ => None,
            }
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Template {
        slots: first_slots.len(),
        sql,
        binds,
    })
}

/// The value standing for `value`, the parameter at `at`, in probe `round`:
/// of its kind, and no value a query fixes.
fn sentinel(round: u64, at: usize, value: &ParamValue) -> ParamValue {
    let place = u64::try_from(at).unwrap_or(u64::MAX);
    match kind_of(value) {
        'i' => ParamValue::Int(sentinel_number(round, place.saturating_add(2))),
        'r' => ParamValue::Real(
            f64::from(u32::try_from(place).unwrap_or(u32::MAX)) + 0.5 + f64::from(round == 1),
        ),
        'b' => ParamValue::Bool(round == 1),
        'n' => ParamValue::Null,
        'u' => ParamValue::Str(
            sentinel_uuid(round, at.saturating_add(1 << 20))
                .hyphenated()
                .to_string(),
        ),
        't' => ParamValue::Str(format!(
            "2001-01-01T00:00:00.{:09}Z",
            round * 1000 + place + 1
        )),
        _ => ParamValue::Str(format!("\u{1}sentinel*{round}*{place}")),
    }
}

/// A number no query fixes, for the place `place` in probe `round`.
fn sentinel_number(round: u64, place: u64) -> i64 {
    let base: i64 = if round == 0 {
        3_000_000_000_000
    } else {
        5_000_000_000_000
    };
    base.saturating_add(i64::try_from(place).unwrap_or(i64::MAX))
}

/// An EHR id no query fixes, for the place `at` in probe `round`.
fn sentinel_uuid(round: u64, at: usize) -> uuid::Uuid {
    let place = u128::try_from(at).unwrap_or(u128::MAX);
    uuid::Uuid::from_u128(
        0x5e17_0000_0000_4000_8000_0000_0000_0000 | (u128::from(round) << 64) | place,
    )
}

/// The values `binds` take from `slots`; `None` when a bind names a value
/// `slots` does not hold.
fn bound(binds: &[Bind], slots: &[Value]) -> Option<SqlxValues> {
    binds
        .iter()
        .map(|bind| match bind {
            Bind::Slot(at) => slots.get(*at).cloned(),
            Bind::Fixed(value) => Some(value.clone()),
        })
        .collect::<Option<Vec<_>>>()
        .map(|values| SqlxValues(sea_query::Values(values)))
}

#[cfg(test)]
mod tests {
    use openehr_query::parser::parse_str;

    use super::{SHAPES, shaped_statement};
    use crate::access_log::view::{LogReader, Restrictions};
    use crate::aql::access_log::plan;
    use crate::aql::access_log::sql::{LogScope, statement};
    use crate::aql::ir::{ParamValue, Params};
    use crate::ids::EhrId;

    /// The queries of [`a_shapes_statement_is_the_built_one`]: each with the
    /// values of two runs of its shape.
    fn runs() -> Vec<(&'static str, &'static str, [ParamValue; 2])> {
        let text = |value: &str| ParamValue::Str(value.to_owned());
        vec![
            (
                "SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/ehr_id/value = $ehr",
                "ehr",
                [
                    text("7d44b88c-4199-4bad-97dc-d78268e01398"),
                    text("1f2e3d4c-5b6a-4978-8695-a4b3c2d1e0f9"),
                ],
            ),
            (
                "SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/time_recorded/value >= $since \
                 ORDER BY l/time_recorded/value ASC",
                "since",
                [text("2026-11-01T00:00:00Z"), text("2026-11-02T08:30:00.5Z")],
            ),
            (
                "SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/accessor/party/external_ref/id/value \
                 LIKE $who AND l/action/defining_code/code_string = 'read'",
                "who",
                [text("dr*"), text("nurse?x")],
            ),
            (
                "SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/items/rm_type = $type",
                "type",
                [text("COMPOSITION"), text("EHR_STATUS")],
            ),
        ]
    }

    /// A run's statement, from its shape's statement or built, is byte for
    /// byte the statement built for it, and so are its values (#6467).
    #[test]
    fn a_shapes_statement_is_the_built_one() {
        let ehrs = [
            EhrId(uuid::Uuid::from_u128(7)),
            EhrId(uuid::Uuid::from_u128(9)),
        ];
        for (aql, name, values) in runs() {
            let plan = plan(&parse_str(aql).expect("parses"))
                .expect("plans")
                .expect("a log query");
            for (round, value) in values.into_iter().enumerate() {
                let params = Params::default().with(name, value);
                for reader in [LogReader::Reviewer, LogReader::Person] {
                    let limit = i64::try_from(round).expect("small") + 10;
                    let scope = LogScope {
                        reader,
                        restrictions: &Restrictions::default(),
                        ehrs: ehrs.get(..=round).expect("two EHRs"),
                        limit: Some(limit),
                        offset: Some(limit * 2),
                    };
                    let built = statement(&plan, &params, &scope).expect("builds");
                    let shaped = shaped_statement(aql, &plan, &params, &scope).expect("builds");
                    assert_eq!(shaped.0, built.0, "{aql}");
                    assert_eq!(shaped.1.0, built.1.0, "{aql}");
                }
            }
        }
        let shapes = SHAPES.read().expect("readable");
        for (aql, ..) in runs() {
            assert!(
                shapes
                    .iter()
                    .any(|((_, text), template)| text == aql && template.is_some()),
                "{aql} keeps its statement"
            );
        }
    }
}
