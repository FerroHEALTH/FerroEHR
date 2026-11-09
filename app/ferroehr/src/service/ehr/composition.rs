// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! COMPOSITION commit/retrieve with implicit CONTRIBUTION creation. The
//! commit-validation
//! choke point and the `VERSIONED_COMPOSITION` cross-version invariant hook
//! live in the sibling [`validation`](super::validation) module.
//!
//! Spec: RM ehr `versioned_composition.adoc`, RM composition
//! (`COMPOSITION.category` / `is_persistent`), ITS-REST
//! `responses/422.yaml` (a well-formed body that fails template/RM
//! validation is 422, not 400). Versioned-object mechanics are RM common
//! master06, delegated to [`crate::versioning`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use openehr_base::prelude::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::lexical::composite_ids_equal;
use serde_json::Value;

use crate::error::refusal::Missing;
use crate::error::{ServiceError, Violation};
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::datetime::parse_at_time;
use crate::service::response::{RawServiceResponse, ReadBody, ResourceMeta, ServiceResponse};
use crate::service::version_update::WriteRequest;
use crate::service::write::{Mode, Resolved};
use crate::storage::version_repo::read::VersionAt;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::precondition::{ensure_preceding, lock_current, tip_uid};
use crate::versioning::change::{Change, Committed, Write};
use crate::versioning::object_version_id::{TreeId, VersionIdError, components, parse_tree_id};
use crate::versioning::read::VersionRead;
use crate::versioning::wire::{revision_history, versioned_object};
use openehr_rm::prelude::Composition;

use crate::validation::versioned::composition_template_id;

impl FerroEhrService {
    /// `create_composition`: commit the first version of a COMPOSITION in
    /// `ehr_id` from the caller's [`WriteRequest`], returning the committed
    /// version identity ([`Committed`] — the `ETag`/`Location`/`Last-Modified`
    /// source). The request's audit attributes, lifecycle state, verbatim
    /// signature and attestations are honoured on the persisted commit
    /// (ITS-REST committal-header merge — MUST).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR does not exist;
    /// [`ServiceError::Conflict`] when the EHR is not modifiable or a live
    /// persistent COMPOSITION for the same template already exists;
    /// [`ServiceError::ValidationFailed`] when RM/terminology/template
    /// validation fails (→ 422); [`ServiceError::Database`] on a storage
    /// failure.
    pub async fn create_composition(
        &self,
        ehr_id: EhrId,
        request: WriteRequest<Composition>,
    ) -> Result<Committed, ServiceError> {
        let Resolved {
            audit,
            envelope,
            incomplete,
        } = self.resolve_write(&request, change_type::CREATION, "COMPOSITION creation")?;
        let composition = openehr_its::json::to_canonical_value(&request.data);
        drop(request);
        // Validation may consult the template store and the routed terminology
        // servers, so it runs before a connection is held; its result surfaces
        // after the EHR gates, in pre-check order. 553|incomplete| relaxes its
        // strictness (master06 §Version Lifecycle).
        let validated = self
            .validate_for_commit(Kind::Composition, &composition, incomplete)
            .await;
        // The committed template identity is promoted to `version.template_id`
        // — the ABAC template attribute resolver (`template_of_version`) and the
        // template-delete guard both read that column, so the direct route
        // stamps it exactly like the CONTRIBUTION route.
        let template_id = composition_template_id(&composition).map(str::to_owned);
        // With no attestations and no event outbox the commit is ONE folded,
        // self-atomic statement, so no BEGIN/COMMIT is needed.
        let mode = if envelope.attestations.is_empty() && !self.signing_ctx().outbox_enabled {
            Mode::Statement
        } else {
            Mode::Transaction
        };
        // ONE pooled connection carries both gates AND the commit: measured at
        // +0.4 ms per extra checkout on #3097.
        self.commit_one(&self.pool, mode, async move |conn| {
            // The EHR-existence (404) and content-writability (409) gates in
            // one round trip: a COMPOSITION is EHR content (RM ehr master04
            // §EHR Creation / §EHR Active Status).
            let commit_now = self.ensure_ehr_content_writable(&mut *conn, ehr_id).await?;
            validated?;
            self.reject_duplicate_persistent(&mut *conn, ehr_id, &composition)
                .await?;
            Ok(Write::new(
                Some(ehr_id),
                Change::create(Kind::Composition, composition, template_id, envelope),
                audit,
            )
            .at(commit_now))
        })
        .await
    }

    /// The latest COMPOSITION of a versioned object, with the version metadata
    /// the wire's `ETag`/`Last-Modified` need. A deleted latest version yields
    /// a null body and no metadata (→ `204`).
    ///
    /// `version` addresses one version of the object; `None` reads the latest.
    ///
    /// # Errors
    /// [`ServiceError`] when the object does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn read_composition_raw(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        version: Option<TreeId>,
    ) -> Result<RawServiceResponse, ServiceError> {
        let raw = match version {
            Some(v) => {
                crate::versioning::read::read_raw(
                    &self.pool,
                    self.spec_profile,
                    VersionAt::Tree(vo_id, v),
                )
                .await?
            }
            None => {
                crate::versioning::read::read_raw(
                    &self.pool,
                    self.spec_profile,
                    VersionAt::Current(vo_id),
                )
                .await?
            }
        }
        .filter(|r| r.read.ehr_id == Some(ehr_id) && r.read.kind == Kind::Composition)
        .ok_or_else(|| {
            ServiceError::not_found(Missing::Composition, format!("COMPOSITION {vo_id}"))
        })?;
        if raw.read.deleted() {
            return Ok(RawServiceResponse {
                body: ReadBody::Value(Value::Null),
                meta: None,
            });
        }
        let read = raw.read;
        let meta = self
            .version_meta(
                ehr_id,
                vo_id,
                &read.creating_system_id,
                read.tree,
                read.time_committed,
            )
            .with_origins(crate::versioning::origins::of_stored(
                read.origins.as_ref(),
                &read.canonical,
                &read.creating_system_id,
            ))
            .with_content(vec![read.content.clone()]);
        let Some(text) = raw.raw_json else {
            let stamped =
                self.with_uid(read.canonical, vo_id, &read.creating_system_id, read.tree)?;
            return Ok(RawServiceResponse {
                body: ReadBody::Value(stamped),
                meta: Some(meta),
            });
        };
        let uid = crate::versioning::object_version_id::object_version_id(
            vo_id,
            &read.creating_system_id,
            read.tree,
        );
        let prefix =
            format!("{{\"uid\": {{\"_type\": \"OBJECT_VERSION_ID\", \"value\": \"{uid}\"}}");
        if text.starts_with(&prefix) {
            return Ok(RawServiceResponse {
                body: ReadBody::RawJson(text),
                meta: Some(meta),
            });
        }
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            ServiceError::exception(format!(
                "the stored body of COMPOSITION {vo_id} is not decodable JSON: {e}"
            ))
        })?;
        let stamped = self.with_uid(value, vo_id, &read.creating_system_id, read.tree)?;
        Ok(RawServiceResponse {
            body: ReadBody::Value(stamped),
            meta: Some(meta),
        })
    }

    /// The COMPOSITION current at `a_time` (time-travel), or the latest when
    /// `a_time` is `None`, with the version metadata the wire's
    /// `ETag`/`Last-Modified` need. A deleted version resolves to an empty
    /// body (→ `204`).
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `a_time` (400-equivalent);
    /// [`ServiceError::NotFound`] when no version existed at that instant or
    /// the object belongs to another EHR; [`ServiceError::Database`] on a
    /// storage failure.
    pub async fn composition_at_time(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        a_time: Option<String>,
    ) -> Result<RawServiceResponse, ServiceError> {
        let Some(at) = a_time.as_deref().map(parse_at_time).transpose()? else {
            return self.read_composition_raw(ehr_id, vo_id, None).await;
        };
        let read = crate::versioning::read::read(
            &self.pool,
            self.spec_profile,
            VersionAt::Instant(vo_id, at),
        )
        .await?
        .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::Composition)
        .ok_or_else(|| {
            ServiceError::not_found(Missing::Composition, format!("COMPOSITION {vo_id}"))
        })?;
        if read.deleted() {
            return Ok(RawServiceResponse {
                body: ReadBody::Value(Value::Null),
                meta: None,
            });
        }
        let resp = self.version_response(ehr_id, vo_id, read)?;
        Ok(RawServiceResponse {
            body: ReadBody::Value(resp.body),
            meta: resp.meta,
        })
    }

    /// The bare COMPOSITION at the named version, with the version metadata
    /// the wire's `ETag`/`Last-Modified` need. A deleted version resolves to
    /// an empty body (→ `204`).
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `OBJECT_VERSION_ID`, an unknown version
    /// (404-equivalent), or a read failure.
    pub async fn composition_at_version(
        &self,
        ehr_id: EhrId,
        a_version_uid: ObjectVersionId,
    ) -> Result<RawServiceResponse, ServiceError> {
        let (vo_id, version) = components(&a_version_uid).map_err(VersionIdError::into_argument)?;
        let read = self
            .read_composition_raw(ehr_id, vo_id, Some(version))
            .await?;
        if let Some(meta) = read.meta.as_ref() {
            super::ensure_addressed_version(&a_version_uid, &meta.uid)?;
        }
        Ok(read)
    }

    /// The `VERSIONED_COMPOSITION` container object with the container
    /// metadata the wire's `ETag`/`Last-Modified` need: the container uid
    /// identity plus the newest held version's commit instant (ITS-REST
    /// overview `Requests_and_responses.md` §"`ETag` and Last-Modified" — both
    /// headers SHOULD accompany a `VERSIONED_OBJECT` response).
    ///
    /// # Errors
    /// [`ServiceError`] when the object does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn versioned_composition(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
    ) -> Result<ServiceResponse, ServiceError> {
        // Ownership rides the container statement itself (the ehr_id filter
        // inside `versioned_object`) — no separate ownership probe.
        let (body, last_modified) =
            versioned_object(&self.pool, vo_id, ehr_id, "VERSIONED_COMPOSITION")
                .await?
                .ok_or_else(|| {
                    ServiceError::not_found(Missing::Composition, format!("COMPOSITION {vo_id}"))
                })?;
        Ok(ServiceResponse::new(
            body,
            super::meta::container_meta(ehr_id, vo_id, last_modified),
        ))
    }

    /// The `REVISION_HISTORY` of a COMPOSITION with the container metadata
    /// the wire's `ETag`/`Last-Modified` need (container uid + newest commit
    /// instant, the derivation of [`Self::versioned_composition`]).
    ///
    /// # Errors
    /// [`ServiceError`] when the object does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn composition_revision_history(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
    ) -> Result<ServiceResponse, ServiceError> {
        let (body, last_modified) = revision_history(&self.pool, ehr_id, vo_id).await?;
        Ok(ServiceResponse::new(
            body,
            super::meta::container_meta(ehr_id, vo_id, last_modified),
        ))
    }

    /// The VERSION envelope of a COMPOSITION at the named version
    /// (`GET …/versioned_composition/{uid}/version/{version_uid}`): an
    /// `ORIGINAL_VERSION`, or an `IMPORTED_VERSION` when the version was
    /// received from another system (RM common master06 §Version and its
    /// Subtypes), with the version metadata the wire's `ETag`/`Last-Modified`
    /// need.
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `OBJECT_VERSION_ID`;
    /// [`ServiceError::NotFound`] when the version does not exist or belongs
    /// to another EHR; [`ServiceError::Database`] on a storage failure.
    pub async fn composition_version_envelope(
        &self,
        ehr_id: EhrId,
        a_version_uid: ObjectVersionId,
    ) -> Result<ServiceResponse, ServiceError> {
        let (vo_id, version) = components(&a_version_uid).map_err(VersionIdError::into_argument)?;
        let read = crate::versioning::read::read(
            &self.pool,
            self.spec_profile,
            VersionAt::Tree(vo_id, version),
        )
        .await?
        .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::Composition)
        .ok_or_else(|| {
            ServiceError::not_found(
                Missing::ObjectVersion,
                format!("COMPOSITION {vo_id} v{version}"),
            )
        })?;
        let resp = self.composition_envelope(ehr_id, &read)?;
        // The served VERSION.uid is the stored full identity; the
        // addressed uid must equal it (Resources.md §Identifier types; BASE
        // master05 case rule) — a fabricated creating_system_id names no
        // VERSION here.
        if let Some(served) = resp
            .body
            .get("uid")
            .and_then(|uid| uid.get("value"))
            .and_then(Value::as_str)
        {
            super::ensure_addressed_version(&a_version_uid, served)?;
        }
        Ok(resp)
    }

    /// The VERSION envelope of a COMPOSITION extant at `a_time`, or the
    /// latest when `a_time` is `None`
    /// (`GET …/versioned_composition/{uid}/version`), with the version
    /// metadata the wire's `ETag`/`Last-Modified` need. A deleted version
    /// still returns `200` with the deleted-lifecycle envelope (no `data`).
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `a_time` (400-equivalent);
    /// [`ServiceError::NotFound`] when no version existed at that instant or
    /// the object belongs to another EHR; [`ServiceError::Database`] on a
    /// storage failure.
    pub async fn composition_version_at_time(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        a_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let at = match a_time.as_deref().map(parse_at_time).transpose()? {
            Some(at) => VersionAt::Instant(vo_id, at),
            None => VersionAt::Current(vo_id),
        };
        let read = crate::versioning::read::read(&self.pool, self.spec_profile, at)
            .await?
            .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::Composition)
            .ok_or_else(|| {
                ServiceError::not_found(
                    Missing::ObjectVersion,
                    format!("COMPOSITION {vo_id} version at time"),
                )
            })?;
        self.composition_envelope(ehr_id, &read)
    }

    /// The VERSION envelope of a loaded COMPOSITION version, its metadata
    /// carrying the origins of the data it serves, as the bare read's does.
    ///
    /// # Errors
    /// The [`Self::envelope_response`] refusals.
    fn composition_envelope(
        &self,
        ehr_id: EhrId,
        read: &VersionRead,
    ) -> Result<ServiceResponse, ServiceError> {
        let origins = crate::versioning::origins::of_stored(
            read.origins.as_ref(),
            &read.canonical,
            &read.creating_system_id,
        );
        let mut resp = self.envelope_response(ehr_id, read)?;
        resp.meta = resp.meta.map(|meta| meta.with_origins(origins));
        Ok(resp)
    }

    /// `update_composition`: commits a new version
    /// of `vo_id` from the caller's full `UPDATE_VERSION` envelope, returning the
    /// committed version identity.
    ///
    /// The whole write pre-check rides one in-transaction statement
    /// ([`crate::storage::version_repo::placement::update_placement`]) under the
    /// per-vo advisory lock: EHR ownership (404), the full-`OBJECT_VERSION_ID`
    /// `If-Match` identity (412, ITS-REST overview §Concurrency control), the
    /// lifecycle (deleted, 404), the EHR's `is_modifiable` flag (409), the stored
    /// template root fragment (422) and the version-tree placement. The CPU-side
    /// envelope resolution and content validation run before the transaction,
    /// never under the lock, validation being free to consult the template store
    /// and routed terminology.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the COMPOSITION does not exist in this
    /// EHR or is already deleted; [`ServiceError::VersionConflict`] on an
    /// `If-Match` mismatch (→ 412); [`ServiceError::Conflict`] when the EHR is
    /// not modifiable (→ 409); [`ServiceError::Unprocessable`] on a
    /// template-id mismatch with the stored composition or a
    /// `VERSIONED_COMPOSITION` invariant violation;
    /// [`ServiceError::ValidationFailed`] when RM/terminology/template
    /// validation fails (→ 422); [`ServiceError::Database`] on a storage
    /// failure.
    pub async fn update_composition(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        request: WriteRequest<Composition>,
    ) -> Result<Committed, ServiceError> {
        let preceding = request.preceding.clone();
        // Request resolution and content validation run BEFORE the write
        // transaction — validation may consult the template store and the
        // routed terminology servers, so no pool connection or advisory lock
        // is held across it; both results surface below in pre-check order.
        let resolved =
            self.resolve_write(&request, change_type::MODIFICATION, "COMPOSITION update");
        let composition = openehr_its::json::to_canonical_value(&request.data);
        drop(request);
        let validated = match &resolved {
            Ok(parts) => Some(
                self.validate_for_commit(Kind::Composition, &composition, parts.incomplete)
                    .await,
            ),
            Err(_) => None,
        };
        let template_id = composition_template_id(&composition).map(str::to_owned);
        self.commit_one(&self.pool, Mode::Transaction, async move |tx| {
            // Serialize concurrent writers of the same object, then read the
            // placement + every pre-check column in ONE statement (thaw riding).
            crate::storage::version_repo::commit::advisory_lock(tx, vo_id).await?;
            let pre = crate::storage::version_repo::placement::update_placement(tx, vo_id).await?;
            let Some(tip) = pre
                .placement
                .tip
                .as_ref()
                .filter(|t| t.ehr_id == Some(ehr_id))
            else {
                return Err(ServiceError::not_found(
                    Missing::Composition,
                    format!("COMPOSITION {vo_id}"),
                ));
            };
            ensure_preceding(&tip_uid(vo_id, tip), preceding.as_ref())?;
            let Resolved {
                audit, envelope, ..
            } = resolved?;
            // The lifecycle (deleted → 404, RM common master06 §Logical
            // Deletion) and the content-write guard are checked from the same
            // merged read.
            if tip.lifecycle_state == crate::versioning::lifecycle::state::DELETED {
                return Err(ServiceError::not_found(
                    Missing::Composition,
                    format!("COMPOSITION {vo_id} is deleted"),
                ));
            }
            // is_modifiable = False forbids content writes (RM ehr master04 §EHR
            // Active Status), after the deleted 404 and before the template 422.
            if pre.is_modifiable == Some(false) {
                return Err(crate::versioning::change::precondition::not_modifiable_error(ehr_id));
            }
            // An update declaring a *different* template than the version it
            // supersedes is a semantic 422.
            // NOTE: `versioned_composition.adoc` pins archetype_node_id and
            // is_persistent across versions but not template_id, so this
            // convention is our own design.
            if let (Some(stored), Some(incoming)) =
                (pre.stored_template.as_deref(), template_id.as_deref())
                && stored != incoming
            {
                return Err(ServiceError::content_invalid(
                    Violation::new(format!(
                        "is {incoming} on the update, but the stored composition was \
                         committed against template {stored} (template_id mismatch)"
                    ))
                    .with_path("COMPOSITION.archetype_details.template_id"),
                ));
            }
            if let Some(validation) = validated {
                validation?;
            }
            // VERSIONED_COMPOSITION cross-version invariants (RM ehr
            // `versioned_composition.adoc`), checked off the same merged read.
            crate::validation::versioned::check_versioned_composition_first_root(
                pre.first_root,
                &composition,
            )?;
            Ok(Write::new(
                Some(ehr_id),
                Change::modify(
                    vo_id,
                    Kind::Composition,
                    composition,
                    None,
                    template_id,
                    envelope,
                ),
                audit,
            )
            .placed(pre.placement))
        })
        .await
    }

    /// The current COMPOSITION version metadata (the latest `version_uid` a
    /// `409`/`412` must echo), or `None` if unknown or outside this EHR.
    ///
    /// # Errors
    /// [`ServiceError`] if the metadata read fails.
    pub async fn composition_latest_meta(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        // The `ETag`/`If-Match` compare needs only the full `OBJECT_VERSION_ID`
        // + commit instant (RM common master06 §Version Identification /
        // §Committal and Audits), never the document reassembly `read_current` pays.
        let Some(m) = crate::storage::version_repo::meta::current_version_meta_scoped(
            &self.pool, vo_id, ehr_id,
        )
        .await?
        else {
            return Ok(None);
        };
        let tree = TreeId::from_columns(m.trunk_version, m.branch_number, m.branch_version);
        Ok(Some(self.version_meta(
            ehr_id,
            vo_id,
            &m.creating_system_id,
            tree,
            m.time_committed,
        )))
    }

    /// The OPT `template_id` a COMPOSITION version was committed against, read
    /// back from the version. `version` = the `VERSION_TREE_ID` lexical form
    /// (`N` or `N.B.V`); `None` = the current version. The ABAC template
    /// attribute for the access pre-checks / any per-version resolver.
    ///
    /// NOTE (settled shape): this resolves through the promoted
    /// `version.template_id` column — one scalar `SELECT`, no node
    /// reassembly — because it runs per authorization check. No openEHR spec
    /// governs the storage mechanics; the promoted column is our own design.
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `version` string or a failing version
    /// read-back.
    pub async fn template_of_version(
        &self,
        vo_id: VoId,
        version: Option<&str>,
    ) -> Result<Option<String>, ServiceError> {
        // One scalar read of the promoted `version.template_id` column —
        // this resolver runs per authorization check, so it must never pay a
        // node reassembly.
        let tree = version.map(parse_tree_id).transpose()?;
        Ok(crate::storage::version_repo::meta::template_id_of(
            &self.pool,
            vo_id,
            tree.map(TreeId::columns),
        )
        .await?
        .flatten())
    }

    /// The template, root archetype and origins of one version, the content
    /// the ABAC template, category and origin attributes are read from (EHDS
    /// Annex II 3.4). `version` = the `VERSION_TREE_ID` lexical form (`N` or
    /// `N.B.V`); `None` = the current version. One indexed read, no node
    /// reassembly. No openEHR spec governs authorization: our own design.
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `version` string or a failing read.
    pub async fn content_of_version(
        &self,
        vo_id: VoId,
        version: Option<&str>,
    ) -> Result<Option<crate::storage::version_repo::meta::VersionContent>, ServiceError> {
        let tree = version.map(parse_tree_id).transpose()?;
        Ok(crate::storage::version_repo::meta::content_of(
            &self.pool,
            vo_id,
            tree.map(TreeId::columns),
        )
        .await?)
    }

    /// `delete_composition`: commits a
    /// `523|deleted|` version of the addressed COMPOSITION (RM common master06
    /// §Logical Deletion), returning the now-deleted version identity
    /// (`204_COMPOSITION_deleted`).
    ///
    /// NOTE: it takes the full `OBJECT_VERSION_ID`, the mandatory
    /// `preceding_version_uid` of `composition_delete.yaml`; the request's
    /// `preceding` is a volunteered `If-Match`.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the COMPOSITION does not exist in this
    /// EHR; [`ServiceError::BadRequest`] when it is already deleted;
    /// [`ServiceError::Conflict`] when the EHR is not modifiable or the
    /// `preceding_version_uid` is stale (→ 409);
    /// [`ServiceError::VersionConflict`] when the `If-Match` names another
    /// version (→ 412); [`ServiceError::Database`] on a storage failure.
    pub async fn delete_composition(
        &self,
        ehr_id: EhrId,
        a_version_uid: &ObjectVersionId,
        request: WriteRequest,
    ) -> Result<Committed, ServiceError> {
        let (vo_id, _) = components(a_version_uid)?;
        // The delete pre-checks need only the owning EHR, the lifecycle and the
        // EHR's flag, never a node reassembly (a deleted version stores no
        // nodes anyway).
        let current =
            crate::storage::version_repo::meta::current_composition_meta(&self.pool, vo_id)
                .await?
                .filter(|m| m.ehr_id == Some(ehr_id))
                .ok_or_else(|| {
                    ServiceError::not_found(Missing::Composition, format!("COMPOSITION {vo_id}"))
                })?;
        if current.lifecycle_state == crate::versioning::lifecycle::state::DELETED {
            return Err(ServiceError::precondition(format!(
                "COMPOSITION {vo_id} is already deleted"
            )));
        }
        // is_modifiable = False forbids content writes (RM ehr master04 §EHR
        // Active Status), after the already-deleted 400 and before the
        // stale-precondition 409.
        if !current.is_modifiable {
            return Err(crate::versioning::change::precondition::not_modifiable_error(ehr_id));
        }
        self.commit_one(&self.pool, Mode::Transaction, async move |tx| {
            let placement = lock_current(tx, Some(ehr_id), vo_id, Kind::Composition).await?;
            let latest_uid = placement
                .tip
                .as_ref()
                .map(|tip| tip_uid(vo_id, tip))
                .unwrap_or_default();
            // The addressed preceding_version_uid must identify THE latest
            // VERSION by its full three-part identity (Resources.md §Identifier
            // types; RM common master06 §Distributed Versioning), compared
            // case-insensitively (BASE master05 §Composite Identifiers and
            // Case): the tree id alone would let a fabricated
            // creating_system_id delete the latest version.
            if !composite_ids_equal(&latest_uid, a_version_uid.value()) {
                return Err(ServiceError::conflict(format!(
                    "preceding_version_uid names version {}, latest is {latest_uid}",
                    a_version_uid.value()
                )));
            }
            // A volunteered `If-Match` is evaluated after the 404/400/409
            // pre-checks, so the RFC 9110 §13.2.1 precedence holds by
            // construction; a false condition refuses the delete with 412
            // (ITS-REST overview Requests_and_responses §"If-Match and
            // accidental overwrites").
            ensure_preceding(&latest_uid, request.preceding.as_ref())?;
            // A DELETE is a commit on a change-controlled resource, so the
            // committal request headers merge here too (ITS-REST overview
            // §"openehr-version and openehr-audit-details": the headers MUST be
            // accepted on PUT, POST and DELETE).
            let audit = self.commit_audit(
                request.audit.as_ref(),
                change_type::DELETED,
                "COMPOSITION delete",
            )?;
            Ok(Write::new(
                Some(ehr_id),
                Change::delete(vo_id, Kind::Composition, None),
                audit,
            )
            .placed(placement))
        })
        .await
    }

    /// The combined EHR-existence and content-writability gate in one round
    /// trip.
    ///
    /// Equivalent to [`crate::storage::ehr_repo::require_ehr`] followed by
    /// [`Self::ensure_content_writable`], a missing EHR still answering 404
    /// before the non-modifiable 409, over a single
    /// [`crate::storage::ehr_repo::ehr_writability`] read. Returns the database
    /// `now()` that statement read, which the create path passes on as the commit
    /// instant. The guarded concepts are RM ehr master04 §EHR Creation and §EHR
    /// Active Status; no openEHR spec governs the query shape (our own design).
    ///
    /// Runs on the CALLER'S executor so a write path can spend one pooled
    /// connection on its gates and its commit: with multi-tenancy on, every
    /// checkout costs a `set_config` round trip (`crate::db`), so the acquire
    /// count is the cost (measured on #3097).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR does not exist;
    /// [`ServiceError::Conflict`] when it is not modifiable;
    /// [`ServiceError::Database`] if the read fails.
    pub(in crate::service) async fn ensure_ehr_content_writable<'e>(
        &self,
        executor: impl sqlx::PgExecutor<'e>,
        ehr_id: EhrId,
    ) -> Result<jiff::Timestamp, ServiceError> {
        let (exists, is_modifiable, now) =
            crate::storage::ehr_repo::ehr_writability(executor, ehr_id).await?;
        if !exists {
            return Err(ServiceError::not_found(
                Missing::Ehr,
                format!("EHR {ehr_id}"),
            ));
        }
        // `None` (no current EHR_STATUS) is treated as modifiable, so the guard
        // never spuriously blocks — identical to `ensure_content_writable`.
        if is_modifiable == Some(false) {
            return Err(crate::versioning::change::precondition::not_modifiable_error(ehr_id));
        }
        Ok(now)
    }
}

// ── ITS-REST MultimediaAdapter (adapter-support extension) ────────────────────

impl FerroEhrService {
    /// Re-inline externalized multimedia content into a canonical body before
    /// it is served (the S3/object-store externalization extension — no
    /// openEHR spec governs storage movement; the served canonical form is
    /// unchanged). With no engine configured the stored form is served
    /// unchanged.
    ///
    /// # Errors
    /// [`ServiceError`] when the configured multimedia engine fails to expand a
    /// reference (e.g. the external object store is unreachable).
    #[cfg(feature = "multimedia")]
    pub async fn expand_multimedia(&self, body: Value) -> Result<Value, ServiceError> {
        let Some(engine) = &self.multimedia else {
            // With no store reachable, serving the stored form is correct only
            // when there is nothing to expand: answering 200 with a compact
            // reference would silently withhold clinical content this server
            // externalized.
            if ferroehr_ext::multimedia::references_external_blob(&body) {
                return Err(ServiceError::exception(
                    "expand a multimedia reference: this record references externalized \
                     multimedia but no object store is configured, so the content cannot be \
                     re-inlined (configure record.multimedia.url, or re-enable multimedia, to \
                     serve it)",
                ));
            }
            return Ok(body);
        };
        let mut body = body;
        engine
            .expand(&mut body)
            .await
            .map_err(|e| ServiceError::internal("expand a multimedia reference", e))?;
        Ok(body)
    }

    /// Serves the stored canonical form unchanged: externalization is compiled
    /// out of this build, so nothing was ever moved out of a body.
    ///
    /// # Errors
    /// Infallible in this configuration; the `Result` mirrors the multimedia
    /// twin so callers are configuration-independent.
    ///
    /// The `async` is the twin's signature, not this body's need: the one
    /// caller lives in `ferroehr-rest` and awaits this method however the
    /// binary was built.
    #[cfg(not(feature = "multimedia"))]
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "signature parity with the multimedia twin, which awaits"
    )]
    pub async fn expand_multimedia(&self, body: Value) -> Result<Value, ServiceError> {
        Ok(body)
    }
}
