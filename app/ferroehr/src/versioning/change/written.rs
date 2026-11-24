// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a write is about to store, as the access evaluator judges it before
//! anything is stored (#5626). No openEHR spec governs this: our own design.

use crate::access_eval::scope::{Item, Object};
use crate::error::ServiceError;
use crate::storage::compact::RootIdentity;
use crate::storage::render::Decomposed;
use crate::storage::render::index::Rendered;
use crate::versioning::body::Body;
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
    let Some(gate) = gate.filter(|_| resolved.rendered.is_some()) else {
        return Ok(());
    };
    let archetype = root_archetype(resolved.body.as_ref());
    gate.admit_written(&Item::Object(Object {
        vo_id: resolved.vo_id.0,
        template: resolved.template_id.as_deref(),
        archetype: archetype.as_deref(),
        origins: Some(&resolved.origins),
        recorded: Some(resolved.time_committed),
        episodes: Some(&resolved.episodes),
    }))
}

/// The root's `archetype_node_id` off the body, for the outbox envelope, the
/// audit record and the access evaluator; `None` for a logical delete.
pub(super) fn root_archetype(body: Option<&Body>) -> Option<String> {
    body?.archetype_node_id().map(str::to_owned)
}

/// The node rows and blob URIs read off the rendering of a body, slimmed
/// against the version identity `root`; none for a logical delete or a
/// version that `shares` its predecessor's rows.
///
/// # Errors
/// The decomposition's refusal of a body that is no single tree.
pub(super) fn nodes(
    rendered: Option<&Rendered>,
    root: &RootIdentity<'_>,
    shares: bool,
) -> Result<Decomposed, ServiceError> {
    let Some(rendered) = rendered.filter(|_| !shares) else {
        return Ok(Decomposed::default());
    };
    crate::telemetry::conversion::record("text_to_nodes");
    Ok(crate::storage::render::decomposed(rendered, root)?)
}
