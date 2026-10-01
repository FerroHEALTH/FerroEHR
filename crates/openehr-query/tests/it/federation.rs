// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The federation directive: lifted out of `FROM` at the token level, the
//! remainder strict AQL, the directive printed back only on request.

#![allow(
    clippy::panic,
    clippy::panic_in_result_fn,
    reason = "integration-test assertions inside Result-returning tests, the shape testing.md prefers"
)]

use openehr_query::federation::{DirectiveKind, parse_federated, to_federated_aql};
use openehr_query::parser::{ParseError, parse_str};
use openehr_query::printer::to_aql;

use crate::corpus::standard_queries;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn an_endpoint_directive_is_lifted_out_of_the_from_clause() -> TestResult {
    let src = r#"SELECT p/id AS endpoint_id, c/uid/value AS composition_id
        FROM ENDPOINT p [ "node_1", "node_2", 'node_3' ] CONTAINS EHR e CONTAINS COMPOSITION c
        WHERE e/ehr_status/subject/external_ref/id/value = '12345'"#;
    let federated = parse_federated(src)?;
    let directive = federated.directive.as_ref().ok_or("a directive")?;
    assert_eq!(directive.kind, DirectiveKind::Endpoint);
    assert_eq!(directive.variable.as_deref(), Some("p"));
    assert_eq!(directive.ids, ["node_1", "node_2", "node_3"]);
    let written = directive.span.bytes().and_then(|b| src.get(b));
    assert_eq!(
        written,
        Some(r#"ENDPOINT p [ "node_1", "node_2", 'node_3' ]"#)
    );
    let node_side = to_aql(&federated.query);
    assert!(!node_side.contains("ENDPOINT"));
    assert_eq!(
        federated.query,
        parse_str(
            "SELECT p/id AS endpoint_id, c/uid/value AS composition_id \
             FROM EHR e CONTAINS COMPOSITION c \
             WHERE e/ehr_status/subject/external_ref/id/value = '12345'"
        )?
    );
    Ok(())
}

#[test]
fn an_organisation_directive_needs_no_variable_and_ignores_case() -> TestResult {
    let federated = parse_federated(
        r#"SELECT c/uid/value FROM organisation ["org-a"] CONTAINS EHR e CONTAINS COMPOSITION c"#,
    )?;
    let directive = federated.directive.as_ref().ok_or("a directive")?;
    assert_eq!(directive.kind, DirectiveKind::Organisation);
    assert_eq!(directive.variable, None);
    assert_eq!(directive.ids, ["org-a"]);
    assert_eq!(
        to_aql(&federated.query),
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c"
    );
    Ok(())
}

#[test]
fn a_directive_inside_a_string_literal_is_a_string() -> TestResult {
    let src = r#"SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE c/name/value = 'FROM ENDPOINT p ["node_1"]'"#;
    let federated = parse_federated(src)?;
    assert_eq!(federated.directive, None);
    assert_eq!(federated.query, parse_str(src)?);
    Ok(())
}

#[test]
fn every_corpus_query_parses_as_it_does_without_the_feature() -> Result<(), ParseError> {
    for src in standard_queries() {
        let federated = parse_federated(&src)?;
        assert_eq!(federated.directive, None, "no directive in: {src}");
        assert_eq!(federated.query, parse_str(&src)?);
        assert_eq!(to_federated_aql(&federated), to_aql(&federated.query));
    }
    Ok(())
}

#[test]
fn a_federated_query_prints_back_and_reparses_to_itself() -> TestResult {
    let federated = parse_federated(
        r#"SELECT c FROM ENDPOINT p ["it's", "n2"] CONTAINS (EHR e CONTAINS COMPOSITION c OR EHR f)"#,
    )?;
    let printed = to_federated_aql(&federated);
    assert_eq!(parse_federated(&printed)?, federated, "via {printed}");
    Ok(())
}

#[test]
fn a_malformed_directive_is_a_located_syntax_error() {
    let cases = [
        ("SELECT c FROM ENDPOINT p [ ] CONTAINS EHR e", "]"),
        (r#"SELECT c FROM ENDPOINT p [ "n1", ] CONTAINS EHR e"#, "]"),
        ("SELECT c FROM ENDPOINT p [ n1 ] CONTAINS EHR e", "n1"),
        (r#"SELECT c FROM ENDPOINT p [ "n1" EHR e"#, "EHR"),
        (r#"SELECT c FROM ENDPOINT p [ "n1" ] EHR e"#, "EHR"),
        (
            r#"SELECT c FROM ENDPOINT p [ "n1" ] NOT CONTAINS EHR e"#,
            "NOT",
        ),
        ("SELECT c FROM ENDPOINT p", ""),
    ];
    for (src, at) in cases {
        let Err(ParseError::Syntax { faults }) = parse_federated(src) else {
            panic!("refused: {src}");
        };
        let located = faults
            .first()
            .and_then(|f| f.bytes.clone())
            .and_then(|b| src.get(b));
        assert_eq!(located, Some(at), "located in: {src}");
    }
}

#[test]
fn the_strict_parser_still_refuses_the_directive() {
    assert!(parse_str(r#"SELECT c FROM ENDPOINT p ["n1", "n2"] CONTAINS EHR e"#).is_err());
}
