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
    let archetype = root_archetype(&resolved.rows);
    gate.admit_written(&Item::Object(Object {
        vo_id: resolved.vo_id.0,
        template: resolved.template_id.as_deref(),
        archetype: archetype.as_deref(),
        origins: Some(&resolved.origins),
        recorded: Some(resolved.time_committed),
    }))
}

/// The root's `archetype_node_id` off the decomposed rows, for the outbox
/// envelope, the audit record and the access evaluator.
pub(super) fn root_archetype(rows: &[NodeRow]) -> Option<String> {
    rows.first()
        .filter(|root| root.num == 0)
        .and_then(|root| root.data.get("archetype_node_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}
