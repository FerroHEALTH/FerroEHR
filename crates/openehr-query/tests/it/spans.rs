// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Source spans on identified paths and WHERE conditions: each slices back to
//! the text it was parsed from, none takes part in equality, and a parse over
//! bare tokens carries none.

#![allow(
    clippy::panic,
    clippy::panic_in_result_fn,
    reason = "integration-test assertions inside Result-returning tests, the shape testing.md prefers"
)]

use openehr_query::ast::{IdentifiedExpr, IdentifiedPath, Span, WhereExpr};
use openehr_query::lexer::lex;
use openehr_query::parser::{ParseError, parse, parse_str};
use openehr_query::visit::{Visit, walk_identified_path, walk_where_expr};

use crate::corpus::standard_queries;

/// Every path span and every condition span of one query, as source text.
#[derive(Default)]
struct Spans<'s> {
    src: &'s str,
    paths: Vec<(String, Option<String>)>,
    conditions: Vec<Option<String>>,
}

impl Spans<'_> {
    fn text(&self, span: &Span) -> Option<String> {
        span.bytes()
            .and_then(|range| self.src.get(range))
            .map(str::to_owned)
    }
}

impl<'ast> Visit<'ast> for Spans<'_> {
    fn visit_identified_path(&mut self, node: &'ast IdentifiedPath) {
        let text = self.text(&node.span);
        self.paths.push((node.root.clone(), text));
        walk_identified_path(self, node);
    }

    fn visit_where_expr(&mut self, node: &'ast WhereExpr) {
        if let WhereExpr::Identified(_, span) = node {
            let text = self.text(span);
            self.conditions.push(text);
        }
        walk_where_expr(self, node);
    }
}

#[test]
fn every_corpus_span_slices_back_to_the_node_it_marks() -> Result<(), ParseError> {
    let mut paths = 0;
    for src in standard_queries() {
        let query = parse_str(&src)?;
        let mut spans = Spans {
            src: &src,
            ..Spans::default()
        };
        spans.visit_select_query(&query);
        for (root, text) in &spans.paths {
            let text = text.as_deref().unwrap_or_default();
            assert!(
                text.starts_with(root.as_str()),
                "path span `{text}` in: {src}"
            );
            assert_eq!(text.trim(), text, "a path span is trimmed: {src}");
            paths += 1;
        }
        for text in &spans.conditions {
            let text = text.as_deref().unwrap_or_default();
            assert!(!text.is_empty(), "condition span in: {src}");
            assert_eq!(text.trim(), text, "a condition span is trimmed: {src}");
        }
    }
    assert!(paths > 0);
    Ok(())
}

#[test]
fn a_condition_span_covers_the_whole_condition_and_its_path() -> Result<(), ParseError> {
    let src = "SELECT o/data/value FROM EHR e CONTAINS OBSERVATION o \
               WHERE o/data[at0001]/value/magnitude >= 140 AND EXISTS e/ehr_status";
    let query = parse_str(src)?;
    let mut spans = Spans {
        src,
        ..Spans::default()
    };
    spans.visit_select_query(&query);
    assert_eq!(
        spans.conditions,
        [
            Some("o/data[at0001]/value/magnitude >= 140".to_owned()),
            Some("EXISTS e/ehr_status".to_owned()),
        ]
    );
    assert_eq!(
        spans.paths,
        [
            ("o".to_owned(), Some("o/data/value".to_owned())),
            (
                "o".to_owned(),
                Some("o/data[at0001]/value/magnitude".to_owned())
            ),
            ("e".to_owned(), Some("e/ehr_status".to_owned())),
        ]
    );
    Ok(())
}

#[test]
fn a_span_names_a_position_without_the_literal_it_holds() -> Result<(), ParseError> {
    let src = "SELECT e/ehr_id/value FROM EHR e \
               WHERE e/ehr_status/subject/external_ref/id/value = '9999999999'";
    let query = parse_str(src)?;
    let Some(WhereExpr::Identified(IdentifiedExpr::Compare { lhs, .. }, span)) = &query.where_
    else {
        panic!("one comparison");
    };
    let openehr_query::ast::CompareOperand::Path(path) = lhs else {
        panic!("a path operand");
    };
    let path_bytes = path.span.bytes().unwrap_or_default();
    let condition = span.bytes().unwrap_or_default();
    assert_eq!(
        src.get(path_bytes.clone()),
        Some("e/ehr_status/subject/external_ref/id/value")
    );
    assert!(condition.start == path_bytes.start && condition.end > path_bytes.end);
    Ok(())
}

#[test]
fn spans_take_no_part_in_equality() -> Result<(), ParseError> {
    let tight = parse_str("SELECT e/ehr_id/value FROM EHR e WHERE e/time_created/value>'2026'")?;
    let loose =
        parse_str("SELECT   e/ehr_id/value\nFROM EHR e\nWHERE   e/time_created/value  >  '2026'")?;
    assert_eq!(tight, loose);
    let positions = |q: &openehr_query::ast::SelectQuery| {
        q.where_.as_ref().and_then(|w| match w {
            WhereExpr::Identified(_, span) => span.bytes(),
            _ => None,
        })
    };
    assert_ne!(positions(&tight), positions(&loose));
    Ok(())
}

#[test]
fn a_parse_over_bare_tokens_carries_no_spans() -> Result<(), ParseError> {
    let src = "SELECT e/ehr_id/value FROM EHR e WHERE e/time_created/value > '2026'";
    let query = parse(&lex(src)?)?;
    let mut spans = Spans {
        src,
        ..Spans::default()
    };
    spans.visit_select_query(&query);
    assert!(spans.paths.iter().all(|(_, text)| text.is_none()));
    assert!(spans.conditions.iter().all(Option::is_none));
    Ok(())
}
