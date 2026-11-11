// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The TOML rendering of the redacted configuration tree.
//!
//! The tree is already redacted by its secret types; the unset keys are left
//! out of a TOML rendering. No openEHR spec governs configuration: our own
//! design.

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
