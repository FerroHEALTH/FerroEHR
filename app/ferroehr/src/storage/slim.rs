// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The stored form of a node fragment: what a `node` row keeps of its
//! canonical JSON fragment once the row's own columns hold the rest.
//!
//! A fragment repeats facts its row already carries as columns: `_type`
//! (`rm_type`), a lowercase `archetype_node_id` (`archetype`), a plain or
//! simply coded `name` (`name`, `name_code`, `name_terminology`), and on an
//! `ELEMENT` a `value` of one of the common data value shapes, which the typed
//! value columns hold instead. Below the root, the `archetype_details` and
//! `archetype_node_id` an archetype root repeats, and the `language` and
//! `encoding` every entry repeats, move to the [`Header`] columns. [`slim`]
//! removes exactly those keys, only when the columns rebuild them byte for
//! byte, and keeps everything else verbatim; [`rebuild`] and its SQL twins
//! `ext.node_name`, `ext.node_value` and `ext.node_fragment` put the removed
//! keys back on read. A fragment with nothing left is stored as SQL `NULL`.
//!
//! No openEHR spec governs storage layout: our own design (RM common
//! `master06-change_control_package.adoc` §Overview admits any physical
//! representation). The canonical JSON is ITS-JSON and does not change.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::{Map, Value};

use crate::storage::header::{Header, code_phrase_of, restore, slim_header};

/// An `ELEMENT.value` held in the typed value columns instead of the
/// fragment.
///
/// Which column carries what depends on [`TypedValue::value_type`]; the
/// mapping is the one `ext.node_value` rebuilds from:
///
/// | type | `num` | `text` | `code` | `terminology` |
/// |---|---|---|---|---|
/// | `DV_QUANTITY` | magnitude | units | | |
/// | `DV_COUNT` | magnitude | | | |
/// | `DV_ORDINAL` | value | symbol value | symbol code | symbol terminology |
/// | `DV_CODED_TEXT` | | value | code | terminology |
/// | `DV_IDENTIFIER` | | id | | |
/// | `DV_BOOLEAN` | | `true` or `false` | | |
/// | the string-valued types | | value | | |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedValue {
    /// The RM type of the value (`DV_QUANTITY`, …), never an alias.
    pub value_type: String,
    /// The number, in the lexical form the canonical JSON carried it in.
    pub num: Option<String>,
    /// The text.
    pub text: Option<String>,
    /// The code string of a coded value.
    pub code: Option<String>,
    /// The terminology id of a coded value.
    pub terminology: Option<String>,
}

/// What a row stores of its fragment.
#[derive(Debug, Clone, PartialEq)]
pub struct Slim {
    /// The keys no column rebuilds, `None` when nothing remains.
    pub data: Option<Value>,
    /// The `ELEMENT.value` the typed columns hold, if any.
    pub value: Option<TypedValue>,
    /// The header keys the row's columns hold, empty on a root row.
    pub header: Header,
}

/// The data value types whose whole value is one string under `value`.
pub const STRING_VALUED: [&str; 7] = [
    "DV_TEXT",
    "DV_DATE_TIME",
    "DV_DATE",
    "DV_TIME",
    "DV_DURATION",
    "DV_URI",
    "DV_EHR_URI",
];

/// Splits a node fragment into what the row stores and what its columns
/// rebuild.
///
/// `rm_type` is the row's `rm_type` column (the fragment's `_type`), and
/// `root` whether the row is the versioned object's root (`num = 0`), whose
/// header keys stay in its fragment.
#[must_use]
pub fn slim(rm_type: &str, root: bool, data: Value) -> Slim {
    let Value::Object(mut map) = data else {
        return Slim {
            data: Some(data),
            value: None,
            header: Header::default(),
        };
    };
    if map.get("_type").and_then(Value::as_str) == Some(rm_type) {
        map.shift_remove("_type");
    }
    let header = if root {
        Header::default()
    } else {
        slim_header(&mut map)
    };
    if map
        .get("archetype_node_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id == id.to_ascii_lowercase())
    {
        map.shift_remove("archetype_node_id");
    }
    if map.get("name").is_some_and(rebuildable_name) {
        map.shift_remove("name");
    }
    let mut value = None;
    if rm_type == "ELEMENT"
        && let Some(typed) = map.get("value").and_then(typed_value)
    {
        map.shift_remove("value");
        value = Some(typed);
    }
    Slim {
        data: (!map.is_empty()).then_some(Value::Object(map)),
        value,
        header,
    }
}

/// The columns of a `node` row that [`rebuild`] puts back into its fragment.
#[derive(Debug, Clone, Copy)]
pub struct Columns<'a> {
    /// `rm_type`, the fragment's `_type`.
    pub rm_type: &'a str,
    /// `archetype`, the lowercase `archetype_node_id`.
    pub archetype: Option<&'a str>,
    /// `name`, the name's value.
    pub name: Option<&'a str>,
    /// `name_code`, a coded name's code string.
    pub name_code: Option<&'a str>,
    /// `name_terminology`, a coded name's terminology id.
    pub name_terminology: Option<&'a str>,
    /// `rm_version`, set when `archetype_details` and `archetype_node_id`
    /// are rebuilt from `archetype` ([`Header::rm_version`]).
    pub rm_version: Option<&'a str>,
    /// `language`, the code of the `language` the fragment omits.
    pub language: Option<&'a str>,
    /// `encoding`, the code of the `encoding` the fragment omits.
    pub encoding: Option<&'a str>,
}

/// The whole canonical fragment of a stored row: what it stores, with every
/// key its columns hold put back.
///
/// The inverse of [`slim`], and the Rust twin of `ext.node_fragment`, which an
/// integration test pins to the same fragments. `None` when a typed number is
/// not a JSON number, which only a damaged row can hold.
#[must_use]
pub fn rebuild(
    data: Option<Value>,
    columns: &Columns<'_>,
    value: Option<&TypedValue>,
) -> Option<Value> {
    let mut map = match data {
        Some(Value::Object(map)) => map,
        Some(other) => return Some(other),
        None => Map::new(),
    };
    restore(
        &mut map,
        columns.archetype,
        &Header {
            rm_version: columns.rm_version.map(str::to_owned),
            language: columns.language.map(str::to_owned),
            encoding: columns.encoding.map(str::to_owned),
        },
    );
    map.insert("_type".to_owned(), Value::from(columns.rm_type));
    if let (Some(archetype), false) = (columns.archetype, map.contains_key("archetype_node_id")) {
        map.insert("archetype_node_id".to_owned(), Value::from(archetype));
    }
    if let (Some(name), false) = (columns.name, map.contains_key("name")) {
        let rebuilt = match (columns.name_code, columns.name_terminology) {
            (Some(code), Some(terminology)) => coded_text(name, code, terminology),
            _ => object([
                ("_type", Value::from("DV_TEXT")),
                ("value", Value::from(name)),
            ]),
        };
        map.insert("name".to_owned(), rebuilt);
    }
    if let Some(value) = value {
        map.insert("value".to_owned(), rebuild_value(value)?);
    }
    Some(Value::Object(map))
}

/// The canonical JSON of a typed value, as `ext.node_value` builds it.
fn rebuild_value(v: &TypedValue) -> Option<Value> {
    let text = || v.text.clone().map_or(Value::Null, Value::from);
    let num = || -> Option<Value> {
        let parsed: Value = serde_json::from_str(v.num.as_deref()?).ok()?;
        parsed.is_number().then_some(parsed)
    };
    let kind = Value::from(v.value_type.as_str());
    let coded = || {
        coded_text(
            v.text.as_deref().unwrap_or_default(),
            v.code.as_deref().unwrap_or_default(),
            v.terminology.as_deref().unwrap_or_default(),
        )
    };
    Some(match v.value_type.as_str() {
        "DV_QUANTITY" => object([("_type", kind), ("magnitude", num()?), ("units", text())]),
        "DV_COUNT" => object([("_type", kind), ("magnitude", num()?)]),
        "DV_CODED_TEXT" => coded(),
        "DV_ORDINAL" => object([("_type", kind), ("value", num()?), ("symbol", coded())]),
        "DV_IDENTIFIER" => object([("_type", kind), ("id", text())]),
        "DV_BOOLEAN" => object([
            ("_type", kind),
            ("value", Value::from(v.text.as_deref() == Some("true"))),
        ]),
        _ => object([("_type", kind), ("value", text())]),
    })
}

/// A `DV_CODED_TEXT` of exactly `value` and a `defining_code`.
fn coded_text(value: &str, code: &str, terminology: &str) -> Value {
    object([
        ("_type", Value::from("DV_CODED_TEXT")),
        ("value", Value::from(value)),
        ("defining_code", code_phrase_of(code, terminology)),
    ])
}

/// A JSON object of `fields`.
pub(crate) fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

/// Whether `name` is a `DV_TEXT` or a simply coded `DV_CODED_TEXT` the name
/// columns rebuild exactly.
fn rebuildable_name(name: &Value) -> bool {
    let Some(fields) = name.as_object() else {
        return false;
    };
    match fields.get("_type").and_then(Value::as_str) {
        Some("DV_TEXT") => has_keys(fields, &["_type", "value"]) && is_string(fields, "value"),
        Some("DV_CODED_TEXT") => coded(fields).is_some(),
        _ => false,
    }
}

/// The typed form of an `ELEMENT.value`, when it has one of the shapes the
/// typed columns rebuild exactly.
fn typed_value(value: &Value) -> Option<TypedValue> {
    let fields = value.as_object()?;
    let value_type = fields.get("_type").and_then(Value::as_str)?;
    let typed = |num: Option<String>, text: Option<String>, coding: Option<(String, String)>| {
        let (code, terminology) = coding.unzip();
        TypedValue {
            value_type: value_type.to_owned(),
            num,
            text,
            code,
            terminology,
        }
    };
    match value_type {
        t if STRING_VALUED.contains(&t) => (has_keys(fields, &["_type", "value"])
            && is_string(fields, "value"))
        .then(|| typed(None, string(fields, "value"), None)),
        "DV_BOOLEAN" => {
            let flag = fields.get("value").and_then(Value::as_bool)?;
            has_keys(fields, &["_type", "value"]).then(|| typed(None, Some(flag.to_string()), None))
        }
        "DV_IDENTIFIER" => (has_keys(fields, &["_type", "id"]) && is_string(fields, "id"))
            .then(|| typed(None, string(fields, "id"), None)),
        "DV_COUNT" => has_keys(fields, &["_type", "magnitude"])
            .then(|| number(fields, "magnitude"))
            .flatten()
            .map(|n| typed(Some(n), None, None)),
        "DV_QUANTITY" => (has_keys(fields, &["_type", "magnitude", "units"])
            && is_string(fields, "units"))
        .then(|| number(fields, "magnitude"))
        .flatten()
        .map(|n| typed(Some(n), string(fields, "units"), None)),
        "DV_CODED_TEXT" => {
            coded(fields).map(|(text, coding)| typed(None, Some(text), Some(coding)))
        }
        "DV_ORDINAL" => {
            if !has_keys(fields, &["_type", "symbol", "value"]) {
                return None;
            }
            let ordinal = number(fields, "value")?;
            let symbol = fields.get("symbol").and_then(Value::as_object)?;
            if symbol.get("_type").and_then(Value::as_str) != Some("DV_CODED_TEXT") {
                return None;
            }
            let (text, coding) = coded(symbol)?;
            Some(typed(Some(ordinal), Some(text), Some(coding)))
        }
        _ => None,
    }
}

/// The value and the `(code_string, terminology_id)` of a `DV_CODED_TEXT`
/// that carries nothing but those: `_type`, `value` and a `defining_code`
/// with its `_type`, `code_string` and a `terminology_id` of `_type` and
/// `value`.
fn coded(fields: &Map<String, Value>) -> Option<(String, (String, String))> {
    if !has_keys(fields, &["_type", "defining_code", "value"]) {
        return None;
    }
    let text = string(fields, "value")?;
    let code = fields.get("defining_code").and_then(Value::as_object)?;
    if !has_keys(code, &["_type", "code_string", "terminology_id"])
        || code.get("_type").and_then(Value::as_str) != Some("CODE_PHRASE")
    {
        return None;
    }
    let code_string = string(code, "code_string")?;
    let terminology = code.get("terminology_id").and_then(Value::as_object)?;
    if !has_keys(terminology, &["_type", "value"])
        || terminology.get("_type").and_then(Value::as_str) != Some("TERMINOLOGY_ID")
    {
        return None;
    }
    Some((text, (code_string, string(terminology, "value")?)))
}

/// Whether `fields` carries exactly `keys`, in any order.
pub(crate) fn has_keys(fields: &Map<String, Value>, keys: &[&str]) -> bool {
    fields.len() == keys.len() && keys.iter().all(|key| fields.contains_key(*key))
}

/// Whether `fields[key]` is a JSON string.
fn is_string(fields: &Map<String, Value>, key: &str) -> bool {
    fields.get(key).is_some_and(Value::is_string)
}

/// `fields[key]` when it is a JSON string.
pub(crate) fn string(fields: &Map<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// `fields[key]` in its JSON lexical form when it is a JSON number.
fn number(fields: &Map<String, Value>, key: &str) -> Option<String> {
    fields
        .get(key)
        .and_then(|value| value.as_number())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::codec::decompose;
    use crate::storage::header::jsonb_key_order;
    use serde_json::json;

    /// What one corpus pass counted.
    #[derive(Debug, Default)]
    struct Tally {
        rows: usize,
        empty: usize,
        typed: usize,
        archetyped: usize,
        language: usize,
        encoding: usize,
    }

    /// `value` with every object's keys in jsonb order, as PostgreSQL hands a
    /// stored `data` back.
    fn jsonb_ordered(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut entries: Vec<(String, Value)> = map
                    .iter()
                    .map(|(key, value)| (key.clone(), jsonb_ordered(value)))
                    .collect();
                entries.sort_by(|(a, _), (b, _)| jsonb_key_order(a, b));
                Value::Object(entries.into_iter().collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(jsonb_ordered).collect()),
            other => other.clone(),
        }
    }

    /// The fragment as a read served it before the header columns: every key
    /// `data` held in jsonb order, then the keys the name, archetype and value
    /// columns append, in the order [`rebuild`] appends them.
    fn served_before(
        fragment: &Value,
        rebuilt: &Value,
        data: Option<&Value>,
        header: &Header,
    ) -> String {
        let held = |key: &str| data.and_then(|d| d.get(key)).is_some();
        let appended: Vec<&str> = ["_type", "archetype_node_id", "name", "value"]
            .into_iter()
            .filter(|key| {
                fragment.get(*key).is_some()
                    && !held(key)
                    && !(*key == "archetype_node_id" && header.rm_version.is_some())
            })
            .collect();
        let mut kept = fragment.clone();
        if let Some(map) = kept.as_object_mut() {
            for key in &appended {
                map.shift_remove(*key);
            }
        }
        let Value::Object(mut out) = jsonb_ordered(&kept) else {
            panic!("a node fragment is an object");
        };
        for key in appended {
            out.insert(key.to_owned(), rebuilt[key].clone());
        }
        serde_json::to_string(&Value::Object(out)).expect("serializable")
    }

    /// Every row of `composition` rebuilds to the fragment the codec cut, and
    /// serializes with the keys in the order a read served before the header
    /// columns existed.
    fn assert_round_trip(composition: Value, tally: &mut Tally) {
        let rows = decompose(composition).expect("the composition decomposes");
        for row in &rows {
            let stored = slim(&row.rm_type, row.num == 0, row.data.clone());
            let columns = Columns {
                rm_type: &row.rm_type,
                archetype: row.archetype.as_deref(),
                name: row.name.as_deref(),
                name_code: row.name_code.as_deref(),
                name_terminology: row.name_terminology.as_deref(),
                rm_version: stored.header.rm_version.as_deref(),
                language: stored.header.language.as_deref(),
                encoding: stored.header.encoding.as_deref(),
            };
            let from_db = stored.data.as_ref().map(jsonb_ordered);
            let rebuilt = rebuild(from_db.clone(), &columns, stored.value.as_ref())
                .expect("a stored row rebuilds");
            assert_eq!(rebuilt, row.data, "row {} at {:?}", row.num, row.path);
            assert_eq!(
                serde_json::to_string(&rebuilt).expect("serializable"),
                served_before(&row.data, &rebuilt, from_db.as_ref(), &stored.header),
                "key order of row {} at {:?}",
                row.num,
                row.path
            );
            tally.rows += 1;
            tally.empty += usize::from(stored.data.is_none());
            tally.typed += usize::from(stored.value.is_some());
            tally.archetyped += usize::from(stored.header.rm_version.is_some());
            tally.language += usize::from(stored.header.language.is_some());
            tally.encoding += usize::from(stored.header.encoding.is_some());
        }
    }

    /// The JSON files of a corpus directory, relative to the crate.
    fn corpus(dir: &str, suffix: &str) -> Vec<Value> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .expect("the vendored corpus is readable")
            .map(|entry| entry.expect("a directory entry").path())
            .filter(|path| path.to_string_lossy().ends_with(suffix))
            .collect();
        files.sort();
        files
            .into_iter()
            .filter_map(|path| {
                let raw = std::fs::read_to_string(&path).expect("a corpus file is readable");
                serde_json::from_str::<Value>(&raw).ok()
            })
            .collect()
    }

    #[test]
    fn every_vendored_example_composition_round_trips() {
        let examples = corpus("../../corpus/templates/ckm", ".example.json");
        assert!(
            examples.len() >= 10,
            "found only {} example compositions",
            examples.len()
        );
        let mut tally = Tally::default();
        for composition in examples {
            assert_round_trip(composition, &mut tally);
        }
        let Tally {
            rows,
            empty,
            typed,
            archetyped,
            language,
            encoding,
        } = tally;
        assert!(
            empty * 4 > rows,
            "only {empty} of {rows} rows store no fragment"
        );
        assert!(
            typed * 10 > rows,
            "only {typed} of {rows} rows hold a typed value"
        );
        // The CKM examples carry 625 non-root archetype roots, 19 of them with
        // an id the recasing does not give back, and 270 entries.
        assert!(
            archetyped >= 600,
            "only {archetyped} archetype headers moved"
        );
        assert!(
            language >= 270 && encoding >= 270,
            "{language} languages, {encoding} encodings"
        );
    }

    #[test]
    fn the_openehr_sdk_composition_corpus_round_trips() {
        let compositions: Vec<Value> = corpus(
            "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json",
            ".json",
        )
        .into_iter()
        .filter(|c| c.get("_type").and_then(Value::as_str) == Some("COMPOSITION"))
        .collect();
        assert!(
            compositions.len() >= 50,
            "found only {}",
            compositions.len()
        );
        let mut tally = Tally::default();
        for composition in compositions {
            assert_round_trip(composition, &mut tally);
        }
        assert!(tally.archetyped > 0 && tally.language > 0, "{tally:?}");
    }

    #[test]
    fn the_keys_the_columns_hold_are_left_out() {
        let stored = slim(
            "ELEMENT",
            false,
            json!({"_type": "ELEMENT", "archetype_node_id": "at0004",
                "name": {"_type": "DV_TEXT", "value": "Systolic"},
                "value": {"_type": "DV_QUANTITY", "magnitude": 120.0, "units": "mm[Hg]"}}),
        );
        assert_eq!(stored.data, None);
        assert_eq!(
            stored.value,
            Some(TypedValue {
                value_type: "DV_QUANTITY".to_owned(),
                num: Some("120.0".to_owned()),
                text: Some("mm[Hg]".to_owned()),
                code: None,
                terminology: None,
            })
        );
    }

    #[test]
    fn what_no_column_rebuilds_exactly_stays_verbatim() {
        // An archetype HRID keeps its casing, a name with a mapping and a
        // quantity with a precision are not the plain shapes, and a null
        // flavour is not a value.
        let fragment = json!({"_type": "ELEMENT",
            "archetype_node_id": "openEHR-EHR-ELEMENT.x.v1",
            "name": {"_type": "DV_TEXT", "value": "x", "mappings": []},
            "value": {"_type": "DV_QUANTITY", "magnitude": 1, "units": "1", "precision": 0},
            "null_flavour": {"_type": "DV_CODED_TEXT", "value": "unknown"}});
        let stored = slim("ELEMENT", false, fragment.clone());
        let mut expected = fragment;
        expected.as_object_mut().map(|m| m.shift_remove("_type"));
        assert_eq!(stored.data, Some(expected));
        assert_eq!(stored.value, None);
    }

    #[test]
    fn a_value_off_an_element_is_never_typed() {
        let stored = slim(
            "CLUSTER",
            false,
            json!({"_type": "CLUSTER", "value": {"_type": "DV_TEXT", "value": "x"}}),
        );
        assert_eq!(stored.value, None);
        assert_eq!(
            stored.data,
            Some(json!({"value": {"_type": "DV_TEXT", "value": "x"}}))
        );
    }

    #[test]
    fn a_boolean_and_an_ordinal_are_typed() {
        let flag = slim(
            "ELEMENT",
            false,
            json!({"_type": "ELEMENT",
            "value": {"_type": "DV_BOOLEAN", "value": false}}),
        );
        assert_eq!(flag.value.map(|v| v.text), Some(Some("false".to_owned())));
        let ordinal = slim(
            "ELEMENT",
            false,
            json!({"_type": "ELEMENT",
            "value": {"_type": "DV_ORDINAL", "value": 2, "symbol": {"_type": "DV_CODED_TEXT",
                "value": "moderate", "defining_code": {"_type": "CODE_PHRASE",
                "code_string": "at0003", "terminology_id": {"_type": "TERMINOLOGY_ID",
                "value": "local"}}}}}),
        );
        let ordinal = ordinal.value.expect("a typed ordinal");
        assert_eq!(ordinal.num.as_deref(), Some("2"));
        assert_eq!(ordinal.text.as_deref(), Some("moderate"));
        assert_eq!(ordinal.code.as_deref(), Some("at0003"));
        assert_eq!(ordinal.terminology.as_deref(), Some("local"));
    }
}
