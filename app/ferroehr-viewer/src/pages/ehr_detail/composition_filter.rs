// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The compositions tab's row filters, lowered to ONE parameterized AQL query.
//!
//! Operator input NEVER becomes AQL text. The statement is assembled from
//! compile-time fragments picked by which filters are FILLED, and every value
//! the operator typed travels as an AQL `query_parameters` binding
//! (`docs/specs/openehr/ITS-REST/specifications/docs/query/Request.md`
//! §`query_parameters`), so a value carrying a quote, a comment marker or a
//! whole second statement is matched as data and can never be parsed as query
//! text.
//!
//! Text filters are substring matches through AQL `LIKE`
//! (`docs/specs/openehr/QUERY/docs/AQL/master03-syntax.adoc` §`LIKE` — `?`
//! matches one character, `*` any sequence, and a literal one "must be escaped
//! by using the backslash `\` character"); the same section's own example is
//! `c/archetype_details/template_id/value LIKE '*encounter*'`.
//!
//! Everything here is a pure function with unit tests. The module is
//! unconditional so the filter values can be the tab's resource source on both
//! targets; only the server function calls [`composition_query`], so the
//! statement fragments link out of the WASM bundle.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use serde_json::{Map, Value};

use crate::error::ViewerError;

/// The projection, source and EHR binding every compositions listing shares.
///
/// The `ehr_id` is the one parameter that is always bound; the filter
/// predicates below append to this same `WHERE`.
const SELECT_FROM: &str = "SELECT c/uid/value, c/name/value, \
c/archetype_details/template_id/value, c/context/start_time/value, \
c/composer/name \
FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = $ehr_id";

/// The template-id predicate (`ARCHETYPED._template_id_` on the composition's
/// `archetype_details`).
const TEMPLATE_PREDICATE: &str = " AND c/archetype_details/template_id/value LIKE $template";

/// The inclusive lower bound on `EVENT_CONTEXT._start_time_`.
const FROM_PREDICATE: &str = " AND c/context/start_time/value >= $from";

/// The inclusive upper bound on `EVENT_CONTEXT._start_time_`.
const TO_PREDICATE: &str = " AND c/context/start_time/value <= $to";

/// The composer predicate (`PARTY_IDENTIFIED._name_` on `COMPOSITION.composer`).
const COMPOSER_PREDICATE: &str = " AND c/composer/name LIKE $composer";

/// Newest first, on the same instant the date filter bounds.
const ORDER_BY: &str = " ORDER BY c/context/start_time/value DESC";

/// The instant a date-only lower bound means: the start of that UTC day.
const DAY_START: &str = "T00:00:00Z";

/// The instant a date-only upper bound means: the last microsecond of that UTC
/// day, so a bound of `2026-07-14` INCLUDES `2026-07-14T14:45:00Z`.
///
/// Microseconds rather than nanoseconds because that is PostgreSQL's
/// `timestamp with time zone` resolution
/// (<https://www.postgresql.org/docs/18/datatype-datetime.html>), which the
/// CDR's temporal comparison ends up in.
const DAY_END: &str = "T23:59:59.999999Z";

/// The compositions tab's row filters, exactly as the URL carries them.
///
/// Every field is already trimmed by [`CompositionFilter::new`]; an empty field
/// means "no bound on this dimension", which is what makes the unfiltered tab
/// issue the same query it always did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompositionFilter {
    /// Substring of the composition's template id (`?template=`).
    pub template: String,
    /// Inclusive lower bound on the composition's context start time
    /// (`?from=`), as a date or an instant.
    pub from: String,
    /// Inclusive upper bound on the composition's context start time
    /// (`?to=`), as a date or an instant.
    pub to: String,
    /// Substring of the composer's name (`?composer=`).
    pub composer: String,
}

impl CompositionFilter {
    /// Build a filter from the four raw URL values, trimming each.
    #[must_use]
    pub fn new(template: &str, from: &str, to: &str, composer: &str) -> Self {
        Self {
            template: template.trim().to_owned(),
            from: from.trim().to_owned(),
            to: to.trim().to_owned(),
            composer: composer.trim().to_owned(),
        }
    }

    /// Whether no filter is set at all — the tab's default state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.template.is_empty()
            && self.from.is_empty()
            && self.to.is_empty()
            && self.composer.is_empty()
    }
}

/// One `POST /query/aql` request's two halves, built together so they cannot
/// drift apart.
///
/// They must not: the CDR answers `400 unbound query parameter` when the text
/// names a parameter the bindings omit (verified against the composed stack,
/// 2026-08-23).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionQuery {
    /// The AQL statement — compile-time fragments only.
    pub aql: String,
    /// The `query_parameters` object every operator-supplied value travels in.
    pub parameters: Value,
}

/// Lower `filter` onto the compositions listing query for `ehr_id`.
///
/// An empty filter yields the plain newest-first listing; each filled field
/// appends its own predicate AND binds its own parameter.
///
/// # Errors
/// [`ViewerError::Invalid`] when a date bound is neither a date nor an
/// instant — the viewer refuses it here rather than sending a value the CDR's
/// temporal comparison cannot read.
pub fn composition_query(
    ehr_id: &str,
    filter: &CompositionFilter,
) -> Result<CompositionQuery, ViewerError> {
    let mut aql = String::from(SELECT_FROM);
    let mut parameters = Map::new();
    drop(parameters.insert("ehr_id".to_owned(), Value::String(ehr_id.to_owned())));

    if !filter.template.is_empty() {
        aql.push_str(TEMPLATE_PREDICATE);
        drop(parameters.insert(
            "template".to_owned(),
            Value::String(contains_pattern(&filter.template)),
        ));
    }
    if !filter.from.is_empty() {
        aql.push_str(FROM_PREDICATE);
        drop(parameters.insert(
            "from".to_owned(),
            Value::String(instant_bound(&filter.from, DAY_START, "The “from” date")?),
        ));
    }
    if !filter.to.is_empty() {
        aql.push_str(TO_PREDICATE);
        drop(parameters.insert(
            "to".to_owned(),
            Value::String(instant_bound(&filter.to, DAY_END, "The “to” date")?),
        ));
    }
    if !filter.composer.is_empty() {
        aql.push_str(COMPOSER_PREDICATE);
        drop(parameters.insert(
            "composer".to_owned(),
            Value::String(contains_pattern(&filter.composer)),
        ));
    }
    aql.push_str(ORDER_BY);

    Ok(CompositionQuery {
        aql,
        parameters: Value::Object(parameters),
    })
}

/// Wrap an operator-typed value in an AQL `LIKE` "contains anywhere" pattern.
///
/// `*` and `?` are AQL's wildcards, so the operator's own `*`, `?` and `\` are
/// backslash-escaped and match literally — the filter box is a substring
/// search, not a pattern language the operator has to know.
fn contains_pattern(value: &str) -> String {
    let mut pattern = String::with_capacity(value.len().saturating_add(2));
    pattern.push('*');
    for character in value.chars() {
        if matches!(character, '*' | '?' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('*');
    pattern
}

/// Complete a date bound to the instant it means, or pass an instant through.
///
/// `<input type="date">` submits `YYYY-MM-DD`, which the CDR's temporal
/// comparison reads as that day's midnight — so an upper bound left uncompleted
/// would silently exclude everything recorded during the bounding day
/// (reproduced against the composed stack, 2026-08-23). `day_suffix` is the end
/// of the range the date names.
///
/// # Errors
/// [`ViewerError::Invalid`] naming `label` when the value is neither.
fn instant_bound(value: &str, day_suffix: &str, label: &str) -> Result<String, ViewerError> {
    let refuse = || {
        ViewerError::Invalid(format!(
            "{label} must be a date (2026-07-14) or an instant (2026-07-14T08:00:00Z) — \
             {value:?} is neither"
        ))
    };
    // A date carries no time designator; anything else must name a time, so the
    // branch is decided by the value's SHAPE rather than by parser leniency.
    if !value.contains(['T', 't']) {
        let date: jiff::civil::Date = value.parse().map_err(|_ignored| refuse())?;
        return Ok(format!("{date}{day_suffix}"));
    }
    if value.parse::<jiff::Timestamp>().is_ok() {
        return Ok(value.to_owned());
    }
    // A `datetime-local`-shaped value carries no offset; the filter's stated
    // vocabulary is UTC, so that is the offset it gains.
    let civil: jiff::civil::DateTime = value.parse().map_err(|_ignored| refuse())?;
    Ok(format!("{civil}Z"))
}

#[cfg(test)]
mod tests {
    use super::{
        CompositionFilter, CompositionQuery, composition_query, contains_pattern, instant_bound,
    };
    use crate::error::ViewerError;
    use std::collections::BTreeSet;

    /// The EHR every fixture query is built for.
    const EHR: &str = "7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d";

    /// Every `$parameter` the statement names, so a test can prove the text and
    /// the bindings agree (the CDR answers `400` on an unbound one).
    fn referenced_parameters(aql: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for tail in aql.split('$').skip(1) {
            let name: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            names.insert(name);
        }
        names
    }

    /// Every parameter the request binds.
    fn bound_parameters(query: &CompositionQuery) -> BTreeSet<String> {
        query
            .parameters
            .as_object()
            .expect("the bindings are a JSON object")
            .keys()
            .cloned()
            .collect()
    }

    /// The statement parses as AQL and binds exactly the parameters it names.
    fn assert_coherent(query: &CompositionQuery) {
        openehr_query::parser::parse_str(&query.aql)
            .unwrap_or_else(|e| panic!("the generated AQL must parse: {e}\n{}", query.aql));
        assert_eq!(
            referenced_parameters(&query.aql),
            bound_parameters(query),
            "every named parameter must be bound and vice versa: {}",
            query.aql
        );
    }

    #[test]
    fn an_empty_filter_is_the_plain_newest_first_listing() {
        let query = composition_query(EHR, &CompositionFilter::default()).expect("query");
        assert_coherent(&query);
        assert!(!query.aql.contains("LIKE"), "{}", query.aql);
        assert!(!query.aql.contains(">="), "{}", query.aql);
        assert_eq!(
            bound_parameters(&query),
            BTreeSet::from(["ehr_id".to_owned()])
        );
        assert_eq!(query.parameters["ehr_id"], EHR);
        assert!(
            query
                .aql
                .ends_with("ORDER BY c/context/start_time/value DESC")
        );
    }

    #[test]
    fn whitespace_only_values_are_no_filter_at_all() {
        let filter = CompositionFilter::new("  ", "\t", " \n ", "   ");
        assert!(filter.is_empty());
        let query = composition_query(EHR, &filter).expect("query");
        assert_eq!(
            query,
            composition_query(EHR, &CompositionFilter::default()).expect("query")
        );
    }

    #[test]
    fn each_filter_adds_its_own_predicate_and_binding() {
        let template = composition_query(EHR, &CompositionFilter::new("evaluation", "", "", ""))
            .expect("query");
        assert_coherent(&template);
        assert!(
            template
                .aql
                .contains("c/archetype_details/template_id/value LIKE $template")
        );
        assert_eq!(template.parameters["template"], "*evaluation*");

        let composer =
            composition_query(EHR, &CompositionFilter::new("", "", "", "Ash")).expect("query");
        assert_coherent(&composer);
        assert!(composer.aql.contains("c/composer/name LIKE $composer"));
        assert_eq!(composer.parameters["composer"], "*Ash*");

        let window = composition_query(
            EHR,
            &CompositionFilter::new("", "2026-03-01", "2026-06-30", ""),
        )
        .expect("query");
        assert_coherent(&window);
        assert!(window.aql.contains("c/context/start_time/value >= $from"));
        assert!(window.aql.contains("c/context/start_time/value <= $to"));
        assert_eq!(window.parameters["from"], "2026-03-01T00:00:00Z");
        assert_eq!(window.parameters["to"], "2026-06-30T23:59:59.999999Z");
    }

    #[test]
    fn all_four_filters_compose_into_one_statement() {
        let query = composition_query(
            EHR,
            &CompositionFilter::new("evaluation", "2026-01-01", "2026-12-31", "Ash"),
        )
        .expect("query");
        assert_coherent(&query);
        assert_eq!(
            bound_parameters(&query),
            BTreeSet::from([
                "ehr_id".to_owned(),
                "template".to_owned(),
                "from".to_owned(),
                "to".to_owned(),
                "composer".to_owned(),
            ])
        );
    }

    #[test]
    fn operator_input_never_reaches_the_statement_text() {
        // The property that makes injection unrepresentable: the statement text
        // is a function of WHICH filters are filled and of nothing else, so two
        // filters differing only in their VALUES produce byte-identical AQL and
        // differ only in the bindings.
        let benign_template =
            composition_query(EHR, &CompositionFilter::new("x", "", "", "")).expect("query");
        let benign_composer =
            composition_query(EHR, &CompositionFilter::new("", "", "", "x")).expect("query");
        for hostile in [
            "' OR 1=1 --",
            "x' AND c/uid/value = '",
            "\"; DROP TABLE node; --",
            "$ehr_id",
            "* OR NOT EXISTS c/uid",
        ] {
            let by_template = composition_query(EHR, &CompositionFilter::new(hostile, "", "", ""))
                .expect("query");
            assert_coherent(&by_template);
            assert_eq!(by_template.aql, benign_template.aql, "hostile: {hostile}");
            assert_eq!(
                by_template.parameters["template"],
                contains_pattern(hostile)
            );

            let by_composer = composition_query(EHR, &CompositionFilter::new("", "", "", hostile))
                .expect("query");
            assert_coherent(&by_composer);
            assert_eq!(by_composer.aql, benign_composer.aql, "hostile: {hostile}");
            assert_eq!(
                by_composer.parameters["composer"],
                contains_pattern(hostile)
            );
        }
        // The EHR id is a bound value too — never interpolated.
        let query = composition_query("' OR 1=1 --", &CompositionFilter::default()).expect("query");
        assert_eq!(
            query.aql,
            composition_query(EHR, &CompositionFilter::default())
                .expect("query")
                .aql
        );
        assert_eq!(query.parameters["ehr_id"], "' OR 1=1 --");
    }

    #[test]
    fn a_wildcard_the_operator_typed_matches_literally() {
        // AQL LIKE reads `*` and `?` as wildcards, so an operator's own must be
        // escaped or the box would silently become a pattern language.
        assert_eq!(contains_pattern("a*b"), "*a\\*b*");
        assert_eq!(contains_pattern("a?b"), "*a\\?b*");
        assert_eq!(contains_pattern("a\\b"), "*a\\\\b*");
        assert_eq!(contains_pattern("plain"), "*plain*");
        assert_eq!(contains_pattern(""), "**");
    }

    #[test]
    fn a_date_bound_covers_the_whole_day_and_an_instant_passes_through() {
        assert_eq!(
            instant_bound("2026-07-14", super::DAY_START, "From").expect("date"),
            "2026-07-14T00:00:00Z"
        );
        assert_eq!(
            instant_bound("2026-07-14", super::DAY_END, "To").expect("date"),
            "2026-07-14T23:59:59.999999Z"
        );
        assert_eq!(
            instant_bound("2026-07-14T08:00:00Z", super::DAY_END, "To").expect("instant"),
            "2026-07-14T08:00:00Z"
        );
        // A `datetime-local` value carries no offset and is read as UTC.
        assert_eq!(
            instant_bound("2026-07-14T08:00", super::DAY_END, "To").expect("civil datetime"),
            "2026-07-14T08:00:00Z"
        );
    }

    #[test]
    fn an_unreadable_date_bound_is_refused_before_the_round_trip() {
        // The CDR answers 500 for a bound its temporal comparison cannot cast
        // (observed 2026-08-23, tracked upstream of this screen); the viewer
        // never sends one.
        for bad in ["not-a-date", "2026-13-40", "yesterday", "2026-07-14T99:99"] {
            let error = composition_query(EHR, &CompositionFilter::new("", bad, "", ""))
                .expect_err("an unreadable bound is refused");
            assert!(
                matches!(error, ViewerError::Invalid(ref message) if message.contains("“from”")),
                "{error:?}"
            );
        }
        let error = composition_query(EHR, &CompositionFilter::new("", "", "nope", ""))
            .expect_err("an unreadable bound is refused");
        assert!(
            matches!(error, ViewerError::Invalid(ref message) if message.contains("“to”")),
            "{error:?}"
        );
    }
}
