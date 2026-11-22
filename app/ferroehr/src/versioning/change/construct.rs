// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The constructors of a [`Change`] a service write builds, and of the
//! [`Write`] that carries it, and the folded statement a write commits in.

use uuid::Uuid;

use super::{Change, Committed, ContributionCtx, Write, WriteEnvelope};
use crate::ids::{AllocatedVoId, EhrId, VoId};
use crate::storage::version_repo::commit::{Folded, FoldedVersion, Guard, outbox_envelope};
use crate::versioning::Kind;
use crate::versioning::SigningCtx;
use crate::versioning::audit::AuditInput;
use crate::versioning::body::Body;
use crate::versioning::object_version_id::TreeId;
use crate::versioning::restriction;

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
            guard: None,
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

    /// The same write off the placement the caller already read, under the
    /// per-object lock it holds or in the gate read a guard requires unchanged.
    pub(crate) fn placed(
        self,
        placement: crate::storage::version_repo::placement::Placement,
    ) -> Self {
        Self {
            placement: Some(placement),
            ..self
        }
    }

    /// The same write, committed only while what its gate read judged still
    /// holds ([`Guard`]).
    pub(crate) fn guarded(self, guard: Guard) -> Self {
        Self {
            guard: Some(guard),
            ..self
        }
    }

    /// The same write, a logical delete of a version that held `deleted`.
    pub(crate) fn deleting(
        self,
        deleted: Option<crate::system_log::categories::ContentIds>,
    ) -> Self {
        Self { deleted, ..self }
    }
}

/// The folded statement of `version` under `contribution`: a guarded one
/// carrying `committed`'s outbox row when eventing is enabled, a standalone
/// one, or one into an existing CONTRIBUTION.
pub(super) fn folded_shape<'a>(
    contribution: ContributionCtx,
    contribution_id: Uuid,
    version: FoldedVersion<'a>,
    ctx: &SigningCtx<'_>,
    committed: &Committed,
) -> Folded<'a> {
    match contribution {
        ContributionCtx::New(Some(guard)) => Folded::Guarded {
            id: contribution_id,
            guard: Guard {
                outbox: ctx.outbox_enabled.then(|| {
                    let entry = [committed.envelope_entry()];
                    outbox_envelope(
                        contribution_id,
                        version.ehr_id,
                        committed.time_committed,
                        &entry,
                    )
                }),
                ..guard
            },
            version,
        },
        ContributionCtx::New(None) => Folded::Standalone {
            id: contribution_id,
            version,
        },
        ContributionCtx::Existing(id) => Folded::Into { id, version },
    }
}
