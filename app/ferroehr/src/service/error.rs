// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The service-layer error model, [`ServiceError`].
//!
//! A variant is the HTTP class of a refusal and its [`Refusal`] kind tells
//! refusals of one class apart. The protocol adapter (`ferroehr-rest`) maps it
//! onto the ITS-REST wire codes (ITS-REST overview §HTTP status codes); no
//! openEHR spec governs the service error model itself: our own design.

use crate::versioning::signature::signer::SignError;
use std::sync::Arc;

use openehr_base::validate::InvariantViolation;
use openehr_its::json::{JsonParseError, SchemaRoot};

use super::refusal::{Clash, Curated, Invalid, Malformed, Missing, Refusal};
use crate::storage::error::{classify_acquire, classify_sqlx, classify_storage};

/// One violated spec rule, carried as data rather than as a pre-formatted
/// sentence.
///
/// A refusal carries up to four independently useful facts: the RM attribute
/// path, the named invariant it breaks, what is wrong (`detail`), and the causes
/// a nested pass reported ([`InvariantViolation`]). They are rendered exactly
/// once, at the protocol edge, as
/// `[<path> ]<detail>[: <cause>; …][ (<invariant>)]`.
#[derive(Debug, Clone)]
pub struct Violation {
    /// The RM attribute path the violation is about (`ATTESTATION.items`).
    path: Option<String>,
    /// The named RM/spec invariant broken (`ATTESTATION.Items_valid`).
    invariant: Option<String>,
    /// What is wrong, WITHOUT the path/invariant/cause decoration.
    detail: String,
    /// Violations a nested machine pass reported (an RM class-invariant run, a
    /// canonical-JSON decode failure with its JSON path).
    causes: Vec<InvariantViolation>,
    /// The failure the refusal was raised FOR (a codec error, a template
    /// compile failure), reachable through [`std::error::Error::source`] and
    /// never part of the rendering above. Attach it with
    /// [`Violation::with_source`].
    source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl Violation {
    /// A violation stating `detail`, with no path, invariant or causes — the
    /// shape for a refusal whose only fact IS the sentence (a third-party
    /// parser's message, a shape rule with no named invariant).
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            path: None,
            invariant: None,
            detail: detail.into(),
            causes: Vec::new(),
            source: None,
        }
    }

    /// Attach the RM attribute path the violation is about.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Attach the name of the RM/spec invariant this violation breaks.
    #[must_use]
    pub fn with_invariant(mut self, invariant: impl Into<String>) -> Self {
        self.invariant = Some(invariant.into());
        self
    }

    /// Attach the violations a nested machine pass reported.
    #[must_use]
    pub fn with_causes(mut self, causes: Vec<InvariantViolation>) -> Self {
        self.causes = causes;
        self
    }

    /// Attach a canonical-JSON decode failure as the single cause, keeping its
    /// JSON path queryable ([`JsonParseError::path`]).
    #[must_use]
    pub fn with_decode_failure(mut self, error: &JsonParseError) -> Self {
        self.causes = vec![InvariantViolation::at(
            error.path().concat(),
            error.to_string(),
        )];
        self
    }

    /// The RM attribute path this violation is about, when it names one.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// The named RM/spec invariant this violation breaks, when it names one.
    #[must_use]
    pub fn invariant(&self) -> Option<&str> {
        self.invariant.as_deref()
    }

    /// What is wrong, without the path/invariant/cause decoration.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// The violations a nested machine pass reported.
    #[must_use]
    pub fn causes(&self) -> &[InvariantViolation] {
        &self.causes
    }

    /// Attach the failure that caused this refusal, leaving the rendering — and
    /// therefore the wire body — byte-identical.
    ///
    /// The cause is carried for the log and for anything walking the chain
    /// ([RFC 0201](https://rust-lang.github.io/rfcs/0201-error-chaining.html)).
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Arc::new(source));
        self
    }
}

/// Equality is over the FACTS a refusal states — path, invariant, detail and
/// the reported causes. The attached [`std::error::Error`] source is
/// diagnostic-only and no error type is comparable, so it takes no part.
impl PartialEq for Violation {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && self.invariant == other.invariant
            && self.detail == other.detail
            && self.causes == other.causes
    }
}

impl Eq for Violation {}

impl std::error::Error for Violation {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.source {
            Some(source) => Some(&**source),
            None => None,
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{path} ")?;
        }
        f.write_str(&self.detail)?;
        if !self.causes.is_empty() {
            let joined = self
                .causes
                .iter()
                .map(|c| c.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            write!(f, ": {joined}")?;
        }
        if let Some(invariant) = &self.invariant {
            write!(f, " ({invariant})")?;
        }
        Ok(())
    }
}

/// Service-layer error, mapped to the ITS-REST wire error at the protocol
/// boundary so the REST layer stays free of persistence concerns.
///
/// A refusal variant carries a [`Refusal`], whose kind tells refusals of one
/// HTTP class apart and whose source is the failure that caused it. A variant
/// that wraps one error is `#[error(transparent)]`, so a chain printer such as
/// [`ErrorChain`] reads every fact once
/// ([RFC 0201](https://rust-lang.github.io/rfcs/0201-error-chaining.html)). No
/// `Display` here is a wire body: both bridges below render the carried message
/// or violation themselves.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// The requested resource does not exist; the [`Missing`] kind says what
    /// was not found. Resources with no kind of their own (event
    /// subscriptions, item tags) use [`Missing::VersionedObject`].
    #[error(transparent)]
    NotFound(Refusal<Missing>),
    /// The request is malformed at the semantic level (e.g. a stale/invalid
    /// `preceding_version_uid`, or an operation on an already-deleted object) —
    /// ITS-REST `400 Bad Request` (`400_already_deleted.yaml`).
    ///
    /// The [`Malformed`] kind is the generic precondition
    /// ([`ServiceError::precondition`]) or the definition store's id pattern.
    #[error(transparent)]
    BadRequest(Refusal<Malformed>),
    /// The request conflicts with current state (e.g. EHR already exists) —
    /// ITS-REST `409 Conflict`.
    ///
    /// The [`Clash`] kind names the state it conflicts with, the generic
    /// storage-classified one included.
    #[error(transparent)]
    Conflict(Refusal<Clash>),
    /// Optimistic-concurrency precondition (`If-Match`) failed, ITS-REST `412`.
    #[error(transparent)]
    VersionConflict(Refusal),
    /// The submitted payload is malformed or fails a structural rule —
    /// carrying the violated rule as DATA ([`Violation`]: path, invariant,
    /// causes), rendered into prose only at the protocol edge.
    #[error("unprocessable content ({kind:?})")]
    Unprocessable {
        /// Which content the refusal is about: the generic
        /// [`Invalid::Content`] ([`ServiceError::content_invalid`]) or a
        /// definition kind.
        kind: Invalid,
        /// The violated rule, as data.
        #[source]
        violation: Violation,
    },
    /// A well-formed payload that fails semantic validation — template, RM,
    /// terminology, or the clinical-side data-minimisation policy
    /// ([`crate::privacy`]) — carrying the per-path violations as the RM
    /// validation data type ([`InvariantViolation`]), which the bridges render
    /// into the ITS-REST 422 body. The service layer never names a protocol
    /// type: the wire shape is chosen at the edge, not at the throw site.
    #[error("{} validation error(s)", .0.len())]
    ValidationFailed(Vec<InvariantViolation>),
    /// A storage/codec failure.
    #[error(transparent)]
    Storage(#[from] crate::storage::error::StorageError),
    /// A database failure.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// No database connection could be obtained, so nothing ran.
    #[error(transparent)]
    Acquire(#[from] crate::db::acquire::AcquireError),
    /// A JSON payload the server was asked to READ is malformed — a
    /// client-caused refusal (`400`) whose wire message NAMES the defect
    /// (offset, expected token), because that is the only thing the caller can
    /// act on. Constructed explicitly at a read site; never by `?`.
    #[error(transparent)]
    JsonRead(serde_json::Error),
    /// The server failed to serialize its OWN data, a `500` and the
    /// `From<serde_json::Error>` default: a serde diagnostic names Rust types
    /// no client can act on, so it goes to `tracing` and the body carries
    /// `INTERNAL_MESSAGE`.
    #[error(transparent)]
    JsonWrite(#[from] serde_json::Error),
    /// A version-signing or read-time integrity failure (RM common master06
    /// §Digital Signature); the carried text is a LOG detail, and both bridges
    /// below answer with `INTERNAL_MESSAGE`.
    #[error("signing: {0}")]
    Signing(String),
    /// The signer failed, with its cause intact as the source (RFC 0201:
    /// `Error::source` is part of the contract).
    ///
    /// A signing failure is a key or configuration problem, and the cause is
    /// what tells them apart. Both bridges below trace the chain and answer
    /// with `INTERNAL_MESSAGE`.
    #[error("signing a version failed")]
    SigningFailed(#[source] SignError),
    /// Canonicalisation failed on the signing or verification path, with the
    /// canonicaliser's error as the source.
    ///
    /// A different cause from a signer failure, and the distinction is the
    /// point: one is a malformed document, the other a key or configuration
    /// problem.
    #[error("canonicalising a version for its signature failed")]
    SigningCanonical(
        #[source] openehr_rm::v1_2::common::change_control::version_impl::CanonicalFormError,
    ),
    /// A server-side fault with no more specific variant.
    ///
    /// The carried message is a LOG detail: both bridges below trace it and
    /// answer with `exception` + `INTERNAL_MESSAGE`, as a `500` must disclose
    /// no internal error value. Construct with [`ServiceError::exception`], or
    /// [`ServiceError::internal`] to carry the failure that caused it.
    #[error(transparent)]
    Internal(Refusal),
    /// A `500` whose message is client-visible text a deployment acts on (no
    /// EHR index provider, a failed identity source); [`ServiceError::fault`].
    #[error(transparent)]
    Fault(Curated),
    /// A `503` the storage classifier raises when nothing the request needs ran
    /// or it rolled back; [`ServiceError::overloaded`].
    #[error(transparent)]
    Overloaded(Curated),
    /// The access record for a completed read or write could not be taken
    /// and the deployment fails closed (`[audit] fail_mode = "closed"`), so
    /// the result is withheld — the wire's `503`, `Retry-After` set, the same
    /// answer the REST layer gives when its own request record is rejected.
    ///
    /// Carries the refusal
    /// [`FerroEhrService::record_access`](crate::service::FerroEhrService::record_access)
    /// produced.
    #[error(transparent)]
    Unrecorded(Refusal),
    /// The object's processing is restricted, the wire's `403` (GDPR Art. 4(3),
    /// 18(2), `docs/law/eu/gdpr/text.html`; [`ServiceError::restricted`]). No
    /// openEHR spec governs it: our own design/extension; the message names
    /// the rule, which tells it from an authorization refusal.
    #[error(transparent)]
    Restricted(Refusal),
    /// A person may not make the change, the wire's `403` (EHDS Art. 5;
    /// [`ServiceError::not_permitted`]). No openEHR spec governs it: our own
    /// design/extension.
    #[error(transparent)]
    Forbidden(Refusal),
    /// The query outran its execution budget, the wire's `408` (`responses/408_Query.yaml`).
    #[error(transparent)]
    Timeout(Refusal),
}

/// The client-visible message of a server-side fault (`500`). Deliberately
/// opaque: a serde/codec/driver diagnostic names Rust types, RM attribute
/// names, SQL aliases and schema objects — server-internal detail the client
/// can neither act on nor be trusted with. The diagnosis belongs in the
/// server's own logs, so every 500-class body carries this sentence and the
/// detail rides one structured `tracing` record instead. (No openEHR spec
/// governs error-body wording beyond the `{ error, message }` shape —
/// ITS-REST overview §HTTP status codes; the opacity is our own design.)
pub(crate) const INTERNAL_MESSAGE: &str = "the server encountered an internal error";

/// A [`std::fmt::Display`] view of an error and its remaining source chain.
///
/// The hops are joined with `: `, the log rendering of a carried cause
/// ([RFC 0201](https://rust-lang.github.io/rfcs/0201-error-chaining.html)). For
/// a trace field only: a chain ends in whatever its innermost source knows (SQL
/// text, a DSN, an internal URL), which a 5xx response body must not disclose.
///
/// # Examples
///
/// ```
/// use ferroehr::service::error::ErrorChain;
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("outer")]
/// struct Outer(#[source] Inner);
/// #[derive(Debug, thiserror::Error)]
/// #[error("inner")]
/// struct Inner;
///
/// assert_eq!(ErrorChain::new(&Outer(Inner)).to_string(), "outer: inner");
/// ```
#[derive(Debug)]
pub struct ErrorChain<'a>(&'a (dyn std::error::Error + 'static));

impl<'a> ErrorChain<'a> {
    /// A chain view rooted at `error`.
    #[must_use]
    pub fn new(error: &'a (dyn std::error::Error + 'static)) -> Self {
        Self(error)
    }
}

impl std::fmt::Display for ErrorChain<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)?;
        let mut next = self.0.source();
        while let Some(cause) = next {
            write!(f, ": {cause}")?;
            next = cause.source();
        }
        Ok(())
    }
}

/// A `422` of `kind` whose [`Violation`] carries only its detail.
fn unprocessable(kind: Invalid, message: String) -> ServiceError {
    ServiceError::Unprocessable {
        kind,
        violation: Violation::new(message),
    }
}

impl ServiceError {
    /// A malformed-request refusal (`400`), the generic precondition.
    ///
    /// The message is client-visible: a `4xx` describes the caller's own
    /// request, so naming the defect there is the contract.
    #[must_use]
    pub fn precondition(message: impl Into<String>) -> Self {
        ServiceError::BadRequest(Refusal::new(Malformed::Precondition, message))
    }

    /// A `404` for a missing `missing`, with a client-visible message.
    #[must_use]
    pub fn not_found(missing: Missing, message: impl Into<String>) -> Self {
        ServiceError::NotFound(Refusal::new(missing, message))
    }

    /// A restriction refusal (`403`) naming the restricted object.
    ///
    /// The message says which object is restricted and that this is no
    /// authorization outcome: GDPR Art. 18(2) leaves storage as the only
    /// processing it admits (`docs/law/eu/gdpr/text.html`), so a caller that
    /// could not tell the two apart would retry against the access layer. No
    /// openEHR spec governs restriction of processing: our own design.
    #[must_use]
    pub fn restricted(subject: &str) -> Self {
        ServiceError::Restricted(Refusal::new(
            (),
            format!(
                "processing of {subject} is restricted: it is held in storage and is not \
                 otherwise processed until the restriction is lifted (GDPR Art. 18)"
            ),
        ))
    }

    /// A refusal (`403`) of a change a person may not make (EHDS Art. 5).
    #[must_use]
    pub fn not_permitted(message: impl Into<String>) -> Self {
        ServiceError::Forbidden(Refusal::new((), message))
    }

    /// A refusal (`403`) of a research-side disclosure the subject objected to.
    ///
    /// GDPR Art. 21(6) gives the subject a right to object to processing "for
    /// scientific or historical research purposes or statistical purposes
    /// pursuant to Article 89(1)" (`docs/law/eu/gdpr/text.html`). It shares the
    /// wire outcome and the status of the Art. 18 refusal because the server is
    /// doing the same thing in both cases — holding the data and declining to
    /// process it — and the message is what says which provision applies. A
    /// read of the same EHR for care is untouched: the objection reaches
    /// research processing, not the care record. No openEHR spec governs the
    /// objection — our own design/extension.
    #[must_use]
    pub fn research_objected(subject: &str) -> Self {
        ServiceError::Restricted(Refusal::new(
            (),
            format!(
                "the subject of {subject} has objected to research processing, so its content \
                 is not exported or published for secondary use (GDPR Art. 21(6))"
            ),
        ))
    }

    /// A state-conflict refusal (`409`) of the generic [`Clash::State`] kind:
    /// a duplicate template, a referenced-artefact delete, an extension
    /// surface.
    #[must_use]
    pub fn conflict(message: impl Into<String>) -> Self {
        ServiceError::Conflict(Refusal::new(Clash::State, message))
    }

    /// An optimistic-concurrency refusal (`412`) — the `version_mismatch`
    /// status.
    #[must_use]
    pub fn version_conflict(message: impl Into<String>) -> Self {
        ServiceError::VersionConflict(Refusal::new((), message))
    }

    /// A semantic refusal (`422`) of the generic [`Invalid::Content`] kind,
    /// carrying the violated rule as data.
    ///
    /// A `422` of a definition kind (an archetype or a template) names its
    /// `kind` directly.
    #[must_use]
    pub fn content_invalid(violation: Violation) -> Self {
        ServiceError::Unprocessable {
            kind: Invalid::Content,
            violation,
        }
    }

    /// Attribute a per-member refusal to its CONTRIBUTION version member.
    ///
    /// The commit route's whole purpose is a multi-member change set, so a
    /// member-scoped refusal names `versions[index]` the way the declared-key
    /// check always has: status rows prefix the message, violation
    /// rows prefix the path, validation rows prefix each violation's path.
    /// Rows that are never member-scoped pass through unchanged.
    #[must_use]
    pub(crate) fn for_version_member(self, index: usize) -> Self {
        let prefix = |message: &mut String| *message = format!("versions[{index}]: {message}");
        match self {
            ServiceError::NotFound(mut err) => {
                prefix(&mut err.message);
                ServiceError::NotFound(err)
            }
            ServiceError::BadRequest(mut err) => {
                prefix(&mut err.message);
                ServiceError::BadRequest(err)
            }
            ServiceError::Conflict(mut err) => {
                prefix(&mut err.message);
                ServiceError::Conflict(err)
            }
            ServiceError::VersionConflict(mut err) => {
                prefix(&mut err.message);
                ServiceError::VersionConflict(err)
            }
            ServiceError::Unprocessable {
                kind,
                mut violation,
            } => {
                violation.path = Some(match violation.path.take() {
                    Some(path) => format!("versions[{index}]/{path}"),
                    None => format!("versions[{index}]"),
                });
                ServiceError::Unprocessable { kind, violation }
            }
            ServiceError::ValidationFailed(mut violations) => {
                for violation in &mut violations {
                    violation.path = if violation.path.is_empty() {
                        format!("versions[{index}]")
                    } else {
                        format!("versions[{index}]/{}", violation.path)
                    };
                }
                ServiceError::ValidationFailed(violations)
            }
            other => other,
        }
    }

    /// A server-side fault (`500`) whose `detail` is a log detail, never a
    /// wire body; [`ServiceError::internal`] carries the failure as well.
    #[must_use]
    pub fn exception(detail: impl Into<String>) -> Self {
        ServiceError::Internal(Refusal::new((), detail))
    }

    /// A server-side fault (`500`) that carries the failure which caused it.
    ///
    /// `context` names the step that failed and is a LOG detail: the client
    /// body is the fixed internal message on both bridges, never this text and
    /// never the cause. The cause is reachable through
    /// [`std::error::Error::source`], which is what lets an operator read the
    /// `sqlx`/codec diagnosis a stringified cause destroys
    /// ([RFC 0201](https://rust-lang.github.io/rfcs/0201-error-chaining.html)).
    #[must_use]
    pub fn internal(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        ServiceError::Internal(Refusal::new((), context).with_source(source))
    }

    /// A malformed-request refusal (`400`) that carries the failure which
    /// caused it.
    ///
    /// `detail` is the client-visible text, identical to what
    /// [`ServiceError::precondition`] carries; the cause rides the source chain
    /// for the log and for callers that branch on it.
    #[must_use]
    pub fn bad_request(
        detail: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        ServiceError::BadRequest(Refusal::new(Malformed::Precondition, detail).with_source(source))
    }

    /// The refusal of a canonical-JSON `data` the strict typed reader could
    /// not decode: `422` when the published ITS-JSON schema admits it (it
    /// parses, and an RM rule such as a mandatory member is what fails),
    /// `400` when it does not ([`its_json_admits`]).
    #[must_use]
    pub fn canonical_decode(data: &serde_json::Value, error: JsonParseError) -> Self {
        if its_json_admits(data) {
            ServiceError::content_invalid(
                Violation::new(format!("content does not satisfy the RM: {error}"))
                    .with_decode_failure(&error)
                    .with_source(error),
            )
        } else {
            ServiceError::bad_request(format!("invalid canonical JSON body: {error}"), error)
        }
    }
}

impl ServiceError {
    /// The validation violations joined into one [`Invalid::Content`] detail:
    /// the flat `422` the party, `PARTY_RELATIONSHIP`, demographic CONTRIBUTION,
    /// EHR Extract import and archive load writes have always answered.
    #[must_use]
    pub fn flattened(self) -> Self {
        match self {
            ServiceError::ValidationFailed(v) => unprocessable(
                Invalid::Content,
                v.into_iter()
                    .map(|e| format!("{}: {}", e.path, e.message))
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
            other => other,
        }
    }
}

/// Whether the published ITS-JSON RM schema admits a canonical-JSON body the
/// strict typed reader refused, which makes the refusal an RM violation
/// (`422`) and not a parse failure (`400`).
///
/// NOTE: ITS-REST `Resources.md` §JSON Format names the ITS-JSON schemas as the
/// canonical JSON a payload "SHOULD be valid against"; `responses/422.yaml` is content that parses.
///
/// The body must name its `_type` and declare no key its RM class lacks, the
/// two facts the schema root cannot judge.
#[must_use]
pub fn its_json_admits(data: &serde_json::Value) -> bool {
    data.get("_type")
        .and_then(serde_json::Value::as_str)
        .is_some()
        && openehr_its::json::reject_undeclared_keys(data).is_ok()
        && openehr_its::json::validate_canonical(data, SchemaRoot::EveryClass).is_ok()
}

impl ServiceError {
    /// The same failure with a storage, driver or acquire failure classified
    /// into the refusal it answers: a `409` conflict, an
    /// [`Overloaded`](ServiceError::Overloaded) `503` or a curated fault. Every
    /// other variant is returned as it is.
    #[must_use]
    pub fn classified(self) -> Self {
        match self {
            ServiceError::Storage(e) => classify_storage(e),
            ServiceError::Database(e) => classify_sqlx(e),
            ServiceError::Acquire(e) => classify_acquire(e),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use openehr_base::validate::InvariantViolation;

    use super::{ServiceError, Violation};

    /// A [`Violation`] renders `[<path> ]<detail>[: <cause>; …][ (<invariant>)]`
    /// — the ONE place the facts become prose. Each fact is independently
    /// readable back off the value, which is what the refusal tests assert.
    #[test]
    fn violation_renders_its_facts_and_keeps_them_queryable() {
        let plain = Violation::new("contribution must contain versions");
        assert_eq!(plain.to_string(), "contribution must contain versions");
        assert_eq!(plain.path(), None);
        assert_eq!(plain.invariant(), None);
        assert_eq!(plain.causes(), []);

        let rule = Violation::new("must be a non-empty list when present")
            .with_path("ATTESTATION.items")
            .with_invariant("ATTESTATION.Items_valid");
        assert_eq!(
            rule.to_string(),
            "ATTESTATION.items must be a non-empty list when present \
             (ATTESTATION.Items_valid)"
        );
        assert_eq!(rule.path(), Some("ATTESTATION.items"));
        assert_eq!(rule.invariant(), Some("ATTESTATION.Items_valid"));

        let nested = Violation::new("is not a valid PARTY_PROXY")
            .with_path("AUDIT_DETAILS.committer")
            .with_causes(vec![
                InvariantViolation::here("Invariant Basic_validity failed"),
                InvariantViolation::at("name", "Invariant Name_valid failed"),
            ]);
        assert_eq!(
            nested.to_string(),
            "AUDIT_DETAILS.committer is not a valid PARTY_PROXY: \
             Invariant Basic_validity failed; Invariant Name_valid failed"
        );
        assert_eq!(nested.causes().len(), 2);
        assert_eq!(nested.causes()[1].path, "name");
    }

    /// A carried cause stays WALKABLE — [`std::error::Error::source`] reaches
    /// the concrete underlying error type, which is the whole point of RFC 0201
    /// chaining. The protocol adapter's tests pin that it never reaches a body.
    #[test]
    fn a_carried_cause_is_walkable() {
        use std::error::Error;

        let err = ServiceError::internal("read a stored version row", cause());
        let first = Error::source(&err).expect("a carried cause must be reachable");

        let mut hops = Vec::new();
        let mut found = None;
        let mut next = Some(first);
        while let Some(step) = next {
            hops.push(step.to_string());
            if let Some(io) = step.downcast_ref::<std::io::Error>() {
                found = Some(io.kind());
            }
            next = step.source();
        }
        assert_eq!(
            found,
            Some(std::io::ErrorKind::PermissionDenied),
            "walking the chain must reach the underlying std::io::Error, got {hops:?}"
        );
    }

    /// A [`Violation`] with a carried cause renders EXACTLY as one without —
    /// the 422 body is the refusal's own facts, and the cause is chain-only.
    #[test]
    fn a_violation_cause_is_chain_only() {
        use std::error::Error;

        let plain =
            Violation::new("is not a valid PARTY_PROXY").with_path("AUDIT_DETAILS.committer");
        let carried = Violation::new("is not a valid PARTY_PROXY")
            .with_path("AUDIT_DETAILS.committer")
            .with_source(cause());
        assert_eq!(carried.to_string(), plain.to_string());
        assert_eq!(carried, plain, "equality is over the facts, not the cause");
        assert!(Error::source(&plain).is_none());
        // The hop must be the CONCRETE cause, not a smart-pointer wrapper
        // around it — otherwise no caller can branch on what actually failed.
        assert_eq!(
            Error::source(&carried)
                .and_then(|c| c.downcast_ref::<std::io::Error>())
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
    }

    /// A stand-in underlying failure whose `Display` is exactly the kind of
    /// internal detail a `500` body must never carry.
    fn cause() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the node table is not readable",
        )
    }

    /// A signing failure reaches its concrete cause; the protocol adapter's
    /// tests pin that it still renders as a server fault.
    ///
    /// Downcasting rather than asserting `source().is_some()` is deliberate:
    /// `Display` forwards either way, so a chain that is broken at the seam
    /// reads correctly in a log while `downcast_ref` returns `None` — the exact
    /// silent loss `reliability.md` records, and the only thing that catches it.
    #[test]
    fn a_signing_failure_reaches_its_concrete_cause() {
        use std::error::Error;

        let pgp =
            crate::versioning::signature::key::PgpSignError::from(pgp::errors::Error::Message {
                message: "no secret key".to_owned(),
                backtrace: None,
            });
        let signing =
            ServiceError::SigningFailed(crate::versioning::signature::signer::SignError::Pgp(pgp));
        assert!(
            !signing.to_string().contains("no secret key"),
            "the cause is the source, not part of the message: {signing}"
        );

        // Walked to the ROOT cause, not to a wrapper. `SignError::Pgp` is
        // `#[error(transparent)]`, and transparent forwards `source()` as well
        // as `Display` — so the wrapper is not a hop in the chain at all, and a
        // test looking for it would fail while the chain was perfectly intact.
        let mut found = false;
        let mut next = Error::source(&signing);
        while let Some(step) = next {
            if step.downcast_ref::<pgp::errors::Error>().is_some() {
                found = true;
            }
            next = step.source();
        }
        assert!(
            found,
            "a signing failure must keep the underlying OpenPGP error reachable, or a \
             caller cannot tell a key problem from a configuration problem without \
             parsing prose"
        );
    }
}
