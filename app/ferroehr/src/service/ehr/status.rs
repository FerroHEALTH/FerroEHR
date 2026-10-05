// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_EHR_STATUS` (`i_ehr_status.adoc`) — the `EHR_STATUS` reads, the five
//! discrete mutators, the versioned-object views, plus two EHR-owned commit
//! hooks: the `is_modifiable` content-write guard and the promoted-subject
//! sync.
//!
//! Spec: RM ehr `ehr_status.adoc` + `master04-ehr_package.adoc` §EHR Status /
//! §EHR Active Status; the versioned-object mechanics are RM common master06,
//! delegated to [`crate::versioning`]. `EHR_STATUS` "is always modifiable"
//! (master04 §EHR Active Status), so its own commits are never gated by the
//! content-write guard — that is how a deactivated EHR is flipped back on.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use crate::ids::{EhrId, VoId};
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::status::{CallStatusType, SmError};
use openehr_its::rest::generated::common::UpdateVersion;
use openehr_rm::prelude::EhrStatus;
use serde_json::Value;
use sqlx::PgConnection;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::update;
use crate::versioning::object_version_id::{TreeId, expected_from_if_match, parse_tree_id};
use crate::versioning::read::{read_current, read_version, version_at};
use crate::versioning::wire::{revision_history, version_envelope, versioned_object};

use super::ensure_if_match;
use crate::service::datetime::parse_at_time;

impl FerroEhrService {
    /// The `EHR_STATUS` of an EHR as canonical JSON with its `uid` set — the
    /// current version, or the one current at `at` (time-travel) when given.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no current `EHR_STATUS` or
    /// none existed at `at`; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn status_at(
        &self,
        ehr_id: EhrId,
        at: Option<jiff::Timestamp>,
    ) -> Result<ServiceResponse, ServiceError> {
        // ONE statement: the container resolution and the version read merged
        // (an EHR holds exactly one EHR_STATUS container — RM ehr `ehr.adoc`).
        let read = match at {
            Some(at) => {
                crate::versioning::read::version_at_of_kind(
                    &self.pool,
                    self.spec_profile,
                    ehr_id,
                    Kind::EhrStatus,
                    at,
                )
                .await?
            }
            None => {
                crate::versioning::read::read_current_of_kind(
                    &self.pool,
                    self.spec_profile,
                    ehr_id,
                    Kind::EhrStatus,
                )
                .await?
            }
        }
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::EhrIdDoesNotExist,
                format!("EHR_STATUS for EHR {ehr_id}"),
            )
        })?;
        let vo_id = read.vo_id;
        Ok(self.version_response(ehr_id, vo_id, read)?)
    }

    /// The **bare** `EHR_STATUS` at a specific version (not the
    /// `ORIGINAL_VERSION` wrapper) — `GET …/ehr_status/{version_uid}`.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the version does not exist or belongs to
    /// another EHR; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn status_by_version(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        version: TreeId,
    ) -> Result<ServiceResponse, ServiceError> {
        let read = read_version(&self.pool, self.spec_profile, vo_id, version)
            .await?
            .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::EhrStatus)
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::ObjectVersionDoesNotExist,
                    format!("EHR_STATUS {vo_id} v{version}"),
                )
            })?;
        Ok(self.version_response(ehr_id, vo_id, read)?)
    }

    /// The one `EHR_STATUS` commit core (`replace_ehr_status` and the discrete
    /// SM mutators both land here): commit a new version from the caller's full
    /// `UPDATE_VERSION` envelope, sync the promoted subject columns, and return
    /// the committed version identity.
    ///
    /// `vo_id` is the `EHR_STATUS` versioned object, resolved once by the
    /// caller's `If-Match` meta pre-read; `if_match` is the
    /// `OBJECT_VERSION_ID` (or bare version) the client believes is current. The
    /// result is the commit's own
    /// [`Committed`](crate::versioning::change::Committed), the written version
    /// identity and commit instant (RM common master06 §Committal and Audits).
    /// The write path never re-reads the row it just wrote; a `Prefer:
    /// return=representation` body is read back at the protocol layer.
    async fn commit_status(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        version: UpdateVersion<EhrStatus>,
        if_match: &str,
    ) -> Result<crate::versioning::change::Committed, ServiceError> {
        let version = super::canonicalize(version);
        let super::CommitParts {
            audit,
            envelope,
            incomplete,
            canonical: body,
        } = super::resolve_envelope(
            version,
            change_type::MODIFICATION,
            "EHR_STATUS update",
            &self.effective_system_id(),
        )?;
        self.validate_for_commit(Kind::EhrStatus, &body, incomplete)
            .await?;
        let expected = expected_from_if_match(if_match)?;

        let mut tx = self.pool.begin().await?;
        let committed = update(
            &mut tx,
            Some(ehr_id),
            vo_id,
            Kind::EhrStatus,
            body.clone(),
            expected,
            None,
            &audit,
            envelope,
            &self.signing_ctx(),
        )
        .await?;
        // Keep the promoted subject columns in sync (the subject may have
        // changed); the is_queryable promotion rides this same UPDATE.
        self.sync_ehr_subject(&mut tx, ehr_id, &body).await?;
        tx.commit().await?;

        Ok(committed)
    }

    /// Apply a single in-place mutation to the current `EHR_STATUS` root and
    /// commit it as a new implicit-CONTRIBUTION version — the shared body of
    /// the discrete `I_EHR_STATUS` mutators (`i_ehr_status.adoc`
    /// §`set_ehr_queryable` … §`update_other_details`), formally equivalent to
    /// the whole-object replace (`master02-overview.adoc` §Interface Calls).
    /// Reuses [`Self::commit_status`] with the current version uid as the
    /// preceding version (server-driven optimistic lock).
    ///
    /// `EHR_STATUS` "is always modifiable" (RM ehr master04 §EHR Active
    /// Status), so this is deliberately not gated by
    /// [`Self::ensure_content_writable`], whose guard scopes to EHR contents;
    /// that is how `clear_ehr_modifiable` disables an EHR yet
    /// `set_ehr_modifiable` re-enables it.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no current `EHR_STATUS`;
    /// [`ServiceError::Unprocessable`] when the mutated status fails structural
    /// validation; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn status_mutate(
        &self,
        ehr_id: EhrId,
        mutate: impl FnOnce(&mut EhrStatus) -> Result<(), ServiceError>,
    ) -> Result<crate::versioning::change::Committed, ServiceError> {
        // Resolve the current EHR_STATUS versioned object once and read its
        // body; the resolved `vo_id` threads into `commit_status` so its commit
        // skips a second `current_vo` resolution.
        let (vo_id, _) = self
            .current_vo(ehr_id, Kind::EhrStatus)
            .await?
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::EhrIdDoesNotExist,
                    format!("EHR_STATUS for EHR {ehr_id}"),
                )
            })?;
        let read = read_current(&self.pool, self.spec_profile, vo_id)
            .await?
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::EhrIdDoesNotExist,
                    format!("EHR_STATUS for EHR {ehr_id}"),
                )
            })?;
        let current = self.version_response(ehr_id, vo_id, read)?;
        let preceding = current
            .meta
            .as_ref()
            .map(|m| m.uid.clone())
            .unwrap_or_default();
        // The stored fragment decodes ONCE into the typed EHR_STATUS, so a
        // mutation that cannot produce a legal one is either unrepresentable or
        // refused by the closure itself (SM `i_ehr_status.adoc`
        // §update_other_details).
        let mut status: EhrStatus = openehr_its::json::from_canonical_value(&current.body)
            .map_err(|e| {
                ServiceError::content_invalid(
                    crate::service::error::Violation::new("is not a canonical EHR_STATUS")
                        .with_path("EHR_STATUS")
                        .with_decode_failure(&e),
                )
            })?;
        // The read injects `uid`; drop it so the re-commit carries only the
        // mutated EHR_STATUS content (the server assigns the new version id).
        status.uid = None;
        mutate(&mut status)?;
        // The SM flag mutators commit a modification of the existing
        // EHR_STATUS: carry the operation's own change type so the commit
        // audit resolves without a client envelope (the `direct()` placeholder
        // is `249|creation|`, which would contradict this update —
        // `versioning::audit::merged_change_type`).
        let mut version = crate::service::version_update::direct_envelope(status);
        *crate::service::version_update::audit_base_mut(&mut version.commit_audit).change_type =
            crate::service::version_update::change_type_coded(change_type::MODIFICATION);
        self.commit_status(ehr_id, vo_id, version, &preceding).await
    }

    /// The `VERSIONED_OBJECT` for an EHR's `EHR_STATUS`.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no current `EHR_STATUS`;
    /// [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn versioned_status(
        &self,
        ehr_id: EhrId,
    ) -> Result<ServiceResponse, ServiceError> {
        let (vo_id, _) = self
            .current_vo(ehr_id, Kind::EhrStatus)
            .await?
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::EhrIdDoesNotExist,
                    format!("EHR_STATUS for EHR {ehr_id}"),
                )
            })?;
        let (body, last_modified) =
            versioned_object(&self.pool, vo_id, ehr_id, "VERSIONED_EHR_STATUS")
                .await?
                .ok_or_else(|| {
                    ServiceError::sm(
                        CallStatusType::VersionedObjectDoesNotExist,
                        format!("versioned object {vo_id}"),
                    )
                })?;
        Ok(ServiceResponse::new(
            body,
            super::meta::container_meta(ehr_id, vo_id, last_modified),
        ))
    }

    /// The `REVISION_HISTORY` of an EHR's `EHR_STATUS`.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no current `EHR_STATUS`;
    /// [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn status_revision_history(
        &self,
        ehr_id: EhrId,
    ) -> Result<ServiceResponse, ServiceError> {
        let (vo_id, _) = self
            .current_vo(ehr_id, Kind::EhrStatus)
            .await?
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::EhrIdDoesNotExist,
                    format!("EHR_STATUS for EHR {ehr_id}"),
                )
            })?;
        let (body, last_modified) = revision_history(&self.pool, ehr_id, vo_id).await?;
        Ok(ServiceResponse::new(
            body,
            super::meta::container_meta(ehr_id, vo_id, last_modified),
        ))
    }

    /// The VERSION envelope of an `EHR_STATUS` at a specific version — an
    /// `ORIGINAL_VERSION`, or an `IMPORTED_VERSION` when the version was
    /// received from another system (RM common master06 §Version and its
    /// Subtypes).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the version does not exist or belongs to
    /// another EHR; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn status_version(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        version: TreeId,
    ) -> Result<Value, ServiceError> {
        let read = read_version(&self.pool, self.spec_profile, vo_id, version)
            .await?
            .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::EhrStatus)
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::ObjectVersionDoesNotExist,
                    format!("EHR_STATUS {vo_id} v{version}"),
                )
            })?;
        version_envelope(&read, self.signer())
    }

    /// The VERSION envelope of an EHR's `EHR_STATUS` extant at `at`, or the
    /// latest when `at` is `None` (`GET …/versioned_ehr_status/version`). The metadata carries the `version_uid` for the
    /// `200_VERSION_at_time` `ETag`/`Location`.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no current `EHR_STATUS` or
    /// none existed at `at`; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn status_version_at_time(
        &self,
        ehr_id: EhrId,
        at: Option<jiff::Timestamp>,
    ) -> Result<ServiceResponse, ServiceError> {
        let (vo_id, _) = self
            .current_vo(ehr_id, Kind::EhrStatus)
            .await?
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::EhrIdDoesNotExist,
                    format!("EHR_STATUS for EHR {ehr_id}"),
                )
            })?;
        let read = match at {
            Some(at) => version_at(&self.pool, self.spec_profile, vo_id, at).await?,
            None => read_current(&self.pool, self.spec_profile, vo_id).await?,
        }
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::ObjectVersionDoesNotExist,
                format!("EHR_STATUS version at time for EHR {ehr_id}"),
            )
        })?;
        let meta = self.version_meta(
            ehr_id,
            vo_id,
            &read.creating_system_id,
            read.tree,
            read.time_committed,
        );
        let ov = version_envelope(&read, self.signer())?;
        Ok(ServiceResponse::new(ov, meta))
    }

    /// The current `EHR_STATUS` version metadata (for a `412`
    /// `ETag`/`Location`).
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the metadata read fails.
    pub(in crate::service) async fn ehr_status_meta(
        &self,
        ehr_id: EhrId,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        self.latest_version_meta(ehr_id, Kind::EhrStatus).await
    }

    /// The current `EHR_STATUS` versioned-object id **and** its version
    /// metadata, resolved and read in ONE metadata-only statement (no node
    /// reassembly). The `PUT …/ehr_status` pre-read: it supplies both the
    /// full-`OBJECT_VERSION_ID` the `If-Match` compare needs (ITS-REST overview
    /// §Concurrency control) and the `vo_id` threaded into the update, so the
    /// write path resolves the versioned object exactly once. `None` when the
    /// EHR has no current `EHR_STATUS`.
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the metadata read fails.
    pub(in crate::service) async fn ehr_status_meta_with_vo(
        &self,
        ehr_id: EhrId,
    ) -> Result<Option<(VoId, ResourceMeta)>, ServiceError> {
        self.latest_version_meta_with_vo(ehr_id, Kind::EhrStatus)
            .await
    }

    /// Whether the EHR is modifiable (RM ehr `EHR_STATUS.is_modifiable`, 1..1
    /// Boolean, RM ehr master04 §EHR Active Status). Read from the promoted
    /// `ehr.is_modifiable` column, kept in lockstep with the current
    /// `EHR_STATUS` by [`Self::sync_ehr_subject`] and the import/archive-load
    /// [`Self::resync_promoted_columns`]; a missing EHR (`None`) is treated as
    /// modifiable so the guard never spuriously blocks.
    ///
    /// The column read is a storage seam
    /// ([`crate::storage::ehr_repo::ehr_is_modifiable`]; no openEHR spec
    /// governs the promoted column — our own storage design).
    async fn ehr_is_modifiable(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        Ok(
            crate::storage::ehr_repo::ehr_is_modifiable(&self.pool, ehr_id)
                .await?
                .unwrap_or(true),
        )
    }

    /// Refuses a write to EHR contents when the EHR is deactivated
    /// (`EHR_STATUS.is_modifiable = False`).
    ///
    /// RM ehr master04 §EHR Active Status scopes the flag to "everything other
    /// than the `EHR_STATUS` object", which "is always modifiable", so this
    /// guard applies to COMPOSITION, DIRECTORY and content writes only. The
    /// pooled read is the direct write paths' fast-fail convenience; the
    /// CONTRIBUTION engine evaluates the flag inside its commit transaction
    /// under a row lock
    /// ([`crate::versioning::change::ensure_content_writable_tx`]).
    ///
    /// NOTE: ITS-REST 1.1.0 enumerates no status code for a write to a
    /// non-modifiable EHR, so `409 Conflict` is used as the closest HTTP
    /// semantics (RFC 9110 §15.5.10, a conflict with the current state of the
    /// target resource).
    ///
    /// # Errors
    /// [`ServiceError::Conflict`] when the EHR is not modifiable;
    /// [`ServiceError::Database`] if the flag read fails.
    pub(in crate::service) async fn ensure_content_writable(
        &self,
        ehr_id: EhrId,
    ) -> Result<(), ServiceError> {
        if self.ehr_is_modifiable(ehr_id).await? {
            Ok(())
        } else {
            Err(crate::versioning::change::not_modifiable_error(ehr_id))
        }
    }

    /// Keep the EHR's promoted subject columns (`ehr.subject_id` /
    /// `subject_namespace`) in sync with the `EHR_STATUS` being committed
    /// (`subject.external_ref.id.value` + `.namespace`). The partial unique
    /// index `ehr_subject_uq` enforces **one EHR per subject** at the database
    /// (RM ehr master04 §EHR Status; ITS-REST `409_EHR.yaml`; CNF
    /// `create_ehr-two_ehrs_same_patient`) — a violation maps to
    /// [`ServiceError::Conflict`] (→ 409). A status without an `external_ref`
    /// (e.g. anonymous `PARTY_SELF`) clears the columns and never conflicts.
    ///
    /// This is the EHR-owned commit hook (RM common master06 §Committal
    /// and Audits), called inside the commit transaction of the EHR-create and
    /// `EHR_STATUS`-update paths and, for the CONTRIBUTION path, through the
    /// [`crate::versioning::CommitEnv::post_status_commit`] hook. The `UPDATE`
    /// stays inline here because it maps the subject-uniqueness constraint
    /// violation to a service-level [`ServiceError::Conflict`]; the
    /// `ehr.subject_*` columns are spec-silent index plumbing (our own design).
    ///
    /// # Errors
    /// [`ServiceError::Conflict`] when the subject already owns another EHR
    /// (`uq_ehr_subject`); [`ServiceError::Database`] on any other SQL failure.
    pub(in crate::service) async fn sync_ehr_subject(
        &self,
        tx: &mut PgConnection,
        ehr_id: EhrId,
        canonical: &Value,
    ) -> Result<(), ServiceError> {
        // The same promoted-column extraction the EHR-create path folds into
        // its initial INSERT (single-sourced so both promote identical values).
        // The is_modifiable sync (the content-write guard, RM ehr master04
        // §EHR Active Status) rides this same UPDATE — zero extra statements.
        let (subject_id, namespace, is_queryable, is_modifiable) = ehr_promoted_columns(canonical);
        sqlx::query(
            "UPDATE ehr SET subject_id = $2, subject_namespace = $3, is_queryable = $4, \
             is_modifiable = $5 WHERE id = $1",
        )
        .bind(ehr_id)
        .bind(subject_id)
        .bind(namespace)
        .bind(is_queryable)
        .bind(is_modifiable)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(db) = &e
                && db.constraint() == Some("uq_ehr_subject")
            {
                // NOTE: SM ehr_call_status_type.adoc declares
                // ehr_for_subject_already_exists for the one-EHR-per-subject rule.
                return ServiceError::sm(
                    CallStatusType::EhrForSubjectAlreadyExists,
                    format!(
                        "an EHR already exists for subject {}@{}",
                        subject_id.unwrap_or("?"),
                        namespace.unwrap_or("?"),
                    ),
                );
            }
            ServiceError::Database(e)
        })?;
        Ok(())
    }

    /// Re-promotes the `ehr` columns from the stored current `EHR_STATUS`, the
    /// seam for the paths that land `EHR_STATUS` versions without the service
    /// write hook: the EHR Extract import and the admin archive load.
    ///
    /// The current status root fragment is read on the caller's transaction
    /// ([`crate::storage::ehr_repo::current_status_root`]) and handed to
    /// [`Self::sync_ehr_subject`], so an imported or loaded EHR is visible to
    /// the subject lookup (SM `I_EHR_SERVICE.get_ehrs_for_subject`) and bound by
    /// the one-EHR-per-subject rule (RM ehr master04 §EHR Status) like any
    /// other. An EHR with no current `EHR_STATUS` keeps its column defaults.
    ///
    /// A subject already owned by another EHR is rejected before the UPDATE so
    /// the caller can report which subject clashed and which EHR holds it. The
    /// `uq_ehr_subject` index remains the backstop for a holder this pre-check
    /// cannot see, such as a concurrent writer or a row RLS hides, which
    /// surfaces as [`Self::sync_ehr_subject`]'s own conflict.
    ///
    /// # Errors
    /// [`ServiceError::Conflict`] when another EHR already owns the status's
    /// subject; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn resync_promoted_columns(
        &self,
        tx: &mut PgConnection,
        ehr_id: EhrId,
    ) -> Result<(), ServiceError> {
        let Some(status) = crate::storage::ehr_repo::current_status_root(tx, ehr_id).await? else {
            return Ok(());
        };
        let (subject_id, namespace, _, _) = ehr_promoted_columns(&status);
        if let (Some(subject_id), Some(namespace)) = (subject_id, namespace)
            && let Some(owner) =
                crate::storage::ehr_repo::ehr_id_by_subject(&mut *tx, subject_id, namespace).await?
            && owner != ehr_id
        {
            // NOTE: SM ehr_call_status_type.adoc declares
            // ehr_for_subject_already_exists for the one-EHR-per-subject rule.
            return Err(ServiceError::sm(
                CallStatusType::EhrForSubjectAlreadyExists,
                format!(
                    "EHR {ehr_id} names subject {subject_id}@{namespace}, which EHR {owner} \
                     already holds (one EHR per subject)"
                ),
            ));
        }
        self.sync_ehr_subject(tx, ehr_id, &status).await
    }
}

/// Extract the promoted `ehr`-table columns from an `EHR_STATUS` canonical
/// value: the subject `(id, namespace)` — only a COMPLETE `external_ref` pair
/// identifies a subject (RM ehr master04 §EHR Status; an anonymous
/// `PARTY_SELF` yields `(None, None)`) — and the two 1..1 Boolean status flags
/// `is_queryable` (RM ehr master04 §EHR Status) and `is_modifiable` (RM ehr
/// master04 §EHR Active Status), each defaulting to `true` (matching the
/// column default and the default `EHR_STATUS` when a raw path omits them).
/// Shared by the EHR-create path's folded INSERT and the update/contribution
/// [`FerroEhrService::sync_ehr_subject`] hook so both promote identical values.
/// No openEHR spec governs the promoted columns — our own storage design.
pub(in crate::service) fn ehr_promoted_columns(
    canonical: &Value,
) -> (Option<&str>, Option<&str>, bool, bool) {
    let subject_id = canonical
        .pointer("/subject/external_ref/id/value")
        .and_then(Value::as_str);
    let namespace = canonical
        .pointer("/subject/external_ref/namespace")
        .and_then(Value::as_str);
    let (subject_id, namespace) = match (subject_id, namespace) {
        (Some(id), Some(ns)) => (Some(id), Some(ns)),
        _ => (None, None),
    };
    let is_queryable = canonical
        .pointer("/is_queryable")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let is_modifiable = canonical
        .pointer("/is_modifiable")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    (subject_id, namespace, is_queryable, is_modifiable)
}

// ── The SM I_EHR_STATUS call surface ──────────────────────────────────────────

impl FerroEhrService {
    /// SM `I_EHR_STATUS.has_ehr_status_version` — whether the EHR's
    /// `EHR_STATUS` versioned object is the one named.
    ///
    /// # Errors
    /// [`SmError`] if the current-object resolution fails.
    pub async fn has_ehr_status_version(
        &self,
        an_ehr_id: EhrId,
        a_version_uid: VoId,
    ) -> Result<bool, SmError> {
        // An EHR holds exactly one EHR_STATUS versioned object; the version
        // exists iff that object's `vo_id` matches.
        Ok(self
            .current_vo(an_ehr_id, Kind::EhrStatus)
            .await?
            .is_some_and(|(vo, _)| vo == a_version_uid))
    }

    /// SM `I_EHR_STATUS.get_ehr_status` — the current `EHR_STATUS` (bare, with
    /// its `uid`).
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` (404-equivalent) or
    /// a read fails.
    pub async fn get_ehr_status(&self, an_ehr_id: EhrId) -> Result<Value, SmError> {
        Ok(self.status_at(an_ehr_id, None).await?.body)
    }

    /// SM `I_EHR_STATUS.get_ehr_status_at_time` — the `EHR_STATUS` current at
    /// `a_time` (time-travel), or the current one when `a_time` is `None`.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `a_time` (400-equivalent), a missing status
    /// at that instant (404-equivalent), or a read failure.
    pub async fn get_ehr_status_at_time(
        &self,
        an_ehr_id: EhrId,
        a_time: Option<String>,
    ) -> Result<Value, SmError> {
        Ok(self
            .ehr_status_at_time_response(an_ehr_id, a_time)
            .await?
            .body)
    }

    /// The bare `EHR_STATUS` at a specific version (not an
    /// `ORIGINAL_VERSION`). `a_version` is the `VERSION_TREE_ID` lexical form.
    ///
    /// # Errors
    /// [`SmError`] for a malformed version id, an unknown version
    /// (404-equivalent), or a read failure.
    pub async fn get_ehr_status_at_version(
        &self,
        an_ehr_id: EhrId,
        a_version_uid: VoId,
        a_version: &str,
    ) -> Result<Value, SmError> {
        Ok(self
            .ehr_status_at_version_response(an_ehr_id, a_version_uid, a_version)
            .await?
            .body)
    }

    /// SM `I_EHR_STATUS.get_versioned_ehr_status` — the `VERSIONED_EHR_STATUS`
    /// container object.
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` (404-equivalent) or
    /// a read fails.
    pub async fn get_versioned_ehr_status(&self, an_ehr_id: EhrId) -> Result<Value, SmError> {
        Ok(self.versioned_status(an_ehr_id).await?.body)
    }

    /// SM `I_EHR_STATUS.set_ehr_queryable` — commit a new `EHR_STATUS` version
    /// with `is_queryable = true`, returning the new version uid.
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` or the commit
    /// fails.
    pub async fn set_ehr_queryable(&self, an_ehr_id: EhrId) -> Result<String, SmError> {
        Ok(self
            .status_mutate(an_ehr_id, |s| {
                s.is_queryable = true;
                Ok(())
            })
            .await?
            .version_uid())
    }

    /// SM `I_EHR_STATUS.clear_ehr_queryable` — commit a new `EHR_STATUS`
    /// version with `is_queryable = false`, returning the new version uid.
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` or the commit
    /// fails.
    pub async fn clear_ehr_queryable(&self, an_ehr_id: EhrId) -> Result<String, SmError> {
        Ok(self
            .status_mutate(an_ehr_id, |s| {
                s.is_queryable = false;
                Ok(())
            })
            .await?
            .version_uid())
    }

    /// SM `I_EHR_STATUS.set_ehr_modifiable` — commit a new `EHR_STATUS` version
    /// with `is_modifiable = true` (reactivating the EHR's contents), returning
    /// the new version uid.
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` or the commit
    /// fails.
    pub async fn set_ehr_modifiable(&self, an_ehr_id: EhrId) -> Result<String, SmError> {
        Ok(self
            .status_mutate(an_ehr_id, |s| {
                s.is_modifiable = true;
                Ok(())
            })
            .await?
            .version_uid())
    }

    /// SM `I_EHR_STATUS.clear_ehr_modifiable` — commit a new `EHR_STATUS`
    /// version with `is_modifiable = false` (deactivating the EHR's contents),
    /// returning the new version uid.
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` or the commit
    /// fails.
    pub async fn clear_ehr_modifiable(&self, an_ehr_id: EhrId) -> Result<String, SmError> {
        // Committable on the EHR it disables: the write guard scopes to EHR
        // *contents*, never to EHR_STATUS (RM ehr master04 §EHR Active Status).
        Ok(self
            .status_mutate(an_ehr_id, |s| {
                s.is_modifiable = false;
                Ok(())
            })
            .await?
            .version_uid())
    }

    /// SM `I_EHR_STATUS.update_other_details` — commit a new `EHR_STATUS`
    /// version with the given `other_details`, returning the new version uid.
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS`, the mutated status
    /// fails validation (a non-`ITEM_STRUCTURE` `other_details` is
    /// 422-equivalent), or the commit fails.
    pub async fn update_other_details(
        &self,
        an_ehr_id: EhrId,
        a_details: Value,
    ) -> Result<String, SmError> {
        // Boxed: the typed EHR_STATUS envelope makes the mutate future large
        // enough to matter on the stack (clippy `large_futures`).
        Ok(Box::pin(self.status_mutate(an_ehr_id, move |s| {
            // The client-supplied details decode through the strict reader, so
            // a non-ITEM_STRUCTURE refuses here, path-named.
            s.other_details = Some(openehr_its::json::from_canonical_value(&a_details).map_err(
                |e| {
                    ServiceError::content_invalid(
                        crate::service::error::Violation::new("is not a canonical ITEM_STRUCTURE")
                            .with_path("EHR_STATUS/other_details")
                            .with_decode_failure(&e),
                    )
                },
            )?);
            Ok(())
        }))
        .await?
        .version_uid())
    }

    /// Replace the whole `EHR_STATUS` in one commit (`PUT …/ehr_status`),
    /// returning the new version uid.
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` (404-equivalent),
    /// the `preceding_version_uid` mismatches the current latest
    /// (412-equivalent), the body fails structural validation
    /// (422-equivalent), or the commit fails.
    pub async fn replace_ehr_status(
        &self,
        an_ehr_id: EhrId,
        a_status: UpdateVersion<EhrStatus>,
    ) -> Result<String, SmError> {
        Ok(Box::pin(self.replace_ehr_status_meta(an_ehr_id, a_status))
            .await?
            .uid)
    }

    /// The `REVISION_HISTORY` of the EHR's `EHR_STATUS`. SM defines no
    /// revision-history operation (the abstract counterpart is RM common
    /// `versioned_object.adoc` `revision_history()`); the wire is the
    /// ITS-REST `revision_history` route.
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` (404-equivalent) or
    /// a read fails.
    pub async fn ehr_status_revision_history(&self, an_ehr_id: EhrId) -> Result<Value, SmError> {
        Ok(self.status_revision_history(an_ehr_id).await?.body)
    }

    /// The VERSION envelope of the EHR's `EHR_STATUS` extant at `a_time`, or
    /// the latest when `a_time` is `None`
    /// (`GET …/versioned_ehr_status/version`).
    ///
    /// # Errors
    /// [`SmError`] for a malformed `a_time` (400-equivalent), a missing version
    /// at that instant (404-equivalent), or a read failure.
    pub async fn ehr_status_version_at_time(
        &self,
        an_ehr_id: EhrId,
        a_time: Option<String>,
    ) -> Result<Value, SmError> {
        let at = a_time.as_deref().map(parse_at_time).transpose()?;
        Ok(self.status_version_at_time(an_ehr_id, at).await?.body)
    }

    /// The VERSION envelope of an `EHR_STATUS` at a specific version
    /// (`GET …/versioned_ehr_status/version/{version_uid}`).
    ///
    /// # Errors
    /// [`SmError`] for a malformed version id, an unknown version
    /// (404-equivalent), or a read failure.
    pub async fn ehr_status_version_envelope(
        &self,
        an_ehr_id: EhrId,
        a_version_uid: VoId,
        a_version: &str,
    ) -> Result<Value, SmError> {
        let tree = parse_tree_id(a_version)?;
        Ok(self.status_version(an_ehr_id, a_version_uid, tree).await?)
    }
}

// ── ITS-REST read/write-response adapter (adapter-support extension) ──────────
//
// The SM `I_EHR_STATUS` calls return the bare `EHR_STATUS` (or its new version
// uid), neither of which carries the commit instant `Requests_and_responses.md`
// §"`ETag` and Last-Modified" wants, so these siblings return the same result
// plus its [`ResourceMeta`] with no second read (no openEHR spec governs the
// envelope — our own design).

impl FerroEhrService {
    /// [`Self::get_ehr_status_at_time`] with the version metadata the wire's
    /// `ETag`/`Last-Modified` need.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `a_time` (400-equivalent), a missing status
    /// at that instant (404-equivalent), or a read failure.
    pub async fn ehr_status_at_time_response(
        &self,
        an_ehr_id: EhrId,
        a_time: Option<String>,
    ) -> Result<ServiceResponse, SmError> {
        let at = a_time.as_deref().map(parse_at_time).transpose()?;
        Ok(self.status_at(an_ehr_id, at).await?)
    }

    /// [`Self::get_ehr_status_at_version`] with the version metadata the
    /// wire's `ETag`/`Last-Modified` need. The body is the **bare**
    /// `EHR_STATUS`, not an `ORIGINAL_VERSION`.
    ///
    /// # Errors
    /// [`SmError`] for a malformed version id, an unknown version
    /// (404-equivalent), or a read failure.
    pub async fn ehr_status_at_version_response(
        &self,
        an_ehr_id: EhrId,
        a_version_uid: VoId,
        a_version: &str,
    ) -> Result<ServiceResponse, SmError> {
        let tree = parse_tree_id(a_version)?;
        Ok(self
            .status_by_version(an_ehr_id, a_version_uid, tree)
            .await?)
    }

    /// [`Self::get_versioned_ehr_status`] with the container metadata the
    /// wire's `ETag`/`Last-Modified` need: the container uid identity plus the
    /// newest held version's commit instant (ITS-REST overview
    /// `Requests_and_responses.md` §"`ETag` and Last-Modified" — both headers
    /// SHOULD accompany a `VERSIONED_OBJECT` response).
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` (404-equivalent) or
    /// a read fails.
    pub async fn versioned_ehr_status_response(
        &self,
        an_ehr_id: EhrId,
    ) -> Result<ServiceResponse, SmError> {
        Ok(self.versioned_status(an_ehr_id).await?)
    }

    /// [`Self::ehr_status_revision_history`] with the container metadata the
    /// wire's `ETag`/`Last-Modified` need (container uid + newest commit
    /// instant — same derivation as
    /// [`Self::versioned_ehr_status_response`]).
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` (404-equivalent) or
    /// a read fails.
    pub async fn ehr_status_revision_history_response(
        &self,
        an_ehr_id: EhrId,
    ) -> Result<ServiceResponse, SmError> {
        Ok(self.status_revision_history(an_ehr_id).await?)
    }

    /// [`Self::replace_ehr_status`] returning the committed version's
    /// [`ResourceMeta`] (uid + commit instant) instead of the bare uid.
    ///
    /// # Errors
    /// [`SmError`] when the EHR has no current `EHR_STATUS` (404-equivalent),
    /// the `preceding_version_uid` mismatches the current latest
    /// (412-equivalent), the body fails structural validation
    /// (422-equivalent), or the commit fails.
    pub async fn replace_ehr_status_meta(
        &self,
        an_ehr_id: EhrId,
        a_status: UpdateVersion<EhrStatus>,
    ) -> Result<ResourceMeta, SmError> {
        // The `If-Match` meta pre-read also yields the `vo_id`, threaded into
        // the write so the versioned object is resolved once (no second
        // `current_vo`). No current EHR_STATUS ⇒ NotFound (404).
        // NOTE: the ITS-REST wire replaces the whole EHR_STATUS in one PUT — the
        // aggregate of the five discrete SM mutators (formal equivalence,
        // `master02-overview.adoc` §Interface Calls); a mismatch → 412.
        let Some((vo_id, latest)) = self.ehr_status_meta_with_vo(an_ehr_id).await? else {
            return Err(ServiceError::sm(
                CallStatusType::EhrIdDoesNotExist,
                format!("EHR_STATUS for EHR {an_ehr_id}"),
            )
            .into());
        };
        ensure_if_match(a_status.preceding_version_uid.as_ref(), Some(&latest))?;
        let if_match = a_status
            .preceding_version_uid
            .as_ref()
            .map(|o| o.value().to_owned())
            .unwrap_or_default();
        let committed = self
            .commit_status(an_ehr_id, vo_id, a_status, &if_match)
            .await?;
        Ok(self.version_meta(
            an_ehr_id,
            committed.vo_id,
            &committed.creating_system_id,
            committed.tree,
            committed.time_committed,
        ))
    }
}
