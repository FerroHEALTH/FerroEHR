// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one write path of a single-object commit: a [`WriteRequest`] resolved
//! against the server rules, and [`FerroEhrService::commit_one`], which owns
//! the connection, the transaction, the change and the commit metric.
//!
//! A direct write is one CONTRIBUTION of one change (RM common
//! `master06-change_control_package.adoc` §Committal and Audits). No openEHR
//! spec governs how the commit is run against the database: our own design.

use std::pin::Pin;

use sqlx::{PgConnection, PgPool};

use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::version_update::WriteRequest;
use crate::system_log::categories::AccessedContent;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use crate::system_log::recorder::WriteFacts;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::change::{Committed, Write, WriteEnvelope, commit, record_commit};
use crate::versioning::lifecycle;

/// How [`FerroEhrService::commit_one`] runs a write against the database.
pub(in crate::service) enum Mode {
    /// BEGIN, the prepare step, the change, COMMIT.
    Transaction,
    /// The prepare step and the change on the bare connection, without BEGIN:
    /// a create with no attestations while the event outbox is off commits as
    /// one folded, self-atomic statement.
    Statement,
    /// [`Mode::Transaction`], with the EHR's promoted subject columns synced
    /// from this `EHR_STATUS` body after the change, inside the same
    /// transaction ([`FerroEhrService::sync_ehr_subject`]).
    SyncSubject(EhrId, crate::service::ehr::status::SubjectColumns),
}

/// A [`WriteRequest`]'s commit metadata, resolved against the server rules.
pub(in crate::service) struct Resolved {
    /// The commit audit ([`FerroEhrService::commit_audit`]).
    pub(in crate::service) audit: AuditInput,
    /// The lifecycle state, verbatim client signature and attestations.
    pub(in crate::service) envelope: WriteEnvelope,
    /// `553|incomplete|`, the lifecycle state that relaxes the existence and
    /// cardinality lower bounds (RM common master06 §Incomplete Content).
    pub(in crate::service) incomplete: bool,
}

impl FerroEhrService {
    /// The commit audit of a direct write: the caller's `UPDATE_AUDIT` merged
    /// with the server rules when the request carried one
    /// ([`AuditInput::from_update`]), else the server's own
    /// ([`Self::audit`]).
    ///
    /// # Errors
    /// The [`AuditInput::from_update`] refusals of a supplied audit.
    pub(in crate::service) fn commit_audit(
        &self,
        supplied: Option<&openehr_its::rest::generated::common::UpdateAudit>,
        change_type: &str,
        description: &str,
    ) -> Result<AuditInput, ServiceError> {
        match supplied {
            Some(update) => AuditInput::from_update(
                update,
                change_type,
                description,
                &self.effective_system_id(),
            ),
            None => Ok(self.audit(change_type, description)),
        }
    }

    /// Resolve a request's commit metadata: the audit, then every supplied
    /// attestation (each converted, never a `filter_map` that would drop one
    /// silently).
    ///
    /// # Errors
    /// The [`Self::commit_audit`] refusals, and the
    /// [`crate::versioning::attestation::AttestationInput::from_update`] ones.
    pub(in crate::service) fn resolve_write<T>(
        &self,
        request: &WriteRequest<T>,
        change_type: &str,
        description: &str,
    ) -> Result<Resolved, ServiceError> {
        let audit = self.commit_audit(request.audit.as_ref(), change_type, description)?;
        let attestations = request
            .attestations
            .iter()
            .map(crate::versioning::attestation::AttestationInput::from_update)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Resolved {
            audit,
            incomplete: request.lifecycle_state.as_deref() == Some(lifecycle::state::INCOMPLETE),
            envelope: WriteEnvelope {
                lifecycle_state: request.lifecycle_state.clone(),
                signature: request.signature.clone(),
                attestations,
            },
        })
    }

    /// Commit one single-object write on `pool`: acquire a connection, open
    /// the transaction the [`Mode`] asks for, run `prepare` on it (the gates
    /// and reads that must share the write's connection, ending in the
    /// [`Write`]), apply the change, commit, and record the commit metric.
    ///
    /// The metric is recorded only after the commit, so a rolled-back write is
    /// never counted.
    ///
    /// # Errors
    /// The `prepare` refusals, the [`commit`] ones, the
    /// [`Self::sync_ehr_subject`] conflict, and a failed acquire, begin or
    /// commit.
    pub(in crate::service) fn commit_one<'s, P>(
        &'s self,
        pool: &'s PgPool,
        mode: Mode,
        prepare: P,
    ) -> Pin<Box<impl Future<Output = Result<Committed, ServiceError>> + 's>>
    where
        P: AsyncFnOnce(&mut PgConnection) -> Result<Write, ServiceError> + 's,
    {
        // Boxed: the write carries the node-decomposition machinery and the
        // caller's prepare state, which would otherwise sit on every caller's
        // stack.
        Box::pin(async move {
            let signing_ctx = self.signing_ctx();
            let mut conn = self.recorded_conn(pool).await?;
            let mut invalidate = None;
            // NOTE: our own design (#5762), a request's write reserves its audit records in its
            // own transaction, so the folded statement runs inside one when they are recorded.
            let mode = match mode {
                Mode::Statement if self.records_writes() => Mode::Transaction,
                mode => mode,
            };
            let (ehr_id, committed) = match mode {
                Mode::Statement => {
                    let write = prepare(&mut conn).await?;
                    let ehr_id = write.ehr_id;
                    (ehr_id, commit(&mut conn, write, &signing_ctx).await?)
                }
                Mode::Transaction | Mode::SyncSubject(..) => {
                    let mut tx = conn.begin().await?;
                    let write = prepare(&mut tx).await?;
                    let ehr_id = write.ehr_id;
                    let committed = commit(&mut tx, write, &signing_ctx).await?;
                    let facts = Self::written_facts(ehr_id, &committed);
                    if let Mode::SyncSubject(ehr_id, status) = mode {
                        self.sync_ehr_subject(&mut tx, ehr_id, &status).await?;
                        self.commit_recorded(tx, &facts, Vec::new()).await?;
                        invalidate = Some(ehr_id);
                    } else {
                        self.commit_recorded(tx, &facts, Vec::new()).await?;
                    }
                    (ehr_id, committed)
                }
            };
            drop(conn);
            // NOTE: our own design (#5589, #5879), the status names the subject whose care
            // relationships decide access, so the copy and the request see the commit; read with
            // the write's connection given back, never a second one beside it.
            if let Some(ehr_id) = invalidate {
                self.invalidate_ehr_access(ehr_id).await;
            }
            if committed.skipped {
                self.record_resend(ehr_id, &committed).await?;
            } else {
                record_commit(&committed);
            }
            Ok(committed)
        })
    }
}

impl FerroEhrService {
    /// Records a skipped identical re-send (`[record.versioning] idempotent_resend`,
    /// #4415) on the access log: one `resend` record naming the version the
    /// content matched, its content digest, the committer and the time, and
    /// no commit audit, contribution or outbox event.
    ///
    /// NOTE: Cadasto/FerroHEALTH-backlog#23 (comment 6602171517) answers an
    /// identical re-send with "a log entry which basically is saying that the
    /// last version was saved one more time", and this record is that entry.
    ///
    /// # Errors
    /// [`ServiceError::Unrecorded`] when the audit sender rejected the record
    /// that cannot be written.
    async fn record_resend(
        &self,
        ehr_id: Option<EhrId>,
        committed: &Committed,
    ) -> Result<(), ServiceError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let (object, domain) = match committed.kind {
            Kind::Composition => (ObjectClass::Composition, AccessDomain::Ehr),
            Kind::EhrStatus | Kind::EhrAccess => (ObjectClass::Ehr, AccessDomain::Ehr),
            Kind::Folder => (ObjectClass::Directory, AccessDomain::Ehr),
            Kind::Agent
            | Kind::Group
            | Kind::Organisation
            | Kind::Person
            | Kind::Role
            | Kind::PartyRelationship => (ObjectClass::Demographic, AccessDomain::Demographic),
        };
        let mut event = AuditEvent::new(EventActionCode::Update, object, EventOutcome::Success);
        event.event_type = Some(EventType::rest(crate::config::versioning::RESEND_OPERATION));
        event.domain = domain;
        event.ehr_id = ehr_id.map(|id| id.to_string());
        // The record names the version matched and the digest the content and
        // that version share; the access record carries no field for a hash,
        // so the digest follows the uid in the resource id.
        let digest = match committed.content_digest {
            Some(bytes) => {
                hex(&bytes).map_err(|e| ServiceError::internal("render the content digest", e))?
            }
            None => String::new(),
        };
        event.object_id = Some(format!("{}#sha256:{digest}", committed.version_uid()));
        self.record_one_access(event).await
    }
}

impl FerroEhrService {
    /// What a single-object write decided of its operation's record: the
    /// EHR, the version it wrote and what that version holds, as its write
    /// response names them; a logical delete holds what the version it
    /// deleted held, as its gate read found it (EHDS Annex II 3.2(c)).
    fn written_facts(ehr_id: Option<EhrId>, committed: &Committed) -> WriteFacts {
        let party = pool_is_party(committed);
        let content = committed
            .content_ids()
            .map(|ids| AccessedContent::Objects(vec![ids]));
        WriteFacts {
            ehr_id: ehr_id.filter(|_| !party).map(|id| id.to_string()),
            object_id: Some(committed.version_uid()),
            origins: Vec::new(),
            content,
            domain: party.then_some(AccessDomain::Demographic),
        }
    }
}

/// Whether `committed` is a party's version, written in the party domain.
fn pool_is_party(committed: &Committed) -> bool {
    matches!(
        committed.kind,
        Kind::Agent
            | Kind::Group
            | Kind::Organisation
            | Kind::Person
            | Kind::Role
            | Kind::PartyRelationship
    )
}

/// `bytes` as lowercase hexadecimal.
///
/// # Errors
/// The formatter's [`std::fmt::Error`], which writing to a `String` never
/// raises; it is propagated rather than swallowed.
fn hex(bytes: &[u8]) -> Result<String, std::fmt::Error> {
    use std::fmt::Write as _;
    bytes
        .iter()
        .try_fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            write!(out, "{byte:02x}").map(|()| out)
        })
}
