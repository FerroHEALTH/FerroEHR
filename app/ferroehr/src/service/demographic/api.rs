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

use openehr_rm::prelude::ItemTag;
use serde_json::Value;
use uuid::Uuid;

use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::datetime::parse_at_time;
use crate::service::demographic::support::Precondition;
use crate::service::demographic::types::PartyKind;
use crate::service::ehr::tags::tag_target_tail;
use crate::service::error::ServiceError;
use crate::service::refusal::Missing;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::version_update::WriteRequest;
use crate::versioning::object_version_id::{VersionIdError, parse_uid_based_id, parse_version_uid};
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
        let version_tail = resp
            .meta
            .as_ref()
            .and_then(|m| m.uid.split_once("::").map(|(_, tail)| tail.to_owned()));
        let version = match version_tail.as_deref() {
            Some(tail) => Some(self.party_tags(vo_id, Some(tail)).await?),
            None => None,
        };
        if let Some(meta) = resp.meta.as_mut() {
            meta.item_tags = Some(container);
            meta.version_item_tags = version;
        }
        Ok(())
    }

    // ── typed party operations ──────────────────────────────────────────────

    /// The `ORIGINAL_VERSION` named by
    /// a full party `OBJECT_VERSION_ID`.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id does not parse as an
    ///   `OBJECT_VERSION_ID`.
    /// - [`ServiceError`] `object_version_does_not_exist` — the object is not a
    ///   party or holds no such version.
    /// - [`ServiceError`] on a storage/database fault or a signing failure while
    ///   assembling the `ORIGINAL_VERSION`.
    pub async fn get_party_at_version(
        &self,
        a_party_version_id: String,
    ) -> Result<Value, ServiceError> {
        let (vo_id, tree) =
            parse_version_uid(&a_party_version_id).map_err(VersionIdError::into_argument)?;
        // The version-addressed read carries `object_version_does_not_exist`
        // itself (`ServiceError` round-trips the granular status losslessly).
        self.party_version(vo_id, tree).await
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
    /// - [`ServiceError`] `precondition_violation` — the id or `version_at_time`
    ///   does not parse.
    /// - [`ServiceError`] mapped from `404` — unknown id, wrong kind for the route,
    ///   or no version at the requested version/instant.
    /// - [`ServiceError`] on a storage/database fault during the read.
    pub async fn party_get(
        &self,
        kind: PartyKind,
        uid_based_id: String,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let decoded = parse_uid_based_id(&uid_based_id).map_err(VersionIdError::into_argument)?;
        let (vo_id, version) = (decoded.vo_id, decoded.tree());
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
    /// - [`ServiceError`] `precondition_violation` — the id or the `If-Match` token
    ///   does not parse.
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
        uid_based_id: String,
        if_match: String,
        request: WriteRequest<Party>,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&uid_based_id)?;
        let precondition = Precondition::parse(Some(&if_match), None)?;
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
    /// `OBJECT_VERSION_ID` identifier"). The expected trunk version comes from
    /// `If-Match` when supplied, else the path OVID.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id does not parse, names
    ///   the versioned object rather than one of its versions, or a supplied
    ///   `If-Match` is malformed (a malformed header is rejected, never
    ///   silently ignored).
    /// - [`ServiceError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`ServiceError`] mapped from `404` — unknown id or wrong kind for the route.
    /// - [`ServiceError`] mapped from `400` — the party is already deleted.
    /// - [`ServiceError`] mapped from `409` — the expected version is stale.
    pub async fn party_delete(
        &self,
        kind: PartyKind,
        uid_based_id: String,
        if_match: Option<String>,
        request: WriteRequest,
    ) -> Result<ServiceResponse, ServiceError> {
        let decoded = parse_uid_based_id(&uid_based_id).map_err(VersionIdError::into_argument)?;
        let (vo_id, path_version) = (decoded.vo_id, decoded.tree());
        // NOTE: ITS-REST `parameters/path/uid_based_id_as_version_uid.yaml` types the
        // delete path as a `version_uid`, so a bare container id is an invalid parameter (400).
        if path_version.is_none() {
            return Err(ServiceError::precondition(format!(
                "uid_based_id {uid_based_id} must be the OBJECT_VERSION_ID of the latest \
                 {} version, not its versioned object id",
                kind.rm_type()
            )));
        }
        // A malformed `If-Match` is rejected, not silently ignored; an absent
        // header falls back to the path OVID's version.
        let precondition = Precondition::parse(if_match.as_deref(), path_version)?;
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
    /// - [`ServiceError`] `precondition_violation` — the id does not parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a party or holds no
    ///   versions.
    /// - [`ServiceError`] on a storage/database fault reading the version spine.
    pub async fn versioned_party_get(
        &self,
        versioned_object_uid: String,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&versioned_object_uid)?;
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
    /// - [`ServiceError`] `precondition_violation` — the id does not parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a party.
    /// - [`ServiceError`] on a storage/database fault reading the version spine.
    pub async fn versioned_party_revision_history(
        &self,
        versioned_object_uid: String,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&versioned_object_uid)?;
        Ok(ServiceResponse::plain(
            self.party_revision_history(vo_id).await?,
        ))
    }

    /// The party `ORIGINAL_VERSION` extant at `version_at_time` (or the latest
    /// when absent), with `ETag`/`Location` metadata.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id or `version_at_time`
    ///   does not parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a party or no version
    ///   existed at the instant.
    /// - [`ServiceError`] on a storage/database fault or a signing failure.
    pub async fn versioned_party_version_get_at_time(
        &self,
        versioned_object_uid: String,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&versioned_object_uid)?;
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        self.party_version_at_time(vo_id, at).await
    }

    /// The party `ORIGINAL_VERSION` named by `version_uid`.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — either id does not parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a party or holds no
    ///   such version.
    /// - [`ServiceError`] on a storage/database fault or a signing failure.
    pub async fn versioned_party_version_get_by_id(
        &self,
        versioned_object_uid: String,
        version_uid: String,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&versioned_object_uid)?;
        let (_, version) =
            parse_version_uid(&version_uid).map_err(VersionIdError::into_argument)?;
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
    /// - [`ServiceError`] `precondition_violation` — the uid is not a UUID.
    /// - [`ServiceError`] mapped from `404` — unknown uid, or an EHR-scoped
    ///   contribution (the demographic surface only sees `ehr_id IS NULL`).
    /// - [`ServiceError`] on a storage/database fault during the read.
    #[expect(
        clippy::map_err_ignore,
        reason = "the mapped error already names the resource and echoes the \
                  rejected token; the discarded `uuid::Error` adds only its own \
                  wording, which is not part of the wire contract"
    )]
    pub async fn demographic_contribution_get(
        &self,
        contribution_uid: String,
    ) -> Result<ServiceResponse, ServiceError> {
        let id = Uuid::parse_str(&contribution_uid).map_err(|_| {
            ServiceError::precondition(format!("invalid contribution id: {contribution_uid}"))
        })?;
        Ok(ServiceResponse::plain(
            self.demographic_contribution(id).await?,
        ))
    }

    // ── demographic item tags ────────────────────────────────────────────────

    /// The `ITEM_TAG`s on one party.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id does not parse.
    /// - [`ServiceError`] on a storage/database fault reading the tag store.
    pub async fn party_tags_get(
        &self,
        kind: PartyKind,
        uid_based_id: String,
    ) -> Result<Vec<ItemTag>, ServiceError> {
        let (vo_id, version) = crate::service::ehr::tags::parse_tag_target(&uid_based_id)?;
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
    /// - [`ServiceError`] `precondition_violation` — the id does not parse.
    /// - [`ServiceError`] mapped from `404` — no live party of the routed kind.
    /// - [`ServiceError`] mapped from `422` — a tag misses its key or violates
    ///   `Inv_key_valid`/`Inv_value_valid` (RM `common.item_tag`).
    /// - [`ServiceError`] on a storage/database fault during the replace.
    pub async fn party_tags_update(
        &self,
        kind: PartyKind,
        uid_based_id: String,
        body: Vec<openehr_its::rest::generated::common::UpdateItemTag>,
    ) -> Result<Vec<ItemTag>, ServiceError> {
        let (vo_id, version) = crate::service::ehr::tags::parse_tag_target(&uid_based_id)?;
        self.replace_party_tags(kind, vo_id, version.as_ref(), &body)
            .await
    }

    /// Delete one `ITEM_TAG` by key from a party.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id does not parse.
    /// - [`ServiceError`] mapped from `404` — no tag with this key exists.
    /// - [`ServiceError`] on a storage/database fault during the delete.
    pub async fn party_tags_delete(
        &self,
        kind: PartyKind,
        uid_based_id: String,
        key: String,
    ) -> Result<ServiceResponse, ServiceError> {
        let (vo_id, version) = crate::service::ehr::tags::parse_tag_target(&uid_based_id)?;
        self.delete_party_tag(kind, vo_id, version.as_ref(), &key)
            .await?;
        Ok(ServiceResponse::plain(Value::Null))
    }

    /// The current party version metadata (the latest `version_uid` for
    /// `ETag`/`Location`/`412` echoes), or `None` if unknown/wrong-kind — the
    /// lean resolve, no node reassembly.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id does not parse.
    /// - [`ServiceError`] on a storage/database fault during the lean read.
    pub async fn demographic_latest_meta(
        &self,
        kind: PartyKind,
        uid_based_id: String,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        let vo_id = argument_vo(&uid_based_id)?;
        self.party_current_meta(kind, vo_id).await
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
    /// - [`ServiceError`] `precondition_violation` — the id or `version_at_time`
    ///   does not parse.
    /// - [`ServiceError`] mapped from `404` — unknown id, wrong kind, or no version
    ///   at the requested version/instant.
    /// - [`ServiceError`] on a storage/database fault during the read.
    pub async fn party_relationship_get(
        &self,
        uid_based_id: String,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let decoded = parse_uid_based_id(&uid_based_id).map_err(VersionIdError::into_argument)?;
        let (vo_id, version) = (decoded.vo_id, decoded.tree());
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        self.read_relationship(vo_id, version, at).await
    }

    /// Update a relationship under a mandatory `If-Match` precondition
    /// (ITS-REST overview §"If-Match and accidental overwrites"). The current
    /// version is resolved ONCE (lean): the same handle serves the `If-Match`
    /// compare and the service write gate.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id or the `If-Match` token
    ///   does not parse.
    /// - [`ServiceError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`ServiceError`] mapped from `404` — unknown id, wrong kind, or a deleted
    ///   current version.
    /// - [`ServiceError`] mapped from `422` — the body fails RM validation.
    /// - [`ServiceError`] mapped from `409` — the expected version is stale.
    pub async fn party_relationship_update(
        &self,
        uid_based_id: String,
        if_match: String,
        request: WriteRequest<PartyRelationship>,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&uid_based_id)?;
        let precondition = Precondition::parse(Some(&if_match), None)?;
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
    /// carries the `preceding_version_uid` as a full `OBJECT_VERSION_ID`;
    /// the expected version comes from `If-Match` when supplied, else the
    /// path OVID.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id does not parse, names
    ///   the versioned object rather than one of its versions, or a supplied
    ///   `If-Match` is malformed (rejected, never silently ignored).
    /// - [`ServiceError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`ServiceError`] mapped from `404` — unknown id or wrong kind.
    /// - [`ServiceError`] mapped from `400` — the relationship is already deleted.
    /// - [`ServiceError`] mapped from `409` — the expected version is stale.
    pub async fn party_relationship_delete(
        &self,
        uid_based_id: String,
        if_match: Option<String>,
        request: WriteRequest,
    ) -> Result<ServiceResponse, ServiceError> {
        let decoded = parse_uid_based_id(&uid_based_id).map_err(VersionIdError::into_argument)?;
        let (vo_id, path_version) = (decoded.vo_id, decoded.tree());
        // NOTE: no openEHR spec defines this route (our own extension); it takes the
        // party delete operations' rule that the path is a version uid, so a container id is a 400.
        if path_version.is_none() {
            return Err(ServiceError::precondition(format!(
                "uid_based_id {uid_based_id} must be the OBJECT_VERSION_ID of the latest \
                 PARTY_RELATIONSHIP version, not its versioned object id"
            )));
        }
        // A malformed `If-Match` is rejected, not silently ignored; an absent
        // header falls back to the path OVID's version.
        let precondition = Precondition::parse(if_match.as_deref(), path_version)?;
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
    /// - [`ServiceError`] `precondition_violation` — the id does not parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a relationship or holds
    ///   no versions.
    /// - [`ServiceError`] on a storage/database fault reading the version spine.
    pub async fn versioned_party_relationship_get(
        &self,
        versioned_object_uid: String,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&versioned_object_uid)?;
        Ok(ServiceResponse::plain(
            self.versioned_relationship(vo_id).await?,
        ))
    }

    /// The `REVISION_HISTORY` of a relationship (RM common master04 §Revision
    /// History).
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id does not parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a relationship.
    /// - [`ServiceError`] on a storage/database fault reading the version spine.
    pub async fn party_relationship_revision_history(
        &self,
        versioned_object_uid: String,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&versioned_object_uid)?;
        Ok(ServiceResponse::plain(
            self.relationship_revision_history(vo_id).await?,
        ))
    }

    /// The relationship `ORIGINAL_VERSION` extant at `version_at_time` (or the
    /// latest when absent), with `ETag`/`Location` metadata.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id or `version_at_time`
    ///   does not parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a relationship or no
    ///   version existed at the instant.
    /// - [`ServiceError`] on a storage/database fault or a signing failure.
    pub async fn party_relationship_version_get_at_time(
        &self,
        versioned_object_uid: String,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&versioned_object_uid)?;
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        self.relationship_version_at_time(vo_id, at).await
    }

    /// The relationship `ORIGINAL_VERSION` named by `version_uid`.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — either id does not parse.
    /// - [`ServiceError`] mapped from `404` — the id is not a relationship or holds
    ///   no such version.
    /// - [`ServiceError`] on a storage/database fault or a signing failure.
    pub async fn party_relationship_version_get_by_id(
        &self,
        versioned_object_uid: String,
        version_uid: String,
    ) -> Result<ServiceResponse, ServiceError> {
        let vo_id = argument_vo(&versioned_object_uid)?;
        let (_, version) =
            parse_version_uid(&version_uid).map_err(VersionIdError::into_argument)?;
        Ok(ServiceResponse::plain(
            self.relationship_version(vo_id, version).await?,
        ))
    }

    /// The current relationship version metadata (the latest `version_uid` for
    /// `ETag`/`Location`/`412` echoes), or `None` if unknown/wrong-kind — the
    /// lean resolve, no node reassembly.
    ///
    /// # Errors
    /// - [`ServiceError`] `precondition_violation` — the id does not parse.
    /// - [`ServiceError`] on a storage/database fault during the lean read.
    pub async fn party_relationship_latest_meta(
        &self,
        uid_based_id: String,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        let vo_id = argument_vo(&uid_based_id)?;
        self.relationship_current_meta(vo_id).await
    }
}

/// The versioned object a `uid_based_id` call argument names, a `400` when malformed.
fn argument_vo(uid_based_id: &str) -> Result<VoId, ServiceError> {
    Ok(parse_uid_based_id(uid_based_id)
        .map_err(VersionIdError::into_argument)?
        .vo_id)
}
