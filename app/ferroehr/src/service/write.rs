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
use crate::storage::error::StorageError;
use crate::storage::version_repo::guarded::GuardFlags;
use crate::system_log::categories::AccessedContent;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use crate::system_log::recorder::WriteFacts;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::change::ready::prepare_write;
use crate::versioning::change::{Committed, Write, WriteEnvelope, commit, record_commit};
use crate::versioning::lifecycle;

/// How [`FerroEhrService::commit_one`] runs a write against the database.
pub(in crate::service) enum Mode {
    /// BEGIN, the prepare step, the change, COMMIT.
    Transaction,
    /// [`Mode::Transaction`], with the EHR's promoted subject columns synced
    /// from this `EHR_STATUS` body after the change, inside the same
    /// transaction ([`FerroEhrService::sync_ehr_subject`]).
    SyncSubject(EhrId, crate::service::ehr::status::SubjectColumns),
}

/// A [`WriteRequest`]'s commit metadata, resolved against the server rules.
#[derive(Clone)]
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
            let mut tx = conn.begin().await?;
            let write = prepare(&mut tx).await?;
            // NOTE: our own design (#6256), from here the write may land, so the request is
            // never run again, whatever its copies held.
            crate::policy_register::observed::wrote();
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
            drop(conn);
            // NOTE: our own design (#5589, #5879), the status names the subject whose care
            // relationships decide access, so the copy and the request see the commit; read with
            // the write's connection given back, never a second one beside it.
            if let Some(ehr_id) = invalidate {
                self.invalidate_ehr_access(ehr_id).await;
            }
            self.counted(ehr_id, committed).await
        })
    }

    /// Commit one COMPOSITION write on `pool` as its gate read judged it
    /// (#6256): acquire the connections, run the attempt `judge` makes on the
    /// bare connection (the gate read in its own statement and the pre-checks,
    /// ending in the guarded [`Write`]), then the one guarded commit statement, in a
    /// transaction when `transaction` holds or the write's records are
    /// reserved. No lock serializes two writers of one object: the statement
    /// requires the head its gate read found, and the version's keys refuse a
    /// second version at the same place.
    ///
    /// The statement writes nothing when what the gate read judged moved
    /// before it ran, and a version's key refuses it when a concurrent commit
    /// took its place first. A moved policy revision refuses the write for the
    /// copies' refresh, so the request runs once more behind the up-front
    /// confirm; anything else is judged again from a new gate read, which
    /// answers as a lock wait would have, at most [`ATTEMPTS`] times.
    ///
    /// # Errors
    /// The `judge` refusals, the [`commit`] ones, `access-refreshing` for a
    /// moved policy, `service_overloaded` once the attempts are spent, and a
    /// failed acquire, begin or commit.
    pub(in crate::service) fn commit_gated<'s, M, J>(
        &'s self,
        pool: &'s PgPool,
        judge: M,
    ) -> Pin<Box<impl Future<Output = Result<Committed, ServiceError>> + 's>>
    where
        M: Fn() -> J + 's,
        J: AsyncFnOnce(&mut PgConnection) -> Result<Write, ServiceError> + 's,
    {
        // Boxed, as `commit_one`.
        Box::pin(async move {
            let signing_ctx = self.signing_ctx();
            for _ in 0..ATTEMPTS {
                let mut conn = self.recorded_conn(pool).await?;
                let write = judge()(&mut conn).await?;
                let ehr_id = write.ehr_id;
                let guard = write.guard.clone().ok_or_else(|| {
                    ServiceError::exception("a COMPOSITION write reached its commit unguarded")
                })?;
                let ready = prepare_write(&mut conn, write, guard, &signing_ctx).await?;
                let facts = Self::written_facts(ehr_id, ready.committed());
                // NOTE: our own design (#5747, #5986, #6256), the records are durable before the
                // statement that makes the write visible inserts their reservation beside it.
                let held = self.hold_records(&mut conn, &facts).await?;
                crate::policy_register::observed::wrote();
                let reservation = held.as_ref().map(|held| held.reservation);
                let outcome = if ready.needs_transaction(&signing_ctx) {
                    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
                    match ready.commit(&mut tx, &signing_ctx, reservation).await {
                        Ok(committed) => tx.commit().await.map(|()| committed).map_err(Into::into),
                        Err(error) => Err(error),
                    }
                } else {
                    ready.commit(&mut conn, &signing_ctx, reservation).await
                };
                if let Some(committed) = self.settle_held(&mut conn, held, outcome).await? {
                    drop(conn);
                    return self.counted(ehr_id, committed).await;
                }
            }
            Err(contended())
        })
    }

    /// What a held write's `outcome` leaves (#6256): its records handed on
    /// when it committed, and discarded at once when it surely wrote nothing,
    /// the write then judged again or answered ([`moved`]); `None` judges it
    /// again.
    ///
    /// # Errors
    /// The write's answer when it is not judged again.
    pub(in crate::service) async fn settle_held<T>(
        &self,
        conn: &mut crate::system_log::reservation::RecordedConn,
        held: Option<crate::system_log::reservation::Held>,
        outcome: Result<T, ServiceError>,
    ) -> Result<Option<T>, ServiceError> {
        match (outcome, held) {
            (Ok(written), held) => {
                if let Some(held) = held {
                    self.written_held(&held);
                }
                Ok(Some(written))
            }
            (Err(error), Some(held)) if unwritten(&error) => {
                self.discard_held(conn, &held).await;
                moved(error).map(|()| None)
            }
            (Err(error), _) => moved(error).map(|()| None),
        }
    }

    /// Count a committed write, or record a skipped identical re-send.
    ///
    /// # Errors
    /// The [`Self::record_resend`] refusal.
    async fn counted(
        &self,
        ehr_id: Option<EhrId>,
        committed: Committed,
    ) -> Result<Committed, ServiceError> {
        if committed.skipped {
            self.record_resend(ehr_id, &committed).await?;
        } else {
            record_commit(&committed);
        }
        Ok(committed)
    }
}

/// What a guarded write consumes ([`FerroEhrService::commit_gated`]): handed
/// to its first attempt, and for a write a later attempt may commit, a copy
/// kept for each one.
pub(in crate::service) struct Materials<T> {
    first: std::sync::Mutex<Option<T>>,
    spare: Option<T>,
}

impl<T> Materials<T> {
    /// `first` for the first attempt alone: a later attempt can only refuse.
    pub(in crate::service) fn once(first: T) -> Self {
        Self {
            first: std::sync::Mutex::new(Some(first)),
            spare: None,
        }
    }

    /// The materials of the first attempt.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when an earlier attempt consumed them.
    pub(in crate::service) fn take_first(&self) -> Result<T, ServiceError> {
        first_attempt(&self.first).ok_or_else(|| {
            ServiceError::overloaded("the record moved while the write was judged", Spent)
        })
    }
}

impl<T: Clone> Materials<T> {
    /// `first` for the first attempt, with a copy for the later ones when
    /// `spare` holds.
    pub(in crate::service) fn new(first: T, spare: bool) -> Self {
        Self {
            spare: spare.then(|| first.clone()),
            first: std::sync::Mutex::new(Some(first)),
        }
    }

    /// The materials of one attempt.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when an earlier attempt consumed them and
    /// no copy was kept: a write that a later attempt could only refuse keeps
    /// none, and one that judges otherwise is asked again.
    pub(in crate::service) fn take(&self) -> Result<T, ServiceError> {
        let taken = self
            .first
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        match (taken, &self.spare) {
            (Some(first), _) => Ok(first),
            (None, Some(spare)) => Ok(spare.clone()),
            (None, None) => Err(ServiceError::overloaded(
                "the record moved while the write was judged",
                Spent,
            )),
        }
    }
}

/// Every attempt of a guarded write found what it was judged on moved.
#[derive(Debug, thiserror::Error)]
#[error("every attempt of this write found the record moved under it")]
struct Contended;

/// An earlier attempt of a guarded write consumed its content.
#[derive(Debug, thiserror::Error)]
#[error("an earlier attempt of this write consumed its content")]
struct Spent;

/// Whether a guarded write that wrote nothing is judged again: `Ok` when
/// what its gate read judged moved or a concurrent commit took its version's
/// place, after counting the retry; otherwise the write's answer, which for a
/// moved policy is `access-refreshing` with the new revisions noted, so the
/// request runs once more behind the up-front confirm.
///
/// # Errors
/// `error` itself when the write is not judged again, and
/// `access-refreshing` for a moved policy.
fn moved(error: ServiceError) -> Result<(), ServiceError> {
    match error {
        ServiceError::Storage(StorageError::GuardMoved(flags)) => {
            crate::policy_register::observed::not_written();
            crate::telemetry::access_path::count_commit_retry(moved_reason(&flags));
            if flags.policy {
                return Ok(());
            }
            if let (Some(policy), Some(definitions)) = (flags.policy_now, flags.definitions_now) {
                crate::policy_register::observed::observe(policy, definitions);
            }
            Err(ServiceError::access_refreshing())
        }
        error if place_taken(&error) => {
            crate::policy_register::observed::not_written();
            crate::telemetry::access_path::count_commit_retry("key");
            Ok(())
        }
        error => Err(error),
    }
}

/// The `503` of a guarded write whose every attempt found what it was judged
/// on moved.
pub(in crate::service) fn contended() -> ServiceError {
    ServiceError::overloaded("the record moved while the write was judged", Contended)
}

/// What `slot` holds, taken: the first attempt of a guarded write takes it,
/// and a later one finds it gone.
pub(in crate::service) fn first_attempt<T>(slot: &std::sync::Mutex<Option<T>>) -> Option<T> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// The attempts a guarded commit makes before it answers `503` (#6256): each
/// one after the first follows a commit that landed in between, so the bound
/// is the contention one write survives, which is our own design.
pub(in crate::service) const ATTEMPTS: usize = 8;

/// Whether a write that answered `error` surely wrote nothing: any failure
/// the server answered, or that came before the statement. A connection lost
/// while the statement ran may have committed it, so its records are left to
/// the reconciler, which releases them when their reservation committed.
fn unwritten(error: &ServiceError) -> bool {
    let lost = |error: &sqlx::Error| {
        matches!(
            error,
            sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::Protocol(_)
        )
    };
    match error {
        ServiceError::Storage(StorageError::Database(error)) | ServiceError::Database(error) => {
            !lost(error)
        }
        _ => true,
    }
}

/// Whether `error` is a version key refusing a second version at the place a
/// concurrent commit of the same object took first (`23505`, PostgreSQL docs
/// Appendix A), which a guarded write judges again.
fn place_taken(error: &ServiceError) -> bool {
    let ServiceError::Storage(StorageError::Database(sqlx::Error::Database(db))) = error else {
        return false;
    };
    db.code().as_deref() == Some("23505")
        && db.constraint().is_some_and(|name| {
            matches!(
                name,
                "version_hot_pkey" | "uq_version_hot_trunk_position" | "uq_version_hot_tree"
            )
        })
}

/// The `reason` label of a guarded commit that wrote nothing: the first of
/// its requirements that no longer held.
fn moved_reason(flags: &GuardFlags) -> &'static str {
    if !flags.policy {
        "policy"
    } else if !flags.ehr {
        "ehr"
    } else if !flags.tip {
        "tip"
    } else if !flags.mark {
        "mark"
    } else {
        "order"
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
