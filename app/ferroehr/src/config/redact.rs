// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The redacted rendering of a configuration tree.
//!
//! Every URL leaf is masked by `ferrohealth-smart`'s [`mask_userinfo`], and
//! the unset keys are left out of a TOML rendering. No openEHR spec governs
//! configuration: our own design.

use ferrohealth_smart::config::mask_userinfo;

/// Masks every URL string in `tree`, recursively, by [`mask_userinfo`].
pub(super) fn mask_url_userinfo(tree: &mut serde_json::Value) {
    match tree {
        serde_json::Value::String(text) => *text = mask_userinfo(text),
        serde_json::Value::Array(items) => items.iter_mut().for_each(mask_url_userinfo),
        serde_json::Value::Object(members) => members.values_mut().for_each(mask_url_userinfo),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

/// Returns `tree` without its `null` object members, the unset keys a TOML
/// rendering leaves out (TOML has no null value).
pub(super) fn without_nulls(tree: serde_json::Value) -> serde_json::Value {
    match tree {
        serde_json::Value::Object(members) => serde_json::Value::Object(
            members
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key, without_nulls(value)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(without_nulls).collect())
        }
        leaf => leaf,
    }
}
