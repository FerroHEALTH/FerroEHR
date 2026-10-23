// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared `ITEM_TAG` kit: ONE row type, ONE wire codec, ONE editor panel
//! and ONE filter form for every tag surface in the viewer.
//!
//! The CDR serves tags on three families of route — the demographic party trio
//! (`/demographic/{kind}/{uid_based_id}/tags`), the EHR-side COMPOSITION and
//! `EHR_STATUS` trios (`/ehr/{ehr_id}/{kind}/{uid_based_id}/tags`), and the two
//! aggregate lists (`/demographic/tags`, `/ehr/{ehr_id}/tags`) — all carrying
//! the SAME resource, so they get one kit rather than one editor per screen.
//!
//! Three wire facts every caller is built around, all from the released
//! operations (`docs/specs/openehr/ITS-REST/specifications/operations/`):
//!
//! - **An update replaces the WHOLE collection.** "Providing an empty list will
//!   effectively remove all `ITEM_TAG` associated with the given target"
//!   (`composition_tags_update.yaml`), so setting one tag means sending every
//!   tag the target should still hold — [`merged_tag_body`] reads the current
//!   collection and merges. That read-modify-write has no conditional header to
//!   ride: no tag operation declares `If-Match`, `ETag` or `Last-Modified` (the
//!   parameter lists are `Prefer` + the two content headers, and
//!   `Requests_and_responses.md` §"`ETag` and `Last-Modified`" scopes those to
//!   resources "that have versioning or unique state identifiers", which a tag
//!   collection is not), so a concurrent tag edit by another client can be
//!   lost. Every panel says so.
//! - **A tag's identity is the `(key, target_path)` PAIR** — "they are uniquely
//!   identified by their `key` and `target_path` pair attributes" (same file) —
//!   which is what the merge and the `<For>` key key on. A DELETE addresses a
//!   `key` alone, so it removes every tag sharing that key.
//! - **`target` and `owner_id` are server-assigned and never sent**: the update
//!   body is the released `UpdateItemTag` shape (`key` required, optional
//!   `value` and `target_path`, and nothing else —
//!   `schemas/common/UpdateItemTag.yaml` is `additionalProperties: false`).
//!
//! DOM hooks: the set form's field ids (`tag-key`, `tag-value`,
//! `tag-target-path`, `tag-save`) are fixed, so a screen mounts at most ONE tag
//! panel; the panel's own section id comes from its caller
//! ([`TagPanelCopy::form_id`]).

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use std::collections::BTreeMap;

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::Value;

use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::tr_with;
use crate::locale::Msg;

/// One `ITEM_TAG` as the tag routes serve it.
///
/// The attributes are the RM class's own
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.item_tag.adoc`):
/// `key`, the optional `value`, the optional `target_path`, the `target` (a
/// `UID_BASED_ID` — a `HIER_OBJECT_ID` for a container target, an
/// `OBJECT_VERSION_ID` for one version) and the `owner_id` reference. Strings
/// only, so the type is WASM-safe over the server-fn boundary.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ItemTagRow {
    /// `ITEM_TAG.key`.
    pub key: String,
    /// `ITEM_TAG.value`, empty when the tag carries none.
    pub value: String,
    /// `ITEM_TAG.target_path`, empty when the tag targets the whole object.
    pub target_path: String,
    /// `ITEM_TAG.target.value` — the tagged container or version id.
    pub target: String,
    /// `ITEM_TAG.owner_id.id.value` — the owning object the CDR assigned.
    pub owner_id: String,
}

impl ItemTagRow {
    /// The tag's identity — the `(key, target_path)` pair the released update
    /// operation names — as the `<For>` key (stable, unique, data-derived).
    #[must_use]
    pub fn identity(&self) -> String {
        format!("{}\u{1f}{}", self.key, self.target_path)
    }

    /// The identity qualified by the target, unique across a whole aggregate
    /// tag list (where one key may sit on many targets).
    #[must_use]
    pub fn global_identity(&self) -> String {
        format!("{}\u{1f}{}", self.target, self.identity())
    }
}

/// The three attributes a tag write carries — the released `UpdateItemTag`
/// members, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagEdit {
    /// The tag key to set (required — `ITEM_TAG` invariant `Inv_key_valid`).
    pub key: String,
    /// The tag value; blank stores a tag with no value.
    pub value: String,
    /// The tag's `target_path`; blank tags the whole object.
    pub target_path: String,
}

/// One target's tags, as an aggregate tag list groups them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagGroup {
    /// The `ITEM_TAG.target` every tag in this group carries.
    pub target: String,
    /// That target's tags, ordered by identity.
    pub tags: Vec<ItemTagRow>,
}

/// The resource shape every tag panel reads: `None` while the panel's tab is
/// inactive (so an unopened tab fetches nothing), `Some(list)` once the CDR
/// has answered.
pub type TagList = Resource<Result<Option<Vec<ItemTagRow>>, ViewerError>>;

/// The copy one tag panel renders itself with — what the collection IS, so no
/// screen has to re-explain the openEHR tag model in its own words.
#[derive(Debug, Clone, Copy)]
pub struct TagPanelCopy {
    /// The heading over the tag table ("Tags").
    pub title: Msg,
    /// One paragraph naming the addressed collection and its whole-collection
    /// replace semantics.
    pub note: Msg,
    /// The empty state's headline.
    pub empty_message: Msg,
    /// The empty state's hint.
    pub empty_hint: Msg,
    /// The set-a-tag section's DOM id — the panel's stable E2E hook.
    pub form_id: &'static str,
}







/// The three released tag filters as a query string, each sent only when
/// non-empty.
///
/// That is exactly the released "omitted parameter constrains nothing"
/// behaviour: "In case no such parameter is provided then all `ITEM_TAG`
/// resources will be retrieved" (`operations/ehr_tags_get.yaml`, and the same
/// sentence in `demographic_tags_get.yaml`). The parameter names are the
/// released ones, which the viewer's own argument names deliberately do not
/// repeat (`tag_key` on both sides would only stutter).
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn tag_filter_query(key: &str, value: &str, target_path: &str) -> String {
    let mut query = String::new();
    for (name, value) in [
        ("tag_key", key.trim()),
        ("tag_value", value.trim()),
        ("tag_target_path", target_path.trim()),
    ] {
        if value.is_empty() {
            continue;
        }
        query.push(if query.is_empty() { '?' } else { '&' });
        query.push_str(name);
        query.push('=');
        query.push_str(&urlencoding::encode(value));
    }
    query
}

/// The whole-collection `UpdateItemTag` body that keeps `existing` and sets one
/// `(key, target_path)` entry.
///
/// Pure and unit-tested: the merge keys on the tag identity the released update
/// operation names (module docs), and it emits only the three declared members
/// (`schemas/common/UpdateItemTag.yaml` is `additionalProperties: false`, so a
/// re-sent `target`/`owner_id` would be a `400`).
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn merged_tag_body(
    existing: &[ItemTagRow],
    key: &str,
    value: &str,
    target_path: &str,
) -> String {
    let mut entries: Vec<Value> = Vec::with_capacity(existing.len().saturating_add(1));
    let mut replaced = false;
    for tag in existing {
        if tag.key == key && tag.target_path == target_path {
            entries.push(update_tag(key, value, target_path));
            replaced = true;
        } else {
            entries.push(update_tag(&tag.key, &tag.value, &tag.target_path));
        }
    }
    if !replaced {
        entries.push(update_tag(key, value, target_path));
    }
    Value::Array(entries).to_string()
}

/// One `UpdateItemTag` entry: `key` always, `value`/`target_path` only when
/// non-empty.
///
/// `ITEM_TAG`'s invariant `Inv_value_valid` refuses an EMPTY `value`
/// (`org.openehr.rm.common.item_tag.adoc`), and an absent `target_path` is what
/// "tags the whole object" means — so a blank field is OMITTED, never sent as
/// `""`.
#[cfg(not(target_arch = "wasm32"))]
fn update_tag(key: &str, value: &str, target_path: &str) -> Value {
    let mut entry = serde_json::Map::new();
    drop(entry.insert("key".to_owned(), Value::String(key.to_owned())));
    if !value.is_empty() {
        drop(entry.insert("value".to_owned(), Value::String(value.to_owned())));
    }
    if !target_path.is_empty() {
        drop(entry.insert(
            "target_path".to_owned(),
            Value::String(target_path.to_owned()),
        ));
    }
    Value::Object(entry)
}

/// Flatten an `ITEM_TAG` list body into [`ItemTagRow`]s. Defensive throughout —
/// an absent optional attribute reads as empty rather than failing the panel.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
#[cfg(not(target_arch = "wasm32"))]
pub fn parse_item_tags(body: &str) -> Result<Vec<ItemTagRow>, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(
            Msg::ItemTagListJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let items = doc.as_array().cloned().unwrap_or_default();
    Ok(items
        .iter()
        .map(|tag| ItemTagRow {
            key: string_at(tag, &["key"]),
            value: string_at(tag, &["value"]),
            target_path: string_at(tag, &["target_path"]),
            target: string_at(tag, &["target", "value"]),
            owner_id: string_at(tag, &["owner_id", "id", "value"]),
        })
        .collect())
}

/// Follow a chain of object keys to a string leaf, or an empty string when any
/// hop is absent or not a string.
#[cfg(not(target_arch = "wasm32"))]
fn string_at(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        match current.get(*key) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    current.as_str().unwrap_or_default().to_owned()
}

/// Group an aggregate tag list by the target each tag names.
///
/// An aggregate list "spans BOTH target forms (a `VERSIONED_OBJECT` container
/// and a specific VERSION) and every taggable kind"
/// (`operations/ehr_tags_get.yaml` retrieves the tags "associated with any
/// target VERSION or `VERSIONED_OBJECT` within the EHR"), so the target is what
/// a reader groups by. Pure and deterministic: targets in `BTreeMap` order,
/// each group's tags by identity — no clock, no hash iteration, so the server
/// pass and hydration render the same table.
#[must_use]
pub fn group_by_target(tags: Vec<ItemTagRow>) -> Vec<TagGroup> {
    let mut by_target: BTreeMap<String, Vec<ItemTagRow>> = BTreeMap::new();
    for tag in tags {
        by_target.entry(tag.target.clone()).or_default().push(tag);
    }
    by_target
        .into_iter()
        .map(|(target, mut tags)| {
            tags.sort_by_key(ItemTagRow::identity);
            TagGroup { target, tags }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ItemTagRow, group_by_target};

    fn row(key: &str, target: &str) -> ItemTagRow {
        ItemTagRow {
            key: key.to_owned(),
            target: target.to_owned(),
            ..ItemTagRow::default()
        }
    }

    #[test]
    fn a_tags_identity_is_its_key_and_target_path_together() {
        let bare = ItemTagRow {
            key: "flag".to_owned(),
            ..ItemTagRow::default()
        };
        let pathed = ItemTagRow {
            key: "flag".to_owned(),
            target_path: "/details".to_owned(),
            ..ItemTagRow::default()
        };
        // The released update operation identifies a tag by the PAIR, so two
        // same-key tags on different paths must be distinct `<For>` keys.
        assert_ne!(bare.identity(), pathed.identity());
        assert_eq!(bare.identity(), bare.clone().identity());
    }

    #[test]
    fn the_same_key_on_two_targets_is_two_rows_in_an_aggregate_list() {
        let here = row("flag", "8849182c");
        let there = row("flag", "b1e6a0c4::sys::2");
        // Within one collection the identities collide by design; across an
        // aggregate list the target disambiguates them.
        assert_eq!(here.identity(), there.identity());
        assert_ne!(here.global_identity(), there.global_identity());
    }

    #[test]
    fn grouping_puts_every_tag_under_its_own_target_deterministically() {
        let groups = group_by_target(vec![
            row("reviewed", "b1e6a0c4::sys::2"),
            row("flag", "8849182c"),
            row("archived", "8849182c"),
        ]);
        assert_eq!(groups.len(), 2);
        // Targets in BTreeMap order, tags by identity inside each group — the
        // same order on the server pass and at hydration.
        assert_eq!(groups[0].target, "8849182c");
        let keys: Vec<&str> = groups[0].tags.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, vec!["archived", "flag"]);
        assert_eq!(groups[1].target, "b1e6a0c4::sys::2");
        assert_eq!(groups[1].tags.len(), 1);
        // The container form and a VERSION of the same object are DISJOINT
        // collections (composition_tags_get.yaml), so they never merge.
        let split = group_by_target(vec![
            row("flag", "8849182c"),
            row("flag", "8849182c::sys::1"),
        ]);
        assert_eq!(split.len(), 2);
        assert_eq!(
            group_by_target(Vec::new()),
            Vec::<crate::bff::item_tags::TagGroup>::new()
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod wire_tests {
    use super::{ItemTagRow, merged_tag_body, parse_item_tags, tag_filter_query};
    use serde_json::Value;

    #[test]
    fn parses_a_served_tag_list_including_the_optional_attributes() {
        let body = r#"[
            {"_type":"ITEM_TAG","key":"flag","value":"follow-up",
             "target":{"_type":"HIER_OBJECT_ID","value":"8849182c"},
             "owner_id":{"_type":"OBJECT_REF","namespace":"local","type":"EHR","id":{"_type":"HIER_OBJECT_ID","value":"7d44b88c"}}},
            {"_type":"ITEM_TAG","key":"reviewed","target_path":"/context/start_time/value",
             "target":{"_type":"OBJECT_VERSION_ID","value":"8849182c::example.org::2"}}
        ]"#;
        let rows = parse_item_tags(body).expect("a valid list");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "flag");
        assert_eq!(rows[0].value, "follow-up");
        assert_eq!(rows[0].target, "8849182c");
        assert_eq!(rows[0].owner_id, "7d44b88c");
        // An absent optional attribute reads as empty, never as a failure.
        assert_eq!(rows[0].target_path, "");
        assert_eq!(rows[1].value, "");
        assert_eq!(rows[1].target_path, "/context/start_time/value");
        // An empty collection is a first-class answer, not an error.
        assert_eq!(
            parse_item_tags("[]").expect("empty list"),
            Vec::<ItemTagRow>::new()
        );
        assert!(parse_item_tags("not json").is_err());
    }

    fn row(key: &str, value: &str, target_path: &str) -> ItemTagRow {
        ItemTagRow {
            key: key.to_owned(),
            value: value.to_owned(),
            target_path: target_path.to_owned(),
            target: "8849182c".to_owned(),
            owner_id: "7d44b88c".to_owned(),
        }
    }

    #[test]
    fn setting_a_new_tag_keeps_every_existing_one() {
        let body = merged_tag_body(&[row("flag", "follow-up", "")], "reviewed", "true", "");
        let entries: Vec<Value> = serde_json::from_str(&body).expect("a JSON array");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["key"], "flag");
        assert_eq!(entries[0]["value"], "follow-up");
        assert_eq!(entries[1]["key"], "reviewed");
        // Only the three declared UpdateItemTag members are ever sent — the
        // schema is additionalProperties: false, so target/owner_id would 400.
        for entry in &entries {
            let keys: Vec<&String> = entry.as_object().expect("an object entry").keys().collect();
            assert!(
                keys.iter()
                    .all(|k| matches!(k.as_str(), "key" | "value" | "target_path")),
                "{keys:?}"
            );
        }
    }

    #[test]
    fn setting_the_same_key_and_path_replaces_that_one_entry() {
        let body = merged_tag_body(
            &[row("flag", "old", ""), row("flag", "kept", "/details")],
            "flag",
            "new",
            "",
        );
        let entries: Vec<Value> = serde_json::from_str(&body).expect("a JSON array");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["value"], "new");
        // The same key on a DIFFERENT target_path is a different tag and stays.
        assert_eq!(entries[1]["value"], "kept");
        assert_eq!(entries[1]["target_path"], "/details");
    }

    #[test]
    fn a_blank_value_or_path_is_omitted_rather_than_sent_empty() {
        // ITEM_TAG Inv_value_valid refuses an empty value; an absent
        // target_path is what "tags the whole object" means.
        let body = merged_tag_body(&[], "flag", "", "");
        let entries: Vec<Value> = serde_json::from_str(&body).expect("a JSON array");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].get("value").is_none());
        assert!(entries[0].get("target_path").is_none());
        assert_eq!(entries[0]["key"], "flag");
    }

    #[test]
    fn only_the_filters_the_reader_filled_reach_the_wire() {
        // An omitted parameter constrains nothing, so a blank field is omitted
        // rather than sent empty (ehr_tags_get.yaml).
        assert_eq!(tag_filter_query("", "", ""), "");
        assert_eq!(tag_filter_query(" flag ", "", ""), "?tag_key=flag");
        assert_eq!(
            tag_filter_query("flag", "follow-up", "/context/start_time/value"),
            "?tag_key=flag&tag_value=follow-up&tag_target_path=%2Fcontext%2Fstart_time%2Fvalue"
        );
        // A filter value can carry anything a key can, so it is percent-encoded
        // rather than concatenated.
        assert_eq!(tag_filter_query("a&b=c", "", ""), "?tag_key=a%26b%3Dc");
    }
}
