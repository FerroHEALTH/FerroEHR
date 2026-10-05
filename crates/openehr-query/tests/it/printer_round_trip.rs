// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The printer's own documented invariant, asserted over every boolean shape:
//! for any AST the parser produces, `parse(to_aql(ast)) == ast`.
//!
//! The shapes here were found by the `aql_query` fuzz harness (`fuzz/`), which
//! parses arbitrary text and asserts the same invariant. Two defects surfaced,
//! both about parentheses the printer dropped:
//!
//! - `AqlParser.g4` states `AND`/`OR` as binary alternatives of one recursive
//!   rule, which ANTLR4 resolves LEFT-associatively, so a same-precedence right
//!   operand must keep its parentheses to survive a re-parse;
//! - `containsExpr: classExprOperand (NOT? CONTAINS containsExpr)?` makes the
//!   `CONTAINS` operand a whole `containsExpr`, so an unparenthesised
//!   `A CONTAINS B` used as a boolean operand absorbs the operator that follows
//!   it — which moves the operator INTO the CONTAINS scope and changes what the
//!   query means.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "integration-test assertions and fixture plumbing"
)]

/// Every AQL source below must satisfy `parse(to_aql(parse(src))) == parse(src)`.
fn assert_round_trips(src: &str) {
    let parsed = openehr_query::parser::parse_str(src)
        .unwrap_or_else(|e| panic!("fixture must parse: {src}\n  {e}"));
    let printed = openehr_query::printer::to_aql(&parsed);
    let reparsed = openehr_query::parser::parse_str(&printed)
        .unwrap_or_else(|e| panic!("printed AQL must reparse: {printed}\n  {e}"));
    assert_eq!(
        reparsed, parsed,
        "printer round-trip drifted the AST\n  source:  {src}\n  printed: {printed}"
    );
}

/// The WHERE tree: both associativity sides of both operators, plus `NOT`.
#[test]
fn where_boolean_shapes_round_trip() {
    const HEAD: &str = "SELECT e/ehr_id/value FROM EHR e WHERE ";
    for tail in [
        // Left-nested — the parser's own default shape, parenthesis-free.
        "(e/a = 1 AND e/b = 1) AND e/c = 1",
        "(e/a = 1 OR e/b = 1) OR e/c = 1",
        // Right-nested at the same precedence — the associativity finding.
        "e/a = 1 AND (e/b = 1 AND e/c = 1)",
        "e/a = 1 OR (e/b = 1 OR e/c = 1)",
        // Both sides right-nested, the shape the fuzzer actually reported.
        "(e/a = 1 AND e/b = 1) AND (e/c = 1 AND e/d = 1)",
        // Precedence: OR below AND, in every position.
        "(e/a = 1 OR e/b = 1) AND e/c = 1",
        "e/a = 1 AND (e/b = 1 OR e/c = 1)",
        "e/a = 1 OR e/b = 1 AND e/c = 1",
        "e/a = 1 AND e/b = 1 OR e/c = 1",
        // NOT binds tighter than AND, so a boolean operand needs parens.
        "NOT (e/a = 1 AND e/b = 1)",
        "NOT (e/a = 1 OR e/b = 1)",
        "NOT e/a = 1 AND e/b = 1",
        "NOT NOT e/a = 1",
    ] {
        assert_round_trips(&format!("{HEAD}{tail}"));
    }
}

/// The FROM tree: the same associativity rule, plus the greedy `CONTAINS`
/// operand.
#[test]
fn contains_boolean_shapes_round_trip() {
    const HEAD: &str = "SELECT c/uid/value FROM ";
    for tail in [
        "(COMPOSITION c AND OBSERVATION o) AND CLUSTER l",
        "COMPOSITION c AND (OBSERVATION o AND CLUSTER l)",
        "COMPOSITION c OR (OBSERVATION o OR CLUSTER l)",
        "(COMPOSITION c OR OBSERVATION o) AND CLUSTER l",
        "COMPOSITION c AND (OBSERVATION o OR CLUSTER l)",
        // The greedy-CONTAINS finding: the parenthesised form keeps the `AND`
        // OUTSIDE the CONTAINS scope, and printing must not move it in.
        "(EHR e CONTAINS COMPOSITION c) AND OBSERVATION o",
        "(EHR e CONTAINS COMPOSITION c) OR OBSERVATION o",
        "OBSERVATION o AND (EHR e CONTAINS COMPOSITION c)",
        // Its unparenthesised twin, where the operator belongs INSIDE.
        "EHR e CONTAINS COMPOSITION c AND OBSERVATION o",
        "EHR e CONTAINS (COMPOSITION c AND OBSERVATION o)",
        "EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o",
    ] {
        assert_round_trips(&format!("{HEAD}{tail}"));
    }
}

/// The two shapes are genuinely DIFFERENT queries, so the printer moving an
/// operator across a `CONTAINS` boundary would be a silent wrong answer, not a
/// cosmetic drift.
#[test]
fn a_parenthesised_contains_is_not_its_unparenthesised_twin() {
    let grouped = openehr_query::parser::parse_str(
        "SELECT c/uid/value FROM (EHR e CONTAINS COMPOSITION c) AND OBSERVATION o",
    )
    .unwrap();
    let greedy = openehr_query::parser::parse_str(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c AND OBSERVATION o",
    )
    .unwrap();
    assert_ne!(
        grouped, greedy,
        "the CONTAINS operand is greedy, so these two sources must not parse alike"
    );
}

/// The nightly fuzz artifact (#2746): this exact input exposed the
/// `Recursive::declare` Rc cycle that leaked one parser graph per
/// `parse_str` call. The leak property itself is guarded by the nightly
/// `LeakSanitizer` lane over the tracked artifact
/// (`fuzz/regressions/aql_query/leak_2746_simple_select.aql`); this pins the
/// behavioural half — the restructured recursion parses and round-trips the
/// same query.
#[test]
fn the_leak_artifact_query_still_parses_and_round_trips() {
    assert_round_trips("SELECT e/ehr_id/value FROM EHR e\n");
}

/// The second #2746 fuzz artifact: the parser DECODES string escapes into the
/// AST, so a printer site emitting the decoded value verbatim re-decodes on
/// reparse and drifts the AST. Every string-literal emission site now
/// re-escapes; these cover the four sites (TERMINOLOGY arguments, LIKE,
/// node-name constraints, primitive strings) with the characters that drift:
/// backslashes, quotes, and the decoded control escapes.
#[test]
fn string_literals_with_escapes_round_trip_at_every_emission_site() {
    for src in [
        // TERMINOLOGY arguments (the artifact's shape).
        r"SELECT TERMINOLOGY('e\\pand', 'open\'ehr', 'a\tb') FROM EHR e",
        // LIKE operand.
        r"SELECT c/uid/value FROM COMPOSITION c WHERE c/name/value LIKE 'O\'Neil\\%'",
        // A primitive string comparison.
        r"SELECT c/uid/value FROM COMPOSITION c WHERE c/name/value = 'a\\b\'c'",
        // A node-name constraint inside a path predicate.
        r"SELECT c/uid/value FROM COMPOSITION c CONTAINS OBSERVATION o[at0001, 'we\'ird\\name']",
    ] {
        assert_round_trips(src);
    }
}

/// The single function call in `src`'s SELECT column.
fn select_call(src: &str) -> openehr_query::ast::FunctionCall {
    let parsed = openehr_query::parser::parse_str(src)
        .unwrap_or_else(|e| panic!("fixture must parse: {src}\n  {e}"));
    match parsed
        .select
        .columns
        .into_iter()
        .next()
        .map(|expr| expr.column)
    {
        Some(openehr_query::ast::ColumnExpr::Function(call)) => call,
        other => panic!("expected one function column in {src}, got {other:?}"),
    }
}

/// Every built-in function QUERY master03 §Functions lists (the string,
/// numeric and date-time groups) parses to its typed variant in upper, lower
/// and mixed case, and the printer writes the name back as spelled.
#[test]
fn every_builtin_function_is_classified_and_round_trips_its_spelling() {
    use openehr_query::ast::{
        BuiltinFunction, DateTimeFunction, FunctionCall, NumericFunction, StringFunction,
    };
    let every = StringFunction::ALL
        .iter()
        .map(|f| BuiltinFunction::String(*f))
        .chain(
            NumericFunction::ALL
                .iter()
                .map(|f| BuiltinFunction::Numeric(*f)),
        )
        .chain(
            DateTimeFunction::ALL
                .iter()
                .map(|f| BuiltinFunction::DateTime(*f)),
        );
    let mut seen = Vec::new();
    for function in every {
        let canonical = function.as_str();
        let mixed: String = canonical
            .chars()
            .enumerate()
            .map(|(i, c)| {
                if i % 2 == 0 {
                    c
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect();
        for spelled in [canonical.to_owned(), canonical.to_ascii_lowercase(), mixed] {
            let src = format!("SELECT {spelled}(e/ehr_id/value) FROM EHR e");
            match select_call(&src) {
                FunctionCall::Builtin {
                    function: parsed,
                    name,
                    ..
                } => {
                    assert_eq!(parsed, function, "{src}");
                    assert_eq!(name, spelled, "{src}");
                }
                other => panic!("{src} must classify as {function:?}, got {other:?}"),
            }
            let printed =
                openehr_query::printer::to_aql(&openehr_query::parser::parse_str(&src).unwrap());
            assert!(printed.contains(&format!("{spelled}(")), "{printed}");
            assert_round_trips(&src);
        }
        seen.push(canonical);
    }
    // The spec's tables, verbatim (master03 §String/Numeric/Date and time
    // functions): a function added to or dropped from a group fails here.
    assert_eq!(
        seen,
        [
            "LENGTH",
            "CONTAINS",
            "POSITION",
            "SUBSTRING",
            "CONCAT",
            "CONCAT_WS",
            "ABS",
            "MOD",
            "CEIL",
            "FLOOR",
            "ROUND",
            "CURRENT_DATE",
            "CURRENT_TIME",
            "CURRENT_DATE_TIME",
            "NOW",
            "CURRENT_TIMEZONE",
        ]
    );
}

/// A name outside AQL's built-in functions, a product aggregate among them,
/// parses to `FunctionCall::Other` with its spelling, and round-trips.
#[test]
fn a_name_outside_aql_is_other_and_round_trips() {
    use openehr_query::ast::FunctionCall;
    for spelled in ["MEDIAN", "median", "my_fn", "LENGTHS", "now2"] {
        let src = format!("SELECT {spelled}(e/ehr_id/value, 1) FROM EHR e");
        match select_call(&src) {
            FunctionCall::Other { name, args } => {
                assert_eq!(name, spelled, "{src}");
                assert_eq!(args.len(), 2, "{src}");
            }
            other => panic!("{src} must be Other, got {other:?}"),
        }
        assert_round_trips(&src);
    }
}

/// The string function `CONTAINS` shares its keyword with containment; in
/// WHERE position it is classified too, and both uses survive a round trip.
#[test]
fn the_contains_function_beside_containment_round_trips() {
    assert_round_trips(
        "SELECT c FROM EHR e Contains COMPOSITION c WHERE contains(c/name/value, 'x') = true",
    );
}
