// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(clippy::expect_used, reason = "test assertions/diagnostics/fixtures")]
//! The one-pass scan of canonical-JSON text for an undeclared key gives the
//! verdict of the walk over the parsed value, on the corpus as it stands and
//! with a key planted on its tagged nodes.

use openehr_its::json::{holds_undeclared_key, reject_undeclared_keys};
use serde_json::{Value, json};

use crate::common::corpus_files;

/// The walk's verdict on `value` and the scan's on its text, which must agree.
fn verdicts_agree(value: &Value) -> bool {
    let walked = reject_undeclared_keys(value).is_err();
    let scanned = holds_undeclared_key(&value.to_string()).expect("the text is JSON");
    assert_eq!(scanned, walked, "on {value}");
    walked
}

/// Plants `key` on the `nth` tagged object of `value` in document order.
fn plant(value: &mut Value, nth: &mut usize, key: &str) -> bool {
    match value {
        Value::Object(members) => {
            if members.contains_key("_type") {
                if *nth == 0 {
                    members.insert(key.to_owned(), json!(1));
                    return true;
                }
                *nth -= 1;
            }
            members.values_mut().any(|child| plant(child, nth, key))
        }
        Value::Array(items) => items.iter_mut().any(|child| plant(child, nth, key)),
        _ => false,
    }
}

#[test]
fn the_scan_agrees_with_the_walk_over_the_corpus() {
    let mut documents = 0;
    let mut refused = 0;
    for path in corpus_files() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        documents += 1;
        refused += usize::from(verdicts_agree(&value));
        assert_eq!(
            holds_undeclared_key(&text).expect("the corpus text is JSON"),
            reject_undeclared_keys(&value).is_err(),
            "on the text of {}",
            path.display()
        );
        for step in [0, 1, 7, 40, 200] {
            let mut planted = value.clone();
            let mut nth = step;
            if plant(&mut planted, &mut nth, "undeclared_by_any_class") {
                refused += usize::from(verdicts_agree(&planted));
            }
        }
    }
    assert!(documents > 50, "the corpus holds {documents} documents");
    assert!(refused > 100, "only {refused} planted keys were refused");
}

#[test]
fn the_scan_reads_a_tag_wherever_and_whatever_it_is() {
    let cases = [
        json!({ "value": "x", "wrong": 1, "_type": "DV_TEXT" }),
        json!({ "_type": "DV_TEXT", "value": "x" }),
        json!({ "_type": 7, "wrong": 1 }),
        json!({ "_type": null, "wrong": 1 }),
        json!({ "_type": { "_type": "DV_TEXT", "wrong": 1 } }),
        json!({ "_type": [{ "_type": "DV_TEXT", "wrong": 1 }] }),
        json!({ "_type": "NO_SUCH_CLASS", "wrong": 1 }),
        json!([[{ "a": { "_type": "CODE_PHRASE", "wrong": [] } }]]),
        json!({ "a": [1, 2.5, true, null, "s", {}] }),
        json!("text"),
    ];
    let refused = cases.iter().filter(|case| verdicts_agree(case)).count();
    assert_eq!(refused, 4);
    // A key written with an escape is the key it spells.
    let escaped = r#"{"_type":"DV_TEXT","value":"x","wrong":1}"#;
    assert!(holds_undeclared_key(escaped).expect("JSON"));
    let declared = r#"{"_type":"DV_TEXT","value":"x"}"#;
    assert!(!holds_undeclared_key(declared).expect("JSON"));
    // A repeated tag is read as its last occurrence, as a parsed value holds it.
    let repeated = r#"{"_type":"DV_TEXT","wrong":1,"_type":"NO_SUCH_CLASS"}"#;
    assert!(!holds_undeclared_key(repeated).expect("JSON"));
    assert!(holds_undeclared_key("{} trailing").is_err());
    assert!(holds_undeclared_key("{").is_err());
}
