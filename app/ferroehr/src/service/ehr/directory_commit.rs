// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The directory (FOLDER) writes, each judged on one gate read and committed
//! in one guarded statement after its access record (#6257), as a
//! COMPOSITION write is (#6256).
//!
//! The gate read returns the EHR's state, the directory slot's tip and which
//! of the FOLDER's local `items` exist; a new hierarchy's `ehr_folder` row is
//! written by the statement itself. Spec: RM ehr `master04-ehr_package.adoc`
//! §Folders and §EHR Active Status, and the EHR class invariant
//! `Directory_in_folders`. No openEHR spec governs how a commit is run
//! against the database: our own design.

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::response::ServiceResponse;
use crate::service::version_update::WriteRequest;
use crate::service::write::{Materials, Resolved, first_attempt};
use crate::storage::version_repo::gate::{self, Ask, Gate, Shape, Target};
use crate::storage::version_repo::guarded::Guard;
use crate::validation::versioned::{item_ref_roots, judge_folder_item_refs, local_item_refs};
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::body::Body;
use crate::versioning::change::precondition::{ensure_preceding, not_modifiable_error, tip_uid};
use crate::versioning::change::{Change, Write};
use crate::versioning::lifecycle::state::DELETED;
use openehr_rm::prelude::Folder;

use super::composition::guard_of;

impl FerroEhrService {
    /// Create the EHR's directory (its root FOLDER).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR does not exist;
    /// [`ServiceError::Unprocessable`] when the FOLDER tree is invalid;
    /// [`ServiceError::Conflict`] when the EHR is not modifiable or already
    /// has a directory; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn commit_new_directory(
        &self,
        ehr_id: EhrId,
        request: WriteRequest<Folder>,
    ) -> Result<ServiceResponse, ServiceError> {
        // 553|incomplete| relaxes the existence/cardinality lower bounds (RM
        // common master06 §Incomplete Content).
        let Resolved {
            audit,
            envelope,
            incomplete,
        } = self.resolve_write(&request, change_type::CREATION, "DIRECTORY creation")?;
        let mut folder = Body::from(request.data);
        // Validation runs before a connection is held, and its refusal surfaces
        // after the EHR's 404, in pre-check order; a valid body's large media go
        // to the blob store first.
        let validated = self
            .accept_body(Kind::Folder, &mut folder, incomplete)
            .await?;
        let refs = local_item_refs(&folder, &self.effective_system_id());
        let items = item_ref_roots(&refs);
        // A create judged again follows a concurrent one that took the slot, so
        // it can only refuse and keeps no copy of its content.
        let materials = Materials::new((folder, audit, envelope), false);
        let validated = std::sync::Mutex::new(Some(validated));
        let (refs, items, materials, validated) = (&refs, &items, &materials, &validated);
        let committed = self
            .commit_gated(&self.pool, move || {
                async move |conn| {
                    let gate = gate::read(conn, directory_ask(ehr_id, items)).await?;
                    self.ensure_gate_policy_held().await?;
                    if gate.is_modifiable.is_none() {
                        return Err(ServiceError::not_found(
                            Missing::Ehr,
                            format!("EHR {ehr_id}"),
                        ));
                    }
                    if let Some(validated) = first_attempt(validated) {
                        validated?;
                    }
                    judge_folder_item_refs(ehr_id, refs, &gate.known_items)?;
                    ensure_modifiable(&gate, ehr_id)?;
                    // `POST /directory` manages the single directory slot = EHR.directory
                    // (= folders[1], RM ehr §EHR Class Directory_in_folders), conflicting
                    // only when a LIVE hierarchy occupies it.
                    // NOTE: no released sentence says whether a logically DELETED directory
                    // still occupies the slot — our choice is that it does not block a
                    // re-create, and 409 is likewise ours.
                    if gate.targets.first().is_some_and(is_live) {
                        return Err(ServiceError::conflict(format!(
                            "EHR {ehr_id} already has a directory"
                        )));
                    }
                    // NOTE: our own design (#6318), the hierarchy takes the rank after the gate
                    // read's highest, so one committed since clashes on `pk_ehr_folder`.
                    let rank = gate.folder_rank.checked_add(1).ok_or_else(|| {
                        ServiceError::exception("the EHR's folder ranks are exhausted")
                    })?;
                    let (folder, audit, envelope) = materials.take()?;
                    Ok(Write::new(
                        Some(ehr_id),
                        Change::create(Kind::Folder, folder, None, envelope),
                        audit,
                    )
                    .at(gate.now)
                    .gated(gate.restriction.clone())
                    .guarded(Guard {
                        folder: Some(rank),
                        ..guard_of(&gate, None)
                    }))
                }
            })
            .await?;
        // The write response is metadata-only: `Committed` already carries the
        // version identity and commit instant (RM common master06 §Committal and
        // Audits), so a representation response re-reads at the protocol layer instead.
        Ok(self.committed_response(ehr_id, &committed))
    }

    /// Update the EHR's directory; the request's `preceding` (`If-Match`) is
    /// checked against the tip its gate read finds.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no directory;
    /// [`ServiceError::VersionConflict`] when `If-Match` names another
    /// version; [`ServiceError::Unprocessable`] when the FOLDER tree is
    /// invalid; [`ServiceError::Conflict`] when the EHR is not modifiable;
    /// [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn commit_directory_update(
        &self,
        ehr_id: EhrId,
        request: WriteRequest<Folder>,
    ) -> Result<ServiceResponse, ServiceError> {
        // Every content check runs before a connection is held, and its refusal
        // surfaces after the `If-Match` check, in pre-check order.
        let resolved = self.resolve_write(&request, change_type::MODIFICATION, "DIRECTORY update");
        let preceding = request.preceding;
        let mut folder = Body::from(request.data);
        let validated = match &resolved {
            Ok(parts) => Some(
                self.accept_body(Kind::Folder, &mut folder, parts.incomplete)
                    .await?,
            ),
            Err(_) => None,
        };
        let refs = local_item_refs(&folder, &self.effective_system_id());
        let items = item_ref_roots(&refs);
        let (resolved, unresolved) = resolved.map_or_else(
            |error| (None, Some(error)),
            |resolved| (Some(resolved), None),
        );
        // An update a later attempt judges again commits on the new tip only when
        // it names no preceding version, so only such a one keeps a copy.
        let materials = Materials::new((folder, resolved), preceding.is_none());
        let unresolved = std::sync::Mutex::new(unresolved);
        let validated = std::sync::Mutex::new(validated);
        let (refs, items, preceding) = (&refs, &items, &preceding);
        let (materials, unresolved, validated) = (&materials, &unresolved, &validated);
        let committed = self
            .commit_gated(&self.pool, move || {
                async move |conn| {
                    let mut gate = gate::read(conn, directory_ask(ehr_id, items)).await?;
                    self.ensure_gate_policy_held().await?;
                    let (target, latest) = directory_target(&mut gate, ehr_id)?;
                    ensure_preceding(&latest, preceding.as_ref())?;
                    if let Some(error) = first_attempt(unresolved) {
                        return Err(error);
                    }
                    if let Some(validation) = first_attempt(validated) {
                        validation?;
                    }
                    judge_folder_item_refs(ehr_id, refs, &gate.known_items)?;
                    // is_modifiable = False forbids content writes to the directory (RM
                    // ehr master04 §EHR Active Status), after the FOLDER's 422.
                    ensure_modifiable(&gate, ehr_id)?;
                    let (
                        folder,
                        Some(Resolved {
                            audit, envelope, ..
                        }),
                    ) = materials.take()?
                    else {
                        return Err(ServiceError::exception(
                            "an unresolved DIRECTORY update reached its write",
                        ));
                    };
                    Ok(Write::new(
                        Some(ehr_id),
                        Change::modify(target.vo_id, Kind::Folder, folder, None, None, envelope),
                        audit,
                    )
                    .guarded(guard_of(&gate, target.head))
                    .placed(target.placement)
                    .gated(gate.restriction.clone())
                    .object_marked(target.restricted))
                }
            })
            .await?;
        Ok(self.committed_response(ehr_id, &committed))
    }

    /// Logically delete the EHR's directory (a new `523|deleted|` version, RM
    /// common master06 §Logical Deletion); the request's `preceding`
    /// (`If-Match`) is checked against the tip its gate read finds. Returns
    /// the committed deleted version's metadata (uid + commit instant) so the
    /// wire 204 carries the weak `ETag`/`Last-Modified` the overview §"`ETag`
    /// and Last-Modified" SHOULDs on versioned resources.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no directory;
    /// [`ServiceError::VersionConflict`] when `If-Match` names another
    /// version; [`ServiceError::Conflict`] when the EHR is not modifiable;
    /// [`ServiceError::Database`] on a storage failure.
    pub async fn delete_directory(
        &self,
        ehr_id: EhrId,
        request: WriteRequest,
    ) -> Result<ServiceResponse, ServiceError> {
        let request = &request;
        let committed = self
            .commit_gated(&self.pool, move || {
                async move |conn| {
                    let mut gate = gate::read(conn, directory_ask(ehr_id, &[])).await?;
                    self.ensure_gate_policy_held().await?;
                    let (target, latest) = directory_target(&mut gate, ehr_id)?;
                    ensure_preceding(&latest, request.preceding.as_ref())?;
                    // is_modifiable = False forbids content writes (RM ehr master04
                    // §EHR Active Status).
                    ensure_modifiable(&gate, ehr_id)?;
                    // The committal request headers merge into the delete audit too
                    // (ITS-REST overview §"openehr-version and openehr-audit-details":
                    // accepted on PUT, POST and DELETE).
                    let audit = self.commit_audit(
                        request.audit.as_ref(),
                        change_type::DELETED,
                        "DIRECTORY delete",
                    )?;
                    Ok(Write::new(
                        Some(ehr_id),
                        Change::delete(target.vo_id, Kind::Folder, None),
                        audit,
                    )
                    .guarded(guard_of(&gate, target.head))
                    .placed(target.placement)
                    .gated(gate.restriction.clone())
                    .object_marked(target.restricted))
                }
            })
            .await?;
        // The 204's identity: the NEW 523|deleted| version the delete
        // committed (RM common master06 §Logical Deletion), carried so the wire
        // serves the weak ETag + Last-Modified the overview §"ETag and
        // Last-Modified" SHOULDs on versioned resources.
        Ok(self.committed_response(ehr_id, &committed))
    }
}

/// The gate read of a directory write in `ehr_id`, reading which of `items`
/// exist.
fn directory_ask(ehr_id: EhrId, items: &[uuid::Uuid]) -> Ask<'_> {
    Ask {
        ehr_id,
        targets: &[],
        shape: Shape::Directory,
        persistent: None,
        addressed: None,
        items,
    }
}

/// The EHR's directory as its gate read found it, with the uid of its tip.
///
/// # Errors
/// [`ServiceError::NotFound`] when the EHR has no directory.
fn directory_target(gate: &mut Gate, ehr_id: EhrId) -> Result<(Target, String), ServiceError> {
    let found = gate.targets.pop().and_then(|target| {
        let latest = target
            .placement
            .tip
            .as_ref()
            .filter(|tip| tip.ehr_id == Some(ehr_id))
            .map(|tip| tip_uid(target.vo_id, tip))?;
        Some((target, latest))
    });
    found.ok_or_else(|| {
        ServiceError::not_found(
            Missing::VersionedObject,
            format!("directory for EHR {ehr_id}"),
        )
    })
}

/// Whether `target`'s tip is a live hierarchy, which occupies the directory
/// slot.
fn is_live(target: &Target) -> bool {
    target
        .placement
        .tip
        .as_ref()
        .is_some_and(|tip| tip.lifecycle_state != DELETED)
}

/// The content-writability pre-check of a directory write, judged on its gate
/// read (RM ehr master04 §EHR Active Status: everything other than the
/// `EHR_STATUS` is read-only while `is_modifiable` is false).
///
/// NOTE: ITS-REST 1.1.0 enumerates no status code for a write to a
/// non-modifiable EHR, so `409 Conflict` is used (RFC 9110 §15.5.10).
///
/// # Errors
/// [`ServiceError::Conflict`] when the EHR is not modifiable.
fn ensure_modifiable(gate: &Gate, ehr_id: EhrId) -> Result<(), ServiceError> {
    if gate.is_modifiable == Some(false) {
        return Err(not_modifiable_error(ehr_id));
    }
    Ok(())
}
