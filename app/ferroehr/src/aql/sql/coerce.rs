// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The typed coercions of a value expression and of a bound value (QUERY
//! master03 §Comparison operators, §ORDER BY, §Built-in Types), and the
//! comparison of a mixed-type (`Raw`) leaf, such as an `ELEMENT`'s whole
//! `DATA_VALUE`, with the operand opposite it.
//!
//! A whole data value is never compared or ordered by its serialized JSON. AQL
//! 1.1.0 says how primitives compare and that ORDER BY uses the comparison of
//! `Ordered` types (§ORDER BY); it does not say how a whole data value compares
//! with a literal, so the reading below is our own design: a value compares
//! with a literal of its own kind and with no other. A number compares with a
//! JSON number and with the RM magnitude or value of a whole `DV_QUANTITY`,
//! `DV_COUNT`, `DV_PROPORTION`, `DV_ORDINAL`, `DV_SCALE` or `DV_DURATION`
//! (RM `data_types` master06 §Quantity package); an ISO 8601 date or date-time
//! string compares with a whole `DV_DATE` or `DV_DATE_TIME` and a time string
//! with a whole `DV_TIME` as the instant each names (§Dates and Times, whose
//! path metadata is the value's own `_type`); every other pair with a whole
//! value is unknown, so it keeps no row under `=`, `!=` or `NOT`. A JSON string,
//! number or Boolean compares as text, as before.

use sea_query::{BinOper, Expr, ExprTrait as _};

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::{Coercion, LeafPath, PathTarget};

use super::ValueMode;
use super::expr::{as_text, call, cast};

/// Apply the value coercion to a jsonb extraction base (QUERY master03
/// §Comparison operators; `DV_ORDERED` ordered-magnitude via `ext.openehr_magnitude`).
pub(super) fn coerce_value(base: Expr, mode: ValueMode, leaf: &LeafPath) -> Expr {
    match mode {
        ValueMode::Projection => base,
        ValueMode::Value(Coercion::Magnitude) => {
            if leaf.types.names().iter().any(|t| t.starts_with("DV_")) {
                call("openehr_magnitude", vec![base])
            } else {
                cast(as_text(base), "numeric")
            }
        }
        ValueMode::Value(Coercion::Boolean) => cast(as_text(base), "boolean"),
        // NOTE: RM `data_types` master07 §Partial Date/Times admits reduced
        // precision, so the leaf reads through the total `ext.openehr_timestamp`
        // — floor completion, NULL for garbage.
        ValueMode::Value(Coercion::Temporal) => {
            // NOTE: QUERY master03 §Dates and Times compares `context/start_time`, the DV
            // object itself, as a date/time quantity, so an object leaf reads its `value`.
            let text = if leaf.types.names().iter().any(|t| t.starts_with("DV_")) {
                as_text(base.binary(BinOper::Custom("->"), cast(Expr::val("value"), "text")))
            } else {
                as_text(base)
            };
            call("openehr_timestamp", vec![text])
        }
        ValueMode::Value(Coercion::Text | Coercion::Raw) => as_text(base),
        ValueMode::RawNumeric
        | ValueMode::RawText
        | ValueMode::RawTemporal(_)
        | ValueMode::RawAmount => raw_key(base, mode),
    }
}

/// Cast a bound right-hand-side value to match the comparison coercion
/// (QUERY master03 §Comparison operators).
///
/// A temporal bound routes through the same total `ext.openehr_timestamp` the
/// leaf side uses (floor completion for reduced precision), after a plan-time
/// shape check: a bound that is no ISO-8601 date/time at all is the CALLER's
/// defect and refuses as a typed error — never a silent empty result and
/// never a driver error surfacing as a 500.
///
/// # Errors
/// [`SqlError::UncoercibleTemporal`] when a temporal bound is not a string
/// holding an ISO-8601 date, time, or date-time (reduced precision is
/// accepted): a number, boolean or NULL bound is refused the same way.
/// [`SqlError::UncoercibleComparison`] for a Boolean bound in a numeric
/// comparison or a number in a Boolean one.
pub(super) fn coerce_rhs(value: sea_query::Value, coercion: Coercion) -> Result<Expr, AqlError> {
    Ok(match coercion {
        Coercion::Magnitude => numeric_value(value)?,
        Coercion::Boolean => {
            // NOTE: QUERY master03 §Built-in Types types a number literal as Integer or Real, which
            // no Boolean equals or orders against: our own design refuses the pair as the caller's.
            if is_number(&value) {
                return Err(SqlError::UncoercibleComparison {
                    value: "a number",
                    compared: "a Boolean",
                }
                .into());
            }
            cast(Expr::val(value), "boolean")
        }
        Coercion::Temporal => {
            // NOTE: QUERY master03 §Dates and Times reads a date/time as a string literal and
            // §Literals, §Comparison operators give `NULL` no comparison meaning (presence is
            // §EXISTS), so only an ISO 8601 string bounds a temporal comparison.
            if !matches!(&value, sea_query::Value::String(Some(s)) if is_iso_temporal(s)) {
                return Err(SqlError::UncoercibleTemporal.into());
            }
            call("openehr_timestamp", vec![cast(Expr::val(value), "text")])
        }
        // A mixed-type comparison that reaches a bound value here has no path
        // opposite it (a function), so the value keeps its own type.
        Coercion::Raw if is_number(&value) => numeric_cast(value),
        Coercion::Raw if matches!(value, sea_query::Value::Bool(Some(_))) => {
            cast(Expr::val(value), "boolean")
        }
        Coercion::Text | Coercion::Raw => cast(Expr::val(value), "text"),
    })
}

/// A bound value as an exact `numeric` (QUERY master03 §Real data, §Integer
/// data).
///
/// A Real binds as the shortest decimal that round-trips its double, which is
/// the literal as written whenever it names one double, and casts from that
/// text: PostgreSQL's `float8` to `numeric` cast keeps 15 significant digits
/// (PostgreSQL 18 docs §8.1.3), so a 17-digit magnitude would compare with
/// another number than the one written. Every other value casts as bound.
///
/// # Errors
/// [`SqlError::UncoercibleComparison`] for a Boolean, which no number equals
/// or orders against.
pub(super) fn numeric_value(value: sea_query::Value) -> Result<Expr, AqlError> {
    if matches!(value, sea_query::Value::Bool(Some(_))) {
        return Err(SqlError::UncoercibleComparison {
            value: "a Boolean",
            compared: "a number",
        }
        .into());
    }
    Ok(numeric_cast(value))
}

/// [`numeric_value`] of a value that is no Boolean.
fn numeric_cast(value: sea_query::Value) -> Expr {
    let exact = |r: f64| cast(Expr::val(r.to_string()), "numeric");
    match value {
        sea_query::Value::Double(Some(r)) => exact(r),
        sea_query::Value::Float(Some(r)) => exact(f64::from(r)),
        other => cast(Expr::val(other), "numeric"),
    }
}

/// Whether a bound value is a number (QUERY master03 §Integer data, §Real
/// data).
fn is_number(value: &sea_query::Value) -> bool {
    matches!(
        value,
        sea_query::Value::TinyInt(Some(_))
            | sea_query::Value::SmallInt(Some(_))
            | sea_query::Value::Int(Some(_))
            | sea_query::Value::BigInt(Some(_))
            | sea_query::Value::TinyUnsigned(Some(_))
            | sea_query::Value::SmallUnsigned(Some(_))
            | sea_query::Value::Unsigned(Some(_))
            | sea_query::Value::BigUnsigned(Some(_))
            | sea_query::Value::Float(Some(_))
            | sea_query::Value::Double(Some(_))
    )
}

/// Whether `s` is an ISO-8601 date, time, or date-time literal in the shapes
/// the temporal floor accepts — reduced precision and compact forms included
/// (BASE `foundation_types` master06 §Time Types). Purely a plan-time shape
/// gate: the SQL-side completion is the semantics.
fn is_iso_temporal(s: &str) -> bool {
    temporal_kinds(s).is_some()
}

/// The whole date/time values an ISO 8601 string compares with: a date or a
/// date-time names an instant on the calendar, a time one on no date (BASE
/// `foundation_types` master06 §Time Types). A string of four digits reads as
/// a year first. `None` for a string that is no date/time at all.
pub(super) fn temporal_kinds(s: &str) -> Option<TemporalKinds> {
    static ISO_TEMPORAL: std::sync::LazyLock<(regex::Regex, regex::Regex)> =
        std::sync::LazyLock::new(|| {
            // The separator set includes the space form PostgreSQL's own parser
            // accepts.
            let time = r"\d{2}(:\d{2}(:\d{2}([.,]\d+)?)?|\d{2}(\d{2}([.,]\d+)?)?)?";
            let offset = r"([Zz]|[+-]\d{2}(:?\d{2})?)?";
            let date = r"\d{4}(-\d{2}(-\d{2})?|\d{2}(\d{2})?)?";
            #[expect(
                clippy::expect_used,
                reason = "a hardcoded regex literal should always compile; covered by \
                          the temporal planner tests"
            )]
            let shapes = (
                regex::Regex::new(&format!("^{date}([Tt ]{time}{offset})?$"))
                    .expect("hardcoded ISO-8601 date shape regex should be valid"),
                regex::Regex::new(&format!("^[Tt]?{time}{offset}$"))
                    .expect("hardcoded ISO-8601 time shape regex should be valid"),
            );
            shapes
        });
    let (date, time) = &*ISO_TEMPORAL;
    if date.is_match(s) {
        Some(TemporalKinds::Date)
    } else if time.is_match(s) {
        Some(TemporalKinds::Time)
    } else {
        None
    }
}

/// The whole date/time values a temporal key reads (RM `data_types` master07
/// §Date/Time package).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TemporalKinds {
    /// A `DV_DATE` or a `DV_DATE_TIME`, which a date or date-time compares with.
    Date,
    /// A `DV_TIME`, which a time compares with.
    Time,
    /// All three, the ORDER BY key.
    Any,
}

impl TemporalKinds {
    /// The `_type` of each value the kinds read.
    fn types(self) -> &'static [&'static str] {
        match self {
            Self::Date => &["DV_DATE", "DV_DATE_TIME"],
            Self::Time => &["DV_TIME"],
            Self::Any => &DATE_TIME_TYPES,
        }
    }
}

/// The whole date/time data values (RM `data_types` master07 §Date/Time package).
const DATE_TIME_TYPES: [&str; 3] = ["DV_DATE", "DV_TIME", "DV_DATE_TIME"];

/// The whole `DV_ORDERED` values whose RM magnitude or value is a number and
/// which no AQL literal types as a date/time (RM `data_types` master06 §Quantity
/// package, `DV_QUANTIFIED.magnitude`, `DV_ORDINAL.value`, `DV_SCALE.value`).
const NUMERIC_TYPES: [&str; 6] = [
    "DV_QUANTITY",
    "DV_COUNT",
    "DV_PROPORTION",
    "DV_ORDINAL",
    "DV_SCALE",
    "DV_DURATION",
];

/// The SQL list of `types`, each a quoted literal of the model's own names.
fn type_list(types: &[&str]) -> String {
    types
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A mixed-type leaf's number: a JSON number as itself, a whole numeric
/// `DV_ORDERED` ([`NUMERIC_TYPES`]) by its magnitude through
/// `ext.openehr_magnitude`, and NULL for anything else, so a number never
/// meets a cast error or a lexical comparison (QUERY master03 §Comparison
/// operators).
pub(super) fn raw_numeric(base: Expr) -> Expr {
    Expr::cust_with_exprs(
        format!(
            "CASE WHEN jsonb_typeof($1) = 'number' THEN ($2 #>> '{{}}'::text[])::numeric \
             WHEN ($3 ->> '_type') IN ({}) THEN openehr_magnitude($4) END",
            type_list(&NUMERIC_TYPES)
        ),
        [base.clone(), base.clone(), base.clone(), base],
    )
}

/// The whole data values `SUM` and `AVG` add: the `DV_AMOUNT` subtypes, for
/// which the RM defines `+` "unlike descendants of `DV_ABSOLUTE_QUANTITY`,
/// such as the date/time types" (RM `data_types` §`DV_AMOUNT`).
const AMOUNT_TYPES: [&str; 4] = ["DV_QUANTITY", "DV_COUNT", "DV_PROPORTION", "DV_DURATION"];

/// A mixed-type leaf's amount: a JSON number as itself, a whole `DV_AMOUNT`
/// ([`AMOUNT_TYPES`]) by its magnitude, a JSON `null` as no value, and any
/// other value failing the statement with SQLSTATE `ZA003` through
/// `ext.aql_not_an_amount`, so `SUM` and `AVG` never add a date or an ordinal
/// (QUERY master03 §SUM, §AVG: "Input values type should be either Integer or
/// Real").
pub(super) fn raw_amount(base: Expr) -> Expr {
    Expr::cust_with_exprs(
        format!(
            "CASE WHEN jsonb_typeof($1) = 'number' THEN ($2 #>> '{{}}'::text[])::numeric \
             WHEN jsonb_typeof($3) = 'null' THEN NULL \
             WHEN ($4 ->> '_type') IN ({}) THEN openehr_magnitude($5) \
             ELSE aql_not_an_amount(coalesce($6 ->> '_type', jsonb_typeof($7))) END",
            type_list(&AMOUNT_TYPES)
        ),
        [
            base.clone(),
            base.clone(),
            base.clone(),
            base.clone(),
            base.clone(),
            base.clone(),
            base,
        ],
    )
}

/// A mixed-type leaf's text when it is a JSON string, number or Boolean, and
/// NULL for a whole object or a list, whose serialized JSON is no value.
pub(super) fn raw_text(base: Expr) -> Expr {
    Expr::cust_with_exprs(
        "CASE WHEN jsonb_typeof($1) IN ('string', 'number', 'boolean') \
         THEN $2 #>> '{}'::text[] END",
        [base.clone(), base],
    )
}

/// The instant a mixed-type leaf names when it is a whole date/time value of
/// `kinds`, read through the total `ext.openehr_timestamp` the temporal
/// coercion uses; NULL otherwise.
pub(super) fn raw_temporal(base: Expr, kinds: TemporalKinds) -> Expr {
    Expr::cust_with_exprs(
        format!(
            "CASE WHEN ($1 ->> '_type') IN ({}) THEN openehr_timestamp($2 ->> 'value') END",
            type_list(kinds.types())
        ),
        [base.clone(), base],
    )
}

/// The keys an ORDER BY on a mixed-type leaf sorts by, most significant first:
/// the instant of a whole date/time value, then the number of a JSON number or
/// a whole numeric value, then the text of a JSON string or Boolean. A key no
/// candidate type can carry is left out. Keys of different kinds never meet,
/// and the order between kinds is our own design (QUERY master03 §ORDER BY
/// orders comparable data only).
pub(super) fn raw_order_modes(types: &[String]) -> Vec<ValueMode> {
    let any = |set: &[&str]| types.iter().any(|t| set.contains(&t.as_str()));
    let mut modes = Vec::with_capacity(3);
    if any(&DATE_TIME_TYPES) {
        modes.push(ValueMode::RawTemporal(TemporalKinds::Any));
    }
    if any(&NUMERIC_TYPES) || any(&["Integer", "Integer64", "Real", "Double"]) {
        modes.push(ValueMode::RawNumeric);
    }
    modes.push(ValueMode::RawText);
    modes
}

/// The candidate types of the leaf a path target reads, empty for a VERSION
/// or EHR field.
pub(super) fn target_types(target: &PathTarget) -> &[String] {
    match target {
        PathTarget::Data(leaf)
        | PathTarget::EhrStatus(leaf)
        | PathTarget::Attestations { leaf, .. } => leaf.types.names(),
        PathTarget::Version { .. } | PathTarget::Ehr { .. } => &[],
    }
}

/// A raw key of `mode` over a mixed-type leaf's jsonb `base`; `base` itself
/// for a mode that is no raw key.
pub(super) fn raw_key(base: Expr, mode: ValueMode) -> Expr {
    match mode {
        ValueMode::RawNumeric => raw_numeric(base),
        ValueMode::RawText => raw_text(base),
        ValueMode::RawAmount => raw_amount(base),
        ValueMode::RawTemporal(kinds) => raw_temporal(base, kinds),
        ValueMode::Projection | ValueMode::Value(_) => base,
    }
}

/// How a mixed-type leaf compares with the bound value opposite it.
pub(super) enum RawPartner {
    /// A number, compared with [`raw_numeric`].
    Number(Expr),
    /// A String or a Boolean as text, compared with [`raw_text`], and, for an
    /// ISO 8601 date/time where the leaf can be a whole date/time value, its
    /// instant compared with [`raw_temporal`] of its kinds.
    Text {
        /// The value as text.
        text: Expr,
        /// The kinds it compares with and its instant.
        temporal: Option<(TemporalKinds, Expr)>,
    },
}

impl RawPartner {
    /// The partner a bound value makes for a leaf of `types`.
    pub(super) fn of(value: sea_query::Value, types: &[String]) -> Self {
        if is_number(&value) {
            return Self::Number(numeric_cast(value));
        }
        let temporal = match &value {
            sea_query::Value::String(Some(s))
                if types.iter().any(|t| DATE_TIME_TYPES.contains(&t.as_str())) =>
            {
                temporal_kinds(s).map(|kinds| {
                    let instant = call(
                        "openehr_timestamp",
                        vec![cast(Expr::val(s.clone()), "text")],
                    );
                    (kinds, instant)
                })
            }
            _ => None,
        };
        Self::Text {
            text: cast(Expr::val(value), "text"),
            temporal,
        }
    }

    /// The leaf keys the comparison reads: the first always, the second for a
    /// date/time partner.
    pub(super) fn modes(&self) -> (ValueMode, Option<ValueMode>) {
        match self {
            Self::Number(_) => (ValueMode::RawNumeric, None),
            Self::Text { temporal, .. } => (
                ValueMode::RawText,
                temporal
                    .as_ref()
                    .map(|(kinds, _)| ValueMode::RawTemporal(*kinds)),
            ),
        }
    }

    /// The comparison `key op partner` (or `partner op key` when the leaf is
    /// on the right), over the keys [`Self::modes`] names. The text and the
    /// temporal key are never both non-NULL, so their comparisons combine by
    /// `COALESCE`, which keeps each one's three-valued answer.
    pub(super) fn compare(
        self,
        op: BinOper,
        leaf_left: bool,
        first: Expr,
        second: Option<Expr>,
    ) -> Expr {
        let side = |key: Expr, value: Expr| {
            if leaf_left {
                key.binary(op, value)
            } else {
                value.binary(op, key)
            }
        };
        match self {
            Self::Number(n) => side(first, n),
            Self::Text { text, temporal } => {
                let by_text = side(first, text);
                match (temporal, second) {
                    (Some((_, instant)), Some(key)) => {
                        call("COALESCE", vec![by_text, side(key, instant)])
                    }
                    _ => by_text,
                }
            }
        }
    }
}
