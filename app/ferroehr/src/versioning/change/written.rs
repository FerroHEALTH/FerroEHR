// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a write is about to store, as the access evaluator judges it before
//! anything is stored (#5626). No openEHR spec governs this: our own design.

use serde_json::Value;

use crate::access_eval::scope::{Item, Object};
use crate::error::ServiceError;
use crate::storage::row::NodeRow;
use crate::versioning::restriction;

/// Refuse, before anything is stored, the content `resolved` would store
/// where `gate`, the access evaluator's verdict of its EHR, refuses it; a
/// logical delete stores no content (#5626).
///
/// # Errors
/// The refusal of [`restriction::WriteGate::admit_written`].
pub(super) fn admit_written(
    gate: Option<&restriction::WriteGate>,
    resolved: &super::ResolvedWrite,
) -> Result<(), ServiceError> {
    let Some(gate) = gate.filter(|_| resolved.canonical_text.is_some()) else {
        return Ok(());
    };
    let archetype = root_archetype(&resolved.body);
    gate.admit_written(&Item::Object(Object {
        vo_id: resolved.vo_id.0,
        template: resolved.template_id.as_deref(),
        archetype: archetype.as_deref(),
        origins: Some(&resolved.origins),
        recorded: Some(resolved.time_committed),
    }))
}

/// The root's `archetype_node_id` off the body, for the outbox envelope, the
/// audit record and the access evaluator; `None` for a logical delete.
pub(super) fn root_archetype(body: &Value) -> Option<String> {
    body.get("archetype_node_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The node rows a body decomposes into, their fragments index-aligned, and
/// the external blob URIs it references ([`nodes`]).
pub(super) type Nodes = (Vec<NodeRow>, Vec<Value>, Vec<String>);

/// The node rows `body` decomposes into, each with its fragment taken out
/// for the write to store uncopied, and the external blob URIs the body
/// references (#5759); none for a logical delete or a version that `shares`
/// its predecessor's rows.
///
/// # Errors
/// The decomposition's refusal of a body that is no single tree.
pub(super) fn nodes(body: Value, shares: bool) -> Result<Nodes, ServiceError> {
    if shares || body.is_null() {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }
    let uris = crate::storage::blob_ref::uris_of(&body);
    let mut rows = crate::storage::codec::decompose(body)?;
    let fragments = rows
        .iter_mut()
        .map(|row| std::mem::take(&mut row.data))
        .collect();
    Ok((rows, fragments, uris))
}
