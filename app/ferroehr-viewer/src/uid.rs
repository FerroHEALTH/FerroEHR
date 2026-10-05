// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR identifier handling shared by the viewer's screens.
//!
//! Component-free and unit-tested, and compiled on BOTH targets: the same
//! reduction runs server-side before a CDR path is built and client-side
//! when a link is rendered.

/// The version CONTAINER id inside a `uid_based_id`: an `OBJECT_VERSION_ID`
/// (`{uuid}::{system}::{tree}`) reduced to its `object_id`, a bare
/// `HIER_OBJECT_ID` returned unchanged.
///
/// Splitting on `::` is the `OBJECT_VERSION_ID` syntax itself (BASE
/// `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
/// §Syntaxes: `object_version_id = object_id "::" creating_system_id "::"
/// version_tree_id`).
///
/// The two forms address the same versioned object but not the same routes —
/// an update takes the container, a delete takes the version, an `ITEM_TAG`
/// collection is a DIFFERENT collection per form — so every caller says which
/// one it means rather than passing a `uid_based_id` through untouched.
#[must_use]
pub fn container_uid_of(uid: &str) -> String {
    uid.trim().split("::").next().unwrap_or_default().to_owned()
}

/// The `uid.value` of a served openEHR body, or an empty string when the body
/// carries none (a `Prefer: return=minimal` write answers with no
/// representation at all) or is not JSON.
///
/// Every `VERSIONABLE` resource the CDR serves carries its own identifier under
/// `uid.value` — an `OBJECT_VERSION_ID` on a VERSION, a `HIER_OBJECT_ID` on a
/// container (RM
/// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.versioned_object.adoc`)
/// — so one reader serves every body the viewer has to identify.
///
/// Compiled on BOTH targets: a BFF write reads the uid out of the answer it
/// just received, and the Commit tab's amend seed reads it out of the document
/// the browser already holds — rather than a second endpoint read for the same
/// claim.
#[must_use]
pub fn uid_value_of(body: &str) -> String {
    #[expect(
        clippy::disallowed_types,
        reason = "the viewer reads the CDR JSON wire over ITS-REST — not the CDR internal seams (#1694)"
    )]
    serde_json::from_str::<serde_json::Value>(body)
        .map(|doc| uid_value_of_document(&doc))
        .unwrap_or_default()
}

/// [`uid_value_of`] over an already-parsed document: the `uid.value` leaf, or an
/// empty string when the document carries none.
///
/// The reader for a caller that has parsed the body for other attributes too —
/// the string form above parses and then calls this, so both answer identically.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR JSON wire over ITS-REST — not the CDR internal seams (#1694)"
)]
#[must_use]
pub fn uid_value_of_document(doc: &serde_json::Value) -> String {
    doc.get("uid")
        .and_then(|uid| uid.get("value"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::container_uid_of;

    #[test]
    fn an_object_version_id_reduces_to_its_container() {
        assert_eq!(
            container_uid_of("8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2"),
            "8849182c-82ad-4088-a07f-48ead4180515"
        );
        // A bare HIER_OBJECT_ID is already the container form.
        assert_eq!(
            container_uid_of("8849182c-82ad-4088-a07f-48ead4180515"),
            "8849182c-82ad-4088-a07f-48ead4180515"
        );
        // Surrounding whitespace from a pasted id never reaches a URL.
        assert_eq!(container_uid_of("  8849182c::sys::1  "), "8849182c");
        assert_eq!(container_uid_of(""), "");
        assert_eq!(container_uid_of("   "), "");
    }

    #[test]
    fn a_served_bodys_uid_reads_back_or_is_empty() {
        // Every versioned family the viewer commits answers the same shape.
        for rm_type in ["FOLDER", "COMPOSITION", "EHR_STATUS", "PERSON"] {
            let body = format!(
                r#"{{"_type":"{rm_type}","uid":{{"_type":"OBJECT_VERSION_ID","value":"7d44::sys::1"}}}}"#
            );
            assert_eq!(super::uid_value_of(&body), "7d44::sys::1");
        }
        // A `Prefer: return=minimal` write answers with no representation.
        assert_eq!(super::uid_value_of(""), "");
        assert_eq!(super::uid_value_of("{}"), "");
        assert_eq!(super::uid_value_of("not json"), "");
    }

    #[test]
    fn the_parsed_form_answers_exactly_as_the_string_form() {
        let body =
            r#"{"_type":"COMPOSITION","uid":{"_type":"OBJECT_VERSION_ID","value":"7d44::sys::2"}}"#;
        #[expect(
            clippy::disallowed_types,
            reason = "the viewer reads the CDR JSON wire over ITS-REST — not the CDR internal seams (#1694)"
        )]
        let doc: serde_json::Value = serde_json::from_str(body).expect("a served body");
        assert_eq!(super::uid_value_of_document(&doc), "7d44::sys::2");
        assert_eq!(
            super::uid_value_of_document(&doc),
            super::uid_value_of(body)
        );
        // An absent, non-object or non-string `uid` reads as empty, never a panic.
        for shape in ["{}", r#"{"uid":"7d44"}"#, r#"{"uid":{"value":7}}"#, "[]"] {
            #[expect(
                clippy::disallowed_types,
                reason = "the viewer reads the CDR JSON wire over ITS-REST — not the CDR internal seams (#1694)"
            )]
            let doc: serde_json::Value = serde_json::from_str(shape).expect("valid JSON");
            assert_eq!(super::uid_value_of_document(&doc), "", "{shape}");
        }
    }
}
