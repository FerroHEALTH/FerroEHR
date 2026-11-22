// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The constructors of a [`Change`] a service write builds, and of the
//! [`Write`] that carries it.

use super::{Change, Write, WriteEnvelope};
use crate::ids::{AllocatedVoId, EhrId, VoId};
use crate::versioning::audit::AuditInput;
use crate::versioning::restriction;
use crate::versioning::Kind;
use crate::versioning::body::Body;
use crate::versioning::object_version_id::TreeId;

impl Change {
    /// A [`Change::Create`] of `canonical` with the caller's envelope.
    pub(crate) fn create(
        kind: Kind,
        canonical: impl Into<Body>,
        template_id: Option<String>,
        envelope: WriteEnvelope,
    ) -> Self {
        Change::Create {
            allocated: None,
            kind,
            body: canonical.into(),
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
        canonical: impl Into<Body>,
        envelope: WriteEnvelope,
    ) -> Self {
        Change::Create {
            allocated: Some(allocated),
            kind,
            body: canonical.into(),
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
        canonical: impl Into<Body>,
        expected: Option<TreeId>,
        template_id: Option<String>,
        envelope: WriteEnvelope,
    ) -> Self {
        Change::Modify {
            vo_id,
            kind,
            body: canonical.into(),
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

impl Write {
    /// A write of `change` to the object in `ehr_id` (or an ehr-less one),
    /// reading its own commit instant and placement.
    pub(crate) fn new(ehr_id: Option<EhrId>, change: Change, audit: AuditInput) -> Self {
        Self {
            ehr_id,
            change,
            audit,
            known: restriction::Known::default(),
            placement: None,
            deleted: None,
        }
    }

    /// The same write at the commit instant `now` the caller already read.
    pub(crate) fn at(self, now: jiff::Timestamp) -> Self {
        Self {
            known: restriction::Known {
                now: Some(now),
                ..self.known
            },
            ..self
        }
    }

    /// The same write off the placement the caller already read under the
    /// per-object lock it holds.
    pub(crate) fn placed(
        self,
        placement: crate::storage::version_repo::placement::Placement,
    ) -> Self {
        Self {
            placement: Some(placement),
            ..self
        }
    }

    /// The same write, a logical delete of a version that held `deleted`.
    pub(crate) fn deleting(self, deleted: Option<crate::system_log::categories::ContentIds>) -> Self {
        Self { deleted, ..self }
    }
}
