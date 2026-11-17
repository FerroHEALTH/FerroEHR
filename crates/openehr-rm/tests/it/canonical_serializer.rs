// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::expect_used,
    reason = "integration-test assertions in helpers outside #[test] fns, which the clippy.toml \
              allow-*-in-tests scoping does not reach"
)]

//! The canonical serializer of #5849 writes the bytes of
//! `canonical_json(&serde_json::to_value(x))` for every serde shape: JSON
//! values over the corpus of the #5682 fast-path tests, typed values with
//! every primitive, option, variant, tuple, map key and raw value, a name
//! given twice, and the top-level members a caller leaves out.

use std::collections::BTreeMap;

use proptest::prelude::*;
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::canonical_fast_path::{any_text, any_value};
use openehr_rm::v1_1::common::change_control::canonical_serializer_impl as stable;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::{
    canonical_json, canonical_object,
};
use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::{
    self as current, CanonicalObject,
};

/// The canonical form of `value` through a `serde_json::Value`, as before.
fn through_value<T: Serialize + ?Sized>(value: &T) -> Result<String, serde_json::Error> {
    canonical_json(&serde_json::to_value(value)?)
}

/// Asserts both generations of the serializer write `value` as the
/// `Value` path does, and fail where it fails.
fn agrees<T: Serialize + ?Sized + std::fmt::Debug>(value: &T) {
    let reference = through_value(value);
    let written = current::canonical_to_string(value);
    let stable_written = stable::canonical_to_string(value);
    if let Ok(reference) = reference {
        assert_eq!(
            written.expect("the serializer writes it"),
            reference,
            "{value:?}"
        );
        assert_eq!(
            stable_written.expect("the serializer writes it"),
            reference,
            "{value:?}"
        );
    } else {
        assert!(
            written.is_err(),
            "the serializer refuses what to_value refuses: {value:?}"
        );
        assert!(
            stable_written.is_err(),
            "both generations refuse it: {value:?}"
        );
    }
}

/// A unit-only enum.
#[derive(Debug, Clone, Serialize)]
enum Unit {
    A,
    B,
}

/// An enum of every variant kind.
#[derive(Debug, Clone, Serialize)]
enum Shape {
    Unit,
    Newtype(String),
    Tuple(i32, Option<bool>),
    Struct { zeta: u8, alpha: Vec<i16> },
}

/// A newtype struct.
#[derive(Debug, Clone, Serialize)]
struct Wrapped(i64);

/// A unit struct.
#[derive(Debug, Clone, Serialize)]
struct Marker;

/// A tuple struct.
#[derive(Debug, Clone, Serialize)]
struct Pair(char, f32);

/// A record with every primitive and compound serde shape, its fields
/// declared out of name order.
#[derive(Debug, Clone, Serialize)]
struct Record {
    zulu: String,
    i8: i8,
    i16: i16,
    i32: i32,
    i64: i64,
    i128: i128,
    u8: u8,
    u16: u16,
    u32: u32,
    u64: u64,
    u128: u128,
    f32: f32,
    f64: f64,
    character: char,
    flag: bool,
    nothing: Option<u8>,
    something: Option<String>,
    unit: (),
    marker: Marker,
    wrapped: Wrapped,
    pair: Pair,
    tuple: (u8, String, Unit),
    shapes: Vec<Shape>,
    by_int: BTreeMap<i64, String>,
    by_bool: BTreeMap<bool, u8>,
    by_char: BTreeMap<char, u8>,
    by_unit: BTreeMap<String, Unit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped: Option<u8>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

fn any_shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        Just(Shape::Unit),
        any_text().prop_map(Shape::Newtype),
        (any::<i32>(), any::<Option<bool>>()).prop_map(|(n, b)| Shape::Tuple(n, b)),
        (any::<u8>(), proptest::collection::vec(any::<i16>(), 0..4))
            .prop_map(|(zeta, alpha)| Shape::Struct { zeta, alpha }),
    ]
}

fn any_unit() -> impl Strategy<Value = Unit> {
    prop_oneof![Just(Unit::A), Just(Unit::B)]
}

/// Any 128-bit integer, weighted to the 64-bit ranges `to_value` accepts and
/// the edges past them.
fn any_i128() -> impl Strategy<Value = i128> {
    prop_oneof![
        any::<i64>().prop_map(i128::from),
        any::<u64>().prop_map(i128::from),
        Just(i128::from(u64::MAX) + 1),
        Just(i128::from(i64::MIN) - 1),
        any::<i128>(),
    ]
}

fn any_u128() -> impl Strategy<Value = u128> {
    prop_oneof![
        any::<u64>().prop_map(u128::from),
        Just(u128::from(u64::MAX) + 1),
        any::<u128>()
    ]
}

/// Any float, the non-finite ones included, which `to_value` writes as null.
fn any_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        any::<f64>(),
        Just(f64::NAN),
        Just(f64::INFINITY),
        Just(f64::NEG_INFINITY),
        any::<i32>().prop_map(f64::from),
    ]
}

fn any_record() -> impl Strategy<Value = Record> {
    let numbers = (
        any::<i8>(),
        any::<i16>(),
        any::<i32>(),
        any::<i64>(),
        any_i128(),
        any::<u8>(),
        any::<u16>(),
        any::<u32>(),
        any::<u64>(),
        any_u128(),
        any::<f32>(),
        any_f64(),
    );
    let others = (
        any_text(),
        any::<char>(),
        any::<bool>(),
        any::<Option<u8>>(),
        proptest::option::of(any_text()),
        any::<i64>(),
        (any::<char>(), any::<f32>()),
        (any::<u8>(), any_text(), any_unit()),
        proptest::collection::vec(any_shape(), 0..4),
    );
    let maps = (
        proptest::collection::btree_map(any::<i64>(), any_text(), 0..4),
        proptest::collection::btree_map(any::<bool>(), any::<u8>(), 0..2),
        proptest::collection::btree_map(any::<char>(), any::<u8>(), 0..4),
        proptest::collection::btree_map(any_text(), any_unit(), 0..4),
        any::<Option<u8>>(),
        proptest::collection::btree_map("[a-z_]{1,8}", any_value(), 0..3),
    );
    (numbers, others, maps).prop_map(|(numbers, others, maps)| {
        let (i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, f32, f64) = numbers;
        let (zulu, character, flag, nothing, something, wrapped, pair, tuple, shapes) = others;
        let (by_int, by_bool, by_char, by_unit, skipped, extra) = maps;
        Record {
            zulu,
            i8,
            i16,
            i32,
            i64,
            i128,
            u8,
            u16,
            u32,
            u64,
            u128,
            f32,
            f64,
            character,
            flag,
            nothing,
            something,
            unit: (),
            marker: Marker,
            wrapped: Wrapped(wrapped),
            pair: Pair(pair.0, pair.1),
            tuple,
            shapes,
            by_int,
            by_bool,
            by_char,
            by_unit,
            skipped,
            extra,
        }
    })
}

/// A map that gives some names twice, in the order given.
#[derive(Debug)]
struct Repeated(Vec<(String, Value)>);

impl Serialize for Repeated {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in &self.0 {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

/// A map whose keys are floats, which `serde_json` writes as their text.
#[derive(Debug)]
struct FloatKeys(Vec<f64>);

impl Serialize for FloatKeys {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for key in &self.0 {
            map.serialize_entry(key, &0)?;
        }
        map.end()
    }
}

/// A byte string.
#[derive(Debug)]
struct Bytes(Vec<u8>);

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8192))]

    #[test]
    fn a_generated_value_is_written_as_the_value_path_writes_it(value in any_value()) {
        agrees(&value);
    }

    #[test]
    fn a_generated_record_is_written_as_the_value_path_writes_it(record in any_record()) {
        agrees(&record);
    }

    #[test]
    fn a_name_given_twice_keeps_its_last_value(
        members in proptest::collection::vec(("[ab]{1,2}", any_value()), 0..8),
    ) {
        agrees(&Repeated(members));
    }

    #[test]
    fn float_keys_are_written_as_serde_json_writes_them(
        keys in proptest::collection::vec(any_f64(), 0..4),
    ) {
        agrees(&FloatKeys(keys));
    }

    #[test]
    fn bytes_are_an_array_of_numbers(bytes in proptest::collection::vec(any::<u8>(), 0..16)) {
        agrees(&Bytes(bytes));
    }

    #[test]
    fn a_left_out_member_is_absent_and_the_rest_is_written_whole(
        value in any_value(),
        skip in proptest::collection::vec(any_text(), 0..3),
    ) {
        let skip: Vec<&str> = skip.iter().map(String::as_str).collect();
        let reference = match &value {
            Value::Object(members) => canonical_object(
                members
                    .iter()
                    .filter(|(name, _)| !skip.contains(&name.as_str()))
                    .map(|(name, member)| (name.as_str(), member)),
            ),
            other => canonical_json(other),
        }
        .expect("a JSON value canonicalises");
        let written = current::canonical_object_without(&value, &skip).expect("it writes");
        prop_assert_eq!(written, reference);
    }
}

#[test]
fn a_raw_json_value_is_written_as_its_canonical_form() {
    for text in [
        r#"{"b": [1, 2.50, "xA"], "a": null}"#,
        "1e21",
        r#""😀""#,
        "[]",
    ] {
        let raw = RawValue::from_string(text.to_owned()).expect("a raw JSON value");
        agrees(&raw);
        let nested: BTreeMap<&str, &RawValue> = [("raw", raw.as_ref())].into_iter().collect();
        agrees(&nested);
    }
}

#[test]
fn a_key_that_is_no_string_is_refused_as_to_value_refuses_it() {
    let mut by_tuple = BTreeMap::new();
    by_tuple.insert((1_u8, 2_u8), 0_u8);
    agrees(&by_tuple);
    let mut by_option = BTreeMap::new();
    by_option.insert(Some(1_u8), 0_u8);
    agrees(&by_option);
    agrees(&FloatKeys(vec![f64::NAN]));
}

#[test]
fn a_member_left_out_is_left_out_only_at_the_top_level() {
    let value = serde_json::json!({
        "link": { "link": 1, "keep": 2 },
        "items": [{ "link": 3 }],
        "proof": "p",
        "uid": "u",
    });
    let written = current::canonical_object_without(&value, &["link", "proof"]).expect("it writes");
    assert_eq!(written, r#"{"items":[{"link":3}],"uid":"u"}"#);
    let variant = Shape::Struct {
        zeta: 1,
        alpha: vec![2],
    };
    assert_eq!(
        current::canonical_object_without(&variant, &["Struct"]).expect("it writes"),
        "{}"
    );
    assert_eq!(
        current::canonical_object_without(&Shape::Tuple(1, None), &["Tuple"]).expect("it writes"),
        "{}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// An object assembled member by member, some from a value and some
    /// from a canonical form written before, in any order and with names
    /// given twice, is the canonical form of the object its last values make.
    #[test]
    fn an_assembled_object_is_the_canonical_form_of_its_members(
        members in proptest::collection::vec(("[a-c]{1,2}", any_value(), any::<bool>()), 0..8),
    ) {
        let mut assembled = CanonicalObject::default();
        let mut expected = serde_json::Map::new();
        for (name, value, written_before) in &members {
            if *written_before {
                let text = current::canonical_to_string(value).expect("it writes");
                assembled.member_canonical(name, &text);
            } else {
                assembled.member(name, value).expect("it writes");
            }
            expected.insert(name.clone(), value.clone());
        }
        prop_assert_eq!(assembled.is_empty(), members.is_empty());
        let reference = canonical_json(&Value::Object(expected)).expect("it canonicalises");
        prop_assert_eq!(assembled.finish().expect("it assembles"), reference);
    }
}
