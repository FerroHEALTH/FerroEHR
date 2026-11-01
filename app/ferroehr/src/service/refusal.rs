// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The payload of a [`ServiceError`](super::error::ServiceError) refusal, and
//! the kinds that tell refusals of one HTTP class apart.
//!
//! No openEHR spec governs the service error model: our own design. A variant
//! of `ServiceError` is one HTTP class; a kind exists only where a caller or a
//! test reads which refusal of that class it got.

use std::sync::Arc;

/// A refusal: what the client reads, which refusal of its class it is, and
/// the failure that caused it.
///
/// The cause is kept out of [`Self::message`] and never interpolated into it:
/// the message reaches response bodies, and a `500`-class body must disclose no
/// internal error value (OWASP REST Security Cheat Sheet §Error handling). It
/// is reachable through [`std::error::Error::source`]
/// ([RFC 0201](https://rust-lang.github.io/rfcs/0201-error-chaining.html)).
#[derive(Debug, Clone)]
pub struct Refusal<K = ()> {
    /// Which refusal of its HTTP class this is.
    pub kind: K,
    /// The client-visible message.
    pub message: String,
    /// The failure this refusal was raised for. `Arc`, because the type is
    /// [`Clone`].
    source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl<K> Refusal<K> {
    /// A refusal of `kind` with `message` and no cause.
    #[must_use]
    pub fn new(kind: K, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    /// Attach the failure that caused this refusal, keeping the message, and
    /// therefore the wire body, unchanged.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Arc::new(source));
        self
    }

    /// The same refusal with an already shared cause.
    #[must_use]
    pub(crate) fn with_shared_source(
        mut self,
        source: Option<Arc<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        self.source = source;
        self
    }

    /// The shared cause, for a conversion that keeps it.
    #[must_use]
    pub(crate) fn shared_source(&self) -> Option<Arc<dyn std::error::Error + Send + Sync>> {
        self.source.clone()
    }
}

impl<K> std::fmt::Display for Refusal<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

// NOTE: hand-written, because `#[derive(thiserror::Error)]` over an
// `Option<Arc<dyn Error>>` yields the `Arc` wrapper as the source hop
// (<https://docs.rs/thiserror/2.0.18/thiserror/index.html>), not the real cause.
impl<K: std::fmt::Debug> std::error::Error for Refusal<K> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.source {
            Some(source) => Some(&**source),
            None => None,
        }
    }
}

/// The payload of a [`Fault`](super::error::ServiceError::Fault).
///
/// The message of this `500` reaches the response body, so it is curated
/// text fixed at the raise site and never a cause's `Display`, SQL, a path or
/// a payload value. The cause rides [`std::error::Error::source`] only.
#[derive(Debug, Clone)]
pub struct Curated {
    /// The client-visible message.
    pub message: &'static str,
    /// The failure this fault was raised for.
    source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl Curated {
    /// The cause, for a conversion that keeps it.
    pub(crate) fn shared_source(&self) -> Option<Arc<dyn std::error::Error + Send + Sync>> {
        self.source.clone()
    }
}

impl std::fmt::Display for Curated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}

// NOTE: hand-written for the reason given on `Refusal`'s impl.
impl std::error::Error for Curated {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.source {
            Some(source) => Some(&**source),
            None => None,
        }
    }
}

/// What a `404` did not find.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Missing {
    /// A version of a versioned object.
    ObjectVersion,
    /// A versioned object, or a resource with no kind of its own.
    VersionedObject,
    /// An EHR.
    Ehr,
    /// A party.
    Party,
    /// A COMPOSITION.
    Composition,
    /// A CONTRIBUTION.
    Contribution,
    /// An archetype or other definition artefact.
    Artefact,
    /// A template.
    Template,
    /// A VERSION.
    Version,
    /// A subject in the EHR index.
    Subject,
    /// A `VERSIONED_COMPOSITION`.
    VersionedComposition,
}

/// Which malformed request a `400` refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Malformed {
    /// An argument-validity precondition failed.
    Precondition,
    /// An archetype identifier pattern is not a valid regex.
    IdPattern,
}

/// Which state a `409` clashes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Clash {
    /// An EHR with the requested id exists.
    DuplicateEhrId,
    /// A COMPOSITION with this id exists.
    CompositionExists,
    /// An EHR for this subject exists.
    EhrForSubject,
    /// Any other state conflict, the storage-classified one included.
    State,
}

/// Which content a `422` refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invalid {
    /// The content is not a valid RM instance.
    Content,
    /// An archetype.
    Archetype,
    /// A template.
    Template,
    /// A definition artefact.
    Artefact,
    /// A query.
    Query,
    /// Archetype or template identifiers the definitions store does not know.
    DefinitionUnknown,
    /// A COMPOSITION fails archetype or template conformance.
    CompositionArchetype,
}

impl super::error::ServiceError {
    /// A `500` whose curated `message` reaches the body
    /// ([`Fault`](super::error::ServiceError::Fault)).
    #[must_use]
    pub fn fault(message: &'static str) -> Self {
        Self::Fault(Curated {
            message,
            source: None,
        })
    }

    /// A [`Fault`](super::error::ServiceError::Fault) that carries the failure
    /// which caused it as its source, never in its message.
    #[must_use]
    pub fn fault_caused(
        message: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Fault(Curated {
            message,
            source: Some(Arc::new(source)),
        })
    }

    /// The client-visible text of this error: a refusal's message, the
    /// rendered violation of a `422`, the joined validation violations, and the
    /// `Display` text of every other variant.
    #[must_use]
    pub fn message(&self) -> String {
        use super::error::ServiceError as E;
        match self {
            E::NotFound(r) => r.message.clone(),
            E::BadRequest(r) => r.message.clone(),
            E::Conflict(r) => r.message.clone(),
            E::VersionConflict(r)
            | E::Internal(r)
            | E::Unrecorded(r)
            | E::Restricted(r)
            | E::Forbidden(r) => r.message.clone(),
            E::Fault(f) => f.message.to_owned(),
            E::Unprocessable { violation, .. } => violation.to_string(),
            E::ValidationFailed(v) => v
                .iter()
                .map(|e| format!("{}: {}", e.path, e.message))
                .collect::<Vec<_>>()
                .join("; "),
            other => other.to_string(),
        }
    }
}
