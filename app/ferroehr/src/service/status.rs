// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The service call-status model, our own design.
//!
//! Every method returns `Result<T, StatusError>`.
//!
//! The single status-to-HTTP table lives with the protocol adapter
//! (`ferroehr-rest::overview::error`), so this module is protocol-free. The
//! ITS-REST status code is the authority in that table.

use std::sync::Arc;

use super::refusal::{Clash, Curated, Invalid, Malformed, Missing, Refusal};

/// The status a failed service call reports, as one flat enum.
///
/// No openEHR spec governs this: our own design. Each variant is one refusal
/// the service layer distinguishes, and the protocol adapter maps it onto the
/// ITS-REST status code.
///
/// Deliberately not `#[non_exhaustive]`: this is an unpublished workspace crate,
/// so the attribute would only force wildcard match arms in the protocol
/// adapter's status-to-HTTP table that silently swallow a future unmapped status.
/// Exhaustive, the compiler flags a missing row when a variant is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceStatus {
    // ── general statuses ────────────────────────────────────────────────────
    /// Call succeeded.
    Success,
    /// Authorisation failure.
    AuthFailure,
    /// Precondition violation occurred.
    PreconditionViolation,
    /// Referenced Object version of a Versioned Object does not exist.
    ObjectVersionDoesNotExist,
    /// No Versioned Object with referenced identifier found.
    VersionedObjectDoesNotExist,
    /// Exception other than precondition violation occurred.
    Exception,
    /// EHR with provided id not found.
    EhrIdDoesNotExist,
    /// Party with provided id not found.
    PartyIdDoesNotExist,
    /// File system locator cannot be written to.
    FileNotWritable,
    /// The committed `preceding_version_uid` does not match the current
    /// version, the optimistic-concurrency failure (ITS-REST `If-Match` →
    /// `412`).
    VersionMismatch,
    /// The requested operation is not implemented by this platform, for
    /// optional or dev-branch routes and unimplemented seams; the wire maps it
    /// to `501 Not Implemented`.
    NotImplemented,
    /// A storage-layer conflict the request could not proceed past — a database
    /// integrity-constraint violation (SQLSTATE class 23) or a serialization/
    /// deadlock failure (40001 / 40P01). The storage bridge classifies these
    /// from `sqlx` rather than the domain layer naming a specific conflict, and
    /// the wire maps it to `409 Conflict`
    /// ("the request could not be processed because it might generate a
    /// duplicate or a conflict", ITS-REST overview §HTTP status codes).
    Conflict,
    /// The service is temporarily unable to serve the request because a backend
    /// resource is exhausted (the `sqlx` connection pool acquire timed out under
    /// sustained load). No openEHR spec governs server overload (our own design,
    /// the admission contract), and the wire maps it to `503 Service
    /// Unavailable` + `Retry-After` (RFC 9110 §15.6.4; the ITS-REST status
    /// subset has no 503, so this is a documented extension).
    ServiceOverloaded,
    /// Processing of the addressed object is restricted, so the server holds
    /// it in storage and does nothing else with it. No openEHR spec governs
    /// restriction of processing, and GDPR Art. 18(2) leaves storage as the
    /// only processing a restriction admits (`docs/law/eu/gdpr/text.html`). The
    /// wire maps it to `403 Forbidden`, the status RFC 9110 §15.5.4 gives a
    /// request the server understood and refuses, and the body names the
    /// restriction.
    ProcessingRestricted,

    // ── EHR statuses ────────────────────────────────────────────────────────
    /// COMPOSITION not found.
    CompositionDoesNotExist,
    /// CONTRIBUTION not found.
    ContributionDoesNotExist,
    /// COMPOSITION fails archetype/template conformance.
    CompositionArchetypeInvalid,
    /// An EHR create with an already-used EHR id.
    EhrCreateFailDuplicateId,
    /// COMPOSITION with this id already exists.
    CompositionAlreadyExists,
    /// An EHR for this subject already exists.
    EhrForSubjectAlreadyExists,

    // ── definition statuses ─────────────────────────────────────────────────
    /// An invalid archetype was provided as a parameter.
    InvalidArchetype,
    /// An invalid template was provided as a parameter.
    InvalidTemplate,
    /// An invalid artefact was provided.
    InvalidArtefact,
    /// An invalid query was provided as a parameter.
    InvalidQuery,
    /// An invalid archetype identifier regex pattern was provided.
    InvalidIdPattern,
    /// A provided archetype identifier does not exist.
    ArtefactDoesNotExist,
    /// A provided template identifier does not exist.
    TemplateDoesNotExist,

    // ── validity and lookup statuses ────────────────────────────────────────
    /// Referenced archetype or template identifiers unknown to the definitions
    /// store, refused on a create or update call.
    DefinitionUnknown,
    /// Content is not a valid RM instance.
    ContentInvalid,
    /// A referenced VERSION does not exist.
    VersionDoesNotExist,
    /// Subject identifier not found in the EHR index.
    SubjectIdDoesNotExist,
    /// `VERSIONED_COMPOSITION` not found.
    VersionedCompositionDoesNotExist,
}

impl ServiceStatus {
    /// The `snake_case` name of this status, as logs and test diagnostics
    /// print it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::AuthFailure => "auth_failure",
            Self::PreconditionViolation => "precondition_violation",
            Self::ObjectVersionDoesNotExist => "object_version_does_not_exist",
            Self::VersionedObjectDoesNotExist => "versioned_object_does_not_exist",
            Self::Exception => "exception",
            Self::EhrIdDoesNotExist => "ehr_id_does_not_exist",
            Self::PartyIdDoesNotExist => "party_id_does_not_exist",
            Self::FileNotWritable => "file_not_writable",
            Self::VersionMismatch => "version_mismatch",
            Self::NotImplemented => "not_implemented",
            Self::Conflict => "conflict",
            Self::ServiceOverloaded => "service_overloaded",
            Self::ProcessingRestricted => "processing_restricted",
            Self::CompositionDoesNotExist => "composition_does_not_exist",
            Self::ContributionDoesNotExist => "contribution_does_not_exist",
            Self::CompositionArchetypeInvalid => "composition_archetype_invalid",
            Self::EhrCreateFailDuplicateId => "ehr_create_fail_duplicate_id",
            Self::CompositionAlreadyExists => "composition_already_exists",
            Self::EhrForSubjectAlreadyExists => "ehr_for_subject_already_exists",
            Self::InvalidArchetype => "invalid_archetype",
            Self::InvalidTemplate => "invalid_template",
            Self::InvalidArtefact => "invalid_artefact",
            Self::InvalidQuery => "invalid_query",
            Self::InvalidIdPattern => "invalid_id_pattern",
            Self::ArtefactDoesNotExist => "artefact_does_not_exist",
            Self::TemplateDoesNotExist => "template_does_not_exist",
            Self::DefinitionUnknown => "definition_unknown",
            Self::ContentInvalid => "content_invalid",
            Self::VersionDoesNotExist => "version_does_not_exist",
            Self::SubjectIdDoesNotExist => "subject_id_does_not_exist",
            Self::VersionedCompositionDoesNotExist => "versioned_composition_does_not_exist",
        }
    }
}

/// The service layer's native error: a [`ServiceStatus`], a message, and the
/// lower-level failure that caused them.
#[derive(Debug, Clone)]
pub struct StatusError {
    /// The status the failed call reports.
    pub status: ServiceStatus,
    /// Human-readable error message.
    pub message: String,
    /// The failure this call status was raised FOR — an `sqlx` driver error, a
    /// codec refusal, an HTTP transport failure — reachable through
    /// [`std::error::Error::source`].
    ///
    /// Kept OUT of [`Self::message`] and never interpolated into it: the
    /// message reaches client response bodies, and a `500`-class body must
    /// disclose no internal error value (OWASP REST Security Cheat Sheet
    /// §Error handling — "Do not pass technical details … to the client").
    /// Attach one with [`StatusError::with_source`].
    ///
    /// `Arc`, not `Box`, because this type is [`Clone`].
    source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

// NOTE: hand-written, because `#[derive(thiserror::Error)]` over an
// `Option<Arc<dyn Error>>` yields the `Arc` WRAPPER as the source hop
// (<https://docs.rs/thiserror/2.0.18/thiserror/index.html>), not the real cause.
impl std::error::Error for StatusError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.source {
            Some(source) => Some(&**source),
            None => None,
        }
    }
}

impl StatusError {
    /// Build a status-coded error with a message.
    #[must_use]
    pub fn new(status: ServiceStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            source: None,
        }
    }

    /// Attach the failure that caused this call status, keeping the message —
    /// and therefore the wire body — byte-identical.
    ///
    /// The cause is carried for the log and for anything walking the chain
    /// ([RFC 0201](https://rust-lang.github.io/rfcs/0201-error-chaining.html)),
    /// never rendered into the message.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Arc::new(source));
        self
    }

    /// `precondition_violation` — an argument-validity precondition failed
    /// (maps to `400` at the wire).
    #[must_use]
    pub fn precondition(message: impl Into<String>) -> Self {
        Self::new(ServiceStatus::PreconditionViolation, message)
    }

    /// `ehr_id_does_not_exist`.
    #[must_use]
    pub fn ehr_not_found(message: impl Into<String>) -> Self {
        Self::new(ServiceStatus::EhrIdDoesNotExist, message)
    }

    /// `version_mismatch` — the optimistic-concurrency (`If-Match`) failure
    /// (→ `412`).
    #[must_use]
    pub fn version_mismatch(message: impl Into<String>) -> Self {
        Self::new(ServiceStatus::VersionMismatch, message)
    }

    /// `exception` — a server-side fault (→ `500`).
    #[must_use]
    pub fn exception(message: impl Into<String>) -> Self {
        Self::new(ServiceStatus::Exception, message)
    }
}

impl StatusError {
    /// The status, the message and the shared cause, for the conversion into a
    /// [`ServiceError`](super::error::ServiceError) refusal.
    pub(crate) fn into_parts(
        self,
    ) -> (
        ServiceStatus,
        String,
        Option<Arc<dyn std::error::Error + Send + Sync>>,
    ) {
        (self.status, self.message, self.source)
    }

    /// The status error a [`Refusal`] of `status` becomes, its cause kept.
    pub(crate) fn from_refusal<K>(status: ServiceStatus, refusal: Refusal<K>) -> Self {
        Self {
            status,
            source: refusal.shared_source(),
            message: refusal.message,
        }
    }
}

impl StatusError {
    /// The `exception` status error a curated fault becomes, its cause kept.
    pub(crate) fn from_curated(fault: &Curated) -> Self {
        Self {
            status: ServiceStatus::Exception,
            source: fault.shared_source(),
            message: fault.message.to_owned(),
        }
    }
}

impl From<Missing> for ServiceStatus {
    fn from(missing: Missing) -> Self {
        match missing {
            Missing::ObjectVersion => Self::ObjectVersionDoesNotExist,
            Missing::VersionedObject => Self::VersionedObjectDoesNotExist,
            Missing::Ehr => Self::EhrIdDoesNotExist,
            Missing::Party => Self::PartyIdDoesNotExist,
            Missing::Composition => Self::CompositionDoesNotExist,
            Missing::Contribution => Self::ContributionDoesNotExist,
            Missing::Artefact => Self::ArtefactDoesNotExist,
            Missing::Template => Self::TemplateDoesNotExist,
            Missing::Version => Self::VersionDoesNotExist,
            Missing::Subject => Self::SubjectIdDoesNotExist,
            Missing::VersionedComposition => Self::VersionedCompositionDoesNotExist,
        }
    }
}

impl From<Malformed> for ServiceStatus {
    fn from(malformed: Malformed) -> Self {
        match malformed {
            Malformed::Precondition => Self::PreconditionViolation,
            Malformed::IdPattern => Self::InvalidIdPattern,
        }
    }
}

impl From<Clash> for ServiceStatus {
    fn from(clash: Clash) -> Self {
        match clash {
            Clash::DuplicateEhrId => Self::EhrCreateFailDuplicateId,
            Clash::CompositionExists => Self::CompositionAlreadyExists,
            Clash::EhrForSubject => Self::EhrForSubjectAlreadyExists,
            Clash::State => Self::Conflict,
        }
    }
}

impl From<Invalid> for ServiceStatus {
    fn from(invalid: Invalid) -> Self {
        match invalid {
            Invalid::Content => Self::ContentInvalid,
            Invalid::Archetype => Self::InvalidArchetype,
            Invalid::Template => Self::InvalidTemplate,
            Invalid::Artefact => Self::InvalidArtefact,
            Invalid::Query => Self::InvalidQuery,
            Invalid::DefinitionUnknown => Self::DefinitionUnknown,
            Invalid::CompositionArchetype => Self::CompositionArchetypeInvalid,
        }
    }
}

/// Sentinel prefix marking an `exception` [`StatusError`] as a **query-execution
/// timeout** rather than a generic server fault — the "message-tagged" 408
/// seam.
///
/// The query chapter ([`super::query`]) aborts a query that overruns its
/// configured execution budget and returns
/// `StatusError::exception(format!("{QUERY_TIMEOUT_TAG}{detail}"))`. The
/// status model has no timeout status, so the protocol adapter recognises the
/// tag, strips it and renders `408 Request Timeout`
/// (`Requests_and_responses.md` §HTTP status codes;
/// `responses/408_Query.yaml`). The tag is a control-char sentinel, so it can
/// never collide with a genuine message and is never shown to clients.
pub const QUERY_TIMEOUT_TAG: &str = "\u{1}query-execution-timeout\u{1}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_error_carries_its_status_and_message() {
        // A failed call carries its status, the status's meaning and its message.
        let err = StatusError::precondition("no such EHR");
        assert_eq!(err.status, ServiceStatus::PreconditionViolation);
        assert_eq!(err.status.as_str(), "precondition_violation");
        assert_eq!(err.message, "no such EHR");
    }
}
