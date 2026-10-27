// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR_STATUS` edit.
//!
//! The draft of `other_details` is read, and the three edited fields are
//! merged into the served document, every other attribute sent back verbatim (RM `docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc`
//! §`EHR_STATUS`).

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use serde_json::Value;

#[cfg(not(target_arch = "wasm32"))]
use crate::error::ViewerError;
use crate::locale::records::RecordsMsg;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::tr;
use crate::locale::tr_with;

/// Read an `other_details` draft: `None` for a blank draft (the attribute is
/// removed), `Some(value)` for a JSON object.
///
/// `EHR_STATUS.other_details` is an `ITEM_STRUCTURE` `0..1` (RM
/// `docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc` §`EHR_STATUS`), so
/// a non-object can never be valid and is refused here, before anything is
/// sent.
///
/// # Errors
/// The operator-facing complaint when the draft is not parseable JSON or not a
/// JSON object.
pub fn parse_other_details(draft: &str) -> Result<Option<Value>, String> {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(trimmed).map_err(|e| {
        tr_with(
            RecordsMsg::OtherDetailsNotValidJson,
            &[("e", (e).to_string().into())],
        )
    })?;
    if value.is_object() {
        Ok(Some(value))
    } else {
        Err(
            "other_details must be a JSON object: an ITEM_STRUCTURE such as \
             {\"_type\": \"ITEM_TREE\", …}"
                .to_owned(),
        )
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Apply the form's three edits to the loaded `EHR_STATUS` document and return
/// the body to PUT.
///
/// `base` is re-sent verbatim apart from `is_queryable`, `is_modifiable` and
/// `other_details` — the merge is a key replacement on the parsed object, never
/// a rebuild, so every other attribute (`subject`, `name`,
/// `archetype_node_id`, `uid`, anything a newer spec release adds) survives
/// unchanged. A blank `other_details` REMOVES the key, which is the only way to
/// express "no other details" for an optional attribute.
///
/// # Errors
/// [`ViewerError::Invalid`] when `base` is not a JSON object (the CDR would
/// answer `422` anyway — `EHR_STATUS` must be a JSON object per
/// RM `docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc` §`EHR_STATUS`), when the `other_details` draft
/// is not a JSON object, or when the merged document cannot be re-serialized.
pub fn apply_status_edits(
    base: &str,
    is_queryable: bool,
    is_modifiable: bool,
    other_details: &str,
) -> Result<String, ViewerError> {
    let mut doc: Value = serde_json::from_str(base).map_err(|e| {
        ViewerError::Invalid(tr_with(
            RecordsMsg::LoadedEhrStatusDocumentNotValid,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let details = parse_other_details(other_details).map_err(ViewerError::Invalid)?;
    let object = doc.as_object_mut().ok_or_else(|| {
        ViewerError::Invalid({ tr(RecordsMsg::LoadedEhrStatusDocumentNotJson) }.clone())
    })?;
    drop(object.insert("is_queryable".to_owned(), Value::Bool(is_queryable)));
    drop(object.insert("is_modifiable".to_owned(), Value::Bool(is_modifiable)));
    match details {
        Some(details) => drop(object.insert("other_details".to_owned(), details)),
        None => drop(object.remove("other_details")),
    }
    serde_json::to_string(&doc).map_err(|e| {
        ViewerError::Invalid(tr_with(
            RecordsMsg::EditedEhrStatusCouldNotSerialized,
            &[("e", (e).to_string().into())],
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::parse_other_details;

    #[test]
    fn a_blank_other_details_draft_removes_the_attribute() {
        assert_eq!(parse_other_details(""), Ok(None));
        assert_eq!(parse_other_details("   \n  "), Ok(None));
    }

    #[test]
    fn an_item_structure_object_is_accepted() {
        let draft = r#"{"_type":"ITEM_TREE","archetype_node_id":"at0001","items":[]}"#;
        let parsed = parse_other_details(draft).expect("an object draft is accepted");
        let value = parsed.expect("some value");
        assert_eq!(value["_type"], "ITEM_TREE");
    }

    #[test]
    fn a_non_object_or_malformed_draft_is_refused_before_any_round_trip() {
        // ITEM_STRUCTURE is an object; an array or scalar can never be valid.
        for draft in ["[]", "\"ITEM_TREE\"", "42", "true", "null"] {
            let message = parse_other_details(draft).expect_err("a non-object draft is refused");
            assert!(message.contains("must be a JSON object"), "{message}");
        }
        let message = parse_other_details("{\"_type\":").expect_err("malformed JSON is refused");
        assert!(message.contains("not valid JSON"), "{message}");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod merge_tests {
    use super::apply_status_edits;
    use serde_json::Value;

    /// The loaded document, carrying attributes the form does not edit.
    const BASE: &str = r#"{
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "name": {"_type": "DV_TEXT", "value": "EHR status"},
        "uid": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.org::1"},
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": "demographic",
                "type": "PERSON",
                "id": {"_type": "GENERIC_ID", "value": "p-42", "scheme": "local"}
            }
        },
        "is_queryable": true,
        "is_modifiable": true,
        "other_details": {"_type": "ITEM_TREE", "items": []}
    }"#;

    #[test]
    fn the_three_edited_fields_change_and_everything_else_survives() {
        let merged = apply_status_edits(
            BASE,
            false,
            true,
            r#"{"_type":"ITEM_TREE","archetype_node_id":"at0002","items":[]}"#,
        )
        .expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&merged).expect("merged JSON");
        assert_eq!(doc["is_queryable"], Value::Bool(false));
        assert_eq!(doc["is_modifiable"], Value::Bool(true));
        assert_eq!(doc["other_details"]["archetype_node_id"], "at0002");
        // Untouched attributes travel back verbatim — the whole point of
        // merging into the served document instead of rebuilding it.
        assert_eq!(doc["_type"], "EHR_STATUS");
        assert_eq!(doc["name"]["value"], "EHR status");
        assert_eq!(
            doc["archetype_node_id"],
            "openEHR-EHR-EHR_STATUS.generic.v1"
        );
        assert_eq!(doc["uid"]["value"], "8849182c::example.org::1");
        assert_eq!(doc["subject"]["external_ref"]["id"]["value"], "p-42");
        assert_eq!(doc["subject"]["external_ref"]["namespace"], "demographic");
    }

    #[test]
    fn a_blank_draft_removes_other_details_and_keeps_the_rest() {
        let merged = apply_status_edits(BASE, true, false, "").expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&merged).expect("merged JSON");
        assert!(doc.get("other_details").is_none());
        assert_eq!(doc["is_queryable"], Value::Bool(true));
        assert_eq!(doc["is_modifiable"], Value::Bool(false));
        assert_eq!(doc["subject"]["_type"], "PARTY_SELF");
    }

    #[test]
    fn a_status_without_other_details_gains_it_when_the_draft_is_filled() {
        let base = r#"{"_type":"EHR_STATUS","is_queryable":true,"is_modifiable":true}"#;
        let merged = apply_status_edits(base, true, true, r#"{"_type":"ITEM_TREE"}"#)
            .expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&merged).expect("merged JSON");
        assert_eq!(doc["other_details"]["_type"], "ITEM_TREE");
    }

    #[test]
    fn a_non_object_base_or_draft_is_refused() {
        assert!(apply_status_edits("not json", true, true, "").is_err());
        assert!(apply_status_edits("[]", true, true, "").is_err());
        assert!(apply_status_edits(BASE, true, true, "[]").is_err());
    }
}
