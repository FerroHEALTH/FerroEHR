// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The wire shapes the shared component kit renders.
//!
//! A screen's own flattened types live with that screen. These two carry what
//! SEVERAL screens read from the CDR's versioned surfaces, and what the shared
//! kit (`version_history`) renders: a
//! `components` module reaching into `pages` for a type would invert the crate's
//! one dependency arrow.
//!
//! Both are flattened here, so a page never re-models the RM.

#![expect(
    clippy::disallowed_types,
    reason = "the revision history is read off the CDR's JSON wire as an untyped tree"
)]

use serde_json::Value;

use crate::error::ViewerError;
use crate::locale::{Msg, tr_with};

/// One entry in a versioned object's revision history, flattened for the
/// version selector, the history table and the audit card.
///
/// The attributes are `AUDIT_DETAILS`'s own
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.audit_details.adoc`)
/// beside the VERSION's `uid`; a revision history is
/// `List<REVISION_HISTORY_ITEM>`, each item a version id plus its audits
/// (`org.openehr.rm.common.revision_history_item.adoc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionEntry {
    /// The `OBJECT_VERSION_ID` value (`uuid::system::version`).
    pub version_id: String,
    /// `AUDIT_DETAILS.time_committed` value.
    pub committed: String,
    /// `AUDIT_DETAILS.change_type` value (the `DV_CODED_TEXT` label).
    pub change_type: String,
    /// `AUDIT_DETAILS.committer`, as [`committer_label`] words it.
    pub committer: String,
}

/// A versioned object's revision history at `url`, newest first.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the `404` of an
/// unknown object included; [`ViewerError::Internal`] when the history is not
/// valid JSON.
pub async fn revision_history(
    cdr: &crate::bff::Cdr,
    url: &str,
) -> Result<Vec<VersionEntry>, ViewerError> {
    let mut entries = parse_versions(&cdr.get(url).ok().await?.body)?;
    // NOTE: REVISION_HISTORY.items arrives most-recent-LAST on the wire (RM
    // common `org.openehr.rm.common.revision_history.adoc` §items); every
    // history shows newest first.
    entries.reverse();
    Ok(entries)
}

/// Parse a `REVISION_HISTORY` body (either the canonical `{ "items": [...] }`
/// wrapper or a bare array) into version entries. Defensive throughout — a
/// missing field reads as empty rather than failing.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
pub(crate) fn parse_versions(body: &str) -> Result<Vec<VersionEntry>, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(
            Msg::RevisionHistoryJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let items = doc
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| doc.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(items.iter().map(version_entry).collect())
}

/// Flatten one `REVISION_HISTORY_ITEM` (`version_id` + `audits`) into a
/// [`VersionEntry`], reading the first audit for the commit metadata.
fn version_entry(item: &Value) -> VersionEntry {
    let version_id = text_at(item, "/version_id/value");
    let audit = item
        .get("audits")
        .and_then(Value::as_array)
        .and_then(|audits| audits.first());
    VersionEntry {
        version_id,
        committed: audit
            .map(|a| text_at(a, "/time_committed/value"))
            .unwrap_or_default(),
        change_type: audit
            .map(|a| text_at(a, "/change_type/value"))
            .unwrap_or_default(),
        committer: audit
            .map(|a| committer_label(|key| Some(text_at(a, &format!("/committer/{key}")))))
            .unwrap_or_default(),
    }
}

/// The string at JSON pointer `pointer` in `value`, or an empty string when it
/// is absent or not a string.
fn text_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// How the viewer names an `AUDIT_DETAILS.committer` whose attribute `key`
/// reads as `attribute(key)`: the person, or their representative, for
/// `PARTY_SELF`, else its `name`.
///
/// RM common master04 §Audit Details: the committer is `PARTY_SELF` "when the
/// committer is the record subject", and what a person inserts is "clearly
/// distinguishable" as theirs (EHDS Art. 5).
pub fn committer_label(attribute: impl Fn(&str) -> Option<String>) -> String {
    if attribute("_type").as_deref() == Some("PARTY_SELF") {
        crate::locale::tr(Msg::CommitterPerson)
    } else {
        attribute("name").unwrap_or_default()
    }
}

/// A versioned object's container facts plus one of its VERSIONs' envelope
/// facts — the shape every History tab's versioned-object card is built from.
///
/// One type for every family: the `VERSIONED_EHR_STATUS` container and the
/// demographic ones carry the same eight facts, differing only in which of them
/// a given family populates and in how each screen lays them out.
///
/// The attributes are the RM classes' own (files under
/// `docs/specs/openehr/RM/docs/UML/classes/`): `VERSIONED_OBJECT._uid_`,
/// `_owner_id_` and `_time_created_`
/// (`org.openehr.rm.common.versioned_object.adoc`); `VERSION._contribution_`,
/// `_signature_` and `_preceding_version_uid_`, whose invariant
/// `Preceding_version_uid_validity` makes it absent exactly for a first version
/// (`org.openehr.rm.common.version.adoc`); and
/// `ORIGINAL_VERSION._lifecycle_state_`
/// (`org.openehr.rm.common.original_version.adoc`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VersionedObjectFacts {
    /// `VERSIONED_OBJECT.uid.value` — the versioned-object id.
    pub object_uid: String,
    /// `VERSIONED_OBJECT.owner_id.id.value` — the owning EHR; empty on a
    /// demographic container, which has no owning EHR.
    pub owner_id: String,
    /// `VERSIONED_OBJECT.time_created.value` — when the first version was
    /// committed.
    pub time_created: String,
    /// The read VERSION's `uid.value` (`OBJECT_VERSION_ID`).
    pub version_id: String,
    /// `ORIGINAL_VERSION.lifecycle_state.value`.
    pub lifecycle_state: String,
    /// `VERSION.preceding_version_uid.value` — empty for a first version.
    pub preceding_version_uid: String,
    /// `VERSION.contribution.id.value`.
    pub contribution_uid: String,
    /// Whether the VERSION carries a `signature`.
    pub signed: bool,
    /// Whether the VERSION carries `data` — a logically deleted version does
    /// not (`docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`
    /// §Logical Deletion).
    pub has_data: bool,
}

/// The container at `object_url` and one of its VERSIONs, read into
/// [`VersionedObjectFacts`].
///
/// The VERSION is `{object_url}/version/{version_uid}`, or `{object_url}/version`
/// for a blank `version_uid`, which "retrieves the _latest_ VERSION" when no
/// `version_at_time` is given
/// (`docs/specs/openehr/ITS-REST/specifications/operations/versioned_composition_version_get_at_time.yaml`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the `404` of an
/// unknown object or version included; [`ViewerError::Internal`] when either
/// body is not valid JSON.
pub async fn read_versioned_facts(
    cdr: &crate::bff::Cdr,
    object_url: &str,
    version_uid: &str,
) -> Result<VersionedObjectFacts, ViewerError> {
    let version_uid = version_uid.trim();
    let version_url = if version_uid.is_empty() {
        format!("{object_url}/version")
    } else {
        format!("{object_url}/version/{}", urlencoding::encode(version_uid))
    };
    let object = cdr.get(object_url).ok().await?.body;
    let version = cdr.get(&version_url).ok().await?.body;
    versioned_facts(&object, &version)
}

/// Flattens a container body plus a VERSION body into
/// [`VersionedObjectFacts`].
///
/// Defensive throughout: an absent attribute reads as empty rather than
/// failing the card.
///
/// # Errors
/// [`ViewerError::Internal`] when either body is not valid JSON.
pub fn versioned_facts(
    object_body: &str,
    version_body: &str,
) -> Result<VersionedObjectFacts, ViewerError> {
    let object: Value = serde_json::from_str(object_body).map_err(|e| {
        ViewerError::Internal(tr_with(
            Msg::VersionedObjectJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let version: Value = serde_json::from_str(version_body).map_err(|e| {
        ViewerError::Internal(tr_with(Msg::VersionJson, &[("e", (e).to_string().into())]))
    })?;
    Ok(VersionedObjectFacts {
        object_uid: crate::uid::uid_value_of_document(&object),
        owner_id: text_at(&object, "/owner_id/id/value"),
        time_created: text_at(&object, "/time_created/value"),
        version_id: crate::uid::uid_value_of_document(&version),
        lifecycle_state: text_at(&version, "/lifecycle_state/value"),
        preceding_version_uid: text_at(&version, "/preceding_version_uid/value"),
        contribution_uid: text_at(&version, "/contribution/id/value"),
        signed: version
            .get("signature")
            .and_then(Value::as_str)
            .is_some_and(|signature| !signature.is_empty()),
        has_data: version.get("data").is_some_and(|data| !data.is_null()),
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_versions, versioned_facts};

    /// A `VERSIONED_COMPOSITION` container body, as the wire carries it.
    const VERSIONED_OBJECT: &str = r#"{
        "_type": "VERSIONED_COMPOSITION",
        "uid": {"_type": "HIER_OBJECT_ID", "value": "7d44aa01"},
        "owner_id": {
            "_type": "OBJECT_REF",
            "namespace": "local",
            "type": "EHR",
            "id": {"_type": "HIER_OBJECT_ID", "value": "e1"}
        },
        "time_created": {"_type": "DV_DATE_TIME", "value": "2026-07-12T10:00:00Z"}
    }"#;

    #[test]
    fn parses_the_versioned_object_and_its_version_envelope() {
        let version = r#"{
            "_type": "ORIGINAL_VERSION",
            "uid": {"_type": "OBJECT_VERSION_ID", "value": "7d44aa01::sys::2"},
            "contribution": {
                "_type": "OBJECT_REF",
                "id": {"_type": "HIER_OBJECT_ID", "value": "c9"}
            },
            "lifecycle_state": {"_type": "DV_CODED_TEXT", "value": "complete"},
            "preceding_version_uid": {"_type": "OBJECT_VERSION_ID", "value": "7d44aa01::sys::1"},
            "signature": "-----BEGIN PGP SIGNATURE-----",
            "data": {"_type": "COMPOSITION"}
        }"#;
        let facts = versioned_facts(VERSIONED_OBJECT, version).expect("valid bodies");
        assert_eq!(facts.object_uid, "7d44aa01");
        assert_eq!(facts.owner_id, "e1");
        assert_eq!(facts.time_created, "2026-07-12T10:00:00Z");
        assert_eq!(facts.version_id, "7d44aa01::sys::2");
        assert_eq!(facts.lifecycle_state, "complete");
        assert_eq!(facts.preceding_version_uid, "7d44aa01::sys::1");
        assert_eq!(facts.contribution_uid, "c9");
        assert!(facts.signed);
        assert!(facts.has_data);
    }

    #[test]
    fn a_deleted_first_version_carries_no_data_no_preceding_and_no_signature() {
        // RM common master06 §Logical Deletion: the deleted version has no
        // `data`; a first version has no `preceding_version_uid`.
        let version = r#"{
            "_type": "ORIGINAL_VERSION",
            "uid": {"_type": "OBJECT_VERSION_ID", "value": "7d44aa01::sys::1"},
            "lifecycle_state": {"_type": "DV_CODED_TEXT", "value": "deleted"}
        }"#;
        let facts = versioned_facts(VERSIONED_OBJECT, version).expect("valid bodies");
        assert_eq!(facts.lifecycle_state, "deleted");
        assert_eq!(facts.preceding_version_uid, "");
        assert_eq!(facts.contribution_uid, "");
        assert!(!facts.signed);
        assert!(!facts.has_data);
    }

    #[test]
    fn a_demographic_container_has_no_owning_ehr() {
        let object = r#"{
            "_type": "VERSIONED_PARTY",
            "uid": {"_type": "HIER_OBJECT_ID", "value": "8849182c"},
            "time_created": {"_type": "DV_DATE_TIME", "value": "2026-07-12T10:00:00Z"}
        }"#;
        let facts = versioned_facts(object, "{}").expect("valid bodies");
        assert_eq!(facts.object_uid, "8849182c");
        assert_eq!(facts.owner_id, "");
        assert_eq!(
            versioned_facts("{}", "{}")
                .expect("valid bodies")
                .object_uid,
            ""
        );
    }

    #[test]
    fn versioned_facts_reject_a_non_json_body_on_either_side() {
        assert!(versioned_facts("not json", "{}").is_err());
        assert!(versioned_facts(VERSIONED_OBJECT, "not json").is_err());
    }

    #[test]
    fn parses_revision_history_wrapper_and_bare_array() {
        let wrapper = r#"{
            "items": [{
                "version_id": {"value": "7d44::sys::2"},
                "audits": [{
                    "time_committed": {"value": "2026-07-12T10:00:00Z"},
                    "change_type": {"value": "creation"},
                    "committer": {"name": "Dr Bob"}
                }]
            }]
        }"#;
        let entries = parse_versions(wrapper).expect("valid history");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].version_id, "7d44::sys::2");
        assert_eq!(entries[0].committed, "2026-07-12T10:00:00Z");
        assert_eq!(entries[0].change_type, "creation");
        assert_eq!(entries[0].committer, "Dr Bob");

        // A bare array (no wrapper) parses identically.
        let bare = r#"[{"version_id":{"value":"a::b::1"},"audits":[]}]"#;
        let entries = parse_versions(bare).expect("valid bare history");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].version_id, "a::b::1");
        assert_eq!(entries[0].committed, "");
    }

    /// EHDS Art. 5: a version the person inserted names its committer as the
    /// person, never by a name a `PARTY_SELF` does not carry.
    #[test]
    fn a_party_self_committer_reads_as_the_person() {
        let history = r#"[{
            "version_id": {"value": "7d44::sys::1"},
            "audits": [{"committer": {"_type": "PARTY_SELF"}}]
        }]"#;
        let entries = parse_versions(history).expect("valid history");
        assert_eq!(
            entries[0].committer,
            crate::locale::tr(crate::locale::Msg::CommitterPerson)
        );
    }
}
