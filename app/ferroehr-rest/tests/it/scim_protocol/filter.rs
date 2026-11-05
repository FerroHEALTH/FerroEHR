// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The filter grammar of RFC 7644 §3.4.2.2 (Figure 1), its precedence, its
//! refusals and its evaluation over the RFC 7643 §8 representations.

use ferroehr_rest::scim::protocol::error::ScimType;
use ferroehr_rest::scim::protocol::filter::{
    self, AttrExp, CompValue, CompareOp, Filter, FilterSupport, Logic, Term, Test,
};
use ferroehr_rest::scim::protocol::path::{AttrName, AttrPath};
use ferroehr_rest::scim::protocol::urn;

use super::source::{self, Doc, Fix};
use super::{held, user_schema};

/// Reads RFC 7644 Figure 2: one filter per paragraph, each line after the
/// first a continuation of the same expression.
fn figure_2() -> Vec<String> {
    let mut filters: Vec<String> = Vec::new();
    for line in source::lines(Doc::Rfc7644, 1243..=1280) {
        if let Some(start) = line.strip_prefix("filter=") {
            filters.push(start.to_owned());
        } else if !line.trim().is_empty() {
            let last = filters
                .last_mut()
                .expect("a continuation follows a filter line");
            if !last.is_empty() {
                last.push(' ');
            }
            last.push_str(line.trim());
        }
    }
    filters
}

fn path(text: &str) -> AttrPath {
    AttrPath::parse(text).expect("the test path should parse")
}

fn compare(attr: &str, op: CompareOp, value: CompValue) -> Filter {
    Logic::Leaf(Term::Attr(AttrExp {
        path: path(attr),
        test: Test::Compare(op, value),
    }))
}

fn present(attr: &str) -> Filter {
    Logic::Leaf(Term::Attr(AttrExp {
        path: path(attr),
        test: Test::Present,
    }))
}

fn sub(attr: &str, op: CompareOp, value: CompValue) -> Logic<AttrExp<AttrName>> {
    let name = AttrName::parse(attr).expect("the test name should parse");
    Logic::Leaf(AttrExp {
        path: name,
        test: Test::Compare(op, value),
    })
}

fn text(value: &str) -> CompValue {
    CompValue::String(value.to_owned())
}

fn refusal(filter: &str) -> String {
    let error = filter::parse(filter).expect_err("the filter should be refused");
    assert_eq!(
        error.scim_type(),
        Some(ScimType::InvalidFilter),
        "{filter}: {error}"
    );
    assert_eq!(error.status(), http::StatusCode::BAD_REQUEST);
    error.detail().to_owned()
}

#[test]
fn every_example_filter_of_figure_2_parses() {
    let filters = figure_2();
    assert_eq!(
        filters.len(),
        17,
        "RFC 7644 Figure 2 lists seventeen filters"
    );
    for text in &filters {
        filter::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"));
    }
}

#[test]
fn figure_2_parses_to_the_expressions_it_spells() {
    let filters = figure_2();
    let parsed: Vec<Filter> = filters
        .iter()
        .map(|f| filter::parse(f).expect("parses"))
        .collect();
    assert_eq!(
        parsed[0],
        compare("userName", CompareOp::Eq, text("bjensen"))
    );
    assert_eq!(
        parsed[1],
        compare("name.familyName", CompareOp::Co, text("O'Malley"))
    );
    assert_eq!(
        parsed[3],
        compare(
            &format!("{}:userName", urn::CORE_USER),
            CompareOp::Sw,
            text("J")
        )
    );
    assert_eq!(parsed[4], present("title"));
    assert_eq!(
        parsed[5],
        compare(
            "meta.lastModified",
            CompareOp::Gt,
            text("2011-05-13T04:42:34Z")
        )
    );
    assert_eq!(
        parsed[10],
        Logic::Or(vec![
            present("title"),
            compare("userType", CompareOp::Eq, text("Intern"))
        ])
    );
    assert_eq!(
        parsed[11],
        compare("schemas", CompareOp::Eq, text(urn::ENTERPRISE_USER))
    );
    assert_eq!(
        parsed[13],
        Logic::And(vec![
            compare("userType", CompareOp::Ne, text("Employee")),
            Logic::Not(Box::new(Logic::Or(vec![
                compare("emails", CompareOp::Co, text("example.com")),
                compare("emails.value", CompareOp::Co, text("example.org")),
            ]))),
        ])
    );
    assert_eq!(
        parsed[15],
        Logic::And(vec![
            compare("userType", CompareOp::Eq, text("Employee")),
            Logic::Leaf(Term::ValuePath(
                path("emails"),
                Logic::And(vec![
                    sub("type", CompareOp::Eq, text("work")),
                    sub("value", CompareOp::Co, text("@example.com")),
                ]),
            )),
        ])
    );
}

#[test]
fn attribute_names_and_operators_are_case_insensitive() {
    // RFC 7644 §3.4.2.2: "the following two expressions will evaluate to the
    // same logical value".
    let first = source::line(Doc::Rfc7644, 940);
    let second = source::line(Doc::Rfc7644, 942);
    let first =
        filter::parse(first.strip_prefix("filter=").expect("a filter line")).expect("parses");
    let second =
        filter::parse(second.strip_prefix("filter=").expect("a filter line")).expect("parses");
    assert_eq!(first, second);
    let shouted = filter::parse("USERNAME EQ \"john\" AND title PR").expect("parses");
    assert_eq!(shouted.to_string(), "USERNAME eq \"john\" and title pr");
}

#[test]
fn logical_precedence_is_not_then_and_then_or() {
    assert_eq!(
        filter::parse("a pr or b pr and c pr").expect("parses"),
        Logic::Or(vec![
            present("a"),
            Logic::And(vec![present("b"), present("c")])
        ])
    );
    assert_eq!(
        filter::parse("not (a pr) and b pr").expect("parses"),
        Logic::And(vec![Logic::Not(Box::new(present("a"))), present("b")])
    );
    assert_eq!(
        filter::parse("(a pr or b pr) and c pr").expect("parses"),
        Logic::And(vec![
            Logic::Or(vec![present("a"), present("b")]),
            present("c")
        ])
    );
}

#[test]
fn comparison_values_are_the_json_literals() {
    assert_eq!(
        filter::parse("active eq true").expect("parses"),
        compare("active", CompareOp::Eq, CompValue::True)
    );
    assert_eq!(
        filter::parse("title eq null").expect("parses"),
        compare("title", CompareOp::Eq, CompValue::Null)
    );
    assert_eq!(
        filter::parse("x gt -1.5e3").expect("parses"),
        compare(
            "x",
            CompareOp::Gt,
            CompValue::Number(serde_json::from_str("-1.5e3").expect("a number"))
        )
    );
    assert_eq!(
        filter::parse(r#"displayName eq "A \"quoted\" é name""#).expect("parses"),
        compare(
            "displayName",
            CompareOp::Eq,
            text("A \"quoted\" \u{e9} name")
        )
    );
    // RFC 8259 §3: "The literal names MUST be lowercase."
    refusal("active eq True");
    refusal("x eq 01");
    refusal("x eq 'single'");
}

#[test]
fn an_unrecognised_operator_is_declined_with_the_example_message() {
    // RFC 7644 §3.4.2.2: "e.g., 'The operator 'regex' is not supported.'"
    assert_eq!(
        refusal("userName regex \"^b\""),
        "The operator 'regex' is not supported."
    );
}

#[test]
fn malformed_filters_are_invalid_filter() {
    for bad in [
        "",
        "   ",
        "userName",
        "userName eq",
        "userName eq \"unterminated",
        "userName pr \"value\"",
        "not userName pr",
        "(userName pr",
        "userName pr)",
        "userName pr and",
        "and userName pr",
        "emails[type eq \"work\"",
        "emails[type[x eq \"y\"] pr]",
        "emails[name.givenName eq \"x\"]",
        "emails[urn:x:y:type eq \"x\"]",
        "name.givenName.extra pr",
        "1abc pr",
        "urn:userName pr",
        "userName eq \"a\" \"b\"",
    ] {
        refusal(bad);
    }
}

#[test]
fn the_server_root_example_with_a_bare_word_value_is_refused() {
    // RFC 7644 §3.4.2.1 prints `meta.resourceType eq User`, whose value is not
    // a `compValue` of Figure 1 (a JSON literal); the grammar wins.
    let example = source::line(Doc::Rfc7644, 908);
    refusal(example.strip_prefix("filter=").expect("a filter line"));
}

#[test]
fn nesting_is_bounded_and_long_chains_are_flat() {
    let deep = format!("{}userName pr{}", "(".repeat(200), ")".repeat(200));
    refusal(&deep);
    let not_deep = format!("{}userName pr{}", "not (".repeat(200), ")".repeat(200));
    refusal(&not_deep);
    let chain = vec!["userName pr"; 20_000].join(" and ");
    match filter::parse(&chain).expect("a long flat chain parses") {
        Logic::And(terms) => assert_eq!(terms.len(), 20_000),
        other => panic!("expected one flat and: {other:?}"),
    }
}

#[test]
fn the_printed_form_parses_back_to_the_same_filter() {
    for text in figure_2() {
        let parsed = filter::parse(&text).expect("parses");
        let printed = parsed.to_string();
        assert_eq!(
            filter::parse(&printed).expect("the printed form parses"),
            parsed,
            "{text} printed as {printed}"
        );
    }
}

#[test]
fn the_ferroehr_subset_admits_eq_and_over_its_three_attributes() {
    // #5014: `eq` and `and` over `userName`, `externalId` and `id`.
    let support =
        FilterSupport::equality_conjunction(urn::CORE_USER, &["userName", "externalId", "id"])
            .expect("valid attribute names");
    for admitted in [
        "userName eq \"bjensen\"",
        "USERNAME Eq \"bjensen\"",
        "externalId eq \"0a21f0f2-8d2a-4f8e-bf98-7363c4aed4ef\" and id eq \"x\"",
        "urn:ietf:params:scim:schemas:core:2.0:User:userName eq \"bjensen\"",
        "(userName eq \"a\")",
    ] {
        support
            .check(&filter::parse(admitted).expect("parses"))
            .unwrap_or_else(|e| panic!("{admitted}: {e}"));
    }
    for (refused, detail) in [
        ("userName co \"b\"", "The operator 'co' is not supported."),
        ("userName pr", "The operator 'pr' is not supported."),
        (
            "userName eq \"a\" or id eq \"b\"",
            "The operator 'or' is not supported.",
        ),
        (
            "not (userName eq \"a\")",
            "The operator 'not' is not supported.",
        ),
        (
            "title eq \"x\"",
            "The attribute 'title' is not supported in a filter.",
        ),
        (
            "emails[type eq \"work\"]",
            "The attribute 'emails' is not supported in a filter.",
        ),
        (
            "name.givenName eq \"x\"",
            "The attribute 'name.givenName' is not supported in a filter.",
        ),
        (
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:userName eq \"x\"",
            "The attribute 'urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:userName' is not supported in a filter.",
        ),
    ] {
        let error = support
            .check(&filter::parse(refused).expect("parses"))
            .expect_err(refused);
        assert_eq!(
            error.scim_type(),
            Some(ScimType::InvalidFilter),
            "{refused}"
        );
        assert_eq!(error.detail(), detail, "{refused}");
    }
}

#[test]
fn ordering_a_boolean_is_invalid_filter() {
    // RFC 7644 §3.4.2.2 Table 3: "Boolean and Binary attributes SHALL cause a
    // failed response (HTTP status code 400) with "scimType" of "invalidFilter"."
    let schema = user_schema();
    for bad in [
        "active gt true",
        "active le false",
        "x509Certificates.value ge \"AA==\"",
    ] {
        let error = filter::parse(bad)
            .expect("parses")
            .validate(&schema)
            .expect_err(bad);
        assert_eq!(error.scim_type(), Some(ScimType::InvalidFilter), "{bad}");
    }
    for bad in [
        "userName eq 5",
        "active eq \"true\"",
        "name eq \"x\"",
        "meta.lastModified gt \"yesterday\"",
    ] {
        let error = filter::parse(bad)
            .expect("parses")
            .validate(&schema)
            .expect_err(bad);
        assert_eq!(error.scim_type(), Some(ScimType::InvalidFilter), "{bad}");
    }
    for good in [
        "active eq true",
        "userName gt \"a\"",
        "meta.lastModified gt \"2011-05-13T04:42:34Z\"",
        "undefined pr",
    ] {
        filter::parse(good)
            .expect("parses")
            .validate(&schema)
            .unwrap_or_else(|e| panic!("{good}: {e}"));
    }
}

#[test]
fn filters_evaluate_over_the_full_user_representation() {
    let schema = user_schema();
    let mut full = source::json(Doc::Rfc7643, 1916..=2110, &[]);
    unwrap_certificate(&mut full);
    let user = held(&full, &schema);
    for (expression, expected) in [
        ("userName eq \"BJENSEN@example.com\"", true),
        ("id eq \"2819C223-7F76-453A-919D-413861904646\"", false),
        ("id eq \"2819c223-7f76-453a-919d-413861904646\"", true),
        ("name.familyName co \"ens\"", true),
        ("name.familyName co \"O'Malley\"", false),
        ("userName sw \"bjen\" and userName ew \"COM\"", true),
        ("title pr", true),
        ("nickName pr and not (userType eq \"Employee\")", false),
        ("userType ne \"Employee\"", false),
        (
            "emails[type eq \"work\" and value co \"@example.com\"]",
            true,
        ),
        (
            "emails[type eq \"home\" and value co \"@example.com\"]",
            false,
        ),
        ("emails co \"jensen.org\"", true),
        ("emails.type eq \"home\"", true),
        ("meta.lastModified gt \"2011-05-13T04:42:33Z\"", true),
        ("meta.lastModified gt \"2011-05-13T04:42:34Z\"", false),
        ("meta.created le \"2010-01-23T04:56:22Z\"", true),
        ("active eq true", true),
        ("timezone eq null", false),
        ("profileUrl eq \"https://login.example.com/bjensen\"", true),
        ("undefinedAttribute pr", false),
        ("undefinedAttribute eq null", true),
        (
            "schemas eq \"urn:ietf:params:scim:schemas:core:2.0:User\"",
            true,
        ),
    ] {
        let filter = filter::parse(expression).expect("parses");
        assert_eq!(
            filter.matches(&user, &schema).expect("evaluates"),
            expected,
            "{expression}"
        );
    }
}

#[test]
fn an_extension_attribute_evaluates_by_its_qualified_name() {
    let schema = user_schema();
    let mut enterprise = source::json(Doc::Rfc7643, 2140..=2323, &[]);
    unwrap_certificate(&mut enterprise);
    let user = held(&enterprise, &schema);
    for (expression, expected) in [
        (
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:employeeNumber eq \"701984\"",
            true,
        ),
        (
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:manager.value pr",
            true,
        ),
        (
            "schemas eq \"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User\"",
            true,
        ),
        // RFC 7644 §3.10: a client "SHOULD fully qualify extended attributes"; a unique unqualified name resolves.
        ("employeeNumber pr", true),
    ] {
        let filter = filter::parse(expression).expect("parses");
        assert_eq!(
            filter.matches(&user, &schema).expect("evaluates"),
            expected,
            "{expression}"
        );
    }
}

/// Removes the line breaks RFC 7643 §8.2 and §8.3 print inside the base64
/// certificate, which RFC 4648 §3.3 does not allow inside the encoding.
pub(crate) fn unwrap_certificate(user: &mut serde_json::Value) {
    let value = &mut user["x509Certificates"][0]["value"];
    let unwrapped: String = value
        .as_str()
        .expect("a certificate string")
        .split_whitespace()
        .collect();
    *value = serde_json::Value::String(unwrapped);
}

#[test]
fn the_elided_figure_2_user_is_a_valid_representation() {
    // RFC 7643 §3 Figure 2 omits values ("...") and so leaves a trailing comma.
    let figure = source::json(
        Doc::Rfc7643,
        799..=832,
        &[Fix::Elisions, Fix::TrailingCommas],
    );
    let user = held(&figure, &user_schema());
    let filter = filter::parse(
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:costCenter eq \"4130\"",
    )
    .expect("parses");
    assert!(filter.matches(&user, &user_schema()).expect("evaluates"));
}
