// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The redacted rendering of a configuration tree.
//!
//! Every URL leaf is masked by `ferrohealth-secret`'s [`RedactedUrl`], and
//! the unset keys are left out of a TOML rendering. No openEHR spec governs
//! configuration: our own design.

use ferrohealth_secret::secret_url::RedactedUrl;

/// Returns `text` masked by `ferrohealth-secret`'s [`RedactedUrl`] when it
/// holds `://`, and as written otherwise: text without `://` has no userinfo
/// to mask, where [`RedactedUrl`] would mask it whole.
#[must_use]
pub fn mask_if_url(text: &str) -> String {
    if text.contains("://") {
        RedactedUrl(text).to_string()
    } else {
        text.to_owned()
    }
}

/// Masks every URL string in `tree`, recursively, by [`mask_if_url`].
pub(super) fn mask_url_userinfo(tree: &mut serde_json::Value) {
    match tree {
        serde_json::Value::String(text) => *text = mask_if_url(text),
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
