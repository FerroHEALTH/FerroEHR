// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The search-statement tests of the audit store.

use super::search_page::build_search_statements;
use super::{
    AuditCriterion, AuditElement, AuditSearchFilter, AuditToken, AuditTokenSystem, RECORD_COLUMNS,
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
        build_search_statements(&filter_with("benign"), &RESOLVED, 0);

    for payload in HOSTILE {
        // The page statement's own values are not inspected: both statements
        // bind from the same condition, so asserting the count's bindings
        // covers both, and the page text is compared byte for byte below.
        let (count_sql, count_values, sql, _page_values) =
            build_search_statements(&filter_with(payload), &RESOLVED, 0);
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
        0,
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
    expected.extend([
        "audit",
        "audit_record",
        "access_log_search",
        "system_search",
        "system_page",
        "access_log_candidate",
        "access_log_by_key",
        "chains",
        "page_keys",
        "position",
        "page_key",
        "page_record",
        "shared_digest",
        "index_cursor",
    ]);
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
        0,
    );
    // Paging binds as parameters rather than being written into the
    // statement, which is stronger than the clamp alone — so the clamped
    // values are asserted where they actually travel.
    assert!(
        sql.contains("LIMIT $") && sql.contains("OFFSET $"),
        "paging must bind, not interpolate: {sql}"
    );
    let bound = format!("{values:?}");
    // The page reads one record past the clamped count, which says whether a
    // next page exists without counting the matches (#5702).
    assert!(
        bound.contains("1001"),
        "the count clamps to the cap, plus the one look-ahead record: {bound}"
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

/// Each sink that delivers from the store reads a record back by the same
/// columns the search does, after whether it is a retried parked record:
/// the retried record and each log's next one alike.
#[test]
fn every_sink_reads_every_record_column() {
    let branches: Vec<&str> = super::outbox::next_statement().split("UNION ALL").collect();
    assert_eq!(
        branches.len(),
        3,
        "a retried record and each log's next one"
    );
    for branch in branches {
        let selected: Vec<&str> = branch
            .trim()
            .trim_start_matches('(')
            .split(" FROM ")
            .next()
            .unwrap_or_default()
            .trim_start_matches("SELECT")
            .split(',')
            .map(|column| column.trim().trim_start_matches("e."))
            .collect();
        let (retry, record) = selected.split_first().expect("the select list");
        assert!(retry.ends_with("AS retry"), "{retry}");
        assert_eq!(record, RECORD_COLUMNS, "{branch}");
    }
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

/// A criterion on the document becomes a condition on `search_keys`
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
        0,
    );
    assert!(sql.contains("search_keys && CAST($"), "{sql}");
    assert!(count_sql.contains("search_keys && CAST($"), "{count_sql}");
    assert!(format!("{values:?}").contains("{3,17}"), "{values:?}");
    let (_, _, unresolved, _) = build_search_statements(&filter, &[], 0);
    assert!(unresolved.contains("false"), "{unresolved}");
}

/// A filter whose criteria are each of their own kind with one value is of
/// the bounded shape set that runs as named statements; its text does not
/// change with the values it binds. A criterion with several values, or a
/// kind named twice, is not.
#[test]
fn a_filter_of_single_valued_distinct_criteria_is_shaped() {
    let shaped = filter_with("a");
    assert!(super::search_page::shaped(&shaped));
    let (count_a, _, page_a, _) = build_search_statements(&shaped, &RESOLVED, 0);
    let (count_b, _, page_b, _) = build_search_statements(&filter_with("other"), &RESOLVED, 0);
    assert_eq!((count_a, page_a), (count_b, page_b));
    let mut several = filter_with("a");
    several.criteria.push(AuditCriterion::Outcome(vec![0, 4]));
    assert!(!super::search_page::shaped(&several));
    let mut twice = filter_with("a");
    twice
        .criteria
        .push(AuditCriterion::Patient(vec!["b".to_owned()]));
    assert!(!super::search_page::shaped(&twice));
    assert!(super::search_page::shaped(&AuditSearchFilter::default()));
}

/// A page that carries the policy revisions selects them beside its records,
/// under the names the confirmation reads (#6320).
#[test]
fn a_carried_page_selects_the_policy_revisions() {
    let carried = AuditSearchFilter {
        carried: super::carried::Carried {
            policy: true,
            party: false,
        },
        ..filter_with("benign")
    };
    let (count_sql, _, sql, _) = build_search_statements(&carried, &RESOLVED, 0);
    assert!(sql.contains("clinical.request_revisions"), "{sql}");
    assert!(sql.contains("\"carried_revisions\""), "{sql}");
    assert!(
        !count_sql.contains("request_revisions"),
        "the total carries nothing"
    );
    let (_, _, plain, _) = build_search_statements(&filter_with("benign"), &RESOLVED, 0);
    assert!(!plain.contains("request_revisions"), "{plain}");
}

/// The searches of [`a_shapes_statements_are_the_built_ones`]: each shape
/// twice, with other values the second time.
fn shaped_searches() -> Vec<(
    AuditSearchFilter,
    Vec<Option<crate::system_log::search::TokenMatch>>,
)> {
    use crate::system_log::search::{ColumnMatch, IdentifierColumn, TokenMatch};
    let at = |second: i64| Some(jiff::Timestamp::constant(second, 0));
    let mut searches = Vec::new();
    for round in 0..2_i16 {
        let wide = i64::from(round);
        let text = |name: &str| format!("{name}-{round}");
        searches.push((
            AuditSearchFilter {
                count: 20 + wide,
                offset: wide * 7,
                ..AuditSearchFilter::default()
            },
            Vec::new(),
        ));
        searches.push((
            AuditSearchFilter {
                from: at(1_700_000_000 + wide),
                until: at(1_800_000_000 + wide),
                until_inclusive: round == 1,
                criteria: vec![
                    AuditCriterion::Patient(vec![text("p"), text("q")]),
                    AuditCriterion::Outcome(vec![0, 4 * round]),
                    AuditCriterion::Action(vec![text("R")]),
                ],
                count: 50,
                total: true,
                ..AuditSearchFilter::default()
            },
            Vec::new(),
        ));
        searches.push((
            AuditSearchFilter {
                until: at(1_900_000_000 - wide),
                criteria: vec![
                    AuditCriterion::Agent(vec![text("agent")]),
                    AuditCriterion::Element(AuditElement::EntityIdentifier, Vec::new()),
                ],
                ..AuditSearchFilter::default()
            },
            vec![
                None,
                Some(TokenMatch {
                    ids: vec![3, i32::from(round) + 5],
                    columns: vec![
                        ColumnMatch {
                            column: IdentifierColumn::ResourceId,
                            sets: vec![-1],
                            value: Some(text("resource")),
                        },
                        ColumnMatch {
                            column: IdentifierColumn::RequestId,
                            sets: vec![-2, -3],
                            value: None,
                        },
                    ],
                }),
            ],
        ));
    }
    searches
}

/// Each search's statements, from its shape's template or built for it, are
/// byte for byte the statements built for it, and so are their values
/// (#6362).
#[test]
fn a_shapes_statements_are_the_built_ones() {
    let shapes = super::page_shapes::PageShapes::default();
    let mut templates = Vec::new();
    for (filter, resolved) in shaped_searches() {
        let cursor = 40 + filter.offset;
        let (count_sql, count_values, sql, values) =
            build_search_statements(&filter, &resolved, cursor);
        let statements = shapes.statements(&filter, &resolved, cursor);
        assert_eq!(statements.template.count_sql(), count_sql);
        assert_eq!(statements.template.sql(), sql);
        assert_eq!(statements.count_values.0, count_values.0);
        assert_eq!(statements.values.0, values.0);
        templates.push(statements.template);
    }
    // The page alone and the token search are each one shape across their
    // values, so the second search of each runs the first one's template.
    for (first, second) in [(0, 3), (2, 5)] {
        assert!(std::sync::Arc::ptr_eq(
            &templates[first],
            &templates[second]
        ));
    }
}
