// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Binding `query_parameters` into the AST (`QUERY/docs/AQL/master03-syntax.adoc`
//! §Parameters; `ITS-REST/specifications/docs/query/Request.md` §Query
//! parameters): every position binds to the literal the hand-written query
//! would carry, nothing is spliced into text, and every refusal names the
//! parameter, never its value.

#![allow(
    clippy::panic,
    clippy::panic_in_result_fn,
    reason = "integration-test assertions inside Result-returning tests, the shape testing.md prefers"
)]

use openehr_query::ast::Primitive;
use openehr_query::bind::{BindError, Fault, FaultKind, Parameters, Position, bind};
use openehr_query::parser::parse_str;
use openehr_query::printer::to_aql;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn string(text: &str) -> Primitive {
    Primitive::String(text.to_owned())
}

/// Binds `parameters` into `template` and asserts the result equals `literal`,
/// both as an AST and after a print → parse round trip.
fn binds_to(template: &str, parameters: &Parameters, literal: &str) -> TestResult {
    let mut query = parse_str(template)?;
    bind(&mut query, parameters)?;
    let expected = parse_str(literal)?;
    assert_eq!(query, expected, "bound {template}");
    assert_eq!(
        parse_str(&to_aql(&query))?,
        expected,
        "reprinted {template}"
    );
    Ok(())
}

#[test]
fn every_parameter_position_binds_to_its_literal() -> TestResult {
    let mut p = Parameters::new();
    p.insert("ehrUid", string("7d44b88c-4199-4bad-97dc-d78268e01398"));
    p.insert(
        "archetypeId",
        string("openEHR-EHR-OBSERVATION.blood_pressure.v2"),
    );
    p.insert("nameValue", string("Systolic"));
    p.insert("systolic", Primitive::Integer(140));
    p.insert("temperature", Primitive::Real(38.5));
    p.insert("pattern", string("Vit*"));
    p.insert("code", string("at0.64"));
    p.insert("flag", Primitive::Boolean(true));
    binds_to(
        "SELECT o/data/value FROM EHR e[ehr_id/value=$ehrUid] \
         CONTAINS OBSERVATION o[$archetypeId] \
         WHERE o/data[at0003, $nameValue]/value/magnitude >= $systolic \
         AND o/x/magnitude > $temperature AND o/name/value LIKE $pattern \
         AND o/y/defining_code/code_string MATCHES {$code, 'at0.65'} \
         AND length(o/z/value) > $systolic AND o/b/value = $flag",
        &p,
        "SELECT o/data/value FROM EHR e[ehr_id/value='7d44b88c-4199-4bad-97dc-d78268e01398'] \
         CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2] \
         WHERE o/data[at0003, 'Systolic']/value/magnitude >= 140 \
         AND o/x/magnitude > 38.5 AND o/name/value LIKE 'Vit*' \
         AND o/y/defining_code/code_string MATCHES {'at0.64', 'at0.65'} \
         AND length(o/z/value) > 140 AND o/b/value = true",
    )
}

#[test]
fn a_version_predicate_and_a_node_predicate_bind() -> TestResult {
    let mut p = Parameters::new();
    p.insert("since", string("2026-01-01T00:00:00Z"));
    p.insert("node", string("at0001"));
    p.insert("other", string("openEHR-EHR-CLUSTER.device.v1"));
    binds_to(
        "SELECT c FROM VERSION v[commit_audit/time_committed > $since] CONTAINS COMPOSITION c \
         CONTAINS CLUSTER k[$node and name/value='x' or $other]",
        &p,
        "SELECT c FROM VERSION v[commit_audit/time_committed > '2026-01-01T00:00:00Z'] \
         CONTAINS COMPOSITION c CONTAINS CLUSTER k[at0001 and name/value='x' or openEHR-EHR-CLUSTER.device.v1]",
    )
}

#[test]
fn a_supplied_dollar_prefix_names_the_same_parameter() -> TestResult {
    let mut p = Parameters::new();
    p.insert("$uid", string("x"));
    binds_to(
        "SELECT c FROM COMPOSITION c WHERE c/uid/value = $uid",
        &p,
        "SELECT c FROM COMPOSITION c WHERE c/uid/value = 'x'",
    )
}

#[test]
fn a_hostile_value_stays_inside_its_string_literal() -> TestResult {
    let hostile = "x' OR c/uid/value != '";
    let mut p = Parameters::new();
    p.insert("uid", string(hostile));
    let mut query = parse_str("SELECT c FROM COMPOSITION c WHERE c/uid/value = $uid")?;
    bind(&mut query, &p)?;
    let reparsed = parse_str(&to_aql(&query))?;
    assert_eq!(reparsed, query);
    let Some(openehr_query::ast::WhereExpr::Identified(
        openehr_query::ast::IdentifiedExpr::Compare {
            rhs: openehr_query::ast::Terminal::Primitive(Primitive::String(bound)),
            ..
        },
        _,
    )) = &reparsed.where_
    else {
        panic!("one comparison against a string");
    };
    assert_eq!(bound, hostile);
    Ok(())
}

#[test]
fn every_fault_is_reported_by_name_and_the_query_is_left_untouched() -> TestResult {
    let secret = "SENTINEL-9999999999";
    let src = "SELECT o FROM EHR e[ehr_id/value=$ehrUid] CONTAINS OBSERVATION o[$archetype] \
               WHERE o/a/value LIKE $pattern AND o/b/value = $missing AND o/c/value = $nan";
    let original = parse_str(src)?;
    let mut query = original.clone();
    let mut p = Parameters::new();
    p.insert("ehrUid", string(secret));
    p.insert("$ehrUid", string(secret));
    p.insert("archetype", string(secret));
    p.insert("pattern", Primitive::Integer(9_999_999_999));
    p.insert("nan", Primitive::Real(f64::NAN));
    p.insert("extra", string(secret));
    p.insert_not_a_literal("object");
    let Err(BindError { faults }) = bind(&mut query, &p) else {
        panic!("the binding is refused");
    };
    let fault = |name: &str, kind| Fault {
        name: name.to_owned(),
        kind,
    };
    assert_eq!(
        faults,
        [
            fault("ehrUid", FaultKind::Duplicate),
            fault("archetype", FaultKind::Unbindable(Position::ArchetypeId)),
            fault("pattern", FaultKind::Unbindable(Position::Pattern)),
            fault("missing", FaultKind::Unbound),
            fault("nan", FaultKind::Unbindable(Position::Value)),
            fault("extra", FaultKind::Unknown),
            fault("object", FaultKind::Unknown),
        ]
    );
    assert_eq!(query, original, "a refused binding changes nothing");
    let error = BindError { faults };
    for rendered in [error.to_string(), format!("{error:?}"), format!("{p:?}")] {
        assert!(!rendered.contains(secret), "a value leaked: {rendered}");
        assert!(
            !rendered.contains("9999999999"),
            "a value leaked: {rendered}"
        );
    }
    Ok(())
}

#[test]
fn a_value_that_is_no_literal_is_refused_by_name() -> TestResult {
    let mut query = parse_str("SELECT c FROM COMPOSITION c WHERE c/uid/value = $uid")?;
    let mut p = Parameters::new();
    p.insert_not_a_literal("uid");
    let error = bind(&mut query, &p).err().ok_or("refused")?;
    assert_eq!(
        error.faults,
        [Fault {
            name: "uid".to_owned(),
            kind: FaultKind::NotALiteral
        }]
    );
    assert_eq!(
        error.to_string(),
        "parameter `uid` is not a string, number, boolean or null"
    );
    Ok(())
}

#[test]
fn a_node_predicate_refuses_a_value_that_is_not_one_token() -> TestResult {
    for value in [
        "at0001 or name/value='x'",
        " at0001",
        "openEHR-EHR-CLUSTER.device.v1]",
    ] {
        let mut query = parse_str("SELECT c FROM COMPOSITION c CONTAINS CLUSTER k[$node and $x]")?;
        let mut p = Parameters::new();
        p.insert("node", string(value));
        p.insert("x", string("at0002"));
        let error = bind(&mut query, &p).err().ok_or("refused")?;
        assert_eq!(
            error.faults,
            [Fault {
                name: "node".to_owned(),
                kind: FaultKind::Unbindable(Position::NodeId)
            }],
            "value {value:?}"
        );
    }
    Ok(())
}

#[test]
fn a_query_without_parameters_binds_an_empty_set() -> TestResult {
    let mut query = parse_str("SELECT c FROM COMPOSITION c")?;
    let before = query.clone();
    bind(&mut query, &Parameters::new())?;
    assert_eq!(query, before);
    Ok(())
}
