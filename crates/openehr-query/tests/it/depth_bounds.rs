// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The nesting bounds hold at their limit and refuse one level beyond it.
//!
//! A `NOT`, `AND` or `OR` chain builds a tree as deep as it is long, and the
//! passes that walk the tree recurse once per level, so an unbounded chain
//! exhausted the stack and aborted the server (#4284). The bounds are checked
//! over the tokens, before the grammar builds anything.

#![allow(
    clippy::panic,
    reason = "integration-test assertions outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_query::lexer::Token;
use openehr_query::parser::{MAX_EXPRESSION_DEPTH, MAX_NESTING, Nesting, ParseError, parse_str};

const FROM: &str = "SELECT c FROM COMPOSITION c WHERE ";

fn not_chain(n: usize) -> String {
    format!("{FROM}{}c/name/value = 'x'", "NOT ".repeat(n))
}

fn or_chain(operators: usize) -> String {
    let terms: Vec<String> = (0..=operators)
        .map(|i| format!("c/name/value = 'x{i}'"))
        .collect();
    format!("{FROM}{}", terms.join(" OR "))
}

fn parens(n: usize) -> String {
    format!("{FROM}{}c/name/value = 'x'{}", "(".repeat(n), ")".repeat(n))
}

fn contains_chain(n: usize) -> String {
    let links: Vec<String> = (0..n).map(|i| format!("SECTION s{i}")).collect();
    let mut src = vec!["SELECT e FROM EHR e".to_owned()];
    src.extend(links);
    src.join(" CONTAINS ")
}

/// The measure and limit of a refusal, failing the test on any other outcome.
fn refusal(src: &str) -> (Nesting, usize, Option<Token>) {
    match parse_str(src) {
        Err(ParseError::TooDeep { measure, limit, at }) => (measure, limit, at.found),
        other => panic!("expected a depth refusal, got {other:?}"),
    }
}

#[test]
fn a_not_chain_at_the_expression_bound_parses_and_one_more_is_refused() {
    parse_str(&not_chain(MAX_EXPRESSION_DEPTH)).expect("a chain at the bound parses");
    assert_eq!(
        refusal(&not_chain(MAX_EXPRESSION_DEPTH + 1)),
        (Nesting::Expression, MAX_EXPRESSION_DEPTH, Some(Token::Not))
    );
}

#[test]
fn an_or_chain_at_the_expression_bound_parses_and_one_more_is_refused() {
    parse_str(&or_chain(MAX_EXPRESSION_DEPTH)).expect("a chain at the bound parses");
    assert_eq!(
        refusal(&or_chain(MAX_EXPRESSION_DEPTH + 1)),
        (Nesting::Expression, MAX_EXPRESSION_DEPTH, Some(Token::Or))
    );
}

#[test]
fn brackets_at_the_nesting_bound_parse_and_one_more_is_refused() {
    parse_str(&parens(MAX_NESTING)).expect("brackets at the bound parse");
    assert_eq!(
        refusal(&parens(MAX_NESTING + 1)),
        (Nesting::Brackets, MAX_NESTING, Some(Token::LeftParen))
    );
}

#[test]
fn a_contains_chain_at_the_nesting_bound_parses_and_one_more_is_refused() {
    parse_str(&contains_chain(MAX_NESTING)).expect("a chain at the bound parses");
    let (measure, limit, found) = refusal(&contains_chain(MAX_NESTING + 1));
    assert_eq!((measure, limit), (Nesting::Brackets, MAX_NESTING));
    assert!(
        matches!(found, Some(Token::Contains(_))),
        "the refusal names the CONTAINS that crossed the bound, got {found:?}"
    );
}

/// The two queries that aborted the 5.5.0 server: 20 000 `NOT`s (80 kB) and
/// 30 000 `OR`s (800 kB).
#[test]
fn the_reported_chains_are_refused_at_the_token_that_crosses_the_bound() {
    for src in [not_chain(20_000), or_chain(30_000)] {
        let Err(ParseError::TooDeep { measure, at, .. }) = parse_str(&src) else {
            panic!("the chain is refused");
        };
        assert_eq!(measure, Nesting::Expression);
        let bytes = at.bytes.expect("a spanned parse locates the refusal");
        let crossing = src.get(bytes).expect("the span lies in the source");
        assert!(
            crossing == "NOT" || crossing == "OR",
            "the span covers the crossing operator, got {crossing:?}"
        );
    }
}

/// Sibling groups do not add up: only the operators on one path count.
#[test]
fn sibling_groups_count_once() {
    let group = format!("({})", vec!["c/name/value = 'x'"; 400].join(" OR "));
    let src = format!("{FROM}{group} AND {group}");
    parse_str(&src).expect("two shallow groups side by side parse");
}

/// A long value list builds no tree, so it is not bounded.
#[test]
fn a_long_matches_list_is_not_bounded() {
    let values: Vec<String> = (0..20_000).map(|i| format!("'x{i}'")).collect();
    let src = format!("{FROM}c/name/value MATCHES {{{}}}", values.join(", "));
    parse_str(&src).expect("a long value list parses");
}

#[test]
fn the_refusal_names_the_measure_and_the_limit() {
    let err = parse_str(&not_chain(MAX_EXPRESSION_DEPTH + 1)).expect_err("refused");
    let message = err.to_string();
    assert!(
        message.contains("expression depth") && message.contains(&MAX_EXPRESSION_DEPTH.to_string()),
        "the message names the measure and the limit, got {message:?}"
    );
}
