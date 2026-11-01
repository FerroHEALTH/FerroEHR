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

/// Serialize the **root** node so it satisfies the normative ITS-REST
/// `Tree` schema (`schemas/web_template/Tree.yaml`), whose `required` set —
/// `id, name, localizedName, rmType, nodeId, min, max, localizedNames,
/// localizedDescriptions, aqlPath, children` — is stricter than the `Child`
/// schema a nested [`WebTemplateNode`] serializes against.
///
/// A well-formed OPT root already carries every required member, so this
/// reproduces the normal node serialization byte-for-byte; it only *fills in*
/// members a sparse root would otherwise omit (empty rubric maps, an unbounded
/// `min`, an empty `nodeId`, no children), so a strict JSON-Schema validator
/// accepts the `WebTemplate`. Children keep the looser `Child`-schema shape
/// (`schemas/web_template/Child.yaml`). Missing scalars default to spec-valid
/// placeholders
/// (`name`/`localizedName` ← the node `id`, `nodeId` ← `""`, `min` ← `0`).
pub(super) fn serialize_root<S: Serializer>(
    node: &WebTemplateNode,
    s: S,
) -> Result<S::Ok, S::Error> {
    // Serialize via the derived (Child-shaped) impl, then ensure the
    // Tree-required members are present. Appended keys are schema-legal (JSON
    // object member order is not significant); nothing already emitted is
    // touched, so the well-formed case is unchanged.
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
