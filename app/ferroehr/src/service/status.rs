// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SM call-status model (`master03-common_package.adoc` §Representing
//! Call Status; `i_status.adoc`, `call_status.adoc`, `call_status_type.adoc`
//! + the service-specific descendants `ehr_call_status_type.adoc`,
//!   `definition_call_status_type.adoc`).
//!
//! The SM's stateful `I_STATUS.last_call_failed()`/`last_call_status()` protocol
//! maps onto a stateless typed-error style the spec sanctions
//! (`master02-overview.adoc` §Functional Style: "Either style can be used, and
//! can be trivially mapped from one to the other"), so every chapter method
//! returns `Result<T, SmError>` and a failed call's [`CallStatus`] object is
//! built on demand via [`SmError::into_call_status`].
//!
//! The single SM to HTTP table lives with the protocol adapter
//! (`ferroehr-rest::overview::error`), so this module is protocol-free. Where
//! the SM name and the wire disagree, the wire's status code wins in that
//! table.

use std::sync::Arc;

/// `CALL_STATUS_TYPE` and its service-specific descendants, as one Rust enum.
///
/// The SM models the extension as inheritance ("Particular services may add more
/// codes by inheriting from this class and defining further specific codes",
/// `master03-common_package.adoc` §Representing Call Status); one flat Rust enum
/// with the provenance documented per variant is the idiomatic equivalent, and
/// every abstract error name an SM interface uses has exactly one variant here.
/// Variants marked *(prose-only)* appear in SM interface `.Errors` blocks but in
/// no vendored enumeration.
///
/// Deliberately not `#[non_exhaustive]`: this is an unpublished workspace crate,
/// so the attribute would only force wildcard match arms in the protocol
/// adapter's SM to HTTP table that silently swallow a future unmapped status.
/// Exhaustive, the compiler flags a missing row when a variant is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallStatusType {
    // ── CALL_STATUS_TYPE (base; `call_status_type.adoc`) ────────────────────
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
    /// (Meaning blank in the source.) The committed `preceding_version_uid`
    /// does not match the current version — the optimistic-concurrency
    /// failure (ITS-REST `If-Match` → `412`).
    VersionMismatch,
    /// The requested operation is not implemented by this platform
    /// (ITS-REST `501`). Not an SM `CALL_STATUS_TYPE` member — an adapter
    /// affordance for optional/dev-branch routes and unimplemented seams;
    /// the wire maps it to `501 Not Implemented`.
    NotImplemented,
    /// A storage-layer conflict the request could not proceed past — a database
    /// integrity-constraint violation (SQLSTATE class 23) or a serialization/
    /// deadlock failure (40001 / 40P01). Not an SM `CALL_STATUS_TYPE` member —
    /// an adapter affordance (like [`Self::NotImplemented`]): the storage bridge
    /// classifies these from `sqlx` rather than the domain layer naming a
    /// specific conflict, and the wire maps it to `409 Conflict`
    /// ("the request could not be processed because it might generate a
    /// duplicate or a conflict", ITS-REST overview §HTTP status codes).
    Conflict,
    /// The service is temporarily unable to serve the request because a backend
    /// resource is exhausted (the `sqlx` connection pool acquire timed out under
    /// sustained load). Not an SM `CALL_STATUS_TYPE` member — an adapter
    /// affordance; no openEHR spec governs server overload (our own design, the
    /// admission contract), and the wire maps it to `503 Service Unavailable` +
    /// `Retry-After` (RFC 9110 §15.6.4; the ITS-REST status subset has no 503,
    /// so this is a documented extension).
    ServiceOverloaded,
    /// Processing of the addressed object is restricted, so the server holds
    /// it in storage and does nothing else with it. Not an SM
    /// `CALL_STATUS_TYPE` member — an adapter affordance (like
    /// [`Self::Conflict`]): no openEHR spec governs restriction of processing,
    /// and GDPR Art. 18(2) leaves storage as the only processing a restriction
    /// admits (`docs/law/eu/gdpr/text.html`). The wire maps it to `403
    /// Forbidden`, the status RFC 9110 §15.5.4 gives a request the server
    /// understood and refuses, and the body names the restriction.
    ProcessingRestricted,

    // ── EHR_CALL_STATUS_TYPE (`ehr_call_status_type.adoc`) ──────────────────
    /// COMPOSITION not found (per-variant meanings are blank in the source).
    CompositionDoesNotExist,
    /// CONTRIBUTION not found.
    ContributionDoesNotExist,
    /// COMPOSITION fails archetype/template conformance.
    CompositionArchetypeInvalid,
    /// `create_ehr_with_id` with an already-used EHR id.
    EhrCreateFailDuplicateId,
    /// COMPOSITION with this id already exists.
    CompositionAlreadyExists,
    /// An EHR for this subject already exists.
    EhrForSubjectAlreadyExists,

    // ── DEFINITION_CALL_STATUS_TYPE (`definition_call_status_type.adoc`) ────
    /// An invalid archetype was provided as a parameter.
    InvalidArchetype,
    /// An invalid template was provided as a parameter.
    InvalidTemplate,
    /// An invalid artefact was provided (meaning blank in the source; added
    /// by SM amendment 0.9.5 / SPECPR-293).
    InvalidArtefact,
    /// An invalid query was provided as a parameter.
    InvalidQuery,
    /// An invalid archetype identifier regex pattern was provided.
    InvalidIdPattern,
    /// A provided archetype identifier does not exist.
    ArtefactDoesNotExist,
    /// A provided template identifier does not exist.
    TemplateDoesNotExist,

    // ── prose-only error names (no vendored enum; see the type docs) ────────
    /// *(prose-only)* Referenced archetype/template identifiers unknown to
    /// the definitions service (`I_VALIDITY_CHECKER.definitions_valid`
    /// precondition failures on create/update calls).
    DefinitionUnknown,
    /// *(prose-only)* Content is not a valid RM instance
    /// (`I_VALIDITY_CHECKER.content_valid` precondition failures).
    ContentInvalid,
    /// *(prose-only)* A referenced VERSION does not exist
    /// (`I_EHR_DIRECTORY.get_directory_at_version`).
    VersionDoesNotExist,
    /// *(prose-only)* Subject identifier not found (`I_EHR_INDEX`).
    SubjectIdDoesNotExist,
    /// *(prose-only)* `VERSIONED_COMPOSITION` not found
    /// (`I_EHR_COMPOSITION.get_versioned_composition`).
    VersionedCompositionDoesNotExist,
}

impl CallStatusType {
    /// The SM abstract name of this status, exactly as the spec text spells
    /// it (`snake_case` enumeration literals).
    #[must_use]
    pub fn sm_name(self) -> &'static str {
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

/// The native error type — a `CALL_STATUS_TYPE` code, a message, and the
/// lower-level failure that caused them.
///
/// Realizes the SM `I_STATUS` protocol (`i_status.adoc`:
/// `last_call_failed()`/`last_call_status()`) in the stateless typed-`Result`
/// style the spec sanctions (`master02-overview.adoc` §Functional Style).
#[derive(Debug, Clone)]
pub struct SmError {
    /// The `CALL_STATUS_TYPE` code the failed call reports.
    pub status: CallStatusType,
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
    /// Attach one with [`SmError::with_source`].
    ///
    /// `Arc`, not `Box`, because this type is [`Clone`].
    source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl std::fmt::Display for SmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

// NOTE: hand-written, because `#[derive(thiserror::Error)]` over an
// `Option<Arc<dyn Error>>` yields the `Arc` WRAPPER as the source hop
// (<https://docs.rs/thiserror/2.0.18/thiserror/index.html>), not the real cause.
impl std::error::Error for SmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.source {
            Some(source) => Some(&**source),
            None => None,
        }
    }
}

impl SmError {
    /// Build a status-coded error with a message.
    #[must_use]
    pub fn new(status: CallStatusType, message: impl Into<String>) -> Self {
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
        Self::new(CallStatusType::PreconditionViolation, message)
    }

    /// `ehr_id_does_not_exist`.
    #[must_use]
    pub fn ehr_not_found(message: impl Into<String>) -> Self {
        Self::new(CallStatusType::EhrIdDoesNotExist, message)
    }

    /// `version_mismatch` — the optimistic-concurrency (`If-Match`) failure
    /// (→ `412`).
    #[must_use]
    pub fn version_mismatch(message: impl Into<String>) -> Self {
        Self::new(CallStatusType::VersionMismatch, message)
    }

    /// `exception` — a server-side fault (→ `500`).
    #[must_use]
    pub fn exception(message: impl Into<String>) -> Self {
        Self::new(CallStatusType::Exception, message)
    }

    /// Build the SM `CALL_STATUS` object for this failed call
    /// (`I_STATUS.last_call_status()` — the caller supplies the call identity
    /// the stateless style does not track).
    #[must_use]
    pub fn into_call_status(
        self,
        call_name: impl Into<String>,
        call_string: impl Into<String>,
    ) -> CallStatus {
        CallStatus {
            code: self.status,
            call_name: call_name.into(),
            call_string: call_string.into(),
            meaning: self.status.sm_name().to_owned(),
            message: self.message,
        }
    }
}

/// `CALL_STATUS` — "Object representing a call status" (`call_status.adoc`).
///
/// All five attributes are mandatory in the SM. Built on demand from a failed
/// call's [`SmError`] via [`SmError::into_call_status`] (the stateless
/// realization of `I_STATUS.last_call_status()`).
#[derive(Debug, Clone)]
pub struct CallStatus {
    /// Call status code for last call.
    pub code: CallStatusType,
    /// Name of call that this status documents.
    pub call_name: String,
    /// Full call, in stringified form, including arguments.
    pub call_string: String,
    /// Meaning of the result status.
    pub meaning: String,
    /// Error message text.
    pub message: String,
}

/// Sentinel prefix marking an `exception` [`SmError`] as a **query-execution
/// timeout** rather than a generic server fault — the "message-tagged" 408
/// seam.
///
/// The query chapter ([`super::query`]) aborts a query that overruns its
/// configured execution budget and returns
/// `SmError::exception(format!("{QUERY_TIMEOUT_TAG}{detail}"))`. The native SM
/// error model carries only a `CALL_STATUS_TYPE` and message with no timeout
/// status, so the protocol adapter recognises the tag, strips it and renders
/// `408 Request Timeout` (`Requests_and_responses.md` §HTTP status codes;
/// `responses/408_Query.yaml`). The tag is a control-char sentinel, so it can
/// never collide with a genuine message and is never shown to clients.
pub const QUERY_TIMEOUT_TAG: &str = "\u{1}query-execution-timeout\u{1}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_status_realizes_i_status_protocol() {
        // master03 §Representing Call Status: a failed call yields a
        // CALL_STATUS with all five attributes populated.
        let err = SmError::precondition("no such EHR");
        let status = err.into_call_status("create_ehr_with_id", "create_ehr_with_id(…)");
        assert_eq!(status.code, CallStatusType::PreconditionViolation);
        assert_eq!(status.meaning, "precondition_violation");
        assert_eq!(status.call_name, "create_ehr_with_id");
        assert_eq!(status.message, "no such EHR");
    }
}
