// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The serialization of a Web Template's root node against the stricter
//! `Tree` schema of ITS-REST `schemas/web_template/Tree.yaml`.

#![expect(
    clippy::disallowed_types,
    reason = "the root is filled in on its serialized wire value (#1694)"
)]

use serde::{Serialize, Serializer};

use super::model::WebTemplateNode;

/// Serialize the root node so it satisfies the ITS-REST `Tree` schema
/// (`schemas/web_template/Tree.yaml`), filling the members a sparse root would
/// omit; children keep the `Child` shape.
pub(super) fn serialize_root<S: Serializer>(
    node: &WebTemplateNode,
    s: S,
) -> Result<S::Ok, S::Error> {
    // Appended members are schema-legal; emitted ones are untouched.
    use serde_json::json;
    let mut value = serde_json::to_value(node).map_err(serde::ser::Error::custom)?;
    if let serde_json::Value::Object(map) = &mut value {
        map.entry("name").or_insert_with(|| json!(node.id));
        map.entry("localizedName")
            .or_insert_with(|| json!(node.name.clone().unwrap_or_else(|| node.id.clone())));
        map.entry("nodeId").or_insert_with(|| json!(""));
        map.entry("min").or_insert_with(|| json!(0));
        map.entry("localizedNames").or_insert_with(|| json!({}));
        map.entry("localizedDescriptions")
            .or_insert_with(|| json!({}));
        map.entry("children").or_insert_with(|| json!([]));
    }
    value.serialize(s)
}
