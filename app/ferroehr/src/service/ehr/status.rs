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
use crate::service::write::{Materials, Resolved, first_attempt};
use crate::storage::ehr_repo::{self, PromotedColumns};
use crate::storage::error::StorageError;
use crate::storage::version_repo::gate::{self, Ask, Shape};
use crate::storage::version_repo::guarded::{Guard, StatusSync};
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::body::Body;
use crate::versioning::change::precondition::{ensure_preceding, tip_uid};
use crate::versioning::change::{Change, Write};

impl FerroEhrService {
    /// Keeps the EHR's promoted subject columns in step with the `EHR_STATUS`
    /// being committed, inside the commit transaction of the EHR create, the
    /// `EHR_STATUS` update and the CONTRIBUTION path.
    ///
    /// The partial unique index `ehr_subject_uq` holds one EHR per subject (RM
    /// ehr master04 §EHR Status; ITS-REST `409_EHR.yaml`; CNF
    /// `create_ehr-two_ehrs_same_patient`). A status without an `external_ref`
    /// clears the columns. The columns are our own design.
    ///
    /// # Errors
    /// [`ServiceError::Conflict`] when the subject already owns another EHR
    /// (`uq_ehr_subject`); [`ServiceError::Database`] on any other SQL failure.
    pub(crate) async fn sync_ehr_subject(
        &self,
        tx: &mut PgConnection,
        ehr_id: EhrId,
        promoted: &SubjectColumns,
    ) -> Result<(), ServiceError> {
        // The same promoted-column extraction the EHR-create path folds into
        // its initial INSERT (single-sourced so both promote identical values).
        // The is_modifiable sync (the content-write guard, RM ehr master04
        // §EHR Active Status) rides this same UPDATE — zero extra statements.
        let digest = self.care_subject_digest(promoted.subject().as_ref());
        ehr_repo::update_promoted_columns(tx, ehr_id, promoted.columns(digest.as_deref()))
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

    /// Re-promotes the `ehr` columns from the stored current `EHR_STATUS`, for
    /// the EHR Extract import and the admin archive load, which bypass the write
    /// hook.
    ///
    /// The current status root is read on the caller's transaction
    /// ([`ehr_repo::current_status_root`]) and handed to
    /// [`Self::sync_ehr_subject`]. A subject another EHR owns is refused first,
    /// naming both; `uq_ehr_subject` remains the backstop for a holder this check
    /// cannot see. An EHR with no current `EHR_STATUS` keeps its defaults.
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
        let promoted = SubjectColumns::of_canonical(&status);
        if let Some((subject_id, namespace)) = promoted.subject.as_ref()
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
        self.sync_ehr_subject(tx, ehr_id, &promoted).await
    }
}

/// The promoted `ehr`-table columns of an `EHR_STATUS`: the subject
/// `(id, namespace)` — only a COMPLETE `external_ref` pair identifies a subject
/// (RM ehr master04 §EHR Status; an anonymous `PARTY_SELF` yields none) — and
/// the two 1..1 Boolean status flags `is_queryable` (RM ehr master04 §EHR
/// Status) and `is_modifiable` (RM ehr master04 §EHR Active Status), each
/// defaulting to `true` (matching the column default and the default
/// `EHR_STATUS` when a raw path omits them). Shared by the EHR-create path's
/// folded INSERT and the update/contribution
/// [`FerroEhrService::sync_ehr_subject`] hook so both promote identical values.
/// No openEHR spec governs the promoted columns — our own storage design.
#[derive(Debug, Clone)]
pub(crate) struct SubjectColumns {
    /// The subject's `(id, namespace)`.
    pub(crate) subject: Option<(String, String)>,
    /// `EHR_STATUS.is_queryable`.
    pub(crate) is_queryable: bool,
    /// `EHR_STATUS.is_modifiable`.
    pub(crate) is_modifiable: bool,
}

impl SubjectColumns {
    /// The columns of a typed `EHR_STATUS`.
    pub(crate) fn of_typed(status: &EhrStatus) -> Self {
        let subject = status.subject.external_ref.as_deref().map(|external| {
            let id = crate::validation::versioned::object_id_value(&external.id).to_owned();
            (id, external.namespace.clone())
        });
        Self {
            subject,
            is_queryable: status.is_queryable,
            is_modifiable: status.is_modifiable,
        }
    }

    /// The columns of an `EHR_STATUS` canonical value.
    pub(crate) fn of_canonical(canonical: &Value) -> Self {
        let subject_id = canonical
            .pointer("/subject/external_ref/id/value")
            .and_then(Value::as_str);
        let namespace = canonical
            .pointer("/subject/external_ref/namespace")
            .and_then(Value::as_str);
        let flag = |pointer: &str| canonical.pointer(pointer).and_then(Value::as_bool);
        Self {
            subject: subject_id
                .zip(namespace)
                .map(|(id, ns)| (id.to_owned(), ns.to_owned())),
            is_queryable: flag("/is_queryable").unwrap_or(true),
            is_modifiable: flag("/is_modifiable").unwrap_or(true),
        }
    }

    /// The columns of an `EHR_STATUS` commit body; `None` for another kind.
    pub(crate) fn of_body(body: &Body) -> Option<Self> {
        match body {
            Body::EhrStatus(status) => Some(Self::of_typed(status)),
            Body::Canonical(canonical) => Some(Self::of_canonical(canonical)),
            _ => None,
        }
    }

    /// The subject the access projection keys care relationships by.
    pub(crate) fn subject(&self) -> Option<crate::storage::access_projection::EhrSubject> {
        let (id, namespace) = self.subject.clone().unzip();
        crate::storage::access_projection::EhrSubject::stored(namespace, id)
    }

    /// The columns as an `EHR_STATUS` commit's guarded statement writes them,
    /// with the subject digest `digest` (#6257).
    pub(crate) fn sync(&self, digest: Option<Vec<u8>>) -> StatusSync {
        let (subject_id, subject_namespace) = self.subject.clone().unzip();
        StatusSync {
            subject_id,
            subject_namespace,
            is_queryable: self.is_queryable,
            is_modifiable: self.is_modifiable,
            subject_digest: digest,
        }
    }

    /// The columns, with the subject digest `digest`.
    pub(crate) fn columns<'a>(&'a self, digest: Option<&'a [u8]>) -> PromotedColumns<'a> {
        PromotedColumns {
            subject_id: self.subject.as_ref().map(|(id, _)| id.as_str()),
            subject_namespace: self.subject.as_ref().map(|(_, ns)| ns.as_str()),
            is_queryable: self.is_queryable,
            is_modifiable: self.is_modifiable,
            subject_digest: digest,
        }
    }
}

/// The EHR's `EHR_STATUS` as its gate read found it, with the uid of its
/// tip.
///
/// # Errors
/// [`ServiceError::NotFound`] when the EHR or its `EHR_STATUS` does not exist.
fn status_target(
    gate: &mut gate::Gate,
    ehr_id: EhrId,
) -> Result<(gate::Target, String), ServiceError> {
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
        ServiceError::not_found(Missing::Ehr, format!("EHR_STATUS for EHR {ehr_id}"))
    })
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

        // Request resolution and validation run before any connection is held, and
        // their refusals surface after the `If-Match` check, in pre-check order.
        let resolved = self.resolve_write(&request, change_type::MODIFICATION, "EHR_STATUS update");
        let preceding = request.preceding;
        let promoted = SubjectColumns::of_typed(&request.data);
        let digest = self.care_subject_digest(promoted.subject().as_ref());
        let mut content = Body::from(request.data);
        // A valid body's large media go to the blob store before a connection is held.
        let validated = match &resolved {
            Ok(parts) => Some(
                self.accept_body(Kind::EhrStatus, &mut content, parts.incomplete)
                    .await?,
            ),
            Err(_) => None,
        };
        // A status a later attempt judges again commits on the new tip only when it
        // names no preceding version, so only such a one keeps a copy.
        let (resolved, unresolved) = resolved.map_or_else(
            |error| (None, Some(error)),
            |resolved| (Some(resolved), None),
        );
        let materials = Materials::new((content, resolved), preceding.is_none());
        let unresolved = std::sync::Mutex::new(unresolved);
        let validated = std::sync::Mutex::new(validated);
        let sync = promoted.sync(digest);
        let (materials, unresolved, validated) = (&materials, &unresolved, &validated);
        let (preceding, sync) = (&preceding, &sync);
        let committed = self
            .commit_gated(&self.pool, move || {
                async move |conn| {
                    // The EHR, its EHR_STATUS found by kind and its tip in ONE statement
                    // (#6257); the guarded commit requires them unchanged.
                    let mut gate = gate::read(
                        conn,
                        Ask {
                            ehr_id: an_ehr_id,
                            targets: &[],
                            shape: Shape::Status,
                            persistent: None,
                            addressed: None,
                            items: &[],
                        },
                    )
                    .await?;
                    self.ensure_gate_policy_held().await?;
                    let (target, latest) = status_target(&mut gate, an_ehr_id)?;
                    let vo_id = target.vo_id;
                    ensure_preceding(&latest, preceding.as_ref())?;
                    if let Some(error) = first_attempt(unresolved) {
                        return Err(error);
                    }
                    let (
                        content,
                        Some(Resolved {
                            audit, envelope, ..
                        }),
                    ) = materials.take()?
                    else {
                        return Err(ServiceError::exception(
                            "an unresolved EHR_STATUS update reached its write",
                        ));
                    };
                    if let Some(validation) = first_attempt(validated) {
                        validation?;
                    }
                    Ok(Write::new(
                        Some(an_ehr_id),
                        Change::modify(vo_id, Kind::EhrStatus, content, None, None, envelope),
                        audit,
                    )
                    .guarded(Guard {
                        status: Some(Box::new(sync.clone())),
                        ..super::composition::guard_of(&gate, target.head)
                    })
                    .placed(target.placement)
                    .gated(gate.restriction.clone())
                    .object_marked(target.restricted))
                }
            })
            .await
            .map_err(ServiceError::flattened)?;
        // NOTE: our own design (#5589, #5879, #6238), the copy is revised by the subject alone
        // (`access_revision_of`), so only a moved subject drops it, with no connection held.
        if committed.subject_moved {
            self.invalidate_ehr_access(an_ehr_id).await;
        }
        Ok(self.version_meta(
            an_ehr_id,
            committed.vo_id,
            &committed.creating_system_id,
            committed.tree,
            committed.time_committed,
        ))
    }
}
