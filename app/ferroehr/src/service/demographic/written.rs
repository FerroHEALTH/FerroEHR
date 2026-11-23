// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the direct party and relationship writes share: the checks before
//! the transaction, the body they store and the one they serve, and the
//! answer built from the commit (#6003). No openEHR spec governs how a write
//! is staged: our own design.

use std::future::Future;

use crate::error::ServiceError;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::support;
use crate::service::demographic::template::demographic_rules;
use crate::service::demographic::validate::Container;
use crate::service::response::{RawServiceResponse, ReadBody, ServiceResponse};
use crate::service::version_update::WriteRequest;
use crate::service::write::Resolved;
use crate::system_log::event::AuditEvent;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::body::Body;
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
        let raw = self
            .on_party_pool(support::load_ehrless_raw(
                &self.demographic_pool,
                self.spec_profile,
                vo_id,
                version,
                at,
                kind,
            ))
            .await??
            .ok_or_else(|| support::missing(kind.as_str(), vo_id, version.is_some() || at.is_some()))?;
        if raw.read.deleted() {
            return Ok(RawServiceResponse {
                body: ReadBody::Value(serde_json::Value::Null),
                meta: None,
            });
        }
        Ok(support::raw_version_response(vo_id, raw)?)
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
