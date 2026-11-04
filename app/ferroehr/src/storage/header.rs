// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The header keys a node repeats, and the columns that hold them instead
//! (#4525, and the root since #4904).
//!
//! Every archetype root repeats its `archetype_node_id` inside
//! `archetype_details`, with the same `rm_version` throughout a version, and
//! every entry repeats the same `language` and `encoding`. `slim_header`
//! moves them to the row's `rm_version`, `language` and `encoding` columns
//! when the columns rebuild them byte for byte, and `restore` puts them back
//! where they stood; `ext.node_archetype_node_id`,
//! `ext.node_archetype_details` and `ext.node_code_phrase` are the SQL twins.
//! The versioned object's root names its template in the header (RM common
//! `ARCHETYPED.template_id`), which the version row holds as `template_id`,
//! so the root's header leaves when the template is the version's.
//!
//! No openEHR spec governs storage layout: our own design (RM common
//! `master06-change_control_package.adoc` §Overview admits any physical
//! representation).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::{Map, Value};

use crate::storage::compact::RootIdentity;
use crate::storage::slim::{has_keys, object, string};

/// The `archetype_details`, `language` and `encoding` of a node, as the
/// row's `rm_version`, `language` and `encoding` columns hold them.
///
/// `rm_version` is set when the fragment's `archetype_details` was an
/// `ARCHETYPED` of exactly its `archetype_id` and `rm_version`, naming the
/// node's own `archetype_node_id` in the casing [`recased`] gives back, and
/// on the root row a `template_id` exactly the version's or none; both keys
/// then leave the fragment. `language` and `encoding` hold the code of a
/// `CODE_PHRASE` in the openEHR `languages` (`ISO_639-1`) and `character
/// sets` (`IANA_character-sets`) code sets (TERM
/// `openehr_external_terminologies.xml`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    /// `archetype_details/rm_version`.
    pub rm_version: Option<String>,
    /// `language/code_string`.
    pub language: Option<String>,
    /// `encoding/code_string`.
    pub encoding: Option<String>,
}

/// The terminology of the openEHR `languages` code set.
pub const LANGUAGE_TERMINOLOGY: &str = "ISO_639-1";

/// The terminology of the openEHR `character sets` code set.
pub const ENCODING_TERMINOLOGY: &str = "IANA_character-sets";

/// Removes the header keys the [`Header`] columns rebuild exactly from a
/// fragment, and returns what the columns hold of them.
///
/// `root` is the version identity of a root row (`num = 0`), whose header may
/// name the version's template, and `None` below it.
pub(crate) fn slim_header(map: &mut Map<String, Value>, root: Option<&RootIdentity<'_>>) -> Header {
    let mut header = Header::default();
    if let Some(rm_version) = derived_archetype_details(map, root) {
        map.shift_remove("archetype_details");
        map.shift_remove("archetype_node_id");
        header.rm_version = Some(rm_version);
    }
    if let Some(code) = map
        .get("language")
        .and_then(|phrase| code_phrase(phrase, LANGUAGE_TERMINOLOGY))
    {
        map.shift_remove("language");
        header.language = Some(code);
    }
    if let Some(code) = map
        .get("encoding")
        .and_then(|phrase| code_phrase(phrase, ENCODING_TERMINOLOGY))
    {
        map.shift_remove("encoding");
        header.encoding = Some(code);
    }
    header
}

/// The `rm_version` of an `archetype_details` that [`archetyped`] rebuilds
/// from the row's `archetype` column: an `ARCHETYPED` of exactly
/// `archetype_id` and `rm_version`, naming the fragment's own
/// `archetype_node_id`, which [`recased`] rebuilds from its lowercase form,
/// and on a root row a `template_id` that is exactly the version's, or none
/// when the version has none.
fn derived_archetype_details(
    map: &Map<String, Value>,
    root: Option<&RootIdentity<'_>>,
) -> Option<String> {
    let node_id = map.get("archetype_node_id").and_then(Value::as_str)?;
    if recased(&node_id.to_ascii_lowercase()).as_deref() != Some(node_id) {
        return None;
    }
    let details = map.get("archetype_details").and_then(Value::as_object)?;
    let template = root.and_then(|root| root.template_id);
    let keys: &[&str] = if template.is_some() {
        &["_type", "archetype_id", "rm_version", "template_id"]
    } else {
        &["_type", "archetype_id", "rm_version"]
    };
    if !has_keys(details, keys)
        || details.get("_type").and_then(Value::as_str) != Some("ARCHETYPED")
        || details.get("template_id") != template.map(template_id).as_ref()
    {
        return None;
    }
    let id = details.get("archetype_id").and_then(Value::as_object)?;
    if !has_keys(id, &["_type", "value"])
        || id.get("_type").and_then(Value::as_str) != Some("ARCHETYPE_ID")
        || id.get("value").and_then(Value::as_str) != Some(node_id)
    {
        return None;
    }
    string(details, "rm_version")
}

/// The code of a `CODE_PHRASE` of exactly `terminology_id` and
/// `code_string`, whose terminology is `terminology`.
fn code_phrase(phrase: &Value, terminology: &str) -> Option<String> {
    let fields = phrase.as_object()?;
    if !has_keys(fields, &["_type", "code_string", "terminology_id"])
        || fields.get("_type").and_then(Value::as_str) != Some("CODE_PHRASE")
    {
        return None;
    }
    let id = fields.get("terminology_id").and_then(Value::as_object)?;
    if !has_keys(id, &["_type", "value"])
        || id.get("_type").and_then(Value::as_str) != Some("TERMINOLOGY_ID")
        || id.get("value").and_then(Value::as_str) != Some(terminology)
    {
        return None;
    }
    string(fields, "code_string")
}

/// Returns the archetype id a lowercase `openehr-…` archetype HRID was written as.
///
/// That is `openEHR-`, its `rm_publisher-rm_package-rm_class` part in
/// capitals, and the concept and version as they are (BASE `base_types`
/// §Archetype Identifiers); `None` for any other id.
///
/// The SQL twin is `ext.node_archetype_node_id`; both act on ASCII alone, so
/// neither depends on a locale.
#[must_use]
pub fn recased(archetype: &str) -> Option<String> {
    let rest = archetype.strip_prefix("openehr-")?;
    let (class, tail) = rest.split_once('.')?;
    let ascii = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'-';
    if class.is_empty() || !class.bytes().all(ascii) {
        return None;
    }
    Some(format!("openEHR-{}.{tail}", class.to_ascii_uppercase()))
}

/// Puts the header keys the [`Header`] columns hold back into `map`, each
/// where it stood while `data` still held it, together with the keys
/// `restored` the compact markers left out (`crate::storage::compact`).
///
/// `template_id` is the version's template on a root row and `None` below
/// it. `data` comes back from PostgreSQL in jsonb key order, shorter keys
/// first and keys of one length bytewise (PostgreSQL 18, §8.14.4 jsonb
/// Indexing), so the restored keys are placed by that order too; the keys
/// the other columns rebuild follow them, as they always have.
pub(crate) fn restore(
    map: &mut Map<String, Value>,
    archetype: Option<&str>,
    header: &Header,
    template_id: Option<&str>,
    mut restored: Vec<(&str, Value)>,
) {
    if let (Some(rm_version), Some(archetype)) = (header.rm_version.as_deref(), archetype)
        && let Some(node_id) = recased(archetype)
    {
        if !map.contains_key("archetype_details") {
            restored.push((
                "archetype_details",
                archetyped(&node_id, rm_version, template_id),
            ));
        }
        if !map.contains_key("archetype_node_id") {
            restored.push(("archetype_node_id", Value::from(node_id)));
        }
    }
    for (key, code, terminology) in [
        ("language", header.language.as_deref(), LANGUAGE_TERMINOLOGY),
        ("encoding", header.encoding.as_deref(), ENCODING_TERMINOLOGY),
    ] {
        if let (Some(code), false) = (code, map.contains_key(key)) {
            restored.push((key, code_phrase_of(code, terminology)));
        }
    }
    if restored.is_empty() {
        return;
    }
    let mut entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
    entries.extend(
        restored
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value)),
    );
    entries.sort_by(|(a, _), (b, _)| jsonb_key_order(a, b));
    map.extend(entries);
}

/// The order PostgreSQL stores jsonb object keys in: by length, then
/// bytewise.
pub(crate) fn jsonb_key_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.as_bytes().cmp(b.as_bytes()))
}

/// The `ARCHETYPED` of `archetype_id`, `rm_version` and, on a root with a
/// template, `template_id`, as `ext.node_archetype_details` builds it, its
/// keys in jsonb order.
fn archetyped(archetype_id: &str, rm_version: &str, template: Option<&str>) -> Value {
    let mut fields = vec![
        ("_type", Value::from("ARCHETYPED")),
        ("rm_version", Value::from(rm_version)),
    ];
    if let Some(template) = template {
        fields.push(("template_id", template_id(template)));
    }
    fields.push((
        "archetype_id",
        object([
            ("_type", Value::from("ARCHETYPE_ID")),
            ("value", Value::from(archetype_id)),
        ]),
    ));
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

/// The `TEMPLATE_ID` of `value`, as the version's root names it.
fn template_id(value: &str) -> Value {
    object([
        ("_type", Value::from("TEMPLATE_ID")),
        ("value", Value::from(value)),
    ])
}

/// The `CODE_PHRASE` of `code` in `terminology`, as `ext.node_code_phrase`
/// builds it, its keys in jsonb order.
pub(crate) fn code_phrase_of(code: &str, terminology: &str) -> Value {
    object([
        ("_type", Value::from("CODE_PHRASE")),
        ("code_string", Value::from(code)),
        (
            "terminology_id",
            object([
                ("_type", Value::from("TERMINOLOGY_ID")),
                ("value", Value::from(terminology)),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::slim::slim;
    use serde_json::json;

    #[test]
    fn the_header_of_an_entry_moves_to_its_columns() {
        let entry = json!({"_type": "OBSERVATION",
            "archetype_node_id": "openEHR-EHR-OBSERVATION.blood_pressure.v2",
            "archetype_details": {"_type": "ARCHETYPED", "rm_version": "1.0.4",
                "archetype_id": {"_type": "ARCHETYPE_ID",
                    "value": "openEHR-EHR-OBSERVATION.blood_pressure.v2"}},
            "language": {"_type": "CODE_PHRASE", "code_string": "en",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_639-1"}},
            "encoding": {"_type": "CODE_PHRASE", "code_string": "UTF-8",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "IANA_character-sets"}},
            "subject": {"_type": "PARTY_SELF"}});
        let stored = slim("OBSERVATION", None, entry.clone());
        assert_eq!(stored.data, None);
        assert!(stored.compact.party_self);
        let header = Header {
            rm_version: Some("1.0.4".to_owned()),
            language: Some("en".to_owned()),
            encoding: Some("UTF-8".to_owned()),
        };
        assert_eq!(stored.header, header);
        // A root row moves them on the same terms when its header names no
        // template and the version has none.
        let root = RootIdentity {
            version_uid: "a::b::1",
            template_id: None,
        };
        assert_eq!(
            slim("OBSERVATION", Some(&root), entry.clone()).header,
            header
        );
        // A version with a template the header does not name keeps the header.
        let templated = RootIdentity {
            version_uid: "a::b::1",
            template_id: Some("t"),
        };
        let kept = slim("OBSERVATION", Some(&templated), entry);
        assert_eq!(kept.header.rm_version, None);
        assert!(
            kept.data
                .is_some_and(|d| d.get("archetype_details").is_some())
        );
    }

    #[test]
    fn the_root_header_leaves_with_the_version_template() {
        let root = json!({"_type": "COMPOSITION",
            "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
            "archetype_details": {"_type": "ARCHETYPED", "rm_version": "1.1.0",
                "archetype_id": {"_type": "ARCHETYPE_ID",
                    "value": "openEHR-EHR-COMPOSITION.encounter.v1"},
                "template_id": {"_type": "TEMPLATE_ID", "value": "Vital signs"}}});
        let identity = RootIdentity {
            version_uid: "a::b::1",
            template_id: Some("Vital signs"),
        };
        let stored = slim("COMPOSITION", Some(&identity), root.clone());
        assert_eq!(stored.data, None);
        assert_eq!(stored.header.rm_version.as_deref(), Some("1.1.0"));
        // Another template, or none, keeps the header verbatim.
        for template_id in [Some("Other"), None] {
            let other = RootIdentity {
                version_uid: "a::b::1",
                template_id,
            };
            let kept = slim("COMPOSITION", Some(&other), root.clone());
            assert_eq!(kept.header, Header::default());
            let mut expected = root.clone();
            expected.as_object_mut().map(|m| m.shift_remove("_type"));
            assert_eq!(kept.data, Some(expected));
        }
    }

    #[test]
    fn a_header_no_column_rebuilds_exactly_stays_verbatim() {
        let cases = [
            // The recasing does not give back a capital in the concept.
            json!({"archetype_node_id": "openEHR-EHR-OBSERVATION.lab-JM.v0",
                "archetype_details": {"_type": "ARCHETYPED", "rm_version": "1.0.4",
                    "archetype_id": {"_type": "ARCHETYPE_ID",
                        "value": "openEHR-EHR-OBSERVATION.lab-JM.v0"}}}),
            // A template id is no part of the rebuilt header.
            json!({"archetype_node_id": "openEHR-EHR-SECTION.x.v1",
                "archetype_details": {"_type": "ARCHETYPED", "rm_version": "1.0.4",
                    "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-SECTION.x.v1"},
                    "template_id": {"_type": "TEMPLATE_ID", "value": "t"}}}),
            // The details name another archetype than the node.
            json!({"archetype_node_id": "openEHR-EHR-SECTION.x.v1",
                "archetype_details": {"_type": "ARCHETYPED", "rm_version": "1.0.4",
                    "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-SECTION.y.v1"}}}),
            // A language outside the openEHR code set, an encoding with a
            // preferred term.
            json!({"language": {"_type": "CODE_PHRASE", "code_string": "en",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_639-2"}},
                "encoding": {"_type": "CODE_PHRASE", "code_string": "UTF-8",
                    "preferred_term": "UTF-8",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "IANA_character-sets"}}}),
        ];
        for fragment in cases {
            let stored = slim("SECTION", None, fragment.clone());
            assert_eq!(stored.header, Header::default(), "{fragment}");
            assert_eq!(stored.data, Some(fragment));
        }
    }

    #[test]
    fn recasing_gives_back_the_written_archetype_id() {
        assert_eq!(
            recased("openehr-ehr-observation.blood_pressure.v2").as_deref(),
            Some("openEHR-EHR-OBSERVATION.blood_pressure.v2")
        );
        assert_eq!(
            recased("openehr-ehr-cluster.device.v1.0.2").as_deref(),
            Some("openEHR-EHR-CLUSTER.device.v1.0.2")
        );
        assert_eq!(recased("org.openehr::openehr-ehr-cluster.x.v1"), None);
        assert_eq!(recased("openehr-ehr-cluster"), None);
        assert_eq!(recased("at0001"), None);
    }
}
