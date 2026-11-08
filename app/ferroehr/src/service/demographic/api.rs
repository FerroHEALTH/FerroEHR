// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The public DEMOGRAPHIC seam on [`FerroEhrService`] — the typed party and
//! relationship operations plus the wire-shaped party / relationship /
//! contribution / tag calls the ITS-REST adapter (`ferroehr-rest`) invokes.
//!
//! Thin adapters that parse the (kind + string) arguments the wire seams supply
//! and delegate to the sibling demographic domain modules. Party / relationship
//! ids parse through the shared BASE decoder in [`crate::versioning`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use std::str::FromStr;

use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_rm::prelude::ItemTag;
use serde_json::Value;
use uuid::Uuid;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::datetime::parse_at_time;
use crate::service::demographic::precondition::Precondition;
use crate::service::demographic::types::PartyKind;
use crate::service::ehr::tags::tag_target_tail;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::version_update::WriteRequest;
use crate::versioning::object_version_id::{TreeId, UidAddress};
use openehr_rm::prelude::{Party, PartyRelationship};

impl FerroEhrService {
    /// Attach the party's stored `ITEM_TAG`s (RM `common.item_tag`) to a response's
    /// metadata seam ([`ResourceMeta::item_tags`]), from which the ITS-REST
    /// adapter derives the `openehr-item-tag`/`openehr-version-item-tag` response
    /// headers. A response without metadata (a deleted read → `Null` body) is
    /// left unchanged. The tags are read from the same store `party_tags_get`
    /// serves, so the header and the tags sub-resource agree.
    ///
    /// # Errors
    /// [`ServiceError`] on a storage/database fault reading the tag store.
    async fn attach_party_item_tags(
        &self,
        vo_id: VoId,
        resp: &mut ServiceResponse,
    ) -> Result<(), ServiceError> {
        if resp.meta.is_none() {
            return Ok(());
        }
        // The two response headers carry DISTINCT collections (overview
        // §"openehr-item-tag and openehr-version-item-tag"): the container's
        // set rides `openehr-item-tag`, the served VERSION's own set rides
        // `openehr-version-item-tag`. The version tail comes from the response
        // metadata's own OBJECT_VERSION_ID.
        let container = self.party_tags(vo_id, None).await?;
        // NOTE: a meta uid that is no OBJECT_VERSION_ID names no version, so no version set rides.
        let served = resp
            .meta
            .as_ref()
            .and_then(|m| ObjectVersionId::from_str(&m.uid).ok());
        let version = match tag_target_tail(served.as_ref()) {
            Some(tail) => Some(self.party_tags(vo_id, Some(tail)).await?),
            None => None,
        };
        if let Some(meta) = resp.meta.as_mut() {
            meta.item_tags = Some(container);
            meta.version_item_tags = version;
        }
        Ok(())
    }

    // ── PARTY CRUD (wire seam) ────────────────────────────────────────────────

    /// Create a party of the routed [`PartyKind`], with the party's stored `ITEM_TAG`s surfaced on the
    /// response metadata for the `openehr-item-tag` headers (a fresh party has
    /// none yet; the wire adapter persists any request-header tags and
    /// re-populates the seam afterwards).
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `422` — the body's `_type` mismatches the
    ///   route or fails RM validation.
    /// - [`ServiceError`] `precondition_violation` — the committed version uid does
    ///   not parse (defensive; server-generated).
    /// - [`ServiceError`] on a storage/database fault during the create transaction
    ///   or the tag read.
    pub async fn party_create(
        &self,
        kind: PartyKind,
        request: WriteRequest<Party>,
    ) -> Result<ServiceResponse, ServiceError> {
        // A freshly created party has no stored ITEM_TAGs by construction, so
        // the response seam needs no tag read here; when the request carried
        // `openehr-item-tag` header tags, the wire adapter persists them after
        // the create and re-populates the seam itself (person_create.yaml).
        self.commit_new_party(
            kind,
            request.map(|data| openehr_its::json::to_canonical_value(&data)),
        )
        .await
        .map_err(ServiceError::flattened)
    }

    /// Read a party of the routed [`PartyKind`] by uid-based id (bare
    /// `HIER_OBJECT_ID` or full `OBJECT_VERSION_ID`), optionally time-travelled
    /// to `version_at_time`; the stored `ITEM_TAG`s ride the metadata seam. A
    /// deleted current version reads `Null` (→ `204`).
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — `version_at_time` does not
    ///   parse.
    /// - [`ServiceError`] mapped from `404` — unknown id, wrong kind for the route,
    ///   or no version at the requested version/instant.
    /// - [`ServiceError`] on a storage/database fault during the read.
    pub async fn party_get(
        &self,
        kind: PartyKind,
        address: UidAddress,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let (vo_id, version) = (address.vo_id, address.tree());
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        let mut resp = self.read_party(kind, vo_id, version, at).await?;
        // Attach the party's stored ITEM_TAGs for the item-tag response headers
        // (person_get.yaml). A deleted read carries no metadata → no-op.
        self.attach_party_item_tags(vo_id, &mut resp).await?;
        Ok(resp)
    }

    /// Update a party of the routed [`PartyKind`] under a mandatory `If-Match`
    /// precondition (ITS-REST overview §"If-Match and accidental
    /// overwrites"). The current version is resolved ONCE (lean,
    /// kind-checked): the same handle serves the `If-Match` `ETag` compare
    /// here and the service write gate.
    ///
    /// # Errors
    /// - [`ServiceError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`ServiceError`] mapped from `404` — unknown id, wrong kind, or a deleted
    ///   current version.
    /// - [`ServiceError`] mapped from `422` — the body fails RM validation.
    /// - [`ServiceError`] mapped from `409` — the expected version is stale, or the
    ///   write transaction fails.
    pub async fn party_update(
        &self,
        kind: PartyKind,
        vo_id: VoId,
        precondition: Precondition,
        request: WriteRequest<Party>,
    ) -> Result<ServiceResponse, ServiceError> {
        let current = self.party_current(kind, vo_id).await?.ok_or_else(|| {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("{} {vo_id}", kind.rm_type()),
            )
        })?;
        self.commit_party_update(
            current,
            precondition,
            request.map(|data| openehr_its::json::to_canonical_value(&data)),
        )
        .await
        .map_err(ServiceError::flattened)
    }

    /// Logically delete a party of the routed [`PartyKind`]
    /// (`delete_party(a_versioned_party_id: UUID)` — our own demographic
    /// design): the path carries the `preceding_version_uid` as a full
    /// `OBJECT_VERSION_ID` (ITS-REST `operations/agent_delete.yaml` and its
    /// four siblings: "The `uid_based_id` MUST be in a form of an
    /// `OBJECT_VERSION_ID` identifier"). The adapter reads the expected trunk
    /// version into `precondition` from `If-Match` when supplied, else from the
    /// path OVID.
    ///
    /// # Errors
    /// - [`ServiceError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`ServiceError`] mapped from `404` — unknown id or wrong kind for the route.
    /// - [`ServiceError`] mapped from `400` — the party is already deleted.
    /// - [`ServiceError`] mapped from `409` — the expected version is stale.
    pub async fn party_delete(
        &self,
        kind: PartyKind,
        vo_id: VoId,
        precondition: Precondition,
        request: WriteRequest,
    ) -> Result<ServiceResponse, ServiceError> {
        let current = self.party_current(kind, vo_id).await?.ok_or_else(|| {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("{} {vo_id}", kind.rm_type()),
            )
        })?;
        self.commit_party_delete(current, precondition, request)
            .await
            .map_err(ServiceError::flattened)
    }

    // ── VERSIONED_PARTY ──────────────────────────────────────────────────────

    /// The `VERSIONED_PARTY` wrapper for a party (any of the five kinds).
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — the id is not a party or holds no
    ///   versions.
    /// - [`ServiceError`] on a storage/database fault reading the version spine.
    pub async fn versioned_party_get(&self, vo_id: VoId) -> Result<ServiceResponse, ServiceError> {
        let body = self.versioned_party(vo_id).await?;
        // ITS-REST overview §"ETag and Last-Modified" derives `Last-Modified`
        // from `VERSION.commit_audit.time_committed.value`, and the container
        // body exposes no commit audit, so the instant comes from the version
        // spine rather than the body.
        let newest =
            crate::storage::version_repo::meta::all_version_meta(&self.demographic_pool, vo_id)
                .await
                .map_err(ServiceError::from)?
                .last()
                .map(|m| m.time_committed);
        let meta = ResourceMeta::new(String::new(), vo_id.to_string());
        let meta = match newest {
            Some(at) => meta.with_last_modified(at),
            None => meta,
        };
        Ok(ServiceResponse::new(body, meta))
    }

    /// The `REVISION_HISTORY` of a party (RM common master04 §Revision
    /// History).
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — the id is not a party.
    /// - [`ServiceError`] on a storage/database fault reading the version spine.
    pub async fn versioned_party_revision_history(
        &self,
        vo_id: VoId,
    ) -> Result<ServiceResponse, ServiceError> {
        Ok(ServiceResponse::plain(
            self.party_revision_history(vo_id).await?,
        ))
    }

    /// The party `ORIGINAL_VERSION` extant at `version_at_time` (or the latest
    /// when absent), with `ETag`/`Location` metadata.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — `version_at_time` does not
    ///   parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a party or no version
    ///   existed at the instant.
    /// - [`ServiceError`] on a storage/database fault or a signing failure.
    pub async fn versioned_party_version_get_at_time(
        &self,
        vo_id: VoId,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        self.party_version_at_time(vo_id, at).await
    }

    /// The party `ORIGINAL_VERSION` named by `version_uid`.
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — the id is not a party or holds no
    ///   such version.
    /// - [`ServiceError`] on a storage/database fault or a signing failure.
    pub async fn versioned_party_version_get_by_id(
        &self,
        vo_id: VoId,
        version: TreeId,
    ) -> Result<ServiceResponse, ServiceError> {
        Ok(ServiceResponse::plain(
            self.party_version(vo_id, version).await?,
        ))
    }

    // ── demographic CONTRIBUTION ─────────────────────────────────────────────

    /// Commit a demographic (ehr-less) CONTRIBUTION — a change-set of party /
    /// relationship versions (RM common master06 §Contributions).
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `422` — the change-set is malformed, contains
    ///   an EHR-kind version (the engine's scope check), or a version fails
    ///   validation.
    /// - [`ServiceError`] mapped from `409` — a preceding version is stale.
    /// - [`ServiceError`] on a storage/database fault during the commit or the
    ///   read-back.
    pub async fn demographic_contribution_create(
        &self,
        body: Value,
    ) -> Result<ServiceResponse, ServiceError> {
        self.create_demographic_contribution(body)
            .await
            .map_err(ServiceError::flattened)
    }

    /// Retrieve a demographic (ehr-less) CONTRIBUTION by uid.
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — unknown uid, or an EHR-scoped
    ///   contribution (the demographic surface only sees `ehr_id IS NULL`).
    /// - [`ServiceError`] on a storage/database fault during the read.
    pub async fn demographic_contribution_get(
        &self,
        id: Uuid,
    ) -> Result<ServiceResponse, ServiceError> {
        Ok(ServiceResponse::plain(
            self.demographic_contribution(id).await?,
        ))
    }

    // ── demographic item tags ────────────────────────────────────────────────

    /// The `ITEM_TAG`s on one party.
    ///
    /// # Errors
    /// - [`ServiceError`] on a storage/database fault reading the tag store.
    pub async fn party_tags_get(
        &self,
        kind: PartyKind,
        target: UidAddress,
    ) -> Result<Vec<ItemTag>, ServiceError> {
        let (vo_id, version) = (target.vo_id, target.version);
        // The released 404 trigger ("when the `uid_based_id` does not exist")
        // plus the kind-checked-routes law: the guard runs on the GET too; an
        // existing target with no tags stays an empty 200 list.
        self.ensure_party_tag_target(kind, vo_id, version.as_ref())
            .await?;
        self.party_tags(vo_id, tag_target_tail(version.as_ref()))
            .await
    }

    /// Replace the whole `ITEM_TAG` collection of a party (PUT full-collection
    /// semantics; an empty list clears all; duplicate keys are last-wins).
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — no live party of the routed kind.
    /// - [`ServiceError`] mapped from `422` — a tag misses its key or violates
    ///   `Inv_key_valid`/`Inv_value_valid` (RM `common.item_tag`).
    /// - [`ServiceError`] on a storage/database fault during the replace.
    pub async fn party_tags_update(
        &self,
        kind: PartyKind,
        target: UidAddress,
        body: Vec<openehr_its::rest::generated::common::UpdateItemTag>,
    ) -> Result<Vec<ItemTag>, ServiceError> {
        let (vo_id, version) = (target.vo_id, target.version);
        self.replace_party_tags(kind, vo_id, version.as_ref(), &body)
            .await
    }

    /// Delete one `ITEM_TAG` by key from a party.
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — no tag with this key exists.
    /// - [`ServiceError`] on a storage/database fault during the delete.
    pub async fn party_tags_delete(
        &self,
        kind: PartyKind,
        target: UidAddress,
        key: String,
    ) -> Result<ServiceResponse, ServiceError> {
        let (vo_id, version) = (target.vo_id, target.version);
        self.delete_party_tag(kind, vo_id, version.as_ref(), &key)
            .await?;
        Ok(ServiceResponse::plain(Value::Null))
    }
}

impl FerroEhrService {
    // ── typed relationship operations ───────────────────────────────────────

    // ── the relationship wire seam ────────────────────────────────────────────

    /// Create a `PARTY_RELATIONSHIP`.
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `422` — the body's `_type` is not
    ///   `PARTY_RELATIONSHIP` or fails RM validation.
    /// - [`ServiceError`] on a storage/database fault during the create transaction.
    pub async fn party_relationship_create(
        &self,
        request: WriteRequest<PartyRelationship>,
    ) -> Result<ServiceResponse, ServiceError> {
        self.create_relationship(request.map(|data| openehr_its::json::to_canonical_value(&data)))
            .await
            .map_err(ServiceError::flattened)
    }

    /// Read a relationship by uid-based id, optionally time-travelled to
    /// `version_at_time`. A deleted current version reads `Null` (→ `204`).
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — `version_at_time` does not
    ///   parse.
    /// - [`ServiceError`] mapped from `404` — unknown id, wrong kind, or no version
    ///   at the requested version/instant.
    /// - [`ServiceError`] on a storage/database fault during the read.
    pub async fn party_relationship_get(
        &self,
        address: UidAddress,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let (vo_id, version) = (address.vo_id, address.tree());
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        self.read_relationship(vo_id, version, at).await
    }

    /// Update a relationship under a mandatory `If-Match` precondition
    /// (ITS-REST overview §"If-Match and accidental overwrites"). The current
    /// version is resolved ONCE (lean): the same handle serves the `If-Match`
    /// compare and the service write gate.
    ///
    /// # Errors
    /// - [`ServiceError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`ServiceError`] mapped from `404` — unknown id, wrong kind, or a deleted
    ///   current version.
    /// - [`ServiceError`] mapped from `422` — the body fails RM validation.
    /// - [`ServiceError`] mapped from `409` — the expected version is stale.
    pub async fn party_relationship_update(
        &self,
        vo_id: VoId,
        precondition: Precondition,
        request: WriteRequest<PartyRelationship>,
    ) -> Result<ServiceResponse, ServiceError> {
        let current = self.relationship_current(vo_id).await?.ok_or_else(|| {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("PARTY_RELATIONSHIP {vo_id}"),
            )
        })?;
        self.commit_relationship_update(
            current,
            precondition,
            request.map(|data| openehr_its::json::to_canonical_value(&data)),
        )
        .await
        .map_err(ServiceError::flattened)
    }

    /// Logically delete a relationship. Mirrors `party_delete`: the path
    /// carries the `preceding_version_uid` as a full `OBJECT_VERSION_ID`, and
    /// `precondition` holds the expected version from `If-Match` when
    /// supplied, else from the path OVID.
    ///
    /// # Errors
    /// - [`ServiceError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`ServiceError`] mapped from `404` — unknown id or wrong kind.
    /// - [`ServiceError`] mapped from `400` — the relationship is already deleted.
    /// - [`ServiceError`] mapped from `409` — the expected version is stale.
    pub async fn party_relationship_delete(
        &self,
        vo_id: VoId,
        precondition: Precondition,
        request: WriteRequest,
    ) -> Result<ServiceResponse, ServiceError> {
        let current = self.relationship_current(vo_id).await?.ok_or_else(|| {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("PARTY_RELATIONSHIP {vo_id}"),
            )
        })?;
        self.commit_relationship_delete(current, precondition, request)
            .await
            .map_err(ServiceError::flattened)
    }

    /// The `VERSIONED_OBJECT` wrapper for a relationship.
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — the id is not a relationship or holds
    ///   no versions.
    /// - [`ServiceError`] on a storage/database fault reading the version spine.
    pub async fn versioned_party_relationship_get(
        &self,
        vo_id: VoId,
    ) -> Result<ServiceResponse, ServiceError> {
        Ok(ServiceResponse::plain(
            self.versioned_relationship(vo_id).await?,
        ))
    }

    /// The `REVISION_HISTORY` of a relationship (RM common master04 §Revision
    /// History).
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — the id is not a relationship.
    /// - [`ServiceError`] on a storage/database fault reading the version spine.
    pub async fn party_relationship_revision_history(
        &self,
        vo_id: VoId,
    ) -> Result<ServiceResponse, ServiceError> {
        Ok(ServiceResponse::plain(
            self.relationship_revision_history(vo_id).await?,
        ))
    }

    /// The relationship `ORIGINAL_VERSION` extant at `version_at_time` (or the
    /// latest when absent), with `ETag`/`Location` metadata.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — `version_at_time` does not
    ///   parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a relationship or no
    ///   version existed at the instant.
    /// - [`ServiceError`] on a storage/database fault or a signing failure.
    pub async fn party_relationship_version_get_at_time(
        &self,
        vo_id: VoId,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        self.relationship_version_at_time(vo_id, at).await
    }

    /// The relationship `ORIGINAL_VERSION` named by `version_uid`.
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — the id is not a relationship or holds
    ///   no such version.
    /// - [`ServiceError`] on a storage/database fault or a signing failure.
    pub async fn party_relationship_version_get_by_id(
        &self,
        vo_id: VoId,
        version: TreeId,
    ) -> Result<ServiceResponse, ServiceError> {
        Ok(ServiceResponse::plain(
            self.relationship_version(vo_id, version).await?,
        ))
    }
}
