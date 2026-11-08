// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The constructors of a [`Change`] a service write builds.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): commit bodies are the canonical \
              form the commit interior carries (stored-content class)"
)]

use serde_json::Value;

use super::{Change, WriteEnvelope};
use crate::ids::{AllocatedVoId, VoId};
use crate::versioning::Kind;
use crate::versioning::object_version_id::TreeId;

impl Change {
    /// A [`Change::Create`] of `canonical` with the caller's envelope.
    pub(crate) fn create(
        kind: Kind,
        canonical: Value,
        template_id: Option<String>,
        envelope: WriteEnvelope,
    ) -> Self {
        Change::Create {
            allocated: None,
            kind,
            canonical,
            template_id,
            signature: envelope.signature,
            lifecycle_state: envelope.lifecycle_state,
            attestations: envelope.attestations,
            other_input_version_uids: Vec::new(),
        }
    }

    /// A [`Change::Create`] of the party `canonical` under the id
    /// `allocated`, which other bodies of the same CONTRIBUTION may already
    /// reference.
    pub(crate) fn create_allocated(
        allocated: AllocatedVoId,
        kind: Kind,
        canonical: Value,
        envelope: WriteEnvelope,
    ) -> Self {
        Change::Create {
            allocated: Some(allocated),
            kind,
            canonical,
            template_id: None,
            signature: envelope.signature,
            lifecycle_state: envelope.lifecycle_state,
            attestations: envelope.attestations,
            other_input_version_uids: Vec::new(),
        }
    }

    /// A [`Change::Modify`] of `vo_id` to `canonical` with the caller's
    /// envelope; `expected` is the tip the caller's `If-Match` names.
    pub(crate) fn modify(
        vo_id: VoId,
        kind: Kind,
        canonical: Value,
        expected: Option<TreeId>,
        template_id: Option<String>,
        envelope: WriteEnvelope,
    ) -> Self {
        Change::Modify {
            vo_id,
            kind,
            canonical,
            expected,
            template_id,
            signature: envelope.signature,
            lifecycle_state: envelope.lifecycle_state,
            attestations: envelope.attestations,
            other_input_version_uids: Vec::new(),
        }
    }

    /// A [`Change::Delete`] of `vo_id`. Deletion fixes the lifecycle state at
    /// `523|deleted|` itself (master06 §Logical Deletion), so it takes no
    /// envelope lifecycle.
    pub(crate) fn delete(vo_id: VoId, kind: Kind, expected: Option<TreeId>) -> Self {
        Change::Delete {
            vo_id,
            kind,
            expected,
            signature: None,
            other_input_version_uids: Vec::new(),
        }
    }
}
