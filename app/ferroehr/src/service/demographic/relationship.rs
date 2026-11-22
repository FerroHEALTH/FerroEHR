// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic `PARTY_RELATIONSHIP` operations and their create factory —
//! the domain logic, built on the shared [`crate::versioning`] machinery with
//! `ehr_id = None` (no EHR scope — our own design).
//!
//! NOTE: every relationship gets its own version container, our own design,
//! where the RM instead makes it compositional data of its source party,
//! versioned with that party and with no `VERSIONED_PARTY_RELATIONSHIP` class
//! (RM demographic `docs/demographic/master02-demographic_package.adoc` §Party
//! Relationships).
//!
//! The two representations are DISJOINT: neither is auto-synchronized into the
//! other, and an inline `relationships` list in a committed PARTY body stays
//! RM-valid data that is validated, stored and served verbatim. A container's
//! `source` and `target` must name parties this server holds, and the container
//! is listed in its target's `reverse_relationships` like an inline one.
//!
//! NOTE: no specification governs this: our own design; a relationship follows
//! the PARTY pattern, an unknown id is a `404` and an update runs the PARTY's
//! structural check (`validate::relationship_check`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use serde_json::Value;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::precondition::Precondition;
use crate::service::demographic::support;
use crate::service::demographic::written::demographic_answer;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::version_update::WriteRequest;
use crate::service::write::{Materials, Resolved, first_attempt};
use crate::storage::version_repo::gate::{self, Target};
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::body::Body;
use crate::versioning::change::{Change, Write};
use crate::versioning::lifecycle::state::DELETED;
use crate::versioning::object_version_id::{TreeId, object_version_id};
use crate::versioning::read::{VersionRead, demographic_current, object_kind};

/// The current version of a `PARTY_RELATIONSHIP`, resolved in ONE lean
/// `version`⋈`audit` read (no node reassembly): the identity a `412` echoes.
/// RM common master06 §Version Identification.
pub(super) struct CurrentRelationship {
    vo_id: VoId,
    tree: TreeId,
    creating_system_id: String,
}

impl CurrentRelationship {
    /// The current version's full `OBJECT_VERSION_ID` `{vo}::{system}::{tree}`
    /// (the `ETag` value / `Location` tail).
    fn ovid(&self) -> String {
        object_version_id(self.vo_id, &self.creating_system_id, self.tree)
    }

    /// The `ResourceMeta` a `412` (`If-Match`) echoes / the latest-meta seam
    /// serves: the current `OBJECT_VERSION_ID` (empty `ehr_id` — relationships
    /// are not EHR-scoped; no commit instant — the relationship meta has never
    /// carried a `Last-Modified`, unlike the party meta).
    pub(super) fn resource_meta(&self) -> ResourceMeta {
        ResourceMeta::new(String::new(), self.ovid())
    }
}

impl FerroEhrService {
    /// Load a version of a relationship, verifying its kind and ehr-less-ness. A
    /// wrong-kind or unknown id is `404`.
    async fn load_relationship_version(
        &self,
        vo_id: VoId,
        version: Option<TreeId>,
        at: Option<jiff::Timestamp>,
    ) -> Result<VersionRead, ServiceError> {
        // The granular status: a version-addressed read that misses is
        // `object_version_does_not_exist`, a current read the generic
        // `versioned_object_does_not_exist`, as for a party.
        let miss = || {
            support::missing(
                "PARTY_RELATIONSHIP",
                vo_id,
                version.is_some() || at.is_some(),
            )
        };
        support::load_ehrless(
            &self.demographic_pool,
            self.spec_profile,
            vo_id,
            version,
            at,
            Some(Kind::PartyRelationship),
        )
        .await?
        .ok_or_else(miss)
    }

    /// Confirm `vo_id` is a relationship (any version) — the check for the
    /// `versioned_party_relationship` reads. A non-relationship id is `404`,
    /// reported with the caller-supplied granular status
    /// (`versioned_object_does_not_exist` for object-addressed reads,
    /// `object_version_does_not_exist` for version-addressed ones, as for a
    /// party).
    async fn ensure_any_relationship(
        &self,
        vo_id: VoId,
        miss: Missing,
    ) -> Result<(), ServiceError> {
        match object_kind(&self.demographic_pool, vo_id).await? {
            Some(Kind::PartyRelationship) => Ok(()),
            _ => Err(ServiceError::not_found(
                miss,
                format!("versioned party relationship {vo_id}"),
            )),
        }
    }

    /// Resolve the current version of a relationship in ONE lean read
    /// ([`demographic_current`]) — kind-checked and ehr-less. `None` when there
    /// is no current version or the id is a non-relationship / EHR-scoped
    /// object.
    pub(super) async fn relationship_current(
        &self,
        vo_id: VoId,
    ) -> Result<Option<CurrentRelationship>, ServiceError> {
        let Some(current) = demographic_current(&self.demographic_pool, vo_id).await? else {
            return Ok(None);
        };
        if current.kind != Kind::PartyRelationship {
            return Ok(None);
        }
        Ok(Some(CurrentRelationship {
            vo_id,
            tree: current.tree,
            creating_system_id: current.creating_system_id,
        }))
    }

    /// The current relationship version metadata (the latest `version_uid` a
    /// `412` echoes in `ETag`/`Location`), or `None` if unknown/wrong-kind —
    /// the lean resolve, no node reassembly.
    ///
    /// # Errors
    /// [`ServiceError`] on a storage/database fault during the lean read.
    pub async fn party_relationship_latest_meta(
        &self,
        vo_id: VoId,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        Ok(self
            .relationship_current(vo_id)
            .await?
            .map(|c| c.resource_meta()))
    }

    /// Create the
    /// first version of a new `PARTY_RELATIONSHIP` (server-side
    /// `VERSIONED_OBJECT` + `ORIGINAL_VERSION` + `CONTRIBUTION`). Returns it with
    /// its `uid` set and the create-response `ETag`/`Location` metadata. The
    /// write response is built from the committed identity + the body already
    /// in hand, unless the commit rewrites the body
    /// ([`support::commit_rewrites_body`]); then the stored version is read back,
    /// so the response is always the stored form.
    pub(super) async fn create_relationship(
        &self,
        request: WriteRequest<Body>,
    ) -> Result<ServiceResponse, ServiceError> {
        // Both halves of the committal merge, the `UPDATE_AUDIT` attributes and
        // the VERSION `lifecycle_state` (ITS-REST overview
        // `Requests_and_responses.md` §"openehr-version and
        // openehr-audit-details": "whatever is provided it MUST be merged with
        // the default VERSION and `VERSION.audit_details` attributes on commit
        // runtime").
        let Resolved {
            audit, envelope, ..
        } = self
            .demographic_write_checks(
                Kind::PartyRelationship,
                &request,
                None,
                (change_type::CREATION, "PARTY_RELATIONSHIP creation"),
            )
            .await?;
        // The large media go to the blob store before a connection is held.
        let (body, served) = self.demographic_stored(request.data, true).await?;
        let create = Change::create(Kind::PartyRelationship, body, None, envelope);
        // A create judged again can only follow a moved requirement of its own new
        // object, which nothing else writes, so it keeps no copy.
        let materials = Materials::once((create, audit));
        let materials = &materials;
        let committed = self
            .commit_gated(&self.demographic_pool, move || {
                async move |conn| {
                    let gate = gate::party(conn, None).await?;
                    let now = gate.now;
                    let (_, prior) = gate.split();
                    let (create, audit) = materials.take_first()?;
                    Ok(Write::new(None, create, audit)
                        .at(now)
                        .guarded(self.party_guard(None, prior)))
                }
            })
            .await?;
        let read_back = self.read_relationship(committed.vo_id, Some(committed.tree), None);
        demographic_answer(served, &committed, read_back).await
    }

    /// Retrieve a relationship by its versioned-object id, optionally at a specific version or instant (else
    /// the latest). A deleted current version resolves to `Null` (→ `204`); an
    /// unknown or wrong-kind id is `404`.
    pub(super) async fn read_relationship(
        &self,
        vo_id: VoId,
        version: Option<TreeId>,
        at: Option<jiff::Timestamp>,
    ) -> Result<ServiceResponse, ServiceError> {
        let read = self.load_relationship_version(vo_id, version, at).await?;
        if read.deleted() {
            return Ok(ServiceResponse::plain(Value::Null));
        }
        Ok(support::version_response(vo_id, read)?)
    }

    /// Commit a new version of the relationship `vo_id`, judged on one gate
    /// read and committed in one guarded statement after its access record
    /// (#6257): a missing relationship is `404`, then `precondition` is judged
    /// against the tip, then a deleted tip is `404`, then the body checks. The
    /// response is the stored form: the body in hand, or a read-back when the
    /// commit rewrites it ([`support::commit_rewrites_body`]).
    ///
    /// # Errors
    /// The refusals in that order, and the commit's.
    pub(super) async fn commit_relationship_update(
        &self,
        vo_id: VoId,
        precondition: Precondition,
        request: WriteRequest<Body>,
    ) -> Result<ServiceResponse, ServiceError> {
        let checked = self
            .demographic_write_checks(
                Kind::PartyRelationship,
                &request,
                None,
                (change_type::MODIFICATION, "PARTY_RELATIONSHIP update"),
            )
            .await;
        // A valid body's large media go to the blob store before a connection is held.
        let (body, served) = self
            .demographic_stored(request.data, checked.is_ok())
            .await?;
        let (resolved, unchecked) = checked.map_or_else(
            |error| (None, Some(error)),
            |resolved| (Some(resolved), None),
        );
        // An update a later attempt judges again commits on the new tip only when it
        // names no preceding version, so only such a one keeps a copy.
        let materials = Materials::new((body, resolved), precondition.names_none());
        let unchecked = std::sync::Mutex::new(unchecked);
        let (materials, unchecked, precondition) = (&materials, &unchecked, &precondition);
        let committed = self
            .commit_gated(&self.demographic_pool, move || {
                async move |conn| {
                    let (target, prior) = gate::party(conn, Some(vo_id)).await?.split();
                    let target = addressed(target, vo_id)?;
                    let tip = target
                        .placement
                        .tip
                        .as_ref()
                        .ok_or_else(|| missing(vo_id))?;
                    precondition.check_full(vo_id, tip)?;
                    precondition.check_tree(tip)?;
                    if tip.lifecycle_state == DELETED {
                        return Err(ServiceError::not_found(
                            Missing::VersionedObject,
                            format!("PARTY_RELATIONSHIP {vo_id} is deleted"),
                        ));
                    }
                    if let Some(error) = first_attempt(unchecked) {
                        return Err(error);
                    }
                    let (
                        body,
                        Some(Resolved {
                            audit, envelope, ..
                        }),
                    ) = materials.take()?
                    else {
                        return Err(ServiceError::exception(
                            "an unchecked PARTY_RELATIONSHIP update reached its write",
                        ));
                    };
                    Ok(Write::new(
                        None,
                        Change::modify(vo_id, Kind::PartyRelationship, body, None, None, envelope),
                        audit,
                    )
                    .guarded(self.party_guard(target.head, prior))
                    .object_marked(target.restricted)
                    .placed(target.placement))
                }
            })
            .await?;
        // Metadata + in-hand body, unless the commit rewrote it: then the
        // stored form is read back.
        let read_back = self.read_relationship(committed.vo_id, Some(committed.tree), None);
        demographic_answer(served, &committed, read_back).await
    }

    /// Commit the logical delete of the relationship `vo_id` as a new
    /// `523|deleted|` version (RM common master06 §Logical Deletion), judged
    /// on one gate read and committed in one guarded statement after its
    /// access record (#6257). `precondition` is the `If-Match`, else the path
    /// `OBJECT_VERSION_ID`, judged against the tip: a stale version is `409`,
    /// an already-deleted target `400`.
    ///
    /// # Errors
    /// `404` for a missing relationship, the precondition's refusal, the
    /// refusal of a tip already deleted, and the commit's.
    pub(super) async fn commit_relationship_delete(
        &self,
        vo_id: VoId,
        precondition: Precondition,
        request: WriteRequest,
    ) -> Result<ServiceResponse, ServiceError> {
        let (request, precondition) = (&request, &precondition);
        let committed = self
            .commit_gated(&self.demographic_pool, move || {
                async move |conn| {
                    let (target, prior) = gate::party(conn, Some(vo_id)).await?.split();
                    let target = addressed(target, vo_id)?;
                    let tip = target
                        .placement
                        .tip
                        .as_ref()
                        .ok_or_else(|| missing(vo_id))?;
                    precondition.check_full(vo_id, tip)?;
                    if tip.lifecycle_state == DELETED {
                        return Err(ServiceError::precondition(format!(
                            "PARTY_RELATIONSHIP {vo_id} is already deleted"
                        )));
                    }
                    // A stale preceding version is a VERSION MISMATCH: the wire
                    // arm answers `409` and echoes the latest `version_uid` in
                    // `ETag`, mirroring the party delete.
                    precondition.check_tree(tip)?;
                    let audit = self.commit_audit(
                        request.audit.as_ref(),
                        change_type::DELETED,
                        "PARTY_RELATIONSHIP delete",
                    )?;
                    Ok(Write::new(
                        None,
                        Change::delete(vo_id, Kind::PartyRelationship, None),
                        audit,
                    )
                    .guarded(self.party_guard(target.head, prior))
                    .object_marked(target.restricted)
                    .placed(target.placement))
                }
            })
            .await?;
        Ok(ServiceResponse::deleted(ResourceMeta::new(
            String::new(),
            object_version_id(vo_id, &committed.creating_system_id, committed.tree),
        )))
    }

    // ── VERSIONED_PARTY_RELATIONSHIP (extension read surface) ─────────────────

    /// The `VERSIONED_OBJECT` wrapper for a relationship. A non-relationship id
    /// is `404`.
    ///
    /// No ITS-REST demographic contract governs this — our own extension by
    /// analogy with the EHR group (assembly + owner NOTE in
    /// `support::versioned_wrapper`).
    pub(super) async fn versioned_relationship(&self, vo_id: VoId) -> Result<Value, ServiceError> {
        self.ensure_any_relationship(vo_id, Missing::VersionedObject)
            .await?;
        self.versioned_wrapper(vo_id, "VERSIONED_OBJECT", "versioned party relationship")
            .await
    }

    /// The `REVISION_HISTORY` of a relationship: one item per version with its
    /// `OBJECT_VERSION_ID` and commit `AUDIT_DETAILS` (RM common master04
    /// §Revision History). A non-relationship id is `404`.
    pub(super) async fn relationship_revision_history(
        &self,
        vo_id: VoId,
    ) -> Result<Value, ServiceError> {
        self.ensure_any_relationship(vo_id, Missing::VersionedObject)
            .await?;
        self.demographic_revision_history(vo_id).await
    }

    /// The
    /// `ORIGINAL_VERSION` at a specific version. A non-relationship id is `404`.
    pub(super) async fn relationship_version(
        &self,
        vo_id: VoId,
        version: TreeId,
    ) -> Result<Value, ServiceError> {
        // Version-addressed: `object_version_does_not_exist` is the
        // does-not-exist code, as for a party.
        self.ensure_any_relationship(vo_id, Missing::ObjectVersion)
            .await?;
        self.demographic_version_envelope(vo_id, version, "party relationship")
            .await
    }

    /// The `ORIGINAL_VERSION` extant at `at` (or the latest when `None`), with
    /// `ETag`/`Location` metadata for the VERSION resource.
    pub(super) async fn relationship_version_at_time(
        &self,
        vo_id: VoId,
        at: Option<jiff::Timestamp>,
    ) -> Result<ServiceResponse, ServiceError> {
        self.ensure_any_relationship(vo_id, Missing::VersionedObject)
            .await?;
        self.demographic_version_envelope_at(vo_id, at, "party relationship")
            .await
    }
}

/// The target of a relationship write its gate read found, when it is a
/// `PARTY_RELATIONSHIP` of no EHR.
///
/// # Errors
/// [`ServiceError::NotFound`] when the object holds no version, is of
/// another kind or belongs to an EHR.
fn addressed(target: Option<Target>, vo_id: VoId) -> Result<Target, ServiceError> {
    target
        .filter(|target| {
            target.placement.tip.as_ref().is_some_and(|tip| {
                tip.ehr_id.is_none() && tip.kind == Kind::PartyRelationship.as_str()
            })
        })
        .ok_or_else(|| missing(vo_id))
}

/// The `404` of a relationship write whose target is no relationship.
fn missing(vo_id: VoId) -> ServiceError {
    ServiceError::not_found(
        Missing::VersionedObject,
        format!("PARTY_RELATIONSHIP {vo_id}"),
    )
}
