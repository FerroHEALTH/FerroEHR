// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::expect_used,
    reason = "integration-test assertions in helpers outside #[test] fns, which the clippy.toml \
              allow-*-in-tests scoping does not reach"
)]

//! The RFC 8785 canonical form `VERSION.canonical_form()` signs (#5220): the
//! samples of RFC 8785 §3.2 and Appendix B, and a differential test holding
//! it byte-equal to the `serde_jcs` crate over the vendored corpora and a
//! generated corpus, on both emitted generations.

use std::path::{Path, PathBuf};

use proptest::prelude::*;
use serde_json::{Value, json};

/// The canonical form of `value` under both generations, which must agree.
fn canonical(value: &Value) -> String {
    let stable =
        openehr_rm::v1_1::common::change_control::canonical_form_impl::canonical_json(value)
            .expect("a JSON value canonicalises");
    let current =
        openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json(value)
            .expect("a JSON value canonicalises");
    assert_eq!(stable, current, "the generation twins agree");
    current
}

/// Asserts the canonical form of `value` is the one `serde_jcs` writes, alone
/// and as a version whose `signature` the canonical form leaves out.
fn agrees_with_the_oracle(value: &Value, origin: &str) {
    let oracle = serde_jcs::to_string(value).expect("serde_jcs canonicalises");
    assert_eq!(canonical(value), oracle, "{origin}");
    let mut version = json!({ "_type": "ORIGINAL_VERSION", "signature": "sig", "data": value });
    let form =
        openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json(&version)
            .expect("a version canonicalises");
    if let Some(object) = version.as_object_mut() {
        object.remove("signature");
    }
    assert_eq!(
        form,
        serde_jcs::to_string(&version).expect("serde_jcs canonicalises"),
        "{origin} as a version"
    );
    let envelope = json!({ "_type": "ORIGINAL_VERSION", "signature": "sig" });
    let joined =
        openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json_with_data(
            &envelope, value,
        )
        .expect("a version canonicalises");
    let expected = if value.is_null() {
        serde_jcs::to_string(&json!({ "_type": "ORIGINAL_VERSION" }))
    } else {
        serde_jcs::to_string(&version)
    }
    .expect("serde_jcs canonicalises");
    assert_eq!(joined, expected, "{origin} joined by reference");
}

#[test]
fn the_rfc_8785_primitive_sample_canonicalises_to_its_published_bytes() {
    // RFC 8785 §3.2.2, the parsed sample: its numbers as written, and its
    // string as the code points its escapes denote.
    let numbers: Value = serde_json::from_str(
        "[333333333.33333329, 1E30, 4.50, 2e-3, 0.000000000000000000000000001]",
    )
    .expect("the numbers parse");
    let string: String = [
        0x20ac, 0x24, 0x0f, 0x0a, 0x41, 0x27, 0x42, 0x22, 0x5c, 0x5c, 0x22, 0x2f,
    ]
    .into_iter()
    .map(|code| char::from_u32(code).expect("a Unicode scalar"))
    .collect();
    let sample = json!({ "numbers": numbers, "string": string, "literals": [null, true, false] });
    let form = canonical(&sample);
    // RFC 8785 §3.2.3, the canonical form of the sample.
    assert_eq!(
        form,
        r#"{"literals":[null,true,false],"numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27],"string":"€$\u000f\nA'B\"\\\\\"/"}"#
    );
    // RFC 8785 §3.2.4, its UTF-8 bytes.
    let published = "7b 22 6c 69 74 65 72 61 6c 73 22 3a 5b 6e 75 6c 6c 2c 74 72 \
        75 65 2c 66 61 6c 73 65 5d 2c 22 6e 75 6d 62 65 72 73 22 3a \
        5b 33 33 33 33 33 33 33 33 33 2e 33 33 33 33 33 33 33 2c 31 \
        65 2b 33 30 2c 34 2e 35 2c 30 2e 30 30 32 2c 31 65 2d 32 37 \
        5d 2c 22 73 74 72 69 6e 67 22 3a 22 e2 82 ac 24 5c 75 30 30 \
        30 66 5c 6e 41 27 42 5c 22 5c 5c 5c 5c 5c 22 2f 22 7d";
    let bytes: Vec<u8> = published
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("a hex byte"))
        .collect();
    assert_eq!(form.as_bytes(), bytes.as_slice());
}

#[test]
fn members_sort_by_their_utf_16_code_units() {
    // RFC 8785 §3.2.3, the sorting sample, each name as the code points its
    // escapes denote, and its expected value order.
    let names = [
        (&[0x20ac][..], "Euro Sign"),
        (&[0x0d], "Carriage Return"),
        (&[0xfb33], "Hebrew Letter Dalet With Dagesh"),
        (&[0x31], "One"),
        (&[0x1_f600], "Emoji: Grinning Face"),
        (&[0x80], "Control"),
        (&[0xf6], "Latin Small Letter O With Diaeresis"),
    ];
    let sample = Value::Object(
        names
            .iter()
            .map(|(codes, value)| {
                let name: String = codes
                    .iter()
                    .map(|code| char::from_u32(*code).expect("a Unicode scalar"))
                    .collect();
                (name, Value::from(*value))
            })
            .collect(),
    );
    let form = canonical(&sample);
    let order = [
        "Carriage Return",
        "One",
        "Control",
        "Latin Small Letter O With Diaeresis",
        "Euro Sign",
        "Emoji: Grinning Face",
        "Hebrew Letter Dalet With Dagesh",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|value| form.find(value).expect("every value is written"))
        .collect();
    assert!(positions.is_sorted(), "{form}");
}

#[test]
fn numbers_take_the_ecmascript_form_of_appendix_b() {
    // RFC 8785 Appendix B, Table 1: the IEEE 754 bits and their JSON form.
    let samples = [
        (0x0000_0000_0000_0000_u64, "0"),
        (0x8000_0000_0000_0000, "0"),
        (0x0000_0000_0000_0001, "5e-324"),
        (0x8000_0000_0000_0001, "-5e-324"),
        (0x7fef_ffff_ffff_ffff, "1.7976931348623157e+308"),
        (0xffef_ffff_ffff_ffff, "-1.7976931348623157e+308"),
        (0x4340_0000_0000_0000, "9007199254740992"),
        (0xc340_0000_0000_0000, "-9007199254740992"),
        (0x4430_0000_0000_0000, "295147905179352830000"),
        (0x44b5_2d02_c7e1_4af5, "9.999999999999997e+22"),
        (0x44b5_2d02_c7e1_4af6, "1e+23"),
        (0x44b5_2d02_c7e1_4af7, "1.0000000000000001e+23"),
        (0x444b_1ae4_d6e2_ef4e, "999999999999999700000"),
        (0x444b_1ae4_d6e2_ef4f, "999999999999999900000"),
        (0x444b_1ae4_d6e2_ef50, "1e+21"),
        (0x3eb0_c6f7_a0b5_ed8c, "9.999999999999997e-7"),
        (0x3eb0_c6f7_a0b5_ed8d, "0.000001"),
        (0x41b3_de43_5555_5553, "333333333.3333332"),
        (0x41b3_de43_5555_5554, "333333333.33333325"),
        (0x41b3_de43_5555_5555, "333333333.3333333"),
        (0x41b3_de43_5555_5556, "333333333.3333334"),
        (0x41b3_de43_5555_5557, "333333333.33333343"),
        (0xbecb_f647_612f_3696, "-0.0000033333333333333333"),
        (0x4314_3ff3_c1cb_0959, "1424953923781206.2"),
    ];
    for (bits, expected) in samples {
        let number = serde_json::Number::from_f64(f64::from_bits(bits)).expect("a finite double");
        assert_eq!(canonical(&Value::Number(number)), expected, "{bits:016x}");
    }
    // NaN and Infinity have no JSON number (Appendix B note 3), so no value
    // carries one to the canonicaliser.
    assert!(serde_json::Number::from_f64(f64::NAN).is_none());
    assert!(serde_json::Number::from_f64(f64::INFINITY).is_none());
    // An integer takes the form of its double (Appendix B note 2).
    assert_eq!(canonical(&json!(u64::MAX)), "18446744073709552000");
    assert_eq!(canonical(&json!(i64::MIN)), "-9223372036854776000");
}

#[test]
fn every_control_character_takes_its_escape() {
    let text: String = (0_u8..0x20)
        .map(char::from)
        .chain(['"', '\\', '/', '\u{7f}'])
        .collect();
    agrees_with_the_oracle(&Value::String(text), "the control characters");
}

/// Every `.json` file under `dir`, sorted.
fn json_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("a corpus directory")
        .map(|entry| entry.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            json_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            found.push(path);
        }
    }
}

#[test]
fn the_vendored_corpora_canonicalise_as_the_oracle_does() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    json_files(&root.join("corpus"), &mut files);
    json_files(&root.join("crates/openehr-its/tests/vendor"), &mut files);
    let mut compared = 0_usize;
    for path in &files {
        let text = std::fs::read_to_string(path).expect("a corpus file");
        // NOTE: a corpus file that is no JSON document (a JSON Lines log) is
        // outside what a canonical form is computed over.
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        agrees_with_the_oracle(&value, &path.display().to_string());
        compared += 1;
    }
    assert!(compared >= 300, "only {compared} corpus documents compared");
}

/// Any JSON value, with every Unicode scalar in its strings and names and
/// every finite double, `i64` and `u64` among its numbers.
fn any_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>()
            .prop_filter_map("a finite double", serde_json::Number::from_f64)
            .prop_map(Value::Number),
        any::<String>().prop_map(Value::String),
    ];
    leaf.prop_recursive(5, 64, 8, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            proptest::collection::vec((any::<String>(), inner), 0..8)
                .prop_map(|members| Value::Object(members.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn a_generated_value_canonicalises_as_the_oracle_does(value in any_value()) {
        agrees_with_the_oracle(&value, &value.to_string());
    }
}
