// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Properties of the filter grammar: the parser never panics on any input, a
//! printed filter parses back to itself, and the case of an operator never
//! changes what a filter means (RFC 7644 §3.4.2.2).

use ferroehr_rest::scim::protocol::filter::{
    self, AttrExp, CompValue, CompareOp, Filter, Logic, Term, Test, ValueFilter,
};
use ferroehr_rest::scim::protocol::path::{AttrName, AttrPath};
use proptest::prelude::*;

fn name() -> impl Strategy<Value = String> {
    prop_oneof!["[a-zA-Z][a-zA-Z0-9_$-]{0,8}", Just("$ref".to_owned())]
}

fn attr_name() -> impl Strategy<Value = AttrName> {
    name().prop_map(|n| AttrName::parse(&n).expect("the strategy yields valid names"))
}

fn attr_path() -> impl Strategy<Value = AttrPath> {
    (
        proptest::option::of(prop_oneof![
            Just("urn:ietf:params:scim:schemas:core:2.0:User".to_owned()),
            Just("urn:ietf:params:scim:schemas:extension:enterprise:2.0:User".to_owned()),
        ]),
        attr_name(),
        proptest::option::of(attr_name()),
    )
        .prop_map(|(schema, name, sub_attr)| AttrPath {
            schema,
            name,
            sub_attr,
        })
}

fn operator() -> impl Strategy<Value = CompareOp> {
    prop_oneof![
        Just(CompareOp::Eq),
        Just(CompareOp::Ne),
        Just(CompareOp::Co),
        Just(CompareOp::Sw),
        Just(CompareOp::Ew),
        Just(CompareOp::Gt),
        Just(CompareOp::Lt),
        Just(CompareOp::Ge),
        Just(CompareOp::Le),
    ]
}

fn value() -> impl Strategy<Value = CompValue> {
    prop_oneof![
        Just(CompValue::True),
        Just(CompValue::False),
        Just(CompValue::Null),
        any::<i64>().prop_map(|n| CompValue::Number(n.into())),
        (-1.0e6f64..1.0e6)
            .prop_map(|f| CompValue::Number(serde_json::Number::from_f64(f).expect("finite"))),
        any::<String>().prop_map(CompValue::String),
    ]
}

fn test() -> impl Strategy<Value = Test> {
    prop_oneof![
        Just(Test::Present),
        (operator(), value()).prop_map(|(op, v)| Test::Compare(op, v))
    ]
}

fn logic<T: std::fmt::Debug + Clone + 'static>(
    leaf: impl Strategy<Value = T> + 'static,
) -> impl Strategy<Value = Logic<T>> {
    leaf.prop_map(Logic::Leaf)
        .prop_recursive(4, 24, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 2..4).prop_map(Logic::And),
                proptest::collection::vec(inner.clone(), 2..4).prop_map(Logic::Or),
                inner.prop_map(|f| Logic::Not(Box::new(f))),
            ]
        })
}

fn value_filter() -> impl Strategy<Value = ValueFilter> {
    logic((attr_name(), test()).prop_map(|(path, test)| AttrExp { path, test }))
}

fn any_filter() -> impl Strategy<Value = Filter> {
    let term = prop_oneof![
        3 => (attr_path(), test()).prop_map(|(path, test)| Term::Attr(AttrExp { path, test })),
        1 => (attr_path().prop_map(|p| AttrPath { sub_attr: None, ..p }), value_filter()).prop_map(|(p, f)| Term::ValuePath(p, f)),
    ];
    logic(term)
}

/// Collapses directly nested `and` (or `or`) into one, the flat form the
/// parser yields for a chain of the same operator, inside value filters too.
fn flat(filter: Filter) -> Filter {
    flat_logic(filter, &|term| match term {
        Term::ValuePath(path, inner) => Term::ValuePath(path, flat_logic(inner, &|leaf| leaf)),
        attr @ Term::Attr(_) => attr,
    })
}

fn flat_logic<T>(logic: Logic<T>, leaf: &dyn Fn(T) -> T) -> Logic<T> {
    match logic {
        Logic::And(terms) => Logic::And(
            terms
                .into_iter()
                .map(|t| flat_logic(t, leaf))
                .flat_map(|t| match t {
                    Logic::And(inner) => inner,
                    other => vec![other],
                })
                .collect(),
        ),
        Logic::Or(terms) => Logic::Or(
            terms
                .into_iter()
                .map(|t| flat_logic(t, leaf))
                .flat_map(|t| match t {
                    Logic::Or(inner) => inner,
                    other => vec![other],
                })
                .collect(),
        ),
        Logic::Not(inner) => Logic::Not(Box::new(flat_logic(*inner, leaf))),
        Logic::Leaf(term) => Logic::Leaf(leaf(term)),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn the_parser_never_panics(input in any::<String>()) {
        let _parsed = filter::parse(&input);
    }

    #[test]
    fn the_parser_never_panics_on_filter_shaped_noise(input in r#"[a-zA-Z0-9 ()\[\]".:$_-]{0,64}"#) {
        let _parsed = filter::parse(&input);
    }

    #[test]
    fn a_printed_filter_parses_back_to_itself(original in any_filter()) {
        let printed = original.to_string();
        let parsed = filter::parse(&printed);
        prop_assert!(parsed.is_ok(), "{printed}: {parsed:?}");
        prop_assert_eq!(parsed.expect("checked"), flat(original), "{}", printed);
    }

    #[test]
    fn operator_case_never_changes_the_filter(original in any_filter(), seed in any::<u64>()) {
        let printed = original.to_string();
        let shouted = shout_keywords(&printed, seed);
        prop_assert_eq!(filter::parse(&shouted).ok(), filter::parse(&printed).ok(), "{}", shouted);
    }
}

/// Upper-cases, by the bits of `seed`, each word outside a string literal
/// that spells an operator; a word that is an attribute name keeps its meaning
/// too, since attribute names are case-insensitive (RFC 7643 §2.1).
fn shout_keywords(text: &str, mut seed: u64) -> String {
    const KEYWORDS: [&str; 13] = [
        "eq", "ne", "co", "sw", "ew", "gt", "lt", "ge", "le", "pr", "and", "or", "not",
    ];
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let mut in_string = false;
    let mut escaped = false;
    let flush = |word: &mut String, out: &mut String, seed: &mut u64| {
        if KEYWORDS.iter().any(|k| word.eq_ignore_ascii_case(k)) {
            *seed = seed.rotate_left(7) ^ 0x9e37_79b9_7f4a_7c15;
            if seed.is_multiple_of(2) {
                *word = word.to_ascii_uppercase();
            }
        }
        out.push_str(word);
        word.clear();
    };
    for c in text.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c.is_ascii_alphabetic() {
            word.push(c);
        } else {
            flush(&mut word, &mut out, &mut seed);
            out.push(c);
            in_string = c == '"';
        }
    }
    flush(&mut word, &mut out, &mut seed);
    out
}
