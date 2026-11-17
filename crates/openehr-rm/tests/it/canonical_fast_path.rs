// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::expect_used,
    reason = "integration-test assertions in helpers outside #[test] fns, which the clippy.toml \
              allow-*-in-tests scoping does not reach"
)]

//! The fast paths of #5682 write the bytes the writer before them wrote: the
//! byte scan of a string with its escapes (RFC 8785 §3.2.2.2), the integer
//! digits (§3.2.2.3 and Appendix B) and the byte order of names (§3.2.3),
//! held against `canonical_reference` over a corpus weighted to every escape
//! class, non-ASCII text, surrogate pairs and the number edge cases.

use proptest::prelude::*;
use serde_json::{Number, Value, json};

use crate::canonical_reference;

/// Asserts both generations write `value` as the reference does.
fn agrees_with_the_reference(value: &Value) {
    let reference = canonical_reference::canonical(value);
    let stable =
        openehr_rm::v1_1::common::change_control::canonical_form_impl::canonical_json(value)
            .expect("a JSON value canonicalises");
    let current =
        openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json(value)
            .expect("a JSON value canonicalises");
    assert_eq!(current, reference, "{value}");
    assert_eq!(stable, reference, "{value}");
}

/// A character from every class the writer treats apart: each control
/// character, the quote and the reverse solidus, plain ASCII, the solidus and
/// DEL (never escaped), Latin-1, U+2028 and U+2029, the BMP below and above
/// the surrogate block, and supplementary characters, which UTF-16 writes as
/// a surrogate pair.
pub(crate) fn any_char() -> impl Strategy<Value = char> {
    prop_oneof![
        (0_u32..0x20).prop_map(|code| char::from_u32(code).expect("a control character")),
        prop::sample::select(vec!['"', '\\', '/', '\u{7f}', '\u{2028}', '\u{2029}']),
        prop::char::range(' ', '~'),
        prop::char::range('\u{80}', '\u{ff}'),
        prop::char::range('\u{100}', '\u{d7ff}'),
        prop::char::range('\u{e000}', '\u{ffff}'),
        prop::char::range('\u{1_0000}', '\u{10_ffff}'),
        any::<char>(),
    ]
}

/// A string of characters from [`any_char`], with long ASCII runs among them.
pub(crate) fn any_text() -> impl Strategy<Value = String> {
    prop_oneof![
        proptest::collection::vec(any_char(), 0..24).prop_map(|chars| chars.into_iter().collect()),
        "[ -~]{0,64}",
        any::<String>(),
    ]
}

/// The integers around the bounds a number takes a different path at: zero,
/// 2^53 where integers stop being exact doubles, and the ends of `i64` and
/// `u64`.
fn edge_integer() -> impl Strategy<Value = Value> {
    let exact = 1_i64 << 53;
    prop_oneof![
        (-4_i64..=4).prop_map(Value::from),
        (-4_i64..=4).prop_map(move |delta| Value::from(exact + delta)),
        (-4_i64..=4).prop_map(move |delta| Value::from(-exact + delta)),
        Just(Value::from(i64::MIN)),
        Just(Value::from(i64::MAX)),
        Just(Value::from(u64::MAX)),
        (0_u64..=4).prop_map(move |delta| Value::from(u64::MAX - delta)),
    ]
}

/// Any finite number: every `i64`, `u64` and finite double, the edge integers,
/// and the integral doubles a stored float carries.
pub(crate) fn any_number() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<u64>()
            .prop_filter_map("a finite double", |bits| Number::from_f64(f64::from_bits(
                bits
            )))
            .prop_map(Value::Number),
        any::<f64>()
            .prop_filter_map("a finite double", Number::from_f64)
            .prop_map(Value::Number),
        prop_oneof![
            any::<f64>().prop_map(f64::trunc),
            any::<i32>().prop_map(f64::from),
            any::<i32>().prop_map(|n| f64::from(n) + 9_007_199_254_740_992.0),
        ]
        .prop_filter_map("an integral double", Number::from_f64)
        .prop_map(Value::Number),
        edge_integer(),
    ]
}

/// Any JSON value over [`any_text`] names and strings and [`any_number`].
pub(crate) fn any_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any_number(),
        any_text().prop_map(Value::String),
    ];
    leaf.prop_recursive(5, 64, 8, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            proptest::collection::vec((any_text(), inner), 0..8)
                .prop_map(|members| Value::Object(members.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8192))]

    #[test]
    fn a_generated_value_is_written_as_the_reference_writes_it(value in any_value()) {
        agrees_with_the_reference(&value);
    }

    #[test]
    fn a_generated_string_is_written_as_the_reference_writes_it(text in any_text()) {
        agrees_with_the_reference(&Value::String(text));
    }

    #[test]
    fn generated_names_sort_as_the_reference_sorts_them(
        names in proptest::collection::vec(any_text(), 0..16),
    ) {
        let object = names.into_iter().map(|name| (name, Value::Null)).collect();
        agrees_with_the_reference(&Value::Object(object));
    }
}

#[test]
fn every_escape_class_is_written_as_the_reference_writes_it() {
    for byte in 0_u8..0x80 {
        let alone = char::from(byte).to_string();
        let framed = format!("ab{alone}cd{alone}");
        agrees_with_the_reference(&json!([alone, framed]));
    }
}

#[test]
fn a_surrogate_pair_is_one_character_and_a_lone_surrogate_never_parses() {
    // RFC 8785 §3.2.2.2 writes a character above U+FFFF as itself; the JSON
    // text carries it as a surrogate pair.
    let pair: Value = serde_json::from_str(r#""a😀b""#).expect("a surrogate pair parses");
    assert_eq!(pair, json!("a\u{1f600}b"));
    agrees_with_the_reference(&pair);
    // RFC 8785 §3.2.2.2 requires valid Unicode; a lone surrogate never reaches
    // the writer because it has no Rust `str`.
    assert!(serde_json::from_str::<Value>(r#""\ud800""#).is_err());
    assert!(serde_json::from_str::<Value>(r#""\udc00x""#).is_err());
}

#[test]
fn names_whose_order_utf_16_reverses_sort_as_the_reference_sorts_them() {
    // U+E000 to U+FFFF sort after a supplementary character in UTF-16 and
    // before it in UTF-8 (RFC 8785 §3.2.3).
    for prefix in ["", "a", "\u{e9}", "\u{fb33}"] {
        let names = [
            format!("{prefix}\u{e000}"),
            format!("{prefix}\u{ffff}"),
            format!("{prefix}\u{1_0000}"),
            format!("{prefix}\u{1_f600}"),
            format!("{prefix}\u{10_ffff}"),
            format!("{prefix}~"),
            prefix.to_owned(),
        ];
        let object = names.into_iter().map(|name| (name, Value::Null)).collect();
        agrees_with_the_reference(&Value::Object(object));
    }
}

#[test]
fn the_appendix_b_numbers_and_the_integer_bounds_are_written_as_the_reference_writes_them() {
    // RFC 8785 Appendix B, Table 1, by its IEEE 754 bits.
    let bits = [
        0x0000_0000_0000_0000_u64,
        0x8000_0000_0000_0000,
        0x0000_0000_0000_0001,
        0x8000_0000_0000_0001,
        0x7fef_ffff_ffff_ffff,
        0xffef_ffff_ffff_ffff,
        0x4340_0000_0000_0000,
        0xc340_0000_0000_0000,
        0x4430_0000_0000_0000,
        0x44b5_2d02_c7e1_4af5,
        0x44b5_2d02_c7e1_4af6,
        0x44b5_2d02_c7e1_4af7,
        0x444b_1ae4_d6e2_ef4e,
        0x444b_1ae4_d6e2_ef4f,
        0x444b_1ae4_d6e2_ef50,
        0x3eb0_c6f7_a0b5_ed8c,
        0x3eb0_c6f7_a0b5_ed8d,
        0x41b3_de43_5555_5553,
        0x41b3_de43_5555_5554,
        0x41b3_de43_5555_5555,
        0x41b3_de43_5555_5556,
        0x41b3_de43_5555_5557,
        0xbecb_f647_612f_3696,
        0x4314_3ff3_c1cb_0959,
    ];
    for bits in bits {
        let number = Number::from_f64(f64::from_bits(bits)).expect("a finite double");
        agrees_with_the_reference(&Value::Number(number));
    }
    // An integer takes the form of its double (Appendix B note 2): every
    // integer near the 2^53 bound, small ones and the ends of `i64` and `u64`.
    let exact = 1_i64 << 53;
    for delta in -16..=16 {
        agrees_with_the_reference(&json!([exact + delta, -exact + delta]));
        agrees_with_the_reference(&json!((exact + delta).unsigned_abs()));
    }
    for integer in [
        0_i64,
        1,
        -1,
        9,
        10,
        -10,
        99,
        100,
        999_999_999,
        i64::MIN,
        i64::MAX,
    ] {
        agrees_with_the_reference(&json!(integer));
    }
    for integer in [
        u64::MAX,
        u64::MAX - 1,
        1 << 63,
        10_u64.pow(19),
        10_u64.pow(16),
    ] {
        agrees_with_the_reference(&json!(integer));
    }
}
