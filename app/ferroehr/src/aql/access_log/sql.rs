// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The statement an AQL query over the access log runs (#5131), over
//! `audit.access_log_record`.
//!
//! A path the store keeps as a search column reads the column, so a condition
//! on the EHR and the time of an entry is served by the `(ehr_id,
//! time_recorded)` index; any other path reads the stored canonical form, and
//! one that crosses a container reaches its members through a lax SQL/JSON
//! path (<https://www.postgresql.org/docs/18/functions-json.html>). For the
//! person, a path the subject view leaves out reads as absent, so a condition
//! can no more test it than a column can show it (EHDS Art. 9(2),
//! `docs/law/eu/ehds/text.html`). No openEHR spec governs AQL over the access
//! log: our own design.

use jiff::Timestamp;
use openehr_query::lexer::CompOp;
use sea_query::{Alias, Expr, ExprTrait as _, Order, PostgresQueryBuilder, Query};
use sea_query_sqlx::{SqlxBinder as _, SqlxValues};

use super::path::{Leaf, LogPath};
use super::{LogCondition, LogPlan, LogQueryError, LogValue, feature};
use crate::access_log::view::{LogReader, Restrictions, subject_withholds};
use crate::aql::error::AqlError;
use crate::aql::ir::{ParamValue, Params};
use crate::aql::sql::expr::aql_like_to_sql;
use crate::ids::EhrId;

/// The alias of the record table in the statement.
const RECORD: &str = "r";

/// Whom a statement reads the log for, and which page of it.
#[derive(Debug, Clone, Copy)]
pub struct LogScope<'a> {
    /// The reader, whose view decides what a path reaches.
    pub reader: LogReader,
    /// The access policy's restrictions of the subject view (EHDS Art. 9(3)).
    pub restrictions: &'a Restrictions,
    /// The EHRs whose entries are read; empty reads every entry.
    pub ehrs: &'a [EhrId],
    /// The most entries answered.
    pub limit: Option<i64>,
    /// The entries skipped.
    pub offset: Option<i64>,
}

/// How a value read for a path compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Typed {
    /// As text.
    Text,
    /// The text of a number in the canonical form, cast to compare.
    Number,
    /// The text of a boolean in the canonical form, cast to compare.
    Boolean,
    /// A `bigint` search column.
    Integer,
    /// A `timestamptz` search column.
    Instant,
}

/// The search columns the record row keeps, by the entry path each holds.
const COLUMNS: &[(&[&str], &str, Typed)] = &[
    (&["uid", "value"], "uid", Typed::Text),
    (&["system_id"], "system_id", Typed::Text),
    (&["time_recorded", "value"], "time_recorded", Typed::Instant),
    (&["ehr_id", "value"], "ehr_id", Typed::Text),
    (&["action", "value"], "action", Typed::Text),
    (&["outcome", "result", "value"], "result", Typed::Text),
    (
        &["accessor", "party", "external_ref", "id", "value"],
        "party_id",
        Typed::Text,
    ),
    (&["channel", "request_id"], "request_id", Typed::Text),
    (&["link", "sequence"], "chain_seq", Typed::Integer),
    (&["link", "algorithm"], "algorithm", Typed::Text),
    (&["link", "previous_digest"], "previous_digest", Typed::Text),
    (&["link", "content_digest"], "content_digest", Typed::Text),
    (&["link", "digest"], "digest", Typed::Text),
    (&["link", "signature"], "signature", Typed::Text),
];

/// Where a path's values are read.
enum Reach {
    /// A search column.
    Column(&'static str, Typed),
    /// The canonical form, at one value.
    Single(String),
    /// The canonical form, at every member of a container.
    Many(String),
    /// Nowhere the reader may see.
    Withheld,
}

/// The statement `plan` runs for `scope`, its values bound from `params`.
///
/// # Errors
/// [`AqlError::Feature`] for a value of another type than its path's, a
/// `LIKE` on a path that is no string, or an unbound parameter.
pub fn statement(
    plan: &LogPlan,
    params: &Params,
    scope: &LogScope<'_>,
) -> Result<(String, SqlxValues), AqlError> {
    let builder = Builder {
        reader: scope.reader,
        restrictions: scope.restrictions,
        params,
    };
    let mut select = Query::select();
    select
        .columns(
            [
                "chain_seq",
                "content",
                "algorithm",
                "previous_digest",
                "content_digest",
                "digest",
                "signature",
            ]
            .map(|name| (Alias::new(RECORD), Alias::new(name))),
        )
        .from_as(
            (Alias::new("audit"), Alias::new("access_log_record")),
            Alias::new(RECORD),
        )
        .and_where(column("record_type").eq("ACCESS_LOG_ENTRY"));
    match scope.ehrs {
        [] => {}
        [one] => {
            select.and_where(column("ehr_id").eq(one.to_string()));
        }
        many => {
            select.and_where(column("ehr_id").is_in(many.iter().map(ToString::to_string)));
        }
    }
    if let Some(filter) = &plan.filter {
        select.and_where(builder.condition(filter)?);
    }
    let tie = match plan.order.last() {
        None => {
            select.order_by_expr(column("time_recorded"), Order::Desc);
            Order::Desc
        }
        Some((_, descending)) => {
            for (path, descending) in &plan.order {
                select.order_by_expr(builder.key(path), direction(*descending));
            }
            direction(*descending)
        }
    };
    select.order_by_expr(column("chain_seq"), tie);
    if let Some(limit) = scope.limit.and_then(|limit| u64::try_from(limit).ok()) {
        select.limit(limit);
    }
    if let Some(offset) = scope.offset.and_then(|offset| u64::try_from(offset).ok()) {
        select.offset(offset);
    }
    Ok(select.build_sqlx(PostgresQueryBuilder))
}

/// The column `name` of the record table.
fn column(name: &str) -> Expr {
    Expr::col((Alias::new(RECORD), Alias::new(name)))
}

/// The sort order `descending` names.
fn direction(descending: bool) -> Order {
    if descending { Order::Desc } else { Order::Asc }
}

/// The condition builder for one reader.
struct Builder<'a> {
    /// The reader.
    reader: LogReader,
    /// The policy's restrictions of the subject view.
    restrictions: &'a Restrictions,
    /// The bound parameters.
    params: &'a Params,
}

impl Builder<'_> {
    /// Where `path` is read for this reader.
    fn reach(&self, path: &LogPath) -> Reach {
        let names = path.names();
        if self.reader == LogReader::Person && subject_withholds(&names, self.restrictions) {
            return Reach::Withheld;
        }
        if let Some((_, name, typed)) = COLUMNS.iter().find(|(steps, ..)| *steps == names) {
            return Reach::Column(name, *typed);
        }
        if path.multiple {
            let members = path.steps.iter().map(|step| format!(".\"{step}\""));
            return Reach::Many(std::iter::once("$".to_owned()).chain(members).collect());
        }
        Reach::Single(format!("{{{}}}", path.steps.join(",")))
    }

    /// The SQL of `condition`.
    fn condition(&self, condition: &LogCondition) -> Result<Expr, AqlError> {
        Ok(match condition {
            LogCondition::Constant(constant) => truth(*constant),
            LogCondition::Not(inner) => self.condition(inner)?.not(),
            LogCondition::And(left, right) => self.condition(left)?.and(self.condition(right)?),
            LogCondition::Or(left, right) => self.condition(left)?.or(self.condition(right)?),
            LogCondition::Exists(path) => self.exists(path),
            LogCondition::Compare { path, op, value } => self.test(path, |operand, typed| {
                self.compare(path, operand, typed, *op, value)
            })?,
            LogCondition::Like { path, pattern } => self.test(path, |operand, typed| {
                self.like(path, operand, typed, pattern)
            })?,
            LogCondition::Matches { path, values } => self.test(path, |operand, typed| {
                values
                    .iter()
                    .map(|value| self.compare(path, operand.clone(), typed, CompOp::Eq, value))
                    .reduce(|any, next| Ok(any?.or(next?)))
                    .unwrap_or_else(|| Ok(truth(false)))
            })?,
        })
    }

    /// `EXISTS path`: whether the reader sees a value at `path`.
    fn exists(&self, path: &LogPath) -> Expr {
        match self.reach(path) {
            Reach::Withheld => truth(false),
            // NOTE: our own design, the link is kept in columns, never in the canonical form.
            _ if path.steps == ["link"] => truth(true),
            Reach::Column(name, _) => column(name).is_not_null(),
            Reach::Single(steps) => {
                Expr::cust_with_values("(r.content::jsonb #> $1::text[]) IS NOT NULL", [steps])
            }
            Reach::Many(jsonpath) => Expr::cust_with_values(
                "jsonb_path_exists(r.content::jsonb, $1::jsonpath)",
                [jsonpath],
            ),
        }
    }

    /// The test `on` builds over `path`'s value, held to every member when
    /// the path reaches a container.
    fn test(
        &self,
        path: &LogPath,
        on: impl Fn(Expr, Typed) -> Result<Expr, AqlError>,
    ) -> Result<Expr, AqlError> {
        let typed = typed_leaf(path.leaf);
        Ok(match self.reach(path) {
            // NOTE: EHDS Art. 9(2), what the subject view leaves out reads as absent.
            Reach::Withheld if path.multiple => truth(false),
            Reach::Withheld => on(Expr::cust("NULL::text"), typed)?,
            Reach::Column(name, column_typed) => on(column(name), column_typed)?,
            Reach::Single(steps) => on(
                Expr::cust_with_values("(r.content::jsonb #>> $1::text[])", [steps]),
                typed,
            )?,
            Reach::Many(jsonpath) => Expr::cust_with_exprs(
                "EXISTS (SELECT 1 FROM jsonb_path_query(r.content::jsonb, $1::jsonpath) AS m(v) \
                     WHERE $2)",
                [
                    Expr::val(jsonpath),
                    on(Expr::cust("(m.v #>> '{}')"), typed)?,
                ],
            ),
        })
    }

    /// `operand op value`, `operand` read as `typed`.
    fn compare(
        &self,
        path: &LogPath,
        operand: Expr,
        typed: Typed,
        op: CompOp,
        value: &LogValue,
    ) -> Result<Expr, AqlError> {
        let value = self.value(value)?;
        let mismatch = |expected| {
            feature(LogQueryError::TypeMismatch {
                path: path.text.clone(),
                expected,
            })
        };
        let (left, right) = match (typed, value) {
            (Typed::Instant, ParamValue::Str(text)) => {
                text.parse::<Timestamp>()
                    .map_err(|_unparsed: jiff::Error| {
                        mismatch("an ISO 8601 date-time with an offset")
                    })?;
                (
                    operand,
                    Expr::cust_with_values("($1)::timestamptz", [text.clone()]),
                )
            }
            (Typed::Instant, _) => return Err(mismatch("an ISO 8601 date-time with an offset")),
            (Typed::Integer, ParamValue::Int(number)) => (operand, Expr::val(*number)),
            (Typed::Integer, ParamValue::Real(number)) => (operand, Expr::val(*number)),
            (Typed::Number, ParamValue::Int(number)) => (cast(operand, typed), Expr::val(*number)),
            (Typed::Number, ParamValue::Real(number)) => (cast(operand, typed), Expr::val(*number)),
            (Typed::Integer | Typed::Number, _) => return Err(mismatch("a number")),
            (Typed::Boolean, ParamValue::Bool(flag)) => (cast(operand, typed), Expr::val(*flag)),
            (Typed::Boolean, _) => return Err(mismatch("true or false")),
            (Typed::Text, ParamValue::Str(text)) => (operand, Expr::val(text.clone())),
            (Typed::Text, _) => return Err(mismatch("a string")),
        };
        Ok(Expr::cust_with_exprs(comparison(op), [left, right]))
    }

    /// `operand LIKE pattern`, the AQL pattern read into SQL's.
    fn like(
        &self,
        path: &LogPath,
        operand: Expr,
        typed: Typed,
        pattern: &LogValue,
    ) -> Result<Expr, AqlError> {
        let ParamValue::Str(pattern) = self.value(pattern)? else {
            return Err(feature(LogQueryError::TypeMismatch {
                path: path.text.clone(),
                expected: "a string pattern",
            }));
        };
        if path.leaf != Leaf::Text {
            return Err(feature(LogQueryError::TypeMismatch {
                path: path.text.clone(),
                expected: "a number or boolean, never a pattern",
            }));
        }
        // NOTE: QUERY §LIKE, a date-time matches by its string representation.
        let operand = if typed == Typed::Instant {
            Expr::cust_with_values(
                "(r.content::jsonb #>> $1::text[])",
                [format!("{{{}}}", path.steps.join(","))],
            )
        } else {
            operand
        };
        Ok(Expr::cust_with_exprs(
            "$1 LIKE $2 ESCAPE '\\'",
            [operand, Expr::val(aql_like_to_sql(pattern))],
        ))
    }

    /// The value `value` stands for.
    fn value<'v>(&'v self, value: &'v LogValue) -> Result<&'v ParamValue, AqlError> {
        match value {
            LogValue::Literal(literal) => Ok(literal),
            LogValue::Parameter(name) => self
                .params
                .get(name)
                .ok_or_else(|| feature(LogQueryError::UnboundParameter(name.clone()))),
        }
    }

    /// The `ORDER BY` expression of `path`.
    fn key(&self, path: &LogPath) -> Expr {
        let typed = typed_leaf(path.leaf);
        match self.reach(path) {
            Reach::Column(name, _) => column(name),
            Reach::Single(steps) => cast(
                Expr::cust_with_values("(r.content::jsonb #>> $1::text[])", [steps]),
                typed,
            ),
            // NOTE: our own design, a key the reader may not see orders as an absent value.
            Reach::Withheld | Reach::Many(_) => Expr::cust("NULL::text"),
        }
    }
}

/// The comparison type of a path read from the canonical form.
fn typed_leaf(leaf: Leaf) -> Typed {
    match leaf {
        Leaf::Number => Typed::Number,
        Leaf::Boolean => Typed::Boolean,
        Leaf::Text | Leaf::Object => Typed::Text,
    }
}

/// `operand`, text of the canonical form, cast to the type it compares as.
fn cast(operand: Expr, typed: Typed) -> Expr {
    match typed {
        Typed::Number => Expr::cust_with_exprs("($1)::numeric", [operand]),
        Typed::Boolean => Expr::cust_with_exprs("($1)::boolean", [operand]),
        Typed::Text | Typed::Integer | Typed::Instant => operand,
    }
}

/// The SQL comparison of `op` between the expressions `$1` and `$2`.
fn comparison(op: CompOp) -> &'static str {
    match op {
        CompOp::Eq => "$1 = $2",
        CompOp::Ne => "$1 <> $2",
        CompOp::Gt => "$1 > $2",
        CompOp::Ge => "$1 >= $2",
        CompOp::Lt => "$1 < $2",
        CompOp::Le => "$1 <= $2",
    }
}

/// The constant `value`.
fn truth(value: bool) -> Expr {
    Expr::cust(if value { "TRUE" } else { "FALSE" })
}

#[cfg(test)]
mod tests {
    use openehr_query::parser::parse_str;
    use uuid::Uuid;

    use super::*;
    use crate::aql::access_log::plan;

    fn sql(aql: &str, reader: LogReader, ehrs: &[EhrId]) -> String {
        let plan = plan(&parse_str(aql).expect("parses"))
            .expect("plans")
            .expect("a log query");
        let scope = LogScope {
            reader,
            restrictions: &Restrictions::default(),
            ehrs,
            limit: Some(10),
            offset: None,
        };
        statement(&plan, &Params::default(), &scope)
            .expect("builds")
            .0
    }

    #[test]
    fn the_ehr_and_time_read_the_indexed_columns() {
        let ehr = EhrId(Uuid::nil());
        let text = sql(
            "SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/time_recorded/value >= \
             '2026-11-01T00:00:00Z'",
            LogReader::Person,
            &[ehr],
        );
        assert!(text.contains(r#""r"."ehr_id" = $2"#), "{text}");
        assert!(
            text.contains(r#""r"."time_recorded" >= ($3)::timestamptz"#),
            "{text}"
        );
        assert!(
            text.contains(r#"ORDER BY "r"."time_recorded" DESC"#),
            "{text}"
        );
    }

    #[test]
    fn a_withheld_path_reads_as_absent_for_the_person() {
        let aql = "SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/accessor/credential/subject = 'dr'";
        let person = sql(aql, LogReader::Person, &[EhrId(Uuid::nil())]);
        assert!(person.contains("NULL::text = $"), "{person}");
        let reviewer = sql(aql, LogReader::Reviewer, &[]);
        assert!(reviewer.contains("#>> $"), "{reviewer}");
    }

    #[test]
    fn a_list_is_tested_member_by_member() {
        let text = sql(
            "SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/items/rm_type = 'COMPOSITION'",
            LogReader::Reviewer,
            &[],
        );
        assert!(text.contains("jsonb_path_query"), "{text}");
    }

    #[test]
    fn a_value_of_another_type_is_refused() {
        let plan = plan(
            &parse_str("SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/time_recorded/value > 3")
                .expect("parses"),
        )
        .expect("plans")
        .expect("a log query");
        let scope = LogScope {
            reader: LogReader::Reviewer,
            restrictions: &Restrictions::default(),
            ehrs: &[],
            limit: None,
            offset: None,
        };
        assert!(statement(&plan, &Params::default(), &scope).is_err());
    }
}
