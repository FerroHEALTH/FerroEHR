// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The search-statement tests of the audit store.

use super::{
    AuditCriterion, AuditElement, AuditSearchFilter, AuditToken, AuditTokenSystem, RECORD_COLUMNS,
    build_search_statements,
};

/// Twenty payloads that break SQL when they reach the statement text rather
/// than a bound parameter.
const HOSTILE: [&str; 20] = [
    "'",
    "\"",
    "--",
    "/*",
    "*/",
    ";",
    "\\",
    "' OR '1'='1",
    "'; DROP TABLE audit.audit_event; --",
    "') OR 1=1 --",
    "1; SELECT pg_sleep(10)",
    "$1",
    "$$",
    "\0",
    "\n",
    "\r\n",
    "0x27",
    "%27",
    "\u{2019}",
    "' UNION SELECT fhir FROM audit.audit_event --",
];

/// The token ids the criteria of [`filter_with`] resolve to: none, so
/// each is read as its column condition.
const RESOLVED: [Option<crate::system_log::search::TokenMatch>; 0] = [];

fn filter_with(value: &str) -> AuditSearchFilter {
    AuditSearchFilter {
        criteria: vec![
            AuditCriterion::Patient(vec![value.to_owned()]),
            AuditCriterion::Agent(vec![value.to_owned()]),
            AuditCriterion::Entity(vec![value.to_owned()]),
            AuditCriterion::Action(vec![value.to_owned()]),
            AuditCriterion::Address(vec![value.to_owned()]),
            AuditCriterion::Element(
                AuditElement::AgentIdentifier,
                vec![AuditToken {
                    system: AuditTokenSystem::Named(value.to_owned()),
                    code: Some(value.to_owned()),
                }],
            ),
        ],
        ..AuditSearchFilter::default()
    }
}

/// Hostile filter values never change the generated SQL — only the bound
/// values.
///
/// The assertion is byte-equality of the statement text against a benign
/// baseline. That is stronger than looking for the payload in the SQL: it
/// catches a value that alters the statement's SHAPE as well as one that
/// appears in it.
#[test]
fn hostile_filter_values_never_change_the_generated_sql() {
    let (baseline_count, _, baseline_page, _) =
        build_search_statements(&filter_with("benign"), &RESOLVED);

    for payload in HOSTILE {
        // The page statement's own values are not inspected: both statements
        // bind from the same condition, so asserting the count's bindings
        // covers both, and the page text is compared byte for byte below.
        let (count_sql, count_values, sql, _page_values) =
            build_search_statements(&filter_with(payload), &RESOLVED);
        assert_eq!(
            count_sql, baseline_count,
            "the count statement changed for {payload:?}"
        );
        assert_eq!(
            sql, baseline_page,
            "the page statement changed for {payload:?}"
        );
        // The "absent from the SQL" check skips payloads that ARE the
        // placeholder syntax (`$1`, `$$`) or a single metacharacter: `$1`
        // legitimately appears because that is what a bound parameter looks
        // like. The byte-equality above already covers them — a value that
        // altered the statement would have changed it — so this is the
        // narrower check, not a weaker one.
        let is_placeholder_shaped = payload.starts_with('$') || payload.len() < 2;
        assert!(
            is_placeholder_shaped || !sql.contains(payload),
            "the payload must not appear in the SQL text: {payload:?} in {sql}"
        );
        // The values still travel — the point is that they travel as
        // parameters, not that they are dropped. Compared in the DEBUG
        // representation on both sides: the bound values render a newline as
        // an escape, so comparing a raw control character against that text
        // would fail for the wrong reason.
        let bound = format!("{count_values:?}");
        let rendered = format!("{payload:?}");
        let rendered = rendered.trim_matches('"');
        assert!(
            bound.contains(rendered),
            "the payload must be bound as a value: {payload:?} not in {bound}"
        );
    }
}

/// The identifiers and the sort direction come from a closed set in this
/// module — the enumeration, recorded so a new filter cannot quietly add one.
///
/// Written as an exact-set assertion over the quoted identifiers the builder
/// emits: a filter that introduced a caller-supplied column would appear here
/// and fail, which is what the guard script cannot see (it reads for
/// string-built SQL, not for an `Alias::new` argument).
#[test]
fn every_identifier_comes_from_the_closed_set() {
    let (count_sql, _, sql, _) = build_search_statements(
        &AuditSearchFilter {
            criteria: vec![
                AuditCriterion::Patient(vec!["p".to_owned()]),
                AuditCriterion::Agent(vec!["a".to_owned()]),
                AuditCriterion::Entity(vec!["e".to_owned()]),
                AuditCriterion::Action(vec!["x".to_owned()]),
                AuditCriterion::Outcome(vec![0]),
            ],
            ..AuditSearchFilter::default()
        },
        &[],
    );

    let mut seen: Vec<String> = format!("{count_sql} {sql}")
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    seen.sort_unstable();
    seen.dedup();

    let mut expected: Vec<&str> = RECORD_COLUMNS.to_vec();
    expected.extend(["audit", "audit_event"]);
    expected.sort_unstable();
    expected.dedup();
    assert_eq!(
        seen,
        expected.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
        "the identifier set moved: {sql}"
    );
    assert!(sql.contains("DESC"), "newest-first is the ITI-81 order");
    assert!(!sql.contains("ASC"), "no ascending sort is emitted");
}

/// `limit`/`offset` are clamped integers, so no caller value reaches the
/// statement as text — including a negative offset or an absurd count.
#[test]
fn paging_is_clamped_and_never_textual() {
    let (_, _, sql, values) = build_search_statements(
        &AuditSearchFilter {
            count: i64::MAX,
            offset: i64::MIN,
            ..AuditSearchFilter::default()
        },
        &[],
    );
    // Paging binds as parameters rather than being written into the
    // statement, which is stronger than the clamp alone — so the clamped
    // values are asserted where they actually travel.
    assert!(
        sql.contains("LIMIT $") && sql.contains("OFFSET $"),
        "paging must bind, not interpolate: {sql}"
    );
    let bound = format!("{values:?}");
    assert!(
        bound.contains("1000"),
        "the count clamps to the cap: {bound}"
    );
    assert!(
        !bound.contains(&i64::MIN.to_string()),
        "a negative offset must never reach the database: {bound}"
    );
}

fn token(system: AuditTokenSystem, code: Option<&str>) -> AuditToken {
    AuditToken {
        system,
        code: code.map(str::to_owned),
    }
}

/// The FHIR Feed reads a record back by the same columns the search does.
#[test]
fn the_feed_reads_every_record_column() {
    let selected: Vec<&str> = super::OLDEST_OWED
        .split(" FROM ")
        .next()
        .unwrap_or_default()
        .trim_start_matches("SELECT")
        .split(',')
        .map(|column| column.trim().trim_start_matches("e."))
        .collect();
    assert_eq!(selected, RECORD_COLUMNS);
}

/// A rendered document with an agent, the patient and an object entity.
fn document() -> serde_json::Value {
    serde_json::json!({
        "type": {"system": "http://dicom.nema.org/resources/ontology/DCM", "code": "110110"},
        "subtype": [{"code": "composition_get"}],
        "agent": [
            {"who": {"identifier": {"value": "alice"}}, "network": {"address": "10.0.0.9"}},
            {"who": {"identifier": {"system": "urn:sys", "value": "ferroehr"}}}
        ],
        "entity": [
            {
                "role": {"system": "http://terminology.hl7.org/CodeSystem/object-role", "code": "1"},
                "what": {"identifier": {"value": "patient-42"}}
            },
            {"what": {"identifier": {"value": "8fa1::ferroehr::1"}}}
        ]
    })
}

/// The three token forms of FHIR R4 Search §3.1.1.4.10 match an
/// `Identifier` by its `value` and a `Coding` by its `code`, as the stored
/// document's SQL/JSON path did.
#[test]
fn a_token_matches_the_document_element_it_names() {
    let document = document();
    let element = |element, tokens| AuditCriterion::Element(element, tokens);
    let any = |code| token(AuditTokenSystem::Any, Some(code));
    let cases = [
        (
            element(AuditElement::AgentIdentifier, vec![any("alice")]),
            true,
        ),
        (
            element(
                AuditElement::AgentIdentifier,
                vec![token(AuditTokenSystem::Absent, Some("alice"))],
            ),
            true,
        ),
        (
            element(
                AuditElement::AgentIdentifier,
                vec![token(AuditTokenSystem::Absent, Some("ferroehr"))],
            ),
            false,
        ),
        (
            element(
                AuditElement::AgentIdentifier,
                vec![token(
                    AuditTokenSystem::Named("urn:sys".to_owned()),
                    Some("ferroehr"),
                )],
            ),
            true,
        ),
        (
            element(
                AuditElement::AgentIdentifier,
                vec![token(AuditTokenSystem::Named("urn:sys".to_owned()), None)],
            ),
            true,
        ),
        (
            element(
                AuditElement::AgentIdentifier,
                vec![token(
                    AuditTokenSystem::Named("urn:other".to_owned()),
                    Some("ferroehr"),
                )],
            ),
            false,
        ),
        (
            element(AuditElement::PatientIdentifier, vec![any("patient-42")]),
            true,
        ),
        (
            element(
                AuditElement::PatientIdentifier,
                vec![any("8fa1::ferroehr::1")],
            ),
            false,
        ),
        (
            element(
                AuditElement::EntityIdentifier,
                vec![any("8fa1::ferroehr::1")],
            ),
            true,
        ),
        (element(AuditElement::Type, vec![any("110110")]), true),
        (
            element(
                AuditElement::Subtype,
                vec![any("composition_get"), any("nothing")],
            ),
            true,
        ),
        (element(AuditElement::Subtype, vec![any("nothing")]), false),
        (
            element(AuditElement::SourceIdentifier, vec![any("ferroehr")]),
            false,
        ),
        (AuditCriterion::Address(vec!["0.0.9".to_owned()]), true),
        (AuditCriterion::Address(vec!["10.1".to_owned()]), false),
    ];
    for (criterion, expected) in cases {
        assert!(criterion.reads_document(), "{criterion:?}");
        assert_eq!(criterion.matches(&document), expected, "{criterion:?}");
    }
}

/// A criterion on the document becomes a condition on `search_tokens`
/// with the token ids it resolved to, bound as one value, and one with no
/// resolved ids matches nothing.
#[test]
fn a_resolved_token_criterion_reads_the_token_index() {
    let filter = AuditSearchFilter {
        criteria: vec![
            AuditCriterion::Patient(vec!["patient-42".to_owned()]),
            AuditCriterion::Element(
                AuditElement::PatientIdentifier,
                vec![token(AuditTokenSystem::Any, Some("patient-42"))],
            ),
        ],
        ..AuditSearchFilter::default()
    };
    let (count_sql, values, sql, _) = build_search_statements(
        &filter,
        &[
            None,
            Some(crate::system_log::search::TokenMatch::from(vec![3, 17])),
        ],
    );
    assert!(sql.contains("search_tokens && CAST($"), "{sql}");
    assert!(count_sql.contains("search_tokens && CAST($"), "{count_sql}");
    assert!(format!("{values:?}").contains("{3,17}"), "{values:?}");
    let (_, _, unresolved, _) = build_search_statements(&filter, &[]);
    assert!(unresolved.contains("false"), "{unresolved}");
}
