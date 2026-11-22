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

use super::validation::{duplicate_persistent, persistent_sibling, persistent_template};
use crate::error::refusal::{Missing, Refusal};
use crate::error::{ServiceError, Violation};
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::datetime::parse_at_time;
use crate::service::ehr::scope::ScopeJudge;
use crate::service::response::{RawServiceResponse, ResourceMeta, ServiceResponse};
use crate::service::version_update::WriteRequest;
use crate::service::write::{Materials, Resolved, first_attempt};
use crate::storage::version_repo::gate::{self, Ask, Gate, Shape, Target};
use crate::storage::version_repo::guarded::Guard;
use crate::storage::version_repo::read::VersionAt;
use crate::system_log::categories::ContentIds;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::body::Body;
use crate::versioning::change::precondition::{ensure_preceding, tip_uid};
use crate::versioning::change::{Change, Committed, Write};
use crate::versioning::object_version_id::{TreeId, VersionIdError, components, parse_tree_id};
use crate::versioning::read::VersionRead;
use crate::versioning::wire::{revision_history, versioned_object};
use openehr_rm::prelude::{Composition, VersionedObject};

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
        let mut composition = Body::composition(request.data);
        // Validation may consult the template store and the routed terminology
        // servers, so it runs before a connection is held; its result surfaces
        // after the EHR gates, in pre-check order. 553|incomplete| relaxes its
        // strictness (master06 §Version Lifecycle). A valid body's large media
        // go to the blob store before a connection is held.
        let validated = self
            .accept_body(Kind::Composition, &mut composition, incomplete)
            .await?;
        let persistent = persistent_template(&composition);
        // A create a later attempt judges again could only commit after a
        // double flip of the EHR, so it keeps no copy of its content.
        let materials = Materials::new((composition, audit, envelope), false);
        let validated = std::sync::Mutex::new(Some(validated));
        let (persistent, materials, validated) = (&persistent, &materials, &validated);
        self.commit_gated(&self.pool, move || {
            async move |conn| {
                // The EHR-existence (404), content-writability (409), restriction and
                // persistent-sibling gates in one round trip: a COMPOSITION is EHR
                // content (RM ehr master04 §EHR Creation / §EHR Active Status).
                let gate = gate::read(
                    conn,
                    Ask {
                        ehr_id,
                        targets: &[],
                        shape: Shape::Plain,
                        persistent: persistent.as_deref().map(persistent_sibling),
                        addressed: None,
                        items: &[],
                    },
                )
                .await?;
                self.ensure_gate_policy_held().await?;
                ensure_ehr_writable(&gate, ehr_id)?;
                // A later attempt follows one that passed validation.
                if let Some(validated) = first_attempt(validated) {
                    validated?;
                }
                if let (true, Some(template_id)) = (gate.persistent_exists, persistent.as_deref()) {
                    return Err(duplicate_persistent(ehr_id, template_id));
                }
                let (composition, audit, envelope) = materials.take()?;
                // The committed template identity is promoted to `version.template_id`,
                // which the SMART template resolver and the template-delete guard read.
                let template_id = composition.template_id().map(str::to_owned);
                Ok(Write::new(
                    Some(ehr_id),
                    Change::create(Kind::Composition, composition, template_id, envelope),
                    audit,
                )
                .at(gate.now)
                .gated(gate.restriction.clone())
                .guarded(guard_of(&gate, None)))
            }
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
        };
        self.composition_served(ehr_id, vo_id, raw)
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
        let raw = crate::versioning::read::read_raw(
            &self.pool,
            self.spec_profile,
            VersionAt::Instant(vo_id, at),
        )
        .await?;
        self.composition_served(ehr_id, vo_id, raw)
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
    ) -> Result<ServiceResponse<VersionedObject>, ServiceError> {
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
    /// ([`crate::storage::version_repo::gate::read`]) under the per-vo
    /// advisory lock: EHR ownership (404), the full-`OBJECT_VERSION_ID`
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
        let mut composition = Body::composition(request.data);
        // A valid body's large media go to the blob store before a connection is held.
        let validated = match &resolved {
            Ok(parts) => Some(
                self.accept_body(Kind::Composition, &mut composition, parts.incomplete)
                    .await?,
            ),
            Err(_) => None,
        };
        let template_id = composition.template_id().map(str::to_owned);
        // An update a later attempt judges again commits on the new tip only
        // when it names no preceding version, so only such a one keeps a copy.
        let (resolved, unresolved) = resolved.map_or_else(
            |error| (None, Some(error)),
            |resolved| (Some(resolved), None),
        );
        let materials = Materials::new((composition, resolved), preceding.is_none());
        let unresolved = std::sync::Mutex::new(unresolved);
        let validated = std::sync::Mutex::new(validated);
        let (materials, unresolved, validated) = (&materials, &unresolved, &validated);
        let (preceding, template_id) = (&preceding, &template_id);
        self.commit_gated(&self.pool, move || {
            async move |conn| {
                // The EHR, the placement and every pre-check column in ONE statement
                // (thaw riding); the guarded commit requires them unchanged.
                let mut gate = gate::read(
                    conn,
                    Ask {
                        ehr_id,
                        targets: &[vo_id],
                        shape: Shape::Update,
                        persistent: None,
                        addressed: None,
                        items: &[],
                    },
                )
                .await?;
                self.ensure_gate_policy_held().await?;
                let pre = gate.targets.pop().ok_or_else(|| {
                    ServiceError::exception(format!("the gate read returned no row for {vo_id}"))
                })?;
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
                if let Some(error) = first_attempt(unresolved) {
                    return Err(error);
                }
                let (
                    composition,
                    Some(Resolved {
                        audit, envelope, ..
                    }),
                ) = materials.take()?
                else {
                    return Err(ServiceError::exception(
                        "an unresolved update reached its write",
                    ));
                };
                ensure_updatable(&gate, (tip, &pre), template_id.as_deref(), (ehr_id, vo_id))?;
                if let Some(validation) = first_attempt(validated) {
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
                        template_id.clone(),
                        envelope,
                    ),
                    audit,
                )
                .guarded(guard_of(&gate, pre.head))
                .placed(pre.placement)
                .gated(gate.restriction.clone())
                .object_marked(pre.restricted))
            }
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
    /// (`N` or `N.B.V`); `None` = the current version. The template the
    /// SMART `composition-` gate checks.
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

    /// `delete_composition`: commits a
    /// `523|deleted|` version of the addressed COMPOSITION (RM common master06
    /// §Logical Deletion), returning the now-deleted version identity
    /// (`204_COMPOSITION_deleted`).
    ///
    /// NOTE: it takes the full `OBJECT_VERSION_ID`, the mandatory
    /// `preceding_version_uid` of `composition_delete.yaml`; the request's
    /// `preceding` is a volunteered `If-Match`.
    ///
    /// `scope` is judged first, on the template of the version addressed.
    ///
    /// # Errors
    /// [`ServiceError::ScopeRefused`] when `scope` does not cover that
    /// template; [`ServiceError::NotFound`] when the COMPOSITION does not
    /// exist in this EHR; [`ServiceError::BadRequest`] when it is already deleted;
    /// [`ServiceError::Conflict`] when the EHR is not modifiable or the
    /// `preceding_version_uid` is stale (→ 409);
    /// [`ServiceError::VersionConflict`] when the `If-Match` names another
    /// version (→ 412); [`ServiceError::Database`] on a storage failure.
    pub async fn delete_composition(
        &self,
        ehr_id: EhrId,
        a_version_uid: &ObjectVersionId,
        request: WriteRequest,
        scope: &dyn ScopeJudge,
    ) -> Result<Committed, ServiceError> {
        let (vo_id, tree) = components(a_version_uid)?;
        let request = &request;
        self.commit_gated(&self.pool, move || {
            async move |conn| {
                // The EHR, the tip and the content it deletes in ONE statement, which
                // the guarded commit requires unchanged. The delete pre-checks need no
                // node reassembly (a deleted version stores none).
                let mut gate = gate::read(
                    conn,
                    Ask {
                        ehr_id,
                        targets: &[vo_id],
                        shape: Shape::Delete,
                        persistent: None,
                        addressed: Some(tree.columns()),
                        items: &[],
                    },
                )
                .await?;
                self.ensure_gate_policy_held().await?;
                let target = gate.targets.pop().ok_or_else(|| {
                    ServiceError::exception(format!("the gate read returned no row for {vo_id}"))
                })?;
                // NOTE: SMART on openEHR master08 §Resource Scopes, the scope is judged first, on
                // the addressed version's template, where the pre-check judged it before (#6271).
                if !scope.covers(target.addressed_template.as_deref()) {
                    return Err(ServiceError::ScopeRefused(Refusal::new(
                        (),
                        "the credential does not permit this request",
                    )));
                }
                let Some(tip) = target
                    .placement
                    .tip
                    .as_ref()
                    .filter(|tip| tip.ehr_id == Some(ehr_id))
                else {
                    return Err(ServiceError::not_found(
                        Missing::Composition,
                        format!("COMPOSITION {vo_id}"),
                    ));
                };
                if tip.lifecycle_state == crate::versioning::lifecycle::state::DELETED {
                    return Err(ServiceError::precondition(format!(
                        "COMPOSITION {vo_id} is already deleted"
                    )));
                }
                // is_modifiable = False forbids content writes (RM ehr master04 §EHR
                // Active Status), after the already-deleted 400 and before the
                // stale-precondition 409.
                if gate.is_modifiable == Some(false) {
                    return Err(
                        crate::versioning::change::precondition::not_modifiable_error(ehr_id),
                    );
                }
                if tip.kind != Kind::Composition.as_str() {
                    return Err(ServiceError::not_found(
                        Missing::VersionedObject,
                        format!(
                            "{} {vo_id} in EHR {:?}",
                            Kind::Composition.as_str(),
                            Some(ehr_id)
                        ),
                    ));
                }
                let latest_uid = tip_uid(vo_id, tip);
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
                let deleted = target.content.as_ref().map(|content| {
                    ContentIds::of_kind(
                        &content.kind,
                        content.template_id.clone(),
                        content.root_archetype.clone(),
                    )
                });
                Ok(Write::new(
                    Some(ehr_id),
                    Change::delete(vo_id, Kind::Composition, None),
                    audit,
                )
                .guarded(guard_of(&gate, target.head))
                .placed(target.placement)
                .gated(gate.restriction.clone())
                .object_marked(target.restricted)
                .deleting(deleted))
            }
        })
        .await
    }
}

/// The EHR-existence (404) and content-writability (409) pre-checks of a
/// COMPOSITION write, judged on its gate read: a missing EHR answers 404
/// before the non-modifiable 409. The guarded concepts are RM ehr master04
/// §EHR Creation and §EHR Active Status; no current `EHR_STATUS` reads as
/// modifiable, so the guard never blocks spuriously.
///
/// # Errors
/// [`ServiceError::NotFound`] when the EHR does not exist;
/// [`ServiceError::Conflict`] when it is not modifiable.
fn ensure_ehr_writable(gate: &Gate, ehr_id: EhrId) -> Result<(), ServiceError> {
    match gate.is_modifiable {
        None => Err(ServiceError::not_found(
            Missing::Ehr,
            format!("EHR {ehr_id}"),
        )),
        Some(false) => Err(crate::versioning::change::precondition::not_modifiable_error(ehr_id)),
        Some(true) => Ok(()),
    }
}

/// What the guarded commit of a write its gate read judged requires still
/// holds: the object's head at `head` (none for a create) and the policy and
/// definition revisions the gate read returned.
pub(super) fn guard_of(gate: &Gate, head: Option<i32>) -> Guard {
    Guard {
        expected_head: head,
        policy: gate.policy,
        definitions: gate.definitions,
        outbox: None,
        reservation: None,
        status: None,
        folder: false,
        party: None,
    }
}

/// Refuse an update declaring a different template than the version it
/// supersedes, a semantic `422`.
///
/// The RM ehr `versioned_composition.adoc` pins `archetype_node_id` and
/// `is_persistent` across versions but not `template_id`, so this convention
/// is our own design.
///
/// # Errors
/// [`ServiceError::Unprocessable`] on a template-id mismatch.
fn ensure_same_template(stored: Option<&str>, incoming: Option<&str>) -> Result<(), ServiceError> {
    match (stored, incoming) {
        (Some(stored), Some(incoming)) if stored != incoming => Err(ServiceError::content_invalid(
            Violation::new(format!(
                "is {incoming} on the update, but the stored composition was committed \
                 against template {stored} (template_id mismatch)"
            ))
            .with_path("COMPOSITION.archetype_details.template_id"),
        )),
        _ => Ok(()),
    }
}

/// The pre-checks an update is judged on after its `If-Match`, off its gate
/// read: a deleted tip answers `404` (RM common master06 §Logical Deletion),
/// an EHR that is not modifiable `409` (RM ehr master04 §EHR Active Status),
/// and a template other than the stored one `422`, in that order.
///
/// # Errors
/// [`ServiceError::NotFound`], [`ServiceError::Conflict`] and
/// [`ServiceError::Unprocessable`] as above.
fn ensure_updatable(
    gate: &Gate,
    (tip, target): (&crate::storage::version_repo::placement::TipRow, &Target),
    template_id: Option<&str>,
    (ehr_id, vo_id): (EhrId, VoId),
) -> Result<(), ServiceError> {
    if tip.lifecycle_state == crate::versioning::lifecycle::state::DELETED {
        return Err(ServiceError::not_found(
            Missing::Composition,
            format!("COMPOSITION {vo_id} is deleted"),
        ));
    }
    if gate.is_modifiable == Some(false) {
        return Err(crate::versioning::change::precondition::not_modifiable_error(ehr_id));
    }
    ensure_same_template(target.stored_template.as_deref(), template_id)
}
