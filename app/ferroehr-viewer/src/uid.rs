// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR identifier handling shared by the viewer's screens.
//!
//! The same reduction runs before a CDR path is built and when a link is
//! drawn.

use std::str::FromStr;

use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;

use crate::error::ViewerError;

/// The version CONTAINER id inside a `uid_based_id`: an `OBJECT_VERSION_ID`
/// (`{uuid}::{system}::{tree}`) reduced to its `object_id`, a bare
/// `HIER_OBJECT_ID` returned unchanged.
///
/// The reduction is `openehr-base`'s `OBJECT_VERSION_ID` parse and its
/// verbatim `object_id` (BASE
/// `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
/// §Syntaxes and §"Composite Identifiers and Case"). Any other value is read
/// as the container id it already is and returned as written.
///
/// The two forms address the same versioned object but not the same routes —
/// an update takes the container, a delete takes the version, an `ITEM_TAG`
/// collection is a DIFFERENT collection per form — so every caller says which
/// one it means rather than passing a `uid_based_id` through untouched.
#[must_use]
pub fn container_uid_of(uid: &str) -> String {
    let uid = uid.trim();
    ObjectVersionId::from_str(uid).map_or_else(
        |_bare| uid.to_owned(),
        |version| version.object_id_str().to_owned(),
    )
}

/// The number of the version `version_uid` names: its `version_tree_id`, `3`
/// of `uid::system::3`, or empty for a value that is no `OBJECT_VERSION_ID`,
/// which names no version.
#[must_use]
pub fn version_number(version_uid: &str) -> String {
    ObjectVersionId::from_str(version_uid).map_or_else(
        |_not_a_version| String::new(),
        |version| version.version_tree_id().value().to_owned(),
    )
}

/// The `OBJECT_VERSION_ID` of version `number` of the object whose version
/// `known` names: `known` with its `version_tree_id` replaced, or `known` as
/// it is when it is no `OBJECT_VERSION_ID`.
#[must_use]
pub fn version_uid_at(known: &str, number: &str) -> String {
    match ObjectVersionId::from_str(known) {
        Ok(version) => format!(
            "{}::{}::{number}",
            version.object_id_str(),
            version.creating_system_id_str()
        ),
        Err(_not_a_version) => known.to_owned(),
    }
}

/// The trimmed `raw` when it is a full `OBJECT_VERSION_ID`, the form a delete
/// of a version requires.
///
/// # Errors
/// [`ViewerError::Invalid`] carrying `refusal` when it is not one.
pub fn full_version_uid(raw: &str, refusal: impl FnOnce() -> String) -> Result<&str, ViewerError> {
    let raw = raw.trim();
    match ObjectVersionId::from_str(raw) {
        Ok(_version) => Ok(raw),
        Err(_not_a_version) => Err(ViewerError::Invalid(refusal())),
    }
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
/// A BFF write reads the uid out of the answer it just received, and the
/// Commit tab's amend seed reads it out of the document it already holds,
/// rather than a second endpoint read for the same claim.
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
    use super::{container_uid_of, version_number, version_uid_at};

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
        assert_eq!(
            container_uid_of("  8849182c-82ad-4088-a07f-48ead4180515::sys::1  "),
            "8849182c-82ad-4088-a07f-48ead4180515"
        );
        assert_eq!(container_uid_of(""), "");
        assert_eq!(container_uid_of("   "), "");
    }

    /// A value that is no `OBJECT_VERSION_ID` under the BASE grammar is read as
    /// the container id it already is, never cut at its first `::`.
    #[test]
    fn a_value_that_is_no_version_id_is_kept_whole() {
        // `8849182c` is no `uid`: it starts with a digit and is no ISO OID.
        assert_eq!(container_uid_of("8849182c::sys::1"), "8849182c::sys::1");
        // Two parts are no OBJECT_VERSION_ID.
        assert_eq!(
            container_uid_of("8849182c-82ad-4088-a07f-48ead4180515::sys"),
            "8849182c-82ad-4088-a07f-48ead4180515::sys"
        );
        // The object_id keeps its case (BASE master05 §"Composite Identifiers and Case").
        assert_eq!(
            container_uid_of("8849182C-82AD-4088-A07F-48EAD4180515::sys::2"),
            "8849182C-82AD-4088-A07F-48EAD4180515"
        );
    }

    #[test]
    fn a_version_number_is_the_version_tree_id() {
        let uid = "8849182C-82AD-4088-A07F-48EAD4180515::ferroehr.local";
        assert_eq!(version_number(&format!("{uid}::3")), "3");
        assert_eq!(version_number(&format!("{uid}::1.2.4")), "1.2.4");
        // A container id or a malformed value names no version.
        assert_eq!(version_number("8849182c-82ad-4088-a07f-48ead4180515"), "");
        assert_eq!(version_number("8849182c::sys::3"), "");
        assert_eq!(version_number(""), "");
    }

    #[test]
    fn a_version_uid_is_rebuilt_on_its_object_and_system() {
        let uid = "8849182C-82AD-4088-A07F-48EAD4180515::ferroehr.local";
        assert_eq!(
            version_uid_at(&format!("{uid}::3"), "1"),
            format!("{uid}::1")
        );
        assert_eq!(
            version_uid_at(&format!("{uid}::1.2.4"), "2"),
            format!("{uid}::2")
        );
        // A value that is no OBJECT_VERSION_ID is kept as it is.
        assert_eq!(version_uid_at("8849182c::sys::3", "1"), "8849182c::sys::3");
        assert_eq!(version_uid_at("", "1"), "");
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
