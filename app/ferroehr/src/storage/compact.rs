// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The compact markers of a node row (#4904).
//!
//! They are the keys a row leaves out of its fragment beyond what its
//! columns hold, each announced by one boolean column so the rebuild never
//! reads an absent key or a JSON shape as a signal.
//!
//! A `HISTORY.origin` and an `EVENT.time` or `ACTION.time` of exactly
//! `_type: DV_DATE_TIME` and a `value` string are stored as that string
//! (`origin_compact`, `time_compact`); a `subject` of exactly
//! `{"_type": "PARTY_SELF"}` leaves (`party_self`); and the root row's
//! `uid`, when it is exactly the `OBJECT_VERSION_ID` of its version row,
//! leaves (`uid_derived`). `compact` is the write half; `restore` and
//! `ext.node_fragment`, `ext.node_date_time` and `ext.version_uid` are the
//! read half. A row written before the markers existed has every marker
//! `NULL` and reads as it did.
//!
//! No openEHR spec governs storage layout: our own design (RM common
//! `master06-change_control_package.adoc` §Overview admits any physical
//! representation). The fields themselves are RM `data_structures`
//! `HISTORY.origin` and `EVENT.time`, RM composition `ACTION.time` and
//! `ENTRY.subject`, RM common `PARTY_SELF` and `LOCATABLE.uid`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::{Map, Value};

use crate::ids::VoId;
use crate::storage::slim::{has_keys, object};

/// The two keys a row stores as a `DV_DATE_TIME`'s value string.
const DATE_TIME_KEYS: [&str; 2] = ["origin", "time"];

/// Which keys a row's fragment leaves out under a marker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per marker column, each independent of the others; no state machine here"
)]
pub struct Compact {
    /// `origin` is stored as the value string of its `DV_DATE_TIME`.
    pub origin: bool,
    /// `time` is stored as the value string of its `DV_DATE_TIME`.
    pub time: bool,
    /// A `subject` of exactly `{"_type": "PARTY_SELF"}` is left out.
    pub party_self: bool,
    /// The root's `uid`, the version's `OBJECT_VERSION_ID`, is left out.
    pub uid_derived: bool,
}

/// What the version row holds that a root row's fragment repeats: the
/// `OBJECT_VERSION_ID` the commit path stamps into the root's `uid` and the
/// `template_id` its `archetype_details` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootIdentity<'a> {
    /// The version's `OBJECT_VERSION_ID` text ([`version_uid`]).
    pub version_uid: &'a str,
    /// `version.template_id`, `None` when the version has none.
    pub template_id: Option<&'a str>,
}

/// The `OBJECT_VERSION_ID` text of a version row.
///
/// The law is the one the AQL engine, `ext.version_uid` and
/// `idx_version_hot_uid` share: `object_id '::' creating_system_id '::'
/// version_tree_id`, the tree id `N` on a trunk row and `N.B.V` on a branch
/// row (BASE `base_types` §`OBJECT_VERSION_ID`, §`VERSION_TREE_ID`).
#[must_use]
pub fn version_uid(
    vo_id: VoId,
    creating_system_id: &str,
    trunk_version: i32,
    branch_number: i32,
    branch_version: i32,
) -> String {
    if branch_number > 0 {
        format!("{vo_id}::{creating_system_id}::{trunk_version}.{branch_number}.{branch_version}")
    } else {
        format!("{vo_id}::{creating_system_id}::{trunk_version}")
    }
}

/// Removes the keys the markers stand for from `map`, and returns the markers.
///
/// `root` is the version identity of a root row (`num = 0`) and `None` below
/// it; only a root row ever leaves its `uid` out.
pub(crate) fn compact(map: &mut Map<String, Value>, root: Option<&RootIdentity<'_>>) -> Compact {
    let mut markers = Compact::default();
    for (key, flag) in DATE_TIME_KEYS
        .into_iter()
        .zip([&mut markers.origin, &mut markers.time])
    {
        if let Some(text) = map.get(key).and_then(date_time_value) {
            map.insert(key.to_owned(), Value::from(text));
            *flag = true;
        }
    }
    if map.get("subject").is_some_and(is_bare_party_self) {
        map.shift_remove("subject");
        markers.party_self = true;
    }
    if let Some(root) = root
        && map
            .get("uid")
            .is_some_and(|uid| *uid == object_version_id(root.version_uid))
    {
        map.shift_remove("uid");
        markers.uid_derived = true;
    }
    markers
}

/// Puts the `DV_DATE_TIME` objects back in place of their value strings, and
/// returns the keys the markers left out, for the caller to place in jsonb
/// order (`crate::storage::header::restore`).
///
/// `version_uid` is the version's `OBJECT_VERSION_ID` for a root row and
/// `None` below it.
pub(crate) fn restore(
    map: &mut Map<String, Value>,
    markers: Compact,
    version_uid: Option<&str>,
) -> Vec<(&'static str, Value)> {
    for (key, flag) in DATE_TIME_KEYS
        .into_iter()
        .zip([markers.origin, markers.time])
    {
        if flag && let Some(Value::String(text)) = map.get(key) {
            let text = text.clone();
            map.insert(key.to_owned(), date_time(&text));
        }
    }
    let mut restored = Vec::new();
    if markers.party_self && !map.contains_key("subject") {
        restored.push(("subject", party_self()));
    }
    if let (true, Some(uid), false) = (markers.uid_derived, version_uid, map.contains_key("uid")) {
        restored.push(("uid", object_version_id(uid)));
    }
    restored
}

/// The `value` of a `DV_DATE_TIME` that carries exactly `_type` and a string
/// `value`.
fn date_time_value(value: &Value) -> Option<String> {
    let fields = value.as_object()?;
    if !has_keys(fields, &["_type", "value"])
        || fields.get("_type").and_then(Value::as_str) != Some("DV_DATE_TIME")
    {
        return None;
    }
    fields
        .get("value")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The `DV_DATE_TIME` of `value`, as `ext.node_date_time` builds it.
fn date_time(value: &str) -> Value {
    object([
        ("_type", Value::from("DV_DATE_TIME")),
        ("value", Value::from(value)),
    ])
}

/// Whether `subject` is exactly `{"_type": "PARTY_SELF"}`.
fn is_bare_party_self(subject: &Value) -> bool {
    *subject == party_self()
}

/// The bare `PARTY_SELF`.
fn party_self() -> Value {
    object([("_type", Value::from("PARTY_SELF"))])
}

/// The `OBJECT_VERSION_ID` of `uid`, as the commit path stamps it.
fn object_version_id(uid: &str) -> Value {
    object([
        ("_type", Value::from("OBJECT_VERSION_ID")),
        ("value", Value::from(uid)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn map(value: Value) -> Map<String, Value> {
        let Value::Object(map) = value else {
            panic!("an object");
        };
        map
    }

    #[test]
    fn the_version_uid_follows_the_tree_id_law() {
        let vo_id = VoId(Uuid::parse_str("0193c6a0-0000-7000-8000-000000000001").unwrap());
        assert_eq!(
            version_uid(vo_id, "sys", 3, 0, 0),
            "0193c6a0-0000-7000-8000-000000000001::sys::3"
        );
        assert_eq!(
            version_uid(vo_id, "sys", 3, 2, 5),
            "0193c6a0-0000-7000-8000-000000000001::sys::3.2.5"
        );
        // The same text the versioning builders synthesize.
        let tree = crate::versioning::object_version_id::TreeId::branch(3, 2, 5);
        assert_eq!(
            version_uid(vo_id, "sys", 3, 2, 5),
            crate::versioning::object_version_id::object_version_id(vo_id, "sys", tree)
        );
    }

    #[test]
    fn a_plain_date_time_and_a_bare_party_self_are_compacted_and_put_back() {
        let mut fields = map(json!({
            "origin": {"_type": "DV_DATE_TIME", "value": "2024-05-01T10:00:00Z"},
            "subject": {"_type": "PARTY_SELF"},
            "period": {"_type": "DV_DURATION", "value": "PT1H"}
        }));
        let markers = compact(&mut fields, None);
        assert_eq!(
            markers,
            Compact {
                origin: true,
                time: false,
                party_self: true,
                uid_derived: false
            }
        );
        assert_eq!(fields.get("origin"), Some(&json!("2024-05-01T10:00:00Z")));
        assert!(!fields.contains_key("subject"));
        let restored = restore(&mut fields, markers, None);
        assert_eq!(
            fields.get("origin"),
            Some(&json!({"_type": "DV_DATE_TIME", "value": "2024-05-01T10:00:00Z"}))
        );
        assert_eq!(restored, vec![("subject", json!({"_type": "PARTY_SELF"}))]);
    }

    #[test]
    fn any_other_shape_stays_verbatim() {
        let cases = [
            // A date-time with a status, or without its type.
            json!({"time": {"_type": "DV_DATE_TIME", "value": "2024", "magnitude_status": "<"}}),
            json!({"time": {"value": "2024-05-01"}}),
            json!({"time": "2024-05-01"}),
            // A subject with a reference, or of another type.
            json!({"subject": {"_type": "PARTY_SELF", "external_ref": {"_type": "PARTY_REF"}}}),
            json!({"subject": {"_type": "PARTY_IDENTIFIED", "name": "x"}}),
            // A uid that is not this version's.
            json!({"uid": {"_type": "OBJECT_VERSION_ID", "value": "a::b::1"}}),
            json!({"uid": {"_type": "HIER_OBJECT_ID", "value": "a::b::2"}}),
        ];
        let root = RootIdentity {
            version_uid: "a::b::2",
            template_id: None,
        };
        for case in cases {
            let mut fields = map(case.clone());
            let markers = compact(&mut fields, Some(&root));
            assert_eq!(markers, Compact::default(), "{case}");
            assert_eq!(Value::Object(fields), case);
        }
    }

    #[test]
    fn the_root_uid_leaves_only_on_a_root_row() {
        let uid = json!({"uid": {"_type": "OBJECT_VERSION_ID", "value": "a::b::2"}});
        let root = RootIdentity {
            version_uid: "a::b::2",
            template_id: None,
        };
        let mut below = map(uid.clone());
        assert!(!compact(&mut below, None).uid_derived);
        let mut at_root = map(uid.clone());
        let markers = compact(&mut at_root, Some(&root));
        assert!(markers.uid_derived && at_root.is_empty());
        let restored = restore(&mut at_root, markers, Some("a::b::2"));
        assert_eq!(
            Value::Object(
                restored
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect()
            ),
            uid
        );
    }
}
