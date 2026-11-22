// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A resolved write signed and decomposed, ready for the statement that
//! writes it: one version's statement, or the one statement a CONTRIBUTION
//! writes its versions in (#6256).
//!
//! The signature is computed over the assembled `ORIGINAL_VERSION` (RM common
//! master06 §Digital Signature), which embeds `time_committed` and
//! `contribution_id`; both are known before any statement, so the statement
//! binds the instant the signature was computed over and stored equals signed
//! by construction. No openEHR spec governs statement batching: our own
//! design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::sign::body_and_signature;
use super::written::{nodes, root_archetype};
use super::{Committed, ResolvedWrite};
use crate::error::ServiceError;
use crate::storage::render::Decomposed;
use crate::storage::version_repo::commit::FoldedVersion;
use crate::versioning::audit::AuditInput;
use crate::versioning::object_version_id::object_version_id;
use crate::versioning::{SigningCtx, attestation};

/// A resolved write signed and decomposed: everything its statement binds.
pub(super) struct Prepared {
    /// The resolved write, its body dropped once signed.
    r: ResolvedWrite,
    /// `VERSION.signature`, and whether the client supplied it.
    signature: (Option<String>, bool),
    /// The node rows and blob URIs of the rendering.
    stored: Decomposed,
    /// The accompanying attestations, completed once and signed as stored.
    attestations: Vec<Value>,
    /// The outcome the statement commits, its instant the one signed.
    pub(super) committed: Committed,
    /// The CONTRIBUTION the version is written under.
    pub(super) contribution_id: Uuid,
}

/// Sign and decompose `r`, written under `contribution_id` with `audit`.
///
/// # Errors
/// [`ServiceError::Signing`] when the canonical form cannot be produced or the
/// signer fails, and the decomposition's refusal of a body that is no single
/// tree.
pub(super) fn prepare(
    ctx: &SigningCtx<'_>,
    audit: &AuditInput,
    contribution_id: Uuid,
    mut r: ResolvedWrite,
) -> Result<Prepared, ServiceError> {
    // The accompanying attestations are completed ONCE before the signature —
    // master06 §Digital Signature signs "the entire Version object" — and
    // reused verbatim for the insert, so signed bytes and stored bytes are the
    // same bytes.
    let attestations: Vec<Value> =
        attestation::complete_accompanying(&r.attestations, &audit.system_id, r.time_committed)
            .iter()
            .map(openehr_its::json::to_canonical_value)
            .collect();
    let signature = body_and_signature(ctx, audit, contribution_id, &r, &attestations)?;
    let archetype = root_archetype(r.body.as_ref());
    // The body is signed: what is left to store is read off its rendering.
    drop(r.body.take());
    // A version that shares its predecessor's rows writes none of its own.
    let version_uid = object_version_id(r.vo_id, &ctx.system_id, r.tree);
    let stored = nodes(
        r.rendered.as_ref(),
        &crate::storage::compact::RootIdentity {
            version_uid: &version_uid,
            template_id: r.template_id.as_deref(),
        },
        r.shared_from.is_some(),
    )?;
    let committed = Committed {
        vo_id: r.vo_id,
        sys_version: r.ordinal,
        tree: r.tree,
        creating_system_id: ctx.system_id.clone(),
        kind: r.kind,
        change_type: audit.change_type.clone(),
        template_id: r.template_id.clone(),
        root_archetype: archetype,
        time_committed: r.time_committed,
        skipped: false,
        content_digest: r.content_digest,
        deleted: None,
        subject_moved: false,
    };
    Ok(Prepared {
        r,
        signature,
        stored,
        attestations,
        committed,
        contribution_id,
    })
}

impl Prepared {
    /// The version columns its statement binds.
    ///
    /// # Errors
    /// [`ServiceError::Internal`] when the rendering is no UTF-8 text.
    pub(super) fn folded(&self) -> Result<FoldedVersion<'_>, ServiceError> {
        let r = &self.r;
        let (trunk_version, branch_number, branch_version) = r.tree.columns();
        let body = r
            .rendered
            .as_ref()
            .map(|rendered| std::str::from_utf8(rendered.text()))
            .transpose()
            .map_err(|e| ServiceError::internal("serialize the canonical body", e))?;
        Ok(FoldedVersion {
            vo_id: r.vo_id,
            kind: r.kind.as_str(),
            ehr_id: r.ehr_id,
            sys_version: r.ordinal,
            trunk_version,
            branch_number,
            branch_version,
            lifecycle_state: &r.lifecycle,
            creating_system_id: &self.committed.creating_system_id,
            preceding_version_uid: r.preceding_uid.as_deref(),
            other_input_version_uids: &r.other_input_version_uids,
            template_id: r.template_id.as_deref(),
            signature: self.signature.0.as_deref(),
            signature_client_supplied: self.signature.1,
            stable_compatible: r.stable_compatible,
            origins: &r.origins,
            body,
            time_committed: r.time_committed,
            rows: &self.stored.rows,
            blob_uris: &self.stored.blob_uris,
            content_digest: r.content_digest.as_ref().map(<[u8; 32]>::as_slice),
            content_sys_version: r.shared_from,
            root_archetype: self.committed.root_archetype.as_deref(),
        })
    }

    /// Whether the version is a new FOLDER hierarchy of an EHR, which its
    /// statement then writes the membership of in place of [`Self::finish`].
    pub(super) fn take_first_folder(&mut self) -> bool {
        std::mem::take(&mut self.r.is_first_folder) && self.r.ehr_id.is_some()
    }

    /// Whether the version's statement leaves rows for [`Self::finish`] to
    /// write: a new FOLDER hierarchy's membership or accompanying
    /// attestations.
    pub(super) fn has_more(&self) -> bool {
        self.r.is_first_folder || !self.attestations.is_empty()
    }

    /// Whether the version writes rows after its statement in the same
    /// connection: [`Self::has_more`]'s, or the blob references its
    /// statement's caller records.
    pub(super) fn writes_after(&self) -> bool {
        self.has_more() || !self.stored.blob_uris.is_empty()
    }

    /// Write what the version's statement leaves: a newly created FOLDER
    /// hierarchy joins `EHR.folders` (RM ehr master04 §Folders), and the
    /// accompanying attestations land beside the version.
    ///
    /// # Errors
    /// The storage errors of either write.
    pub(super) async fn finish(&self, tx: &mut PgConnection) -> Result<(), ServiceError> {
        if self.r.is_first_folder
            && let Some(ehr_id) = self.r.ehr_id
        {
            crate::storage::version_repo::commit::insert_ehr_folder_rank(tx, ehr_id, self.r.vo_id)
                .await?;
        }
        attestation::insert_at_committal_attestations(
            tx,
            self.r.vo_id,
            self.r.ordinal,
            self.contribution_id,
            &self.attestations,
        )
        .await?;
        Ok(())
    }
}
