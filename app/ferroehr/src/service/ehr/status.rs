// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR_STATUS` operations: the five discrete mutators, plus two EHR-owned
//! commit hooks: the `is_modifiable` content-write guard and the promoted-subject
//! sync. The reads live in [`super::object`], shared with `EHR_ACCESS`.
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

use crate::error::refusal::{Clash, Missing, Refusal};
use crate::ids::EhrId;
use crate::service::response::ResourceMeta;
use openehr_rm::prelude::EhrStatus;
use serde_json::Value;
use sqlx::PgConnection;

use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::service::version_update::WriteRequest;
use crate::service::write::{Mode, Resolved};
use crate::storage::ehr_repo::{self, PromotedColumns};
use crate::storage::error::StorageError;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::precondition::{ensure_preceding, lock_current, tip_uid};
use crate::versioning::change::{Change, Write};

impl FerroEhrService {
    /// Whether the EHR is modifiable (RM ehr `EHR_STATUS.is_modifiable`, 1..1
    /// Boolean, RM ehr master04 §EHR Active Status). Read from the promoted
    /// `ehr.is_modifiable` column, kept in lockstep with the current
    /// `EHR_STATUS` by [`Self::sync_ehr_subject`] and the import/archive-load
    /// [`Self::resync_promoted_columns`]; a missing EHR (`None`) is treated as
    /// modifiable so the guard never spuriously blocks.
    ///
    /// The column read is a storage seam
    /// ([`ehr_repo::ehr_is_modifiable`]; no openEHR spec
    /// governs the promoted column — our own storage design).
    async fn ehr_is_modifiable(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        Ok(ehr_repo::ehr_is_modifiable(&self.pool, ehr_id)
            .await?
            .unwrap_or(true))
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
    /// ([`crate::versioning::change::precondition::ensure_content_writable_tx`]).
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
            Err(crate::versioning::change::precondition::not_modifiable_error(ehr_id))
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
    /// `EHR_STATUS`-update paths and, for the CONTRIBUTION path, from
    /// [`crate::service::contribution`]. The `UPDATE`
    /// stays inline here because it maps the subject-uniqueness constraint
    /// violation to a service-level [`ServiceError::Conflict`]; the
    /// `ehr.subject_*` columns are spec-silent index plumbing (our own design).
    ///
    /// # Errors
    /// [`ServiceError::Conflict`] when the subject already owns another EHR
    /// (`uq_ehr_subject`); [`ServiceError::Database`] on any other SQL failure.
    pub(crate) async fn sync_ehr_subject(
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
        let columns = PromotedColumns {
            subject_id,
            subject_namespace: namespace,
            is_queryable,
            is_modifiable,
        };
        ehr_repo::update_promoted_columns(tx, ehr_id, columns)
            .await
            .map_err(|e| match e {
                // NOTE: ITS-REST EHR API `409_EHR` refuses a second EHR for one subject
                // id and namespace pair; the status token is our own.
                StorageError::SubjectInUse(subject, namespace) => {
                    ServiceError::Conflict(Refusal::new(
                        Clash::EhrForSubject,
                        format!("an EHR already exists for subject {subject}@{namespace}"),
                    ))
                }
                other => ServiceError::Storage(other),
            })
    }

    /// Re-promotes the `ehr` columns from the stored current `EHR_STATUS`, the
    /// seam for the paths that land `EHR_STATUS` versions without the service
    /// write hook: the EHR Extract import and the admin archive load.
    ///
    /// The current status root fragment is read on the caller's transaction
    /// ([`ehr_repo::current_status_root`]) and handed to
    /// [`Self::sync_ehr_subject`], so an imported or loaded EHR is visible to
    /// the subject lookup (`get_ehrs_for_subject`) and bound by
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
        let Some(status) = ehr_repo::current_status_root(tx, ehr_id).await? else {
            return Ok(());
        };
        let (subject_id, namespace, _, _) = ehr_promoted_columns(&status);
        if let (Some(subject_id), Some(namespace)) = (subject_id, namespace)
            && let Some(owner) =
                ehr_repo::ehr_id_by_subject(&mut *tx, subject_id, namespace).await?
            && owner != ehr_id
        {
            // NOTE: ITS-REST EHR API `409_EHR` refuses a second EHR for one subject
            // id and namespace pair; the status token is our own.
            return Err(ServiceError::Conflict(Refusal::new(
                Clash::EhrForSubject,
                format!(
                    "EHR {ehr_id} names subject {subject_id}@{namespace}, which EHR {owner} \
                     already holds (one EHR per subject)"
                ),
            )));
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

impl FerroEhrService {
    /// Replace the whole `EHR_STATUS` in one commit (`PUT …/ehr_status`),
    /// returning the committed version's [`ResourceMeta`] (uid + commit
    /// instant).
    ///
    /// # Errors
    /// [`ServiceError`] when the EHR has no current `EHR_STATUS` (404-equivalent),
    /// the `preceding_version_uid` mismatches the current latest
    /// (412-equivalent), the body fails structural validation
    /// (422-equivalent), or the commit fails.
    pub async fn replace_ehr_status(
        &self,
        an_ehr_id: EhrId,
        request: WriteRequest<EhrStatus>,
    ) -> Result<ResourceMeta, ServiceError> {
        // NOTE: the ITS-REST wire replaces the whole EHR_STATUS in one PUT; a
        // mismatch → 412.
        let Some((vo_id, _)) = self.current_vo(an_ehr_id, Kind::EhrStatus).await? else {
            return Err(ServiceError::not_found(
                Missing::Ehr,
                format!("EHR_STATUS for EHR {an_ehr_id}"),
            ));
        };
        // Request resolution and validation run before the transaction, and
        // their refusals surface after the `If-Match` check, in pre-check order.
        let resolved = self.resolve_write(&request, change_type::MODIFICATION, "EHR_STATUS update");
        let body = openehr_its::json::to_canonical_value(&request.data);
        let validated = match &resolved {
            Ok(parts) => Some(
                self.validate_for_commit(Kind::EhrStatus, &body, parts.incomplete)
                    .await,
            ),
            Err(_) => None,
        };
        let preceding = request.preceding;
        let content = body.clone();
        // Keep the promoted subject columns in sync (the subject may have
        // changed); the is_queryable promotion rides this same UPDATE.
        let committed = self
            .commit_one(
                &self.pool,
                Mode::SyncSubject(an_ehr_id, &body),
                async move |tx| {
                    let placement =
                        lock_current(tx, Some(an_ehr_id), vo_id, Kind::EhrStatus).await?;
                    if let Some(tip) = placement.tip.as_ref() {
                        ensure_preceding(&tip_uid(vo_id, tip), preceding.as_ref())?;
                    }
                    let Resolved {
                        audit, envelope, ..
                    } = resolved?;
                    if let Some(validation) = validated {
                        validation?;
                    }
                    Ok(Write::new(
                        Some(an_ehr_id),
                        Change::modify(vo_id, Kind::EhrStatus, content, None, None, envelope),
                        audit,
                    )
                    .placed(placement))
                },
            )
            .await
            .map_err(ServiceError::flattened)?;
        Ok(self.version_meta(
            an_ehr_id,
            committed.vo_id,
            &committed.creating_system_id,
            committed.tree,
            committed.time_committed,
        ))
    }
}
