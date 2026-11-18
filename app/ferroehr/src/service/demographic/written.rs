// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the direct party and relationship writes share: the checks before
//! the transaction, the body they store and the one they serve, and the
//! answer built from the commit (#6003). No openEHR spec governs how a write
//! is staged: our own design.

use std::future::Future;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::party::CurrentParty;
use crate::service::demographic::precondition::Precondition;
use crate::service::demographic::relationship::CurrentRelationship;
use crate::service::demographic::support;
use crate::service::demographic::template::demographic_rules;
use crate::service::demographic::types::PartyKind;
use crate::service::demographic::validate::Container;
use crate::service::response::{RawServiceResponse, ReadBody, ServiceResponse};
use crate::service::version_update::WriteRequest;
use crate::service::write::{Mode, Resolved};
use crate::storage::version_repo::placement::Placement;
use crate::system_log::event::AuditEvent;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::body::Body;
use crate::versioning::change::Write;
use crate::versioning::change::precondition::lock_current;
use crate::versioning::change::{Change, Committed, CommittedContribution};

impl FerroEhrService {
    /// The checks of a direct demographic write of `request`, of `kind`, as
    /// part of `container`: the rules of the body, the committal merge under
    /// `change` (the `audit_change_type` code and the audit's description),
    /// then the template gate under the merged lifecycle state.
    ///
    /// # Errors
    /// The first refusal, in that order.
    pub(super) async fn demographic_write_checks(
        &self,
        kind: Kind,
        request: &WriteRequest<Body>,
        container: Option<Container>,
        change: (&str, &str),
    ) -> Result<Resolved, ServiceError> {
        let rendered = demographic_rules(kind, &request.data, container)?;
        let resolved = self.resolve_write(request, change.0, change.1)?;
        self.demographic_template_gate(kind, &request.data, rendered.as_ref(), resolved.incomplete)
            .await?;
        Ok(resolved)
    }

    /// The current version of the party `vo_id` of `kind` a write addresses.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when there is none; the storage error of the
    /// read.
    pub(super) async fn existing_party(
        &self,
        kind: PartyKind,
        vo_id: VoId,
    ) -> Result<CurrentParty, ServiceError> {
        self.party_current(kind, vo_id).await?.ok_or_else(|| {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("{} {vo_id}", kind.rm_type()),
            )
        })
    }

    /// The current version of the relationship `vo_id` a write addresses.
    ///
    /// # Errors
    /// As [`Self::existing_party`].
    pub(super) async fn existing_relationship(
        &self,
        vo_id: VoId,
    ) -> Result<CurrentRelationship, ServiceError> {
        self.relationship_current(vo_id).await?.ok_or_else(|| {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("PARTY_RELATIONSHIP {vo_id}"),
            )
        })
    }

    /// Commits the party versions `changes` as one CONTRIBUTION under
    /// `audit`, their large media offloaded first, with the access records
    /// `records` beside the write's own.
    ///
    /// # Errors
    /// The offload's, the commit engine's and the access record's refusals.
    pub(super) async fn commit_party_set(
        &self,
        audit: &AuditInput,
        mut changes: Vec<(AuditInput, Change)>,
        records: Vec<AuditEvent>,
    ) -> Result<CommittedContribution, ServiceError> {
        // The set's large media go to the blob store before a connection is held.
        for (_, change) in &mut changes {
            self.externalize_change(change).await?;
        }
        let mut conn = self.recorded_conn(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        let committed = crate::versioning::change::commit_contribution(
            &mut tx,
            None,
            None,
            audit,
            changes,
            Vec::new(),
            &self.signing_ctx(),
        )
        .await?;
        let facts = crate::system_log::recorder::WriteFacts::of_parties(&committed);
        self.commit_recorded(tx, &facts, records).await?;
        Ok(committed)
    }

    /// The version of the demographic object `vo_id` of `kind` a read
    /// addresses, with its body kept as the stored text where nothing needs it
    /// parsed; a deleted version reads `Null` (→ `204`).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] for no such version
    /// ([`support::missing`]); the storage and identifier faults of the read.
    pub(super) async fn demographic_read_raw(
        &self,
        kind: Kind,
        vo_id: VoId,
        version: Option<crate::versioning::object_version_id::TreeId>,
        at: Option<jiff::Timestamp>,
    ) -> Result<RawServiceResponse, ServiceError> {
        let raw = support::load_ehrless_raw(
            &self.demographic_pool,
            self.spec_profile,
            vo_id,
            version,
            at,
            kind,
        )
        .await?
        .ok_or_else(|| support::missing(kind.as_str(), vo_id, version.is_some() || at.is_some()))?;
        if raw.read.deleted() {
            return Ok(RawServiceResponse {
                body: ReadBody::Value(serde_json::Value::Null),
                meta: None,
            });
        }
        Ok(support::raw_version_response(vo_id, raw)?)
    }

    /// Commits the new version `body` of the demographic object `vo_id`, of
    /// `kind`, once its tip meets `precondition` and `checked`, the checks
    /// made before the transaction, passed.
    ///
    /// # Errors
    /// The precondition's refusal, `checked`'s, and the commit's.
    pub(super) async fn commit_demographic_update(
        &self,
        (kind, vo_id): (Kind, VoId),
        precondition: Precondition,
        checked: Result<Resolved, ServiceError>,
        body: Body,
    ) -> Result<Committed, ServiceError> {
        self.commit_one(&self.demographic_pool, Mode::Transaction, async move |tx| {
            let placement = locked_tip(tx, kind, vo_id, &precondition).await?;
            if let Some(tip) = placement.tip.as_ref() {
                precondition.check_tree(tip)?;
            }
            let Resolved {
                audit, envelope, ..
            } = checked?;
            Ok(Write::new(
                None,
                Change::modify(vo_id, kind, body, None, None, envelope),
                audit,
            )
            .placed(placement))
        })
        .await
    }

    /// The body a direct demographic write stores, its large media offloaded
    /// when `offload` holds, and the one it serves: the body in hand, unless
    /// the commit rewrites it ([`support::commit_rewrites_body`]), when the
    /// stored form is read back instead.
    ///
    /// # Errors
    /// The offload's [`ServiceError`].
    pub(super) async fn demographic_stored(
        &self,
        mut body: Body,
        offload: bool,
    ) -> Result<(Body, Option<Body>), ServiceError> {
        let served = (!support::commit_rewrites_body(self)).then(|| body.clone());
        if offload {
            self.externalize_body(&mut body).await?;
        }
        Ok((body, served))
    }
}

/// The placement of the next version of the demographic object `vo_id`, of
/// `kind`, its tip locked on `tx` and held to the full version id
/// `precondition` names.
///
/// # Errors
/// The lock's storage error, and `precondition`'s refusal.
pub(super) async fn locked_tip(
    tx: &mut sqlx::PgConnection,
    kind: Kind,
    vo_id: VoId,
    precondition: &Precondition,
) -> Result<Placement, ServiceError> {
    let placement = lock_current(tx, None, vo_id, kind).await?;
    if let Some(tip) = placement.tip.as_ref() {
        precondition.check_full(vo_id, tip)?;
    }
    Ok(placement)
}

/// The answer to a direct demographic write: `served` with the committed
/// identity, or the stored version `read_back` reads when there is none.
///
/// # Errors
/// An identifier fault of the committed version, or `read_back`'s.
pub(super) async fn demographic_answer(
    served: Option<Body>,
    committed: &Committed,
    read_back: impl Future<Output = Result<ServiceResponse, ServiceError>>,
) -> Result<ServiceResponse, ServiceError> {
    match served {
        Some(body) => Ok(support::committed_response(
            body.into_canonical(),
            committed,
        )?),
        None => read_back.await,
    }
}
