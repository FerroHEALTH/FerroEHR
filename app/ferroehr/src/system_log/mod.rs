// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! IHE ATNA (Audit Trail and Node Authentication) audit trail, the platform's
//! system log.
//!
//! openEHR fixes no audit-record shape, so everything below follows external
//! standards, cited as external standards and never as openEHR spec text.
//!
//! One audit record per audited API operation is written to the local Audit
//! Record Repository before its response (the `audit` schema, always on while
//! auditing is, served back via the RESTful-ATNA ITI-81 retrieval) and
//! delivered from it, in both official ATNA formats, to the configured
//! sinks: the classic DICOM Audit Message
//! feed (DICOM PS3.15 §A.5 XML over RFC 5424 syslog on RFC 5425 TLS; IHE ITI
//! TF-2 ITI-20), and the FHIR R4 `AuditEvent` feed (IHE BALP shape; ITI-20 ATX:
//! FHIR Feed). The shared FerroAUDIT crates write both formats and send them:
//! `ihe-atna` the ATNA trail and the BALP `AuditEvent`, `ehds-logging` the EHDS
//! access record, its categories, its retention and its emergency mark.
//! FerroEHR decides which accesses are logged, what each record says, and
//! where it is stored. This is authorized defensive security-audit logging for
//! a healthcare system.
//!
//! ## Scope boundary
//!
//! This ATNA system log is the security surveillance record of API access: who
//! did what to which resource, with what outcome. The RM change-control audit is
//! orthogonal and lives in the versioning path, where every VERSION or
//! CONTRIBUTION write records its own authorship in `AUDIT_DETAILS` (BASE
//! `architecture_overview/master07-security.adoc` §Integrity). Do not duplicate
//! it here.
//!
//! ## Seams
//!
//! The ITS-REST operation to classification mapping is the protocol adapter's
//! concern (`ferroehr-rest::system_log::classify`); its audit middleware builds
//! an [`event::AuditEvent`] per request and writes it through
//! [`FerroEhrService::record_events`]. The binary boots the subsystem via
//! [`sender::start`] and supplies the DB-backed [`sender::SubjectResolver`]; the
//! sender is installed with
//! [`FerroEhrService::with_audit`](crate::service::FerroEhrService::with_audit).
//!
//! ## Module map
//! - [`event`] — the transport-agnostic audit event model.
//! - [`access_context`] — the request-scoped purpose of use, for records the
//!   service layer emits by itself.
//! - [`categories`] — the EHDS priority-category map and the classifier every
//!   access record is stamped through.
//! - [`codes`] — DCM / RFC-3881 code constants and the ATNA rendering of the
//!   event enums.
//! - [`message`] — the DICOM `AuditMessage` of a record, on `ihe-atna`'s
//!   message model and writer.
//! - `fhir` — the FHIR R4 `AuditEvent` of a record (the `fhir` cargo feature):
//!   the EHDS access record written by `ehds-logging`, every other record a
//!   BALP exchange written by `ihe-atna`.
//! - `syslog` — the ITI-20 syslog sink: `ihe-atna`'s RFC 5424 frame over its
//!   RFC 5425 TLS connection, delivered from the store (the `fhir` cargo
//!   feature).
//! - [`render`] — the render policy a stored record's FHIR `AuditEvent` is
//!   rendered under, and the rendering of a stored record.
//! - [`store`] — the local Audit Record Repository and the ITI-81 search filter.
//! - [`sender`] — the sender that writes each record before its response, the
//!   chainer and the forwarders on the audit runtime.
//! - [`config`] — the `[audit]` section struct ([`config::AuditConfig`]).

#![cfg_attr(
    feature = "fhir",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): a carrier of an \
              already-rendered FHIR document stays a JSON value"
    )
)]

pub(crate) mod access_binding;
pub mod access_context;
pub mod binding;
pub mod categories;
pub mod chain_check;
pub mod codes;
pub mod config;
pub mod event;
#[cfg(feature = "fhir")]
mod feed;
#[cfg(feature = "fhir")]
pub mod fhir;
#[cfg(feature = "fhir")]
mod fhir_accessor;
pub mod message;
pub mod recorder;
pub mod render;
mod reservation;
pub mod retention;
pub mod seal;
#[cfg(feature = "fhir")]
pub(crate) mod search;
pub mod sender;
pub mod store;
pub(crate) mod token_digest;

use crate::system_log::sender::AuditSender;
#[cfg(feature = "fhir")]
pub mod syslog;
#[cfg(feature = "fhir")]
mod withheld;

use event::AuditEvent;

// The paths the binary and the config tree consume (`crate::system_log::sender::start`,
// `ferroehr::system_log::{AuditConfig, AuditHandle, AuditSender, SubjectResolver}`).

use crate::error::ServiceError;
use crate::error::refusal::Refusal;
use crate::service::FerroEhrService;

/// Errors raised while rendering or shipping an audit record.
///
/// Every variant that has an underlying failure carries it as
/// [`std::error::Error::source`] (RFC 0201), so a caller can walk or match the
/// cause instead of parsing prose. The boxed sources are the ones several
/// unrelated types converge on: the `ihe-atna` message, repository and feed
/// errors, and the file reads of the TLS material.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// XML serialization of the DICOM Audit Message failed.
    #[error("audit message serialization failed")]
    Xml(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The syslog TLS connection could not be established, or the ITI-20 feed
    /// request could not be sent.
    #[error("audit transport error")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The ITI-20 ATX:FHIR Feed answered a non-success status.
    #[error("the FHIR audit feed answered {0}")]
    FeedRejected(http::StatusCode),
    /// The local Audit Record Repository write/reap failed.
    #[error("audit store error")]
    Store(#[from] sqlx::Error),
    /// No connection to the local Audit Record Repository could be obtained.
    #[error("audit store unavailable")]
    Acquire(#[from] crate::db::acquire::AcquireError),
    /// Rendering the FHIR `AuditEvent` document failed.
    #[error("audit rendering failed")]
    Render(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The `[audit.categories]` map does not compile; the boot validation
    /// reports the same findings.
    #[error("the audit category map is invalid: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    CategoryMap(Vec<categories::CategoryMapError>),
    /// `ehds-logging` refused an `[audit] emergency_purpose_codes` entry.
    #[error("the audit emergency purposes are invalid")]
    Emergency(#[source] ehds_logging::emergency::EmergencyError),
    /// `ehds-logging` refused a retention or label policy of the access record.
    #[error("the access record policy is invalid")]
    Policy(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The access-log entries of a batch could not be appended.
    #[error("the access-log entries could not be written")]
    AccessLog(#[from] crate::access_log::store::AccessLogError),
    /// A stored access-log entry does not read as the audit record its
    /// projections are written from.
    #[cfg(feature = "fhir")]
    #[error("the access-log entry does not read as an audit record")]
    Recorded(#[from] crate::access_log::recorded::RecordedError),
    /// A batch named a retention end count other than its record count.
    #[error("{ends} retention ends for {records} audit records")]
    RetentionEnds {
        /// How many ends the batch named.
        ends: usize,
        /// How many records it held.
        records: usize,
    },
    /// A stored record holds a value this build does not read, or names a
    /// render policy the store does not hold.
    #[error("audit record {chain_seq} cannot be read back: {reason}")]
    Unreadable {
        /// The record's chain position.
        chain_seq: i64,
        /// What could not be read.
        reason: &'static str,
    },
    /// A search token or shared set of a record cannot be read back from the
    /// store after it was stored.
    #[error("an audit search token cannot be read back from the store")]
    SearchToken,
    /// The audit subsystem's own runtime could not start.
    #[error("the audit runtime could not start")]
    Runtime(#[source] std::io::Error),
    /// An access-log entry's uid is no UUID, so the log cannot key it.
    #[error("an access-log entry's uid is no UUID")]
    EntryUid(#[source] uuid::Error),
    /// A JSON column of a stored record, or the render policy it names, does
    /// not read as the shape this build writes.
    #[error("audit record {chain_seq} cannot be read back: {column} does not read")]
    UnreadableJson {
        /// The record's chain position.
        chain_seq: i64,
        /// The column that does not read.
        column: &'static str,
        /// Why it does not read.
        #[source]
        source: serde_json::Error,
    },
}

/// The loud slim-build refusal of auditing.
///
/// The local Audit Record Repository every record is written to before its
/// access is reported carries the access-log entry and its FHIR R4
/// `AuditEvent` projection, which a binary built without the `fhir` cargo
/// feature cannot write. A configuration that turns auditing on is a boot
/// error, never an access reported without its record.
///
/// # Errors
/// The refusal message when `[audit] enabled` is true.
#[cfg(not(feature = "fhir"))]
pub fn require_fhir_disabled(cfg: &config::AuditConfig) -> Result<(), String> {
    if cfg.enabled {
        return Err(
            "[audit] enabled = true, but this binary was built without the `fhir` cargo \
             feature (the audit record repository writes the access-log entry and its FHIR \
             AuditEvent projection); set [audit] enabled = false or use the full build"
                .to_owned(),
        );
    }
    Ok(())
}

/// The platform's system log: it records resolved audit events through the
/// optional ATNA [`AuditSender`] the binary wires in
/// ([`FerroEhrService::with_audit`](crate::service::FerroEhrService::with_audit)).
/// With no sender wired, auditing is off and nothing is recorded.
impl FerroEhrService {
    /// Write `events`, the records of one act, durably before the act is
    /// reported; nothing when no sender is wired.
    ///
    /// # Errors
    /// [`ServiceError::Unrecorded`] (the wire's `503`) when the records could
    /// not be written: the act is not reported as done.
    pub async fn record_events(&self, events: Vec<AuditEvent>) -> Result<(), ServiceError> {
        let Some(sender) = &self.audit else {
            return Ok(());
        };
        // NOTE: boxed, so every service future that records carries a pointer, not the write.
        Box::pin(sender.record(events, None))
            .await
            .map_err(|unwritten| {
                ServiceError::Unrecorded(
                    Refusal::new((), "audit trail unavailable").with_source(unwritten),
                )
            })
    }

    /// Record `event` as one act on one object of the current request: one
    /// result, under the request's purpose and permit and the deployment's
    /// legal basis.
    ///
    /// # Errors
    /// As [`Self::record_access`].
    pub async fn record_one_access(&self, mut event: AuditEvent) -> Result<(), ServiceError> {
        self.stamp_one_access(&mut event);
        self.record_access(event).await
    }

    /// Record `event` as an act of the request's committer, under the
    /// request's purpose and permit and the deployment's legal basis.
    ///
    /// # Errors
    /// As [`Self::record_access`].
    pub(crate) async fn record_committer_access(
        &self,
        mut event: AuditEvent,
    ) -> Result<(), ServiceError> {
        self.stamp_committer(&mut event);
        self.record_access(event).await
    }

    /// Stamp `event` as an act of the request's committer, under the
    /// request's purpose and permit and the deployment's legal basis.
    pub(crate) fn stamp_committer(&self, event: &mut AuditEvent) {
        event.purpose = access_context::current_purpose();
        event.permit = access_context::current_permit();
        if let Some(committer) = crate::versioning::committer::current_committer() {
            event.user_id = committer.subject;
        }
        event.legal_basis = self.audit_legal_basis().map(str::to_owned);
    }

    /// Stamp `event` as one act on one object of the current request: one
    /// result, the request's origin, purpose and permit, the deployment's
    /// legal basis, and the caller ([`stamp_caller`]).
    pub(crate) fn stamp_one_access(&self, event: &mut AuditEvent) {
        event.result_count = Some(1);
        let origin = access_context::current_origin();
        event.request_id = origin.request_id;
        event.client_ip = origin.client_address;
        event.purpose = access_context::current_purpose();
        event.permit = access_context::current_permit();
        event.legal_basis = self.audit_legal_basis().map(str::to_owned);
        stamp_caller(event);
    }

    /// Record an access event, stamped with the caller, before the access is
    /// reported: the service-layer twin of the REST middleware's record.
    ///
    /// # Errors
    /// As [`Self::record_events`].
    pub async fn record_access(&self, mut event: AuditEvent) -> Result<(), ServiceError> {
        stamp_caller(&mut event);
        self.record_events(vec![event]).await
    }

    /// Whether `purpose` is one of `[audit] emergency_purpose_codes`, the codes
    /// that declare an emergency access in the vital interest of the person
    /// (EHDS Art. 11(5)); `false` when no audit sender is wired.
    #[must_use]
    pub fn is_emergency_purpose(&self, purpose: Option<&str>) -> bool {
        self.audit
            .as_ref()
            .is_some_and(|sender| sender.is_emergency_purpose(purpose))
    }

    /// Whether ATNA auditing is on (a sender is wired and its master switch set).
    pub fn audit_enabled(&self) -> bool {
        self.audit.as_ref().is_some_and(AuditSender::enabled)
    }

    /// Whether login / application-activity events are suppressed (the
    /// deployment default; see [`config::AuditConfig::suppress_login_events`]).
    pub fn suppress_login_events(&self) -> bool {
        self.audit
            .as_ref()
            .is_some_and(AuditSender::suppress_login_events)
    }

    /// The request header a caller declares its purpose of use in
    /// ([`config::AuditConfig::purpose_header`]), lowercased; `None` when no
    /// audit sender is wired.
    #[must_use]
    pub fn audit_purpose_header(&self) -> Option<&str> {
        self.audit.as_ref().map(AuditSender::purpose_header)
    }

    /// The request header a caller declares the data permit it acts under in
    /// ([`config::AuditConfig::permit_header`]), lowercased; `None` when no
    /// audit sender is wired.
    #[must_use]
    pub fn audit_permit_header(&self) -> Option<&str> {
        self.audit.as_ref().map(AuditSender::permit_header)
    }

    /// Whether `code` is a purpose code this deployment records
    /// ([`config::AuditConfig::purpose_codes`]).
    #[must_use]
    pub fn audit_accepts_purpose(&self, code: &str) -> bool {
        self.audit
            .as_ref()
            .is_some_and(|sender| sender.accepts_purpose(code))
    }

    /// The legal basis recorded on every access event
    /// ([`config::AuditConfig::legal_basis`]).
    #[must_use]
    pub fn audit_legal_basis(&self) -> Option<&str> {
        self.audit.as_ref().and_then(AuditSender::legal_basis)
    }

    /// Classifies what an access touched through the configured category map
    /// ([`crate::policy_register`]), as data the access wrote when
    /// `action` creates, updates or deletes and as data it read otherwise;
    /// `None` when no audit sender is wired, because no record will carry it.
    #[must_use]
    pub fn audit_classify(
        &self,
        content: &categories::AccessedContent,
        action: event::EventActionCode,
    ) -> Option<categories::CategoryRecord> {
        let written = matches!(
            action,
            event::EventActionCode::Create
                | event::EventActionCode::Update
                | event::EventActionCode::Delete
        );
        self.audit
            .as_ref()
            .map(|sender| sender.categories().classify(content, written))
    }

    /// The content identities an access record of `vo_ids` is classified by:
    /// each object's newest version that carries content, so a container read
    /// or a logical delete names the data it addresses (EHDS Annex II 3.2(c),
    /// `docs/law/eu/ehds/text.html`).
    ///
    /// Empty without a reading when auditing is off, because no record will
    /// carry it.
    ///
    /// # Errors
    /// [`ServiceError`] when the read fails.
    pub async fn audit_content_of_objects(
        &self,
        vo_ids: &[crate::ids::VoId],
    ) -> Result<Vec<categories::ContentIds>, ServiceError> {
        let mut conn = crate::db::acquire::acquire(&self.pool).await?;
        self.audit_content_of_objects_on(&mut conn, vo_ids).await
    }

    /// [`Self::audit_content_of_objects`] on `conn`, which a write's own
    /// transaction can be, so it reads what the write landed.
    ///
    /// # Errors
    /// [`ServiceError`] when the read fails.
    pub(crate) async fn audit_content_of_objects_on(
        &self,
        conn: &mut sqlx::PgConnection,
        vo_ids: &[crate::ids::VoId],
    ) -> Result<Vec<categories::ContentIds>, ServiceError> {
        if !self.audit_enabled() {
            return Ok(Vec::new());
        }
        let served = crate::storage::version_repo::read::content_of_objects(conn, vo_ids)
            .await
            .map_err(ServiceError::from)?;
        Ok(served.iter().map(content_ids).collect())
    }

    /// The content identities a CONTRIBUTION read is classified by: those of
    /// the versions it committed, each logical delete read through to the
    /// version it deleted.
    ///
    /// Empty without a reading when auditing is off.
    ///
    /// # Errors
    /// [`ServiceError`] when the read fails.
    pub async fn audit_content_of_contribution(
        &self,
        contribution_id: uuid::Uuid,
    ) -> Result<Vec<categories::ContentIds>, ServiceError> {
        if !self.audit_enabled() {
            return Ok(Vec::new());
        }
        let served = crate::storage::version_repo::read::content_of_contribution(
            &self.pool,
            contribution_id,
        )
        .await
        .map_err(ServiceError::from)?;
        Ok(served.iter().map(content_ids).collect())
    }

    /// Whether the local Audit Record Repository is available (the store is
    /// wired), i.e. the ITI-81 retrieval surface can be served.
    #[must_use]
    pub fn audit_search_enabled(&self) -> bool {
        self.audit_store.is_some()
    }

    /// The RESTful-ATNA **ITI-81** retrieval: the FHIR `AuditEvent` documents
    /// of the matching records, rendered from the store, newest first, plus
    /// the total match count when the filter asks. The read side of the system log (the retrieval
    /// semantics are IHE's — the `RESTful` ATNA supplement's ITI-81 FHIR search
    /// on `AuditEvent`).
    ///
    /// # Errors
    /// [`ServiceError::BadRequest`] when no local store is wired, or a server
    /// fault when the store query fails.
    #[cfg(feature = "fhir")]
    pub async fn audit_event_search(
        &self,
        filter: &store::AuditSearchFilter,
    ) -> Result<store::AuditPage, ServiceError> {
        let Some(audit_store) = &self.audit_store else {
            return Err(ServiceError::precondition(
                "the local audit record repository is not enabled ([audit.store])",
            ));
        };
        audit_store
            .search(filter)
            .await
            .map_err(|e| ServiceError::internal("search the audit record repository", e))
    }
}

/// Stamp the caller of the current request onto a record the service layer
/// built, where the request's principal is gone: the roles the
/// authentication layer published for the scope (#3239) and the committer.
fn stamp_caller(event: &mut AuditEvent) {
    if event.roles.is_empty() {
        event.roles = access_context::current_roles();
    }
    // NOTE: EHDS Annex II 3.2 (`docs/law/eu/ehds/text.html`), a record of an EHR the
    // request reached names the grounds the access evaluator gave it.
    if event.ehr_id.is_some()
        && let Some(access) = crate::access_eval::condition::current()
    {
        crate::access_eval::record::stamp(event, &access);
    }
    // NOTE: EHDS Art. 9(2)(a) (`docs/law/eu/ehds/text.html`), the record names
    // who accessed the data, and the request's authenticated committer is that.
    if event.user_id.is_empty()
        && let Some(committer) = crate::versioning::committer::current_committer()
    {
        event.user_id = committer.subject;
    }
}

/// The classifier's identities of one stored content tuple.
fn content_ids(
    served: &crate::storage::version_repo::read::ServedContent,
) -> categories::ContentIds {
    categories::ContentIds::of_kind(
        &served.kind,
        served.template_id.clone(),
        served.root_archetype.clone(),
    )
}
