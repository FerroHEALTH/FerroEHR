// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An ODIN terminology code in an instance value, read as the RM `CODE_PHRASE`
//! it denotes (#5817).
//!
//! A `_default` is an instance of the RM (`ADL2/master06-default_values.adoc`
//! §Syntax), and an RM coded value carries its code as a `CODE_PHRASE` (RM
//! `data_types` §Text package): `terminology_id`, a `TERMINOLOGY_ID` whose
//! `value` is `name(version)` when a version is stated (BASE
//! `base_types/master05-identification_package.adoc` §`TERMINOLOGY_ID`), and
//! `code_string`. The ODIN form is `TERM_CODE_REF`, `[name(version)::code]`
//! (`base_lexer.g4`).

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use openehr_base::prelude::TerminologyCode;
use serde_json::{Map, Value, json};

/// The canonical JSON of the `CODE_PHRASE` an ODIN term code denotes.
pub(crate) fn to_json(code: &TerminologyCode) -> Value {
    let terminology = match &code.terminology_version {
        Some(version) => format!("{}({version})", code.terminology_id),
        None => code.terminology_id.clone(),
    };
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
        "code_string": code.code_string,
    })
}

/// The ODIN term code of a canonical-JSON `CODE_PHRASE` that carries nothing
/// beside its terminology and code, so the term code states all of it.
pub(crate) fn literal(m: &Map<String, Value>) -> Option<String> {
    if m.get("_type").and_then(Value::as_str) != Some("CODE_PHRASE") || m.len() != 3 {
        return None;
    }
    let terminology = m
        .get("terminology_id")
        .and_then(Value::as_object)
        .filter(|id| {
            id.len() == 2 && id.get("_type").and_then(Value::as_str) == Some("TERMINOLOGY_ID")
        })
        .and_then(|id| id.get("value"))
        .and_then(Value::as_str)?;
    let code = m.get("code_string").and_then(Value::as_str)?;
    Some(format!("[{terminology}::{code}]"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{literal, to_json};
    use crate::odin::parse_term_code;

    #[test]
    fn a_term_code_is_a_code_phrase_and_back() {
        for raw in ["[snomed_ct::163033001]", "[snomed_ct(2024-01)::163033001]"] {
            let value = to_json(&parse_term_code(raw));
            let serde_json::Value::Object(m) = &value else {
                panic!("a CODE_PHRASE object");
            };
            assert_eq!(literal(m).as_deref(), Some(raw));
        }
        assert_eq!(
            to_json(&parse_term_code("[snomed_ct(2024-01)::163033001]")),
            json!({"_type": "CODE_PHRASE",
                   "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "snomed_ct(2024-01)"},
                   "code_string": "163033001"})
        );
    }

    #[test]
    fn a_code_phrase_with_more_than_its_code_keeps_the_object_form() {
        let with_term = json!({"_type": "CODE_PHRASE",
            "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "snomed_ct"},
            "code_string": "163033001", "preferred_term": "lying BP"});
        let serde_json::Value::Object(m) = &with_term else {
            panic!("an object");
        };
        assert_eq!(literal(m), None);
    }
}
