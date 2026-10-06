// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_EHR_COMPOSITION` (`i_ehr_composition.adoc`) — COMPOSITION
//! commit/retrieve with implicit CONTRIBUTION creation. The commit-validation
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

use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::datetime::parse_at_time;
#[cfg(feature = "multimedia")]
use crate::service::error::internal_fault;
use crate::service::error::{ServiceError, Violation};
use crate::service::response::{RawServiceResponse, ReadBody, ResourceMeta, ServiceResponse};
use crate::service::status::{CallStatusType, SmError};
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{create, delete, update_with_placement};
use crate::versioning::object_version_id::{TreeId, components, parse_tree_id};
use crate::versioning::read::{read_current, read_version, version_at};
use crate::versioning::wire::{revision_history, version_envelope, versioned_object};
use openehr_its::rest::generated::common::UpdateVersion;
use openehr_rm::prelude::Composition;

use super::resolve_envelope;
use super::validation::composition_template_id;

impl FerroEhrService {
    /// `create_composition` (SM `i_ehr_composition.adoc`): commit the first
    /// version of a COMPOSITION in `ehr_id` from the caller's full
    /// `UPDATE_VERSION` envelope, returning the committed version identity
    /// ([`Committed`](crate::versioning::change::Committed) — the
    /// `ETag`/`Location`/`Last-Modified` source). The envelope's audit
    /// attributes, lifecycle state, verbatim signature and attestations are
    /// honoured on the persisted commit (ITS-REST committal-header merge —
    /// MUST).
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
        version: UpdateVersion<Composition>,
    ) -> Result<crate::versioning::change::Committed, ServiceError> {
        let version = super::canonicalize(version);
        // 553|incomplete| relaxes validation strictness (master06 §Version
        // Lifecycle).
        let super::CommitParts {
            audit,
            envelope,
            incomplete,
            canonical: composition,
        } = resolve_envelope(
            version,
            change_type::CREATION,
            "COMPOSITION creation",
            &self.effective_system_id(),
        )?;
        // ONE pooled connection carries both gates AND the commit, rather
        // than the three or four checkouts this path used to take: measured at
        // +0.4 ms per commit on #3097.
        let mut conn = self.pool.acquire().await?;
        // The EHR-existence (404) and content-writability (409) gates in one
        // round trip: a COMPOSITION is EHR content (RM ehr master04 §EHR
        // Creation / §EHR Active Status).
        let commit_now = self.ensure_ehr_content_writable(&mut *conn, ehr_id).await?;
        self.validate_for_commit(Kind::Composition, &composition, incomplete)
            .await?;
        self.reject_duplicate_persistent(&mut *conn, ehr_id, &composition)
            .await?;

        // The committed template identity is promoted to `version.template_id`
        // — the ABAC template attribute resolver (`template_of_version`) and the
        // template-delete guard both read that column, so the direct route
        // stamps it exactly like the CONTRIBUTION route.
        let template_id = composition_template_id(&composition).map(str::to_owned);

        // With no attestations and no event outbox the commit is ONE folded,
        // self-atomic statement, so the explicit BEGIN/COMMIT is skipped. The
        // versioned-write future is boxed: inlining it puts the whole
        // node-decomposition machinery on every caller's stack.
        let signing_ctx = self.signing_ctx();
        let committed = if envelope.attestations.is_empty() && !signing_ctx.outbox_enabled {
            Box::pin(create(
                &mut conn,
                Some(ehr_id),
                Kind::Composition,
                composition,
                template_id.as_deref(),
                &audit,
                envelope,
                &signing_ctx,
                Some(commit_now),
            ))
            .await?
        } else {
            let mut tx = sqlx::Connection::begin(&mut *conn).await?;
            let committed = Box::pin(create(
                &mut tx,
                Some(ehr_id),
                Kind::Composition,
                composition,
                template_id.as_deref(),
                &audit,
                envelope,
                &signing_ctx,
                Some(commit_now),
            ))
            .await?;
            tx.commit().await?;
            committed
        };
        crate::telemetry::metrics::metrics()
            .db_transactions
            .add(1, &[opentelemetry::KeyValue::new("outcome", "commit")]);
        crate::versioning::change::meter_committed(&committed);

        Ok(committed)
    }

    /// Retrieve a COMPOSITION by its versioned-object id, optionally at a
    /// specific version (else the latest). A deleted version resolves to
    /// `Value::Null` (→ `204`) — never 404 or 500.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the version does not exist or belongs
    /// to another EHR; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn read_composition(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        version: Option<TreeId>,
    ) -> Result<ServiceResponse, ServiceError> {
        let read = match version {
            Some(v) => read_version(&self.pool, self.spec_profile, vo_id, v).await?,
            None => read_current(&self.pool, self.spec_profile, vo_id).await?,
        }
        .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::Composition)
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::CompositionDoesNotExist,
                format!("COMPOSITION {vo_id}"),
            )
        })?;

        if read.deleted() {
            return Ok(ServiceResponse::plain(Value::Null));
        }
        Ok(self.version_response(ehr_id, vo_id, read)?)
    }

    /// [`Self::read_composition`] with the body kept as the stored canonical
    /// text wherever nothing needs it parsed — the JSON-accept passthrough.
    ///
    /// The stored body is uid-stamped at commit (`stamp_version_uid` runs before
    /// decomposition) and jsonb renders object keys length-first, so a stamped
    /// COMPOSITION's text opens with its own `uid` and the prefix compare below
    /// proves the stamp byte-exactly. Any other shape, such as a verbatim-loaded
    /// foreign body, falls back to the parsed re-stamp path.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the version does not exist or belongs
    /// to another EHR; [`ServiceError::Database`] on a storage failure;
    /// [`ServiceError::Internal`] on undecodable stored text.
    pub(in crate::service) async fn read_composition_raw(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        version: Option<TreeId>,
    ) -> Result<RawServiceResponse, ServiceError> {
        let raw = match version {
            Some(v) => {
                crate::versioning::read::read_version_raw(&self.pool, self.spec_profile, vo_id, v)
                    .await?
            }
            None => {
                crate::versioning::read::read_current_raw(&self.pool, self.spec_profile, vo_id)
                    .await?
            }
        }
        .filter(|r| r.read.ehr_id == Some(ehr_id) && r.read.kind == Kind::Composition)
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::CompositionDoesNotExist,
                format!("COMPOSITION {vo_id}"),
            )
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

    /// A COMPOSITION as it was at an instant (time-travel), with its `uid`
    /// set. A deleted version resolves to an empty body (→ `204`).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when no version existed at `at` or the
    /// object belongs to another EHR; [`ServiceError::Database`] on a storage
    /// failure.
    pub(in crate::service) async fn composition_at_time(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        at: jiff::Timestamp,
    ) -> Result<ServiceResponse, ServiceError> {
        let read = version_at(&self.pool, self.spec_profile, vo_id, at)
            .await?
            .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::Composition)
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::CompositionDoesNotExist,
                    format!("COMPOSITION {vo_id}"),
                )
            })?;
        if read.deleted() {
            return Ok(ServiceResponse::plain(Value::Null));
        }
        Ok(self.version_response(ehr_id, vo_id, read)?)
    }

    /// The `VERSIONED_OBJECT` for a COMPOSITION (verifies EHR ownership).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the object does not exist or belongs to
    /// another EHR; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn versioned_composition(
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
                    ServiceError::sm(
                        CallStatusType::CompositionDoesNotExist,
                        format!("COMPOSITION {vo_id}"),
                    )
                })?;
        Ok(ServiceResponse::new(
            body,
            super::meta::container_meta(ehr_id, vo_id, last_modified),
        ))
    }

    /// The `REVISION_HISTORY` of a COMPOSITION.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the object does not exist in this EHR;
    /// [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn composition_revision_history_value(
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

    /// The VERSION envelope of a COMPOSITION at a specific version — an
    /// `ORIGINAL_VERSION`, or an `IMPORTED_VERSION` when the version was
    /// received from another system (RM common master06 §Version and its
    /// Subtypes).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the version does not exist or belongs
    /// to another EHR; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn composition_version(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        version: TreeId,
    ) -> Result<Value, ServiceError> {
        let read = read_version(&self.pool, self.spec_profile, vo_id, version)
            .await?
            .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::Composition)
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::ObjectVersionDoesNotExist,
                    format!("COMPOSITION {vo_id} v{version}"),
                )
            })?;
        version_envelope(&read, self.signer())
    }

    /// The VERSION envelope of a COMPOSITION extant at `at`, or the latest
    /// when `at` is `None` (`GET …/versioned_composition/{uid}/version`). A deleted version still returns `200` with the
    /// deleted-lifecycle envelope (no `data`).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when no version existed at `at` or the
    /// object belongs to another EHR; [`ServiceError::Database`] on a storage
    /// failure.
    pub(in crate::service) async fn composition_version_at_time_read(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        at: Option<jiff::Timestamp>,
    ) -> Result<ServiceResponse, ServiceError> {
        let read = match at {
            Some(at) => version_at(&self.pool, self.spec_profile, vo_id, at).await?,
            None => read_current(&self.pool, self.spec_profile, vo_id).await?,
        }
        .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::Composition)
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::ObjectVersionDoesNotExist,
                format!("COMPOSITION {vo_id} version at time"),
            )
        })?;
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
        let ov = version_envelope(&read, self.signer())?;
        Ok(ServiceResponse::new(ov, meta))
    }

    /// `update_composition` (SM `i_ehr_composition.adoc`): commits a new version
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
        version: UpdateVersion<Composition>,
    ) -> Result<crate::versioning::change::Committed, ServiceError> {
        let version = super::canonicalize(version);
        let preceding_version_uid = version.preceding_version_uid.clone();
        // Envelope resolution and content validation run BEFORE the write
        // transaction — validation may consult the template store and the
        // routed terminology servers, so no pool connection or advisory lock
        // is held across it; both results surface below in pre-check order.
        let resolved = resolve_envelope(
            version,
            change_type::MODIFICATION,
            "COMPOSITION update",
            &self.effective_system_id(),
        );
        let validated = match &resolved {
            Ok(parts) => Some(
                self.validate_for_commit(Kind::Composition, &parts.canonical, parts.incomplete)
                    .await,
            ),
            Err(_) => None,
        };
        let mut tx = self.pool.begin().await?;
        // Serialize concurrent writers of the same object, then read the
        // placement + every pre-check column in ONE statement (thaw riding).
        crate::storage::version_repo::commit::advisory_lock(&mut tx, vo_id).await?;
        let pre = crate::storage::version_repo::placement::update_placement(&mut tx, vo_id).await?;
        let Some(tip) = pre
            .placement
            .tip
            .as_ref()
            .filter(|t| t.ehr_id == Some(ehr_id))
        else {
            return Err(ServiceError::sm(
                CallStatusType::CompositionDoesNotExist,
                format!("COMPOSITION {vo_id}"),
            ));
        };
        let expected = self.update_if_match_gate(
            ehr_id,
            vo_id,
            tip,
            pre.tip_time_committed,
            preceding_version_uid.as_ref(),
        )?;
        let super::CommitParts {
            audit,
            envelope,
            incomplete: _,
            canonical: composition,
        } = resolved?;
        // The lifecycle (deleted → 404, RM common master06 §Logical Deletion)
        // and the content-write guard are checked from the same merged read.
        if tip.lifecycle_state == crate::versioning::lifecycle::state::DELETED {
            return Err(ServiceError::sm(
                CallStatusType::CompositionDoesNotExist,
                format!("COMPOSITION {vo_id} is deleted"),
            ));
        }
        // is_modifiable = False forbids content writes (RM ehr master04 §EHR
        // Active Status), after the deleted 404 and before the template 422.
        if pre.is_modifiable == Some(false) {
            return Err(crate::versioning::change::not_modifiable_error(ehr_id));
        }
        // An update declaring a *different* template than the version it
        // supersedes is a semantic 422.
        // NOTE: `versioned_composition.adoc` pins archetype_node_id and
        // is_persistent across versions but not template_id, so this
        // convention is our own design.
        let stored_template = pre.stored_template.as_deref();
        if let (Some(stored), Some(incoming)) =
            (stored_template, composition_template_id(&composition))
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

        let template_id = composition_template_id(&composition).map(str::to_owned);

        // VERSIONED_COMPOSITION cross-version invariants (RM ehr
        // `versioned_composition.adoc`), checked off the same merged read.
        super::validation::check_versioned_composition_first_root(pre.first_root, &composition)?;
        let committed = update_with_placement(
            &mut tx,
            Some(ehr_id),
            vo_id,
            Kind::Composition,
            composition,
            expected,
            template_id.as_deref(),
            &audit,
            envelope,
            &self.signing_ctx(),
            pre.placement,
        )
        .await?;
        tx.commit().await?;
        crate::telemetry::metrics::metrics()
            .db_transactions
            .add(1, &[opentelemetry::KeyValue::new("outcome", "commit")]);
        crate::versioning::change::meter_committed(&committed);

        Ok(committed)
    }

    /// The composition-update `If-Match` gate off the merged in-transaction
    /// read: the full-`OBJECT_VERSION_ID` compare against the current tip
    /// (412 on mismatch — ITS-REST overview §Concurrency control), returning
    /// the expected `VERSION_TREE_ID` the placement decision pins.
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] on an `If-Match` mismatch (→ 412);
    /// the `components` rejection of a malformed `preceding_version_uid`;
    /// [`ServiceError::Internal`] when the tip carries no commit audit.
    fn update_if_match_gate(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        tip: &crate::storage::version_repo::placement::TipRow,
        tip_time_committed: Option<jiff::Timestamp>,
        preceding_version_uid: Option<&ObjectVersionId>,
    ) -> Result<Option<TreeId>, ServiceError> {
        let tree = TreeId::from_columns(tip.trunk_version, tip.branch_number, tip.branch_version);
        let tip_time_committed = tip_time_committed.ok_or_else(|| {
            ServiceError::exception(format!(
                "the current version of COMPOSITION {vo_id} has no commit audit"
            ))
        })?;
        let latest = self.version_meta(
            ehr_id,
            vo_id,
            &tip.creating_system_id,
            tree,
            tip_time_committed,
        );
        super::ensure_if_match(preceding_version_uid, Some(&latest))?;
        preceding_version_uid
            .map(|o| components(o).map(|(_, v)| v))
            .transpose()
            .map_err(ServiceError::from)
    }

    /// The current COMPOSITION version metadata (the latest `version_uid` a
    /// `409`/`412` must echo), or `None` if unknown/deleted.
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the metadata read fails.
    pub(in crate::service) async fn composition_current_meta(
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

    /// `delete_composition` (SM `i_ehr_composition.adoc`): commits a
    /// `523|deleted|` version of the addressed COMPOSITION (RM common master06
    /// §Logical Deletion), returning the now-deleted version identity
    /// (`204_COMPOSITION_deleted`).
    ///
    /// NOTE: it takes the full `OBJECT_VERSION_ID`, the mandatory
    /// `preceding_version_uid` of `composition_delete.yaml`, which is stronger
    /// than the SM's `UUID`; the SM is internally inconsistent, its
    /// `has_composition` taking an `OBJECT_VERSION_ID`.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the COMPOSITION does not exist in this
    /// EHR; [`ServiceError::BadRequest`] when it is already deleted;
    /// [`ServiceError::Conflict`] when the EHR is not modifiable or the
    /// `preceding_version_uid` is stale (→ 409); [`ServiceError::Database`] on
    /// a storage failure.
    pub async fn delete_composition(
        &self,
        ehr_id: EhrId,
        a_version_uid: &ObjectVersionId,
        volunteered_if_match: Option<&ObjectVersionId>,
        update_audit: Option<&openehr_its::rest::generated::common::UpdateAudit>,
    ) -> Result<crate::versioning::change::Committed, ServiceError> {
        let (vo_id, expected) = components(a_version_uid)?;
        // The delete pre-checks need only the owning EHR, the lifecycle and the
        // current `VERSION_TREE_ID` — never a node reassembly (a deleted
        // version stores no nodes anyway).
        let current =
            crate::storage::version_repo::meta::current_composition_meta(&self.pool, vo_id)
                .await?
                .filter(|m| m.ehr_id == Some(ehr_id))
                .ok_or_else(|| {
                    ServiceError::sm(
                        CallStatusType::CompositionDoesNotExist,
                        format!("COMPOSITION {vo_id}"),
                    )
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
            return Err(crate::versioning::change::not_modifiable_error(ehr_id));
        }
        let current_tree = TreeId::from_columns(
            current.trunk_version,
            current.branch_number,
            current.branch_version,
        );
        // The addressed preceding_version_uid must identify THE latest VERSION
        // by its full three-part identity (Resources.md §Identifier types; RM
        // common master06 §Distributed Versioning), compared case-insensitively
        // (BASE master05 §Composite Identifiers and Case): the tree id alone
        // would let a fabricated creating_system_id delete the latest version.
        let latest_uid = crate::versioning::object_version_id::object_version_id(
            vo_id,
            &current.creating_system_id,
            current_tree,
        );
        if !composite_ids_equal(&latest_uid, a_version_uid.value()) {
            return Err(ServiceError::conflict(format!(
                "preceding_version_uid names version {}, latest is {latest_uid}",
                a_version_uid.value()
            )));
        }
        // A volunteered `If-Match` is evaluated after the 404/400/409
        // pre-checks, so the RFC 9110 §13.2.1 precedence holds by construction;
        // a false condition refuses the delete with 412 (ITS-REST overview
        // Requests_and_responses §"If-Match and accidental overwrites").
        if let Some(condition) = volunteered_if_match
            && !composite_ids_equal(&latest_uid, condition.value())
        {
            return Err(ServiceError::version_conflict(format!(
                "If-Match names version {}, latest is {latest_uid}",
                condition.value()
            )));
        }
        let _ = expected;

        let mut tx = self.pool.begin().await?;
        // A DELETE is a commit on a change-controlled resource, so the
        // committal request headers merge here too (ITS-REST overview
        // §"openehr-version and openehr-audit-details": the headers MUST be
        // accepted on PUT, POST and DELETE).
        let audit = match update_audit {
            Some(u) => crate::versioning::audit::AuditInput::from_update(
                u,
                change_type::DELETED,
                "COMPOSITION delete",
                &self.effective_system_id(),
            )?,
            None => self.audit(change_type::DELETED, "COMPOSITION delete"),
        };
        let committed = delete(
            &mut tx,
            Some(ehr_id),
            vo_id,
            Kind::Composition,
            Some(expected),
            &audit,
            None,
            &self.signing_ctx(),
        )
        .await?;
        tx.commit().await?;
        crate::telemetry::metrics::metrics()
            .db_transactions
            .add(1, &[opentelemetry::KeyValue::new("outcome", "commit")]);
        crate::versioning::change::meter_committed(&committed);
        Ok(committed)
    }

    /// The EHR-existence precheck (SM `ehr_does_not_exist` → `NotFound`); also
    /// the [`crate::versioning::CommitEnv`] `ensure_ehr_exists` hook.
    /// The existence read is a storage seam
    /// ([`crate::storage::version_repo::meta::ehr_exists`]; no openEHR spec governs
    /// the SQL — our own design).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR does not exist;
    /// [`ServiceError::Database`] if the existence read fails.
    #[expect(
        clippy::same_name_method,
        reason = "the `CommitEnv` seam (service/commit_env.rs) deliberately \
                  mirrors these chapter method names so the versioning layer \
                  calls them by their own vocabulary; that impl disambiguates \
                  explicitly with `FerroEhrService::<name>(self, …)`"
    )]
    pub(in crate::service) async fn ensure_ehr_exists(
        &self,
        ehr_id: EhrId,
    ) -> Result<(), ServiceError> {
        if crate::storage::version_repo::meta::ehr_exists(&self.pool, ehr_id).await? {
            Ok(())
        } else {
            Err(ServiceError::sm(
                CallStatusType::EhrIdDoesNotExist,
                format!("EHR {ehr_id}"),
            ))
        }
    }

    /// The combined EHR-existence and content-writability gate in one round
    /// trip.
    ///
    /// Equivalent to [`Self::ensure_ehr_exists`] followed by
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
            return Err(ServiceError::sm(
                CallStatusType::EhrIdDoesNotExist,
                format!("EHR {ehr_id}"),
            ));
        }
        // `None` (no current EHR_STATUS) is treated as modifiable, so the guard
        // never spuriously blocks — identical to `ensure_content_writable`.
        if is_modifiable == Some(false) {
            return Err(crate::versioning::change::not_modifiable_error(ehr_id));
        }
        Ok(now)
    }
}

// ── The SM I_EHR_COMPOSITION call surface ─────────────────────────────────────

impl FerroEhrService {
    /// SM `I_EHR_COMPOSITION.has_composition` — whether the named version
    /// exists in the EHR.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `OBJECT_VERSION_ID` or a failing read (a
    /// missing version is `Ok(false)`).
    pub async fn has_composition(
        &self,
        an_ehr_id: EhrId,
        a_version_uid: ObjectVersionId,
    ) -> Result<bool, SmError> {
        let (vo_id, version) = components(&a_version_uid)?;
        match self.read_composition(an_ehr_id, vo_id, Some(version)).await {
            Ok(read) => Ok(match read.meta.as_ref() {
                Some(meta) => super::ensure_addressed_version(&a_version_uid, &meta.uid).is_ok(),
                // A deleted version serves no representation/meta; the
                // fetched tree row existing is the answer.
                None => true,
            }),
            Err(ServiceError::NotFound(_)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// The latest COMPOSITION of a versioned object (bare, with its `uid`); a
    /// deleted latest version yields `Value::Null` (→ 204).
    ///
    /// # Errors
    /// [`SmError`] when the object does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn get_composition_latest(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
    ) -> Result<Value, SmError> {
        let resp = self
            .composition_latest_response(an_ehr_id, a_versioned_object_uid)
            .await?;
        Ok(Self::read_body_value(a_versioned_object_uid, resp.body)?)
    }

    /// The typed value of a [`ReadBody`], mapping an undecodable stored text
    /// to an internal fault (the stored body is our own jsonb rendering).
    fn read_body_value(vo_id: VoId, body: ReadBody) -> Result<Value, ServiceError> {
        body.into_value().map_err(|e| {
            ServiceError::exception(format!(
                "the stored body of COMPOSITION {vo_id} is not decodable JSON: {e}"
            ))
        })
    }

    /// SM `I_EHR_COMPOSITION.get_composition_at_time` — the COMPOSITION
    /// current at `a_time`, or the latest when `a_time` is `None`.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `a_time` (400-equivalent), a missing
    /// version at that instant (404-equivalent), or a read failure.
    pub async fn get_composition_at_time(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
        a_time: Option<String>,
    ) -> Result<Value, SmError> {
        let resp = self
            .composition_at_time_response(an_ehr_id, a_versioned_object_uid, a_time)
            .await?;
        Ok(Self::read_body_value(a_versioned_object_uid, resp.body)?)
    }

    /// SM `I_EHR_COMPOSITION.get_composition_at_version` — the bare
    /// COMPOSITION at the named version.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `OBJECT_VERSION_ID`, an unknown version
    /// (404-equivalent), or a read failure.
    pub async fn get_composition_at_version(
        &self,
        an_ehr_id: EhrId,
        a_version_uid: ObjectVersionId,
    ) -> Result<Value, SmError> {
        let (vo_id, _) = components(&a_version_uid)?;
        let resp = self
            .composition_at_version_response(an_ehr_id, a_version_uid)
            .await?;
        Ok(Self::read_body_value(vo_id, resp.body)?)
    }

    /// SM `I_EHR_COMPOSITION.get_versioned_composition` — the
    /// `VERSIONED_COMPOSITION` container object.
    ///
    /// # Errors
    /// [`SmError`] when the object does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn get_versioned_composition(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
    ) -> Result<Value, SmError> {
        Ok(self
            .versioned_composition(an_ehr_id, a_versioned_object_uid)
            .await?
            .body)
    }

    /// [`Self::get_versioned_composition`] with the container metadata the
    /// wire's `ETag`/`Last-Modified` need: the container uid identity plus the
    /// newest held version's commit instant (ITS-REST overview
    /// `Requests_and_responses.md` §"`ETag` and Last-Modified" — both headers
    /// SHOULD accompany a `VERSIONED_OBJECT` response).
    ///
    /// # Errors
    /// [`SmError`] when the object does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn versioned_composition_response(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
    ) -> Result<ServiceResponse, SmError> {
        Ok(self
            .versioned_composition(an_ehr_id, a_versioned_object_uid)
            .await?)
    }

    /// SM `I_EHR_COMPOSITION.get_revision_history` — the `REVISION_HISTORY`
    /// of a COMPOSITION.
    ///
    /// # Errors
    /// [`SmError`] when the object does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn composition_revision_history(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
    ) -> Result<Value, SmError> {
        Ok(self
            .composition_revision_history_value(an_ehr_id, a_versioned_object_uid)
            .await?
            .body)
    }

    /// [`Self::composition_revision_history`] with the container metadata the
    /// wire's `ETag`/`Last-Modified` need (container uid + newest commit
    /// instant — same derivation as
    /// [`Self::versioned_composition_response`]).
    ///
    /// # Errors
    /// [`SmError`] when the object does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn composition_revision_history_response(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
    ) -> Result<ServiceResponse, SmError> {
        Ok(self
            .composition_revision_history_value(an_ehr_id, a_versioned_object_uid)
            .await?)
    }

    /// The VERSION envelope of a COMPOSITION extant at `a_time`, or the
    /// latest when `a_time` is `None`
    /// (`GET …/versioned_composition/{uid}/version`).
    ///
    /// # Errors
    /// [`SmError`] for a malformed `a_time` (400-equivalent), a missing
    /// version at that instant (404-equivalent), or a read failure.
    pub async fn composition_version_at_time(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
        a_time: Option<String>,
    ) -> Result<Value, SmError> {
        let at = a_time.as_deref().map(parse_at_time).transpose()?;
        Ok(self
            .composition_version_at_time_read(an_ehr_id, a_versioned_object_uid, at)
            .await?
            .body)
    }

    /// The VERSION envelope of a COMPOSITION at the named version
    /// (`GET …/versioned_composition/{uid}/version/{version_uid}`).
    ///
    /// # Errors
    /// [`SmError`] for a malformed `OBJECT_VERSION_ID`, an unknown version
    /// (404-equivalent), or a read failure.
    pub async fn composition_version_envelope(
        &self,
        an_ehr_id: EhrId,
        a_version_uid: ObjectVersionId,
    ) -> Result<Value, SmError> {
        let (vo_id, version) = components(&a_version_uid)?;
        let body = self.composition_version(an_ehr_id, vo_id, version).await?;
        // The served VERSION.uid is the stored full identity; the
        // addressed uid must equal it (Resources.md §Identifier types; BASE
        // master05 case rule) — a fabricated creating_system_id names no
        // VERSION here.
        if let Some(served) = body
            .get("uid")
            .and_then(|uid| uid.get("value"))
            .and_then(Value::as_str)
        {
            super::ensure_addressed_version(&a_version_uid, served)?;
        }
        Ok(body)
    }
}

// ── ITS-REST read-response adapter (adapter-support extension) ────────────────
//
// The SM reads above return the bare COMPOSITION, which carries no commit
// audit, yet ITS-REST derives `Last-Modified` from
// `VERSION.commit_audit.time_committed.value` (`Requests_and_responses.md`
// §"`ETag` and Last-Modified"), so these siblings return the read plus its
// [`ResourceMeta`] (no openEHR spec governs the envelope — our own design).

impl FerroEhrService {
    /// [`Self::get_composition_latest`] with the version metadata the wire's
    /// `ETag`/`Last-Modified` need. A deleted latest version yields a null body
    /// and no metadata (→ `204`).
    ///
    /// # Errors
    /// [`SmError`] when the object does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn composition_latest_response(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
    ) -> Result<RawServiceResponse, SmError> {
        Ok(self
            .read_composition_raw(an_ehr_id, a_versioned_object_uid, None)
            .await?)
    }

    /// [`Self::get_composition_at_time`] with the version metadata the wire's
    /// `ETag`/`Last-Modified` need.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `a_time` (400-equivalent), a missing
    /// version at that instant (404-equivalent), or a read failure.
    pub async fn composition_at_time_response(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
        a_time: Option<String>,
    ) -> Result<RawServiceResponse, SmError> {
        match a_time.as_deref() {
            None => Ok(self
                .read_composition_raw(an_ehr_id, a_versioned_object_uid, None)
                .await?),
            Some(raw) => {
                let resp = self
                    .composition_at_time(an_ehr_id, a_versioned_object_uid, parse_at_time(raw)?)
                    .await?;
                Ok(RawServiceResponse {
                    body: ReadBody::Value(resp.body),
                    meta: resp.meta,
                })
            }
        }
    }

    /// [`Self::get_composition_at_version`] with the version metadata the
    /// wire's `ETag`/`Last-Modified` need.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `OBJECT_VERSION_ID`, an unknown version
    /// (404-equivalent), or a read failure.
    pub async fn composition_at_version_response(
        &self,
        an_ehr_id: EhrId,
        a_version_uid: ObjectVersionId,
    ) -> Result<RawServiceResponse, SmError> {
        let (vo_id, version) = components(&a_version_uid)?;
        let read = self
            .read_composition_raw(an_ehr_id, vo_id, Some(version))
            .await?;
        if let Some(meta) = read.meta.as_ref() {
            super::ensure_addressed_version(&a_version_uid, &meta.uid)?;
        }
        Ok(read)
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
    /// [`SmError`] when the configured multimedia engine fails to expand a
    /// reference (e.g. the external object store is unreachable).
    #[cfg(feature = "multimedia")]
    pub async fn expand_multimedia(&self, body: Value) -> Result<Value, SmError> {
        let Some(engine) = &self.multimedia else {
            // With no store reachable, serving the stored form is correct only
            // when there is nothing to expand: answering 200 with a compact
            // reference would silently withhold clinical content this server
            // externalized.
            if ferroehr_ext::multimedia::references_external_blob(&body) {
                return Err(internal_fault(
                    "expand a multimedia reference",
                    &"this record references externalized multimedia but no object store is \
                      configured, so the content cannot be re-inlined (configure \
                      multimedia.endpoint, or re-enable multimedia, to serve it)",
                ));
            }
            return Ok(body);
        };
        let mut body = body;
        engine
            .expand(&mut body)
            .await
            .map_err(|e| internal_fault("expand a multimedia reference", &e))?;
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
    pub async fn expand_multimedia(&self, body: Value) -> Result<Value, SmError> {
        Ok(body)
    }
}
