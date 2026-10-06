// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! IHE ATNA (Audit Trail and Node Authentication) audit trail, the platform's
//! realization of the SM System Log component (`I_SYSTEM_LOG`).
//!
//! The one normative openEHR statement for this component is a single line of
//! the SM platform component table, "System Log | IHE ATNA-compliant system
//! log" (`docs/specs/openehr/SM/docs/openehr_platform/master02-overview.adoc`);
//! the `I_SYSTEM_LOG` interface (`UML/classes/i_system_log.adoc`) is an empty
//! stub. Everything below realizes that mandate against external standards,
//! cited as external standards and never as openEHR spec text.
//!
//! One audit record per audited API operation is rendered in both official ATNA
//! formats and fanned out to the configured sinks: the local Audit Record
//! Repository (the `audit` schema, the durability anchor, on by default, served
//! back via the RESTful-ATNA ITI-81 retrieval), the classic DICOM Audit Message
//! feed (DICOM PS3.15 §A.5 XML over RFC 5424 syslog; RFC 5426 UDP or RFC 5425
//! TLS; IHE ITI TF-2 ITI-20), and the FHIR R4 `AuditEvent` feed (IHE BALP shape;
//! ITI-20 ATX:FHIR Feed). This is authorized defensive security-audit logging
//! for a healthcare system.
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
//! an [`event::AuditEvent`] per request and hands it to the platform through
//! [`FerroEhrService::emit`]. The binary boots the subsystem via
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
//! - [`message`] — the DICOM `AuditMessage` model and `quick-xml` serializer.
//! - `fhir` — the FHIR R4 `AuditEvent` rendering per the IHE BALP content
//!   profiles (the `fhir` cargo feature, over `ferroehr_ext::fhir::audit`).
//! - [`syslog`] — RFC 5424 assembly plus RFC 5426 UDP / RFC 5425 TLS transports.
//! - [`store`] — the local Audit Record Repository and the ITI-81 search filter.
//! - [`sender`] — the bounded-mpsc sender, background drain, sink fan-out and
//!   fail modes.
//! - [`config`] — the `[audit]` section struct ([`config::AuditConfig`]).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): a carrier of an \
              already-rendered FHIR document stays a JSON value"
)]

pub mod access_context;
pub mod categories;
pub mod chain_check;
pub mod codes;
pub mod config;
pub mod event;
#[cfg(feature = "fhir")]
pub mod fhir;
pub mod message;
pub mod sender;
pub mod store;

use crate::system_log::sender::AuditSender;
pub mod syslog;

use event::{AuditEvent, EmitOutcome};

// The paths the binary and the config tree consume (`crate::system_log::sender::start`,
// `ferroehr::system_log::{AuditConfig, AuditHandle, AuditSender, SubjectResolver}`).

use crate::service::FerroEhrService;
use crate::service::status::{CallStatusType, SmError};

/// Errors raised while rendering or shipping an audit record.
///
/// Every variant that has an underlying failure carries it as
/// [`std::error::Error::source`] (RFC 0201), so a caller can walk or match the
/// cause instead of parsing prose. The boxed sources are the ones several
/// unrelated types converge on: XML serialization fails as either an
/// `std::io::Error` or a `FromUtf8Error`, and the transport as either an
/// `std::io::Error` (syslog UDP/TLS) or a `reqwest::Error` (the ITI-20 feed).
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// XML serialization of the DICOM Audit Message failed.
    #[error("audit message serialization failed")]
    Xml(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The syslog transport (UDP/TLS) could not be established, or the ITI-20
    /// feed request could not be sent.
    #[error("audit transport error")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The ITI-20 ATX:FHIR Feed answered a non-success status.
    #[error("the FHIR audit feed answered {0}")]
    FeedRejected(http::StatusCode),
    /// The local Audit Record Repository write/reap failed.
    #[error("audit store error")]
    Store(#[from] sqlx::Error),
    /// Rendering the FHIR `AuditEvent` document failed.
    #[error("audit rendering failed")]
    Render(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The `[audit.categories]` map does not compile; the boot validation
    /// reports the same findings.
    #[error("the audit category map is invalid: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    CategoryMap(Vec<categories::CategoryMapError>),
}

// quick-xml's `Writer` over an in-memory buffer surfaces write failures as
// `std::io::Error`; in the DICOM serializer these can only be a buffer fault.
impl From<std::io::Error> for AuditError {
    fn from(e: std::io::Error) -> Self {
        AuditError::Xml(Box::new(e))
    }
}

/// The loud slim-build refusal for the FHIR-rendering audit sinks.
///
/// Both the local Audit Record Repository and the ITI-20 ATX:FHIR Feed carry
/// a FHIR R4 `AuditEvent` document, which a binary built without the `fhir`
/// cargo feature cannot render. A configuration that enables either is a
/// boot error, never a silently document-less audit trail (the syslog sink
/// needs no FHIR and stays available).
///
/// # Errors
/// The refusal message when `[audit.store]` or `[audit.fhir_feed]` is
/// enabled.
#[cfg(not(feature = "fhir"))]
pub fn require_fhir_disabled(cfg: &config::AuditConfig) -> Result<(), String> {
    if !cfg.enabled {
        return Ok(());
    }
    if cfg.store.enabled {
        return Err(
            "audit.store.enabled = true, but this binary was built without the \
             `fhir` cargo feature (the audit record repository stores FHIR \
             AuditEvent documents)"
                .to_owned(),
        );
    }
    if cfg.fhir_feed.enabled {
        return Err(
            "audit.fhir_feed.enabled = true, but this binary was built without \
             the `fhir` cargo feature"
                .to_owned(),
        );
    }
    Ok(())
}

/// The platform realizes the SM `I_SYSTEM_LOG` component: it emits resolved
/// audit events through the optional ATNA [`AuditSender`] the binary wires in
/// ([`FerroEhrService::with_audit`](crate::service::FerroEhrService::with_audit)).
/// With no sender wired, auditing is off and every emit is
/// [`EmitOutcome::Dropped`].
impl FerroEhrService {
    /// Enqueue a resolved audit event on the system log (non-blocking).
    /// Returns [`EmitOutcome::Dropped`] when no sender is wired, and the
    /// sender's own outcome otherwise (see [`AuditSender::emit`]).
    #[must_use]
    pub fn emit(&self, event: AuditEvent) -> EmitOutcome {
        // `map_or` (not `map(..).unwrap_or(..)`) keeps clippy happy; behaviour
        // is identical to `Dropped` when no sender is wired.
        self.audit
            .as_ref()
            .map_or(EmitOutcome::Dropped, |s| s.emit(event))
    }

    /// Record an access event and honour the configured fail mode.
    ///
    /// The service-layer twin of the REST middleware's rejection: under
    /// `[audit] fail_mode = "closed"` a record the sender cannot take
    /// ([`EmitOutcome::Rejected`]) becomes an error the caller returns in
    /// place of its result, so no read or write completes unrecorded.
    /// `Enqueued`, and `Dropped` under `fail_mode = "open"`, complete.
    ///
    /// # Errors
    /// [`SmError`] with the `service_overloaded` status (the wire's `503`)
    /// when the sender rejected the record.
    pub fn record_access(&self, mut event: AuditEvent) -> Result<(), SmError> {
        // A service-layer record is built where the request's principal is
        // gone; the roles the authentication layer published for the scope
        // are stamped here so every emitter carries them (#3239).
        if event.roles.is_empty() {
            event.roles = access_context::current_roles();
        }
        match self.emit(event) {
            EmitOutcome::Enqueued | EmitOutcome::Dropped => Ok(()),
            EmitOutcome::Rejected => Err(SmError::new(
                CallStatusType::ServiceOverloaded,
                "audit trail unavailable (fail-closed)",
            )),
        }
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
    /// ([`config::AuditConfig::categories`]); `None` when no audit sender is
    /// wired, because no record will carry it.
    #[must_use]
    pub fn audit_classify(
        &self,
        content: &categories::AccessedContent,
    ) -> Option<categories::CategoryRecord> {
        self.audit
            .as_ref()
            .map(|sender| sender.categories().classify(content))
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
    /// [`SmError`] when the read fails.
    pub async fn audit_content_of_objects(
        &self,
        vo_ids: &[crate::ids::VoId],
    ) -> Result<Vec<categories::ContentIds>, SmError> {
        if !self.audit_enabled() {
            return Ok(Vec::new());
        }
        let served = crate::storage::version_repo::read::content_of_objects(&self.pool, vo_ids)
            .await
            .map_err(crate::service::error::ServiceError::from)?;
        Ok(served.iter().map(content_ids).collect())
    }

    /// The content identities a CONTRIBUTION read is classified by: those of
    /// the versions it committed, each logical delete read through to the
    /// version it deleted.
    ///
    /// Empty without a reading when auditing is off.
    ///
    /// # Errors
    /// [`SmError`] when the read fails.
    pub async fn audit_content_of_contribution(
        &self,
        contribution_id: uuid::Uuid,
    ) -> Result<Vec<categories::ContentIds>, SmError> {
        if !self.audit_enabled() {
            return Ok(Vec::new());
        }
        let served = crate::storage::version_repo::read::content_of_contribution(
            &self.pool,
            contribution_id,
        )
        .await
        .map_err(crate::service::error::ServiceError::from)?;
        Ok(served.iter().map(content_ids).collect())
    }

    /// Whether the local Audit Record Repository is available (the store is
    /// wired), i.e. the ITI-81 retrieval surface can be served.
    #[must_use]
    pub fn audit_search_enabled(&self) -> bool {
        self.audit_store.is_some()
    }

    /// The RESTful-ATNA **ITI-81** retrieval: the stored FHIR `AuditEvent`
    /// documents matching the filter, newest first, plus the total match
    /// count. The read side of the SM System Log component (the only
    /// normative openEHR statement is the "IHE ATNA-compliant system log"
    /// line above; the retrieval semantics are IHE's — the `RESTful` ATNA
    /// supplement's ITI-81 FHIR search on `AuditEvent`).
    ///
    /// # Errors
    /// [`SmError`] `precondition_violation` when no local store is wired, or
    /// `exception` when the store query fails.
    pub async fn audit_event_search(
        &self,
        filter: &store::AuditSearchFilter,
    ) -> Result<(i64, Vec<serde_json::Value>), SmError> {
        let Some(audit_store) = &self.audit_store else {
            return Err(SmError::precondition(
                "the local audit record repository is not enabled ([audit.store])",
            ));
        };
        audit_store.search(filter).await.map_err(|e| {
            crate::service::error::internal_fault("search the audit record repository", &e)
        })
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
