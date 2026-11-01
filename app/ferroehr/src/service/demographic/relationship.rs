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
//! RM-valid data that is validated, stored and served verbatim.
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
use sqlx::Connection as _;

use crate::db::acquire::acquire;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::support;
use crate::service::demographic::validate::relationship_check;
use crate::service::error::ServiceError;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::status::ServiceStatus;
use crate::service::version_update::Committal;
use crate::versioning::audit::change_type;
use crate::versioning::change::WriteEnvelope;
use crate::versioning::change::{create, delete, update};
use crate::versioning::object_version_id::{TreeId, object_version_id};
use crate::versioning::read::{VersionRead, demographic_current, object_kind};
use crate::versioning::{CommitEnv, Kind};
use openehr_its::rest::generated::common::UpdateAudit;

/// The current version of a `PARTY_RELATIONSHIP`, resolved in ONE lean
/// `version`⋈`audit` read (no node reassembly) — the relationship analogue
/// of the party's current handle: the write paths resolve the target **once**
/// and thread it through the `If-Match` compare and the existence / not-deleted
/// gates. RM common master06 §Version Identification / §Logical Deletion.
pub(super) struct CurrentRelationship {
    vo_id: VoId,
    tree: TreeId,
    creating_system_id: String,
    deleted: bool,
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
            ServiceError::new(
                if version.is_some() || at.is_some() {
                    ServiceStatus::ObjectVersionDoesNotExist
                } else {
                    ServiceStatus::VersionedObjectDoesNotExist
                },
                format!("PARTY_RELATIONSHIP {vo_id}"),
            )
        };
        if object_kind(&self.demographic_pool, vo_id).await? != Some(Kind::PartyRelationship) {
            return Err(miss());
        }
        support::load_ehrless(
            &self.demographic_pool,
            self.spec_profile,
            vo_id,
            version,
            at,
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
        miss: ServiceStatus,
    ) -> Result<(), ServiceError> {
        match object_kind(&self.demographic_pool, vo_id).await? {
            Some(Kind::PartyRelationship) => Ok(()),
            _ => Err(ServiceError::new(
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
            deleted: current.deleted,
        }))
    }

    /// The current relationship version metadata (the latest `version_uid` a
    /// `412` echoes in `ETag`/`Location`), or `None` if unknown/wrong-kind —
    /// the lean resolve, no node reassembly.
    pub(super) async fn relationship_current_meta(
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
        body: Value,
        committal: Option<&Committal>,
    ) -> Result<ServiceResponse, ServiceError> {
        relationship_check(&body, false)?;

        // Both halves of the committal merge — the `UPDATE_AUDIT` attributes
        // and the VERSION `lifecycle_state` (ITS-REST overview
        // `Requests_and_responses.md` §"openehr-version and
        // openehr-audit-details": "whatever is provided it MUST be merged with
        // the default VERSION and `VERSION.audit_details` attributes on commit
        // runtime").
        let audit = self.demographic_audit(
            committal.map(|c| &c.audit),
            change_type::CREATION,
            "PARTY_RELATIONSHIP creation",
        )?;
        let ctx = CommitEnv::signing_ctx(self);
        let canonical = (!support::commit_rewrites_body(self)).then(|| body.clone());
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        let committed = create(
            &mut tx,
            None,
            Kind::PartyRelationship,
            body,
            None,
            &audit,
            WriteEnvelope {
                lifecycle_state: committal.and_then(|c| c.lifecycle_state.clone()),
                ..WriteEnvelope::default()
            },
            &ctx,
            None,
        )
        .await?;
        tx.commit().await?;
        support::record_commit();

        match canonical {
            Some(canonical) => Ok(support::committed_response(canonical, &committed)?),
            None => {
                self.read_relationship(committed.vo_id, Some(committed.tree), None)
                    .await
            }
        }
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

    /// Commit a new version of an already-resolved relationship.
    ///
    /// A deleted current is `404`; a stale
    /// `expected` → the engine's version conflict. The response is the stored
    /// form: the body in hand, or a read-back when the commit rewrites it
    /// ([`support::commit_rewrites_body`]).
    pub(super) async fn commit_relationship_update(
        &self,
        current: CurrentRelationship,
        body: Value,
        expected: Option<TreeId>,
        committal: Option<&Committal>,
    ) -> Result<ServiceResponse, ServiceError> {
        if current.deleted {
            return Err(ServiceError::new(
                ServiceStatus::VersionedObjectDoesNotExist,
                format!("PARTY_RELATIONSHIP {} is deleted", current.vo_id),
            ));
        }
        relationship_check(&body, false)?;

        let audit = self.demographic_audit(
            committal.map(|c| &c.audit),
            change_type::MODIFICATION,
            "PARTY_RELATIONSHIP update",
        )?;
        let ctx = CommitEnv::signing_ctx(self);
        let canonical = (!support::commit_rewrites_body(self)).then(|| body.clone());
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        let committed = update(
            &mut tx,
            None,
            current.vo_id,
            Kind::PartyRelationship,
            body,
            expected,
            None,
            &audit,
            WriteEnvelope {
                lifecycle_state: committal.and_then(|c| c.lifecycle_state.clone()),
                ..WriteEnvelope::default()
            },
            &ctx,
        )
        .await?;
        tx.commit().await?;
        support::record_commit();

        // Metadata + in-hand body, unless the commit rewrote it: then the
        // stored form is read back.
        match canonical {
            Some(canonical) => Ok(support::committed_response(canonical, &committed)?),
            None => {
                self.read_relationship(committed.vo_id, Some(committed.tree), None)
                    .await
            }
        }
    }

    /// Commit the logical delete of an already-resolved relationship
    /// as a new `523|deleted|` version — RM common master06 §Logical Deletion.
    /// `expected` is the caller-supplied trunk version (from `If-Match` or the
    /// path `OBJECT_VERSION_ID`); when `Some`, a stale value → `409`; when
    /// `None`, the current version is deleted unconditionally (no precondition
    /// supplied — ITS-REST overview §Concurrency control, mirroring the party
    /// delete). An already-deleted target → `400`.
    pub(super) async fn commit_relationship_delete(
        &self,
        current: CurrentRelationship,
        expected: Option<TreeId>,
        update_audit: Option<&UpdateAudit>,
    ) -> Result<ServiceResponse, ServiceError> {
        if current.deleted {
            return Err(ServiceError::precondition(format!(
                "PARTY_RELATIONSHIP {} is already deleted",
                current.vo_id
            )));
        }
        if let Some(expected) = expected
            && current.tree != expected
        {
            // A stale preceding version is a VERSION MISMATCH — the wire arm
            // answers `409` and echoes the latest `version_uid` in `ETag`,
            // mirroring the party delete's convention
            // (`responses/409_PERSON_with_uid_based_id.yaml`).
            return Err(ServiceError::version_conflict(format!(
                "preceding_version_uid names version {expected}, latest is {}",
                current.tree
            )));
        }

        let audit = self.demographic_audit(
            update_audit,
            change_type::DELETED,
            "PARTY_RELATIONSHIP delete",
        )?;
        let ctx = CommitEnv::signing_ctx(self);
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        let committed = delete(
            &mut tx,
            None,
            current.vo_id,
            Kind::PartyRelationship,
            Some(current.tree),
            &audit,
            None,
            &ctx,
        )
        .await?;
        tx.commit().await?;
        support::record_commit();

        Ok(ServiceResponse::deleted(ResourceMeta::new(
            String::new(),
            object_version_id(current.vo_id, &committed.creating_system_id, committed.tree),
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
        self.ensure_any_relationship(vo_id, ServiceStatus::VersionedObjectDoesNotExist)
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
        self.ensure_any_relationship(vo_id, ServiceStatus::VersionedObjectDoesNotExist)
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
        self.ensure_any_relationship(vo_id, ServiceStatus::ObjectVersionDoesNotExist)
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
        self.ensure_any_relationship(vo_id, ServiceStatus::VersionedObjectDoesNotExist)
            .await?;
        self.demographic_version_envelope_at(vo_id, at, "party relationship")
            .await
    }
}
