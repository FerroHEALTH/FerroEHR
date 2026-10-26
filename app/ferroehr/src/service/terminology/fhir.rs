// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`FhirTerminologyProvider`], the SM `I_TERMINOLOGY_SERVICE` calls realized
//! against the one FHIR R4B terminology server through the published
//! `ferrobridge-term` client.
//!
//! The one terminology server (`[terminology.server]`) is what the routing
//! layer (`super::routing`) asks for every terminology the in-process
//! `openehr-term` bundle (`super::bundle`) does not carry. BASE
//! `architecture_overview/master12-terminology.adoc` models this backend as an
//! external "terminology query server". SM contract:
//! `docs/specs/openehr/SM/docs/UML/classes/i_terminology_service.adoc`; the
//! client and the server it points at are our own design. `ferrobridge-term`
//! owns the HTTP exchange and the FHIR decoding over the generated `fhir-types`
//! operation contracts (<https://docs.rs/ferrobridge-term/0.1.130>); this
//! module owns the SM error mapping and the extract shape (the `fhir` cargo
//! feature; a slim build refuses a configured server at boot).
//!
//! # SM call → FHIR operation mapping
//!
//! | SM call | FHIR R4B operation |
//! |---|---|
//! | `value_set_validate` | `ValueSet/$validate-code` |
//! | `get_value_set` | `ValueSet/$expand` → [`TerminologyExtract`] |
//! | `subsumes` | `CodeSystem/$subsumes` (outcome `subsumes`) |
//! | `has_term` / `get_term` | `CodeSystem/$lookup` |
//!
//! The enumerating calls (`get_terminology_ids`, `get_terminology_description`,
//! `has_terminology`) have no faithful FHIR operation, so the routing layer
//! answers them from the in-process bundle and
//! [`FhirTerminologyProvider::get_terminology_description`] is an explicit
//! `NotImplemented`. The SM `at_date` (an `Iso8601_date` on
//! `has_term`/`get_term`/`value_set_validate`) is sent as the FHIR `date` input
//! of `$lookup` and `$validate-code`.
//!
//! A `ValueSet/$expand` may nest members under `contains`. The flat
//! `Terminology_extract._terms_` keeps the membership view and
//! `Terminology_extract._relationships_` preserves the tree as
//! `Term_relationship`s under the `CHILD_RELATION` name
//! (`terminology_extract.adoc` §Structured value set), defined in `_relations_`
//! by the FHIR `child` concept property URI (`FHIR_CHILD_PROPERTY`), an
//! `external_code` relation (`terminology_relation.adoc`
//! `Inv_valid_definition`).
//!
//! NOTE: a value set, terminology or code the server does not know (HTTP `404`,
//! or `$validate-code result=false`) is a `Pre_has_*` precondition failure
//! ([`CallStatusType::VersionedObjectDoesNotExist`]) and a transport fault is
//! [`SmError::exception`]; the fail-open choice belongs to the caller
//! ([`config::TerminologyServerConfig::fail_on_error`](super::config::TerminologyServerConfig::fail_on_error)).

use std::collections::BTreeMap;

use crate::service::status::{CallStatusType, SmError};
use crate::service::terminology::types::{
    DefinedTerm, TermEntry, TermRelationship, TerminologyDescription, TerminologyExtract,
    TerminologyRelation,
};

use super::config::TerminologyServerConfig;

/// The configuration key the client-credentials grant sits under, named in
/// every refusal of it.
#[cfg(feature = "fhir")]
const OAUTH_KEY: &str = "terminology.server.oauth";

/// The `Term_relationship.relation_name` under which a FHIR `$expand`
/// parent→child `contains` nesting is preserved (`terminology_extract.adoc`).
const CHILD_RELATION: &str = "child";
/// The FHIR R4B concept-property URI that defines the parent→child relation,
/// carried as the `Terminology_relation.external_code`
/// (`terminology_relation.adoc`).
const FHIR_CHILD_PROPERTY: &str = "http://hl7.org/fhir/concept-properties#child";

/// The `ferrobridge-term` client of the one server.
#[cfg(feature = "fhir")]
type Wire = ferrobridge_term::client::Client;

/// A slim build holds no client: [`FhirTerminologyProvider::new`] refuses a
/// configured server, so no provider value exists.
#[cfg(not(feature = "fhir"))]
#[derive(Debug, Clone)]
enum Wire {}

/// A FHIR R4B terminology-server client realizing the SM
/// `I_TERMINOLOGY_SERVICE` lookup/validation calls against the one configured
/// server.
#[derive(Debug, Clone)]
pub struct FhirTerminologyProvider {
    /// The client of the server, carrying the timeout and the credentials.
    wire: Wire,
    /// The base URL with any userinfo masked, the server's name on the trace
    /// record.
    endpoint: String,
    /// Whether a binding this server cannot resolve refuses the composition
    /// ([`TerminologyServerConfig::fail_on_error`]).
    fail_on_error: bool,
}

/// Why an operation answered "not found", read from its `OperationOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "fhir"),
    expect(
        dead_code,
        reason = "a slim build sends no request, so nothing is absent"
    )
)]
enum Absence {
    /// The server does not serve the code system asked about (`not-found`).
    UnknownSystem,
    /// The code system is served and the code is not in it (`invalid-code`).
    UnknownCode,
    /// A `404`/`410` with no classifying issue: the resource is unknown, and
    /// whether that is the system or the code is not stated.
    Unclassified,
}

/// The three answers to "does this code exist in this code system": the
/// distinction VETDF turns on (a terminology the server does not serve is a
/// warning, not a refusal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermExistence {
    /// The server serves the code system and knows the code.
    Exists,
    /// The server serves the code system and the code is not in it.
    Absent,
    /// No verification was possible; the reason, for the operator's log.
    Unverifiable(&'static str),
}

impl FhirTerminologyProvider {
    /// Build the client from `[terminology.server]`, with the
    /// client-credentials token source attached when `oauth` is set.
    ///
    /// # Errors
    ///
    /// [`SmError::exception`] if the URL is empty (after trimming) or not a
    /// URL, the `oauth` grant is incomplete or refused by the token client, or
    /// the HTTP client cannot be built (e.g. TLS backend init failure).
    #[cfg_attr(
        not(feature = "fhir"),
        expect(
            unused_variables,
            reason = "a slim build refuses the server before reading its configuration"
        )
    )]
    pub fn new(cfg: &TerminologyServerConfig) -> Result<Self, SmError> {
        // A slim build carries no FHIR client, so a configured server is
        // refused loudly rather than silently ignored.
        #[cfg(not(feature = "fhir"))]
        return Err(SmError::exception(
            "terminology.server is enabled, but this binary was built without the `fhir` cargo \
             feature (the in-process bundle remains the terminology)",
        ));
        #[cfg(feature = "fhir")]
        Ok(Self {
            wire: wire::connect(cfg)?,
            endpoint: cfg.url.redacted(),
            fail_on_error: cfg.fail_on_error,
        })
    }

    /// Whether a binding this server cannot resolve refuses the composition
    /// (fail-closed) or is tolerated (fail-open), the deployment posture of
    /// register AMB-172.
    #[must_use]
    pub fn fail_on_error(&self) -> bool {
        self.fail_on_error
    }

    /// Whether `code` exists in the code system `system`, as three answers
    /// rather than two: exists, is absent from a system the server serves, or
    /// could not be verified because the server does not serve the system (or
    /// could not say). The ADL2 VETDF check needs the third answer, because an
    /// inaccessible terminology is a warning there and not a refusal (AM ADL2
    /// `master03-archetype_package.adoc` §Validity Rules, VETDF).
    ///
    /// Reads `CodeSystem/$lookup`. A bare `404` that names neither cause is
    /// resolved with one more question, `CodeSystem?url=<system>&_summary=count`:
    /// a served system makes the code absent, an unserved one makes the
    /// binding unverifiable, and a server that cannot answer the question
    /// leaves it unverifiable too.
    ///
    /// # Errors
    ///
    /// Exception on a transport fault, a non-2xx status carrying no
    /// classifying `OperationOutcome` and not `404`/`410`, or a malformed body.
    pub async fn term_existence(&self, system: &str, code: &str) -> Result<TermExistence, SmError> {
        match self.lookup(system, code, None).await? {
            Ok(_) => Ok(TermExistence::Exists),
            Err(Absence::UnknownCode) => Ok(TermExistence::Absent),
            Err(Absence::UnknownSystem) => Ok(TermExistence::Unverifiable(
                "the terminology server does not serve the code system",
            )),
            // NOTE: AM ADL2 master03 §Validity Rules VETDF, a terminology the
            // tool cannot reach is a warning, so a failed probe is unverifiable.
            Err(Absence::Unclassified) => match self.code_system_count(system).await {
                Ok(n) if n > 0 => Ok(TermExistence::Absent),
                Ok(_) => Ok(TermExistence::Unverifiable(
                    "the terminology server does not serve the code system",
                )),
                Err(_) => Ok(TermExistence::Unverifiable(
                    "the terminology server could not say whether it serves the code system",
                )),
            },
        }
    }

    /// A `500` for an upstream-terminology fault, with the OPERATOR detail
    /// (which server, which operation, the upstream diagnostic) on the trace
    /// record and NOT on the wire.
    ///
    /// The detail here is the deployment's own configuration — the server's
    /// address and the upstream server's error — which is exactly the
    /// class of fact a client must not be able to read out of a
    /// response body, while the operator still needs it in full. The server
    /// carries no per-request diagnostic surface (`/management` mounts
    /// info/prometheus/metrics/env/loggers only), so the trace record IS the
    /// operator channel. (No openEHR spec governs the content of a `500` body
    /// — our own design/extension; the status itself is the ITS-REST overview
    /// `Requests_and_responses.md` §HTTP status codes row.)
    #[cfg_attr(
        not(feature = "fhir"),
        expect(
            dead_code,
            reason = "a slim build sends no request, so nothing can fault"
        )
    )]
    fn provider_fault(&self, operation: &str, detail: &dyn std::fmt::Display) -> SmError {
        tracing::error!(
            server = %self.endpoint,
            operation,
            error = %detail,
            "terminology server fault → 500"
        );
        SmError::exception(crate::service::error::INTERNAL_MESSAGE.to_owned())
    }

    /// A `Pre_has_*` precondition failure (`VersionedObjectDoesNotExist`).
    ///
    /// The body carries only what the client can act on, the kind and id it
    /// asked about; which server answered is deployment configuration and goes
    /// to the trace record only.
    fn not_found(&self, what: &str, id: &str) -> SmError {
        tracing::debug!(server = %self.endpoint, what, id, "terminology lookup: not found");
        SmError::new(
            CallStatusType::VersionedObjectDoesNotExist,
            format!("{what} '{id}' not found"),
        )
    }

    /// `value_set_validate` → FHIR `ValueSet/$validate-code`.
    ///
    /// `terminology_id` is passed as the FHIR `system`; `value_set_id` as the
    /// value-set `url`; `at_date` as the FHIR `date`. A known value set
    /// with a non-member code → `Ok(false)`.
    ///
    /// # Errors
    ///
    /// Precondition on an empty `candidate_code`;
    /// `VersionedObjectDoesNotExist` on an unknown value set (`404`);
    /// exception on a transport fault or a `$validate-code` response with no
    /// `result` parameter.
    pub async fn value_set_validate(
        &self,
        terminology_id: &str,
        value_set_id: &str,
        candidate_code: &str,
        at_date: Option<String>,
    ) -> Result<bool, SmError> {
        if candidate_code.is_empty() {
            return Err(SmError::precondition("candidate_code must not be empty"));
        }
        self.validate(value_set_id, terminology_id, candidate_code, at_date)
            .await?
            .map_err(|_absence| self.not_found("value set", value_set_id))
    }

    /// `get_value_set` → FHIR `ValueSet/$expand`, mapped to a
    /// [`TerminologyExtract`] (flat `terms` for membership; the `contains`
    /// tree preserved as `relationships`). `terminology_id` is
    /// unused — the value set is identified by its URL (`value_set_code`).
    ///
    /// # Errors
    ///
    /// `VersionedObjectDoesNotExist` when `$expand` answers `404`; exception
    /// on a transport fault / non-2xx / malformed response.
    pub async fn get_value_set(
        &self,
        _terminology_id: &str,
        value_set_code: &str,
    ) -> Result<TerminologyExtract, SmError> {
        let expansion = self
            .expand(value_set_code)
            .await?
            .map_err(|_absence| self.not_found("value set", value_set_code))?;
        Ok(expansion.into_extract(value_set_code))
    }

    /// `subsumes` → FHIR `CodeSystem/$subsumes` (`codeA` = `ref_code`,
    /// `codeB` = `candidate_child_code`). True iff the outcome is `subsumes`
    /// — the SM's *strict* subsumption (`equivalent` is excluded).
    ///
    /// # Errors
    ///
    /// `VersionedObjectDoesNotExist` when the server answers `404` for the
    /// terminology; exception on a transport fault or a response with no
    /// `outcome` parameter.
    pub async fn subsumes(
        &self,
        terminology_id: &str,
        ref_code: &str,
        candidate_child_code: &str,
    ) -> Result<bool, SmError> {
        self.strictly_subsumes(terminology_id, ref_code, candidate_child_code)
            .await?
            .map_err(|_absence| self.not_found("terminology", terminology_id))
    }

    /// `has_term` → FHIR `CodeSystem/$lookup`: `true` when the lookup
    /// resolves (`200`), `false` when the code is unknown (`404`). `at_date`
    /// → the FHIR `date` parameter.
    ///
    /// # Errors
    ///
    /// Exception on a transport fault, any non-2xx status other than
    /// `404`/`410`, or a malformed response body.
    pub async fn has_term(
        &self,
        terminology_id: &str,
        code: &str,
        at_date: Option<String>,
    ) -> Result<bool, SmError> {
        Ok(self
            .lookup(terminology_id, code, at_date.as_deref())
            .await?
            .is_ok())
    }

    /// `get_term` → FHIR `CodeSystem/$lookup`, mapped to a single-term
    /// [`TerminologyExtract`] (the `display` becomes the term text). `at_date`
    /// → the FHIR `date` parameter.
    ///
    /// NOTE (attributes): the SM `attributes` allow-list filters
    /// the meta-model attributes returned. `$lookup` returns only the concept
    /// `display` (mapped to the term text), so there is nothing further to
    /// filter; `attributes` is accepted and has no effect on the returned
    /// extract.
    ///
    /// # Errors
    ///
    /// `VersionedObjectDoesNotExist` when `$lookup` answers `404` (unknown
    /// term); exception on a transport fault / non-2xx / malformed response,
    /// a lookup answer without its `name` or `display` among them.
    pub async fn get_term(
        &self,
        terminology_id: &str,
        code: &str,
        _attributes: Option<BTreeMap<String, String>>,
        at_date: Option<String>,
    ) -> Result<TerminologyExtract, SmError> {
        let text = self
            .lookup(terminology_id, code, at_date.as_deref())
            .await?
            .map_err(|_absence| self.not_found("term", code))?;
        let mut terms = BTreeMap::new();
        terms.insert(
            code.to_owned(),
            TermEntry::Defined(DefinedTerm {
                code: code.to_owned(),
                text,
                language: None,
                is_preferred_term: None,
            }),
        );
        Ok(TerminologyExtract {
            terminology_id: terminology_id.to_owned(),
            terminology_version: None,
            terms: Some(terms),
            relationships: None,
            relations: None,
        })
    }

    /// `has_value_set` → FHIR `ValueSet/$expand`: `true` when the value set
    /// expands (`200`), `false` when it is unknown (`404`). `terminology_id`
    /// is unused — the value set is identified by its URL.
    ///
    /// # Errors
    ///
    /// Exception on a transport fault, any non-2xx status other than
    /// `404`/`410`, or a malformed response body.
    pub async fn has_value_set(
        &self,
        _terminology_id: &str,
        value_set_code: &str,
    ) -> Result<bool, SmError> {
        Ok(self.expand(value_set_code).await?.is_ok())
    }

    /// `get_terminology_description` → not modelled for a FHIR TS (NOTE
    /// at module head; the routing layer answers this from the bundle).
    ///
    /// # Errors
    ///
    /// Always [`CallStatusType::NotImplemented`].
    #[expect(
        clippy::unused_self,
        reason = "the SM interface declares this call on the service; the \
                  protocol adapter invokes every SM call uniformly, so the \
                  receiver stays even where this realization ignores it"
    )]
    pub fn get_terminology_description(
        &self,
        _terminology_id: &str,
    ) -> Result<TerminologyDescription, SmError> {
        Err(SmError::new(
            CallStatusType::NotImplemented,
            "a FHIR terminology server does not expose terminology descriptions",
        ))
    }
}

/// A `$expand` expansion reduced to the extract's membership + hierarchy.
#[derive(Debug, Clone, Default)]
struct Expansion {
    /// Every member with a code, flattened (the membership view).
    terms: BTreeMap<String, TermEntry>,
    /// One `Term_relationship` per member that nests children.
    relationships: Vec<TermRelationship>,
}

impl Expansion {
    /// Map the expansion into a [`TerminologyExtract`]: the flat `terms` map
    /// keyed by code (the membership view) plus, when the expansion nests
    /// members, the parent→child tree preserved as `relationships`.
    fn into_extract(self, terminology_id: &str) -> TerminologyExtract {
        // The `child` relation is defined by the FHIR concept-property URI
        // (an `external_code` relation; `terminology_relation.adoc`
        // `Inv_valid_definition`), present only when a hierarchy was emitted.
        let relations = (!self.relationships.is_empty()).then(|| {
            let mut m = BTreeMap::new();
            m.insert(
                CHILD_RELATION.to_owned(),
                TerminologyRelation::external(CHILD_RELATION, FHIR_CHILD_PROPERTY),
            );
            m
        });
        TerminologyExtract {
            terminology_id: terminology_id.to_owned(),
            terminology_version: None,
            terms: if self.terms.is_empty() {
                None
            } else {
                Some(self.terms)
            },
            relationships: if self.relationships.is_empty() {
                None
            } else {
                Some(self.relationships)
            },
            relations,
        }
    }
}

/// The operations on the server, each answering a value, a classified
/// [`Absence`], or an [`SmError`] for a fault.
#[cfg(feature = "fhir")]
mod wire {
    use std::sync::Arc;
    use std::time::Duration;

    use ferrobridge_term::client::Client;
    use ferrobridge_term::config::{Config, Credentials, RetryPolicy, WireVersion};
    use ferrobridge_term::error::{Error, UpstreamError};
    use ferrobridge_term::outcome::{
        Coded, ExpandOutcome, ExpansionContains, LookupOutcome, SubsumesOutcome, Subsumption,
        ValidateOutcome, Validation,
    };
    use ferrobridge_term::smart::SmartToken;
    use ferrohealth_smart::client::cache::TokenSource;
    use http::StatusCode;

    use super::{
        Absence, CHILD_RELATION, Expansion, FhirTerminologyProvider, OAUTH_KEY,
        TerminologyServerConfig,
    };
    use crate::service::error::ErrorChain;
    use crate::service::status::SmError;
    use crate::service::terminology::types::{DefinedTerm, TermCode, TermEntry, TermRelationship};

    /// The `tx-issue-type` code for "the code system is not known to the
    /// server" (<https://build.fhir.org/ig/HL7/fhir-tools-ig/CodeSystem-tx-issue-type.html>).
    const TX_ISSUE_NOT_FOUND: &str = "not-found";

    /// The `tx-issue-type` code for "the code is not in the code system".
    const TX_ISSUE_INVALID_CODE: &str = "invalid-code";

    /// Builds the client of the server `cfg` names.
    pub(super) fn connect(cfg: &TerminologyServerConfig) -> Result<Client, SmError> {
        let raw = cfg.url.expose().trim();
        if raw.is_empty() {
            return Err(SmError::exception(
                "terminology.server.url must not be empty",
            ));
        }
        let base = url::Url::parse(raw).map_err(|e| {
            SmError::exception("terminology.server.url is not a URL").with_source(e)
        })?;
        // NOTE: no openEHR spec governs retrying — our own design: one attempt,
        // so `request_timeout_ms` bounds the whole call on the commit path.
        let mut config = Config::new(base, WireVersion::R4B)
            .with_timeout(Duration::from_millis(cfg.request_timeout_ms))
            .with_retry(RetryPolicy {
                max_attempts: 1,
                ..RetryPolicy::default()
            });
        if let Some(oauth) = &cfg.oauth {
            let grant = oauth.resolve().map_err(|e| {
                SmError::exception(format!("{OAUTH_KEY}: the grant is refused")).with_source(e)
            })?;
            let source = TokenSource::new(grant).map_err(|e| {
                SmError::exception(format!("{OAUTH_KEY}: the grant is refused")).with_source(e)
            })?;
            config =
                config.with_credentials(Credentials::OAuth(Arc::new(SmartToken::from(source))));
        }
        Client::new(config).map_err(|e| {
            SmError::exception("building the terminology client failed").with_source(e)
        })
    }

    /// Why a refusal says the asked-about thing is unknown, from the
    /// `tx-issue-type` coding of its `OperationOutcome` or a `404`/`410`.
    ///
    /// The coding is the HL7 terminology-service vocabulary
    /// (<https://build.fhir.org/ig/HL7/fhir-tools-ig/CodeSystem-tx-issue-type.html>);
    /// the status alone cannot be read, because servers differ on it (`FerroTERM`
    /// answers an unknown code with `400`, an unknown system with `404`).
    fn absence(upstream: &UpstreamError) -> Option<Absence> {
        if upstream.has_tx_issue(TX_ISSUE_NOT_FOUND) {
            Some(Absence::UnknownSystem)
        } else if upstream.has_tx_issue(TX_ISSUE_INVALID_CODE) {
            Some(Absence::UnknownCode)
        } else if upstream.status() == StatusCode::NOT_FOUND
            || upstream.status() == StatusCode::GONE
        {
            Some(Absence::Unclassified)
        } else {
            None
        }
    }

    impl FhirTerminologyProvider {
        /// The classified absence a failed call states, or the fault it is.
        fn absence_or_fault(&self, operation: &str, error: &Error) -> Result<Absence, SmError> {
            match error {
                Error::Token { .. } | Error::Renewal { .. } => Err(self.token_failure(error)),
                Error::Refused { upstream, .. }
                | Error::ServerFailure { upstream, .. }
                | Error::Unauthorized { upstream, .. } => absence(upstream)
                    .ok_or_else(|| self.provider_fault(operation, &ErrorChain::new(error))),
                _ => Err(self.provider_fault(operation, &ErrorChain::new(error))),
            }
        }

        /// The refusal of a request whose bearer token could not be obtained:
        /// the operator detail (the server and the authorization server's own
        /// error) goes to the trace record, the wire body stays the curated 500
        /// message, so a client cannot read the credential configuration out of
        /// a response. No openEHR spec governs 500 body content — our own design.
        fn token_failure(&self, error: &Error) -> SmError {
            tracing::error!(
                server = %self.endpoint,
                error = %ErrorChain::new(error),
                "terminology oauth2 client-credentials grant failed → 500"
            );
            SmError::exception(crate::service::error::INTERNAL_MESSAGE.to_owned())
        }

        /// `CodeSystem/$lookup`: the concept's display, or why it is absent.
        pub(super) async fn lookup(
            &self,
            system: &str,
            code: &str,
            at_date: Option<&str>,
        ) -> Result<Result<String, Absence>, SmError> {
            let answer = match at_date {
                Some(date) => self.wire.lookup_at(system, code, None, date).await,
                None => self.wire.lookup(system, code, None).await,
            };
            match answer {
                Ok(LookupOutcome::Found { display, .. }) => Ok(Ok(display)),
                Ok(LookupOutcome::NotFound { outcome }) => {
                    Ok(Err(absence(&outcome).unwrap_or(Absence::Unclassified)))
                }
                Ok(_) => Err(self.provider_fault(
                    ferrobridge_term::LOOKUP,
                    &format_args!("an outcome this client does not know"),
                )),
                Err(e) => self.absence_or_fault(ferrobridge_term::LOOKUP, &e).map(Err),
            }
        }

        /// `ValueSet/$validate-code`: membership, or why the value set is
        /// absent.
        pub(super) async fn validate(
            &self,
            value_set_url: &str,
            system: &str,
            code: &str,
            at_date: Option<String>,
        ) -> Result<Result<bool, Absence>, SmError> {
            let validation = Validation {
                value_set_url: value_set_url.to_owned(),
                value_set_version: None,
                date: at_date,
                coded: Coded::Code {
                    system: system.to_owned(),
                    code: code.to_owned(),
                    version: None,
                    display: None,
                },
            };
            match self.wire.validate(&validation).await {
                Ok(ValidateOutcome::Valid { .. }) => Ok(Ok(true)),
                Ok(ValidateOutcome::Invalid { .. }) => Ok(Ok(false)),
                Ok(_) => Err(self.provider_fault(
                    ferrobridge_term::VALIDATE_CODE,
                    &format_args!("an outcome this client does not know"),
                )),
                Err(e) => self
                    .absence_or_fault(ferrobridge_term::VALIDATE_CODE, &e)
                    .map(Err),
            }
        }

        /// `CodeSystem/$subsumes`: whether `code_a` strictly subsumes
        /// `code_b`, or why the code system is absent.
        pub(super) async fn strictly_subsumes(
            &self,
            system: &str,
            code_a: &str,
            code_b: &str,
        ) -> Result<Result<bool, Absence>, SmError> {
            match self
                .wire
                .subsumes(&Subsumption::new(system, code_a, code_b))
                .await
            {
                Ok(outcome) => Ok(Ok(outcome == SubsumesOutcome::Subsumes)),
                Err(e) => self
                    .absence_or_fault(ferrobridge_term::SUBSUMES, &e)
                    .map(Err),
            }
        }

        /// `ValueSet/$expand`: the expansion, or why the value set is absent.
        pub(super) async fn expand(
            &self,
            value_set_url: &str,
        ) -> Result<Result<Expansion, Absence>, SmError> {
            match self
                .wire
                .expand(&ferrobridge_term::outcome::Expansion::new(value_set_url))
                .await
            {
                Ok(ExpandOutcome::Expanded(expanded)) => {
                    let mut expansion = Expansion::default();
                    for member in &expanded.contains {
                        collect(member, &mut expansion);
                    }
                    Ok(Ok(expansion))
                }
                Ok(ExpandOutcome::NotFound { outcome }) => {
                    Ok(Err(absence(&outcome).unwrap_or(Absence::Unclassified)))
                }
                Ok(_) => Err(self.provider_fault(
                    ferrobridge_term::EXPAND,
                    &format_args!("an outcome this client does not know"),
                )),
                Err(e) => self.absence_or_fault(ferrobridge_term::EXPAND, &e).map(Err),
            }
        }

        /// `CodeSystem?url=<system>&_summary=count`: how many code systems the
        /// server holds under `system`.
        pub(super) async fn code_system_count(&self, system: &str) -> Result<u32, SmError> {
            self.wire.code_system_count(system).await.map_err(|e| {
                self.provider_fault(ferrobridge_term::CODE_SYSTEM_SEARCH, &ErrorChain::new(&e))
            })
        }
    }

    /// Collect one expansion member (and its descendants) into the flat `terms`
    /// map, recording a `Term_relationship` from the member to each direct child
    /// so the `$expand` hierarchy is not lost.
    pub(super) fn collect(member: &ExpansionContains, out: &mut Expansion) {
        if let Some(code) = &member.code {
            let entry = match &member.display {
                Some(text) => TermEntry::Defined(DefinedTerm {
                    code: code.clone(),
                    text: text.clone(),
                    language: None,
                    is_preferred_term: None,
                }),
                None => TermEntry::Bare(TermCode { code: code.clone() }),
            };
            out.terms.insert(code.clone(), entry);

            let child_codes: Vec<String> = member
                .contains
                .iter()
                .filter_map(|c| c.code.clone())
                .collect();
            if !child_codes.is_empty() {
                out.relationships.push(TermRelationship {
                    origin_code: code.clone(),
                    relation_name: CHILD_RELATION.to_owned(),
                    target_codes: Some(child_codes),
                });
            }
        }
        for child in &member.contains {
            collect(child, out);
        }
    }
}

/// A slim build holds no provider, so these are never called.
#[cfg(not(feature = "fhir"))]
impl FhirTerminologyProvider {
    /// Unreachable: no provider exists without the `fhir` feature.
    fn lookup(
        &self,
        _system: &str,
        _code: &str,
        _at_date: Option<&str>,
    ) -> std::future::Ready<Result<Result<String, Absence>, SmError>> {
        match self.wire {}
    }

    /// Unreachable: no provider exists without the `fhir` feature.
    fn validate(
        &self,
        _value_set_url: &str,
        _system: &str,
        _code: &str,
        _at_date: Option<String>,
    ) -> std::future::Ready<Result<Result<bool, Absence>, SmError>> {
        match self.wire {}
    }

    /// Unreachable: no provider exists without the `fhir` feature.
    fn strictly_subsumes(
        &self,
        _system: &str,
        _code_a: &str,
        _code_b: &str,
    ) -> std::future::Ready<Result<Result<bool, Absence>, SmError>> {
        match self.wire {}
    }

    /// Unreachable: no provider exists without the `fhir` feature.
    fn expand(
        &self,
        _value_set_url: &str,
    ) -> std::future::Ready<Result<Result<Expansion, Absence>, SmError>> {
        match self.wire {}
    }

    /// Unreachable: no provider exists without the `fhir` feature.
    fn code_system_count(&self, _system: &str) -> std::future::Ready<Result<u32, SmError>> {
        match self.wire {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_url_rejected() {
        let cfg = TerminologyServerConfig {
            enabled: true,
            url: ferrohealth_secret::secret_url::SecretUrl::new("  "),
            ..TerminologyServerConfig::default()
        };
        assert!(FhirTerminologyProvider::new(&cfg).is_err());
    }

    #[cfg(feature = "fhir")]
    mod expansion {
        use ferrobridge_term::outcome::ExpansionContains;

        use super::super::wire::collect;
        use super::super::{CHILD_RELATION, Expansion, FHIR_CHILD_PROPERTY};
        use crate::service::terminology::types::TermEntry;

        /// One `expansion.contains` entry of system `s`.
        fn entry(
            code: Option<&str>,
            display: Option<&str>,
            contains: Vec<ExpansionContains>,
        ) -> ExpansionContains {
            ExpansionContains {
                system: Some("s".to_owned()),
                version: None,
                code: code.map(str::to_owned),
                display: display.map(str::to_owned),
                r#abstract: None,
                inactive: None,
                designations: Vec::new(),
                contains,
            }
        }

        fn expansion(members: &[ExpansionContains]) -> Expansion {
            let mut out = Expansion::default();
            for member in members {
                collect(member, &mut out);
            }
            out
        }

        #[test]
        fn expansion_membership_and_flattening() {
            let vs = expansion(&[
                entry(Some("B"), Some("Buccal"), Vec::new()),
                entry(
                    Some("L"),
                    None,
                    vec![entry(Some("O"), Some("Occlusal"), Vec::new())],
                ),
            ]);
            let ext = vs.into_extract("http://example/vs/surface");
            let terms = ext.terms.expect("terms");
            assert!(terms.contains_key("B"));
            assert!(terms.contains_key("O")); // nested
            assert!(!terms.contains_key("Z"));
            assert_eq!(terms.len(), 3);
            assert!(matches!(terms.get("L"), Some(TermEntry::Bare(_))));
            assert!(matches!(terms.get("B"), Some(TermEntry::Defined(_))));
        }

        #[test]
        fn expansion_hierarchy_preserved_as_relationships() {
            // The parent→child `contains` tree survives as
            // relationships, with the `child` relation defined by its FHIR
            // property URI.
            let vs = expansion(&[entry(
                Some("L"),
                Some("Lower"),
                vec![entry(Some("O"), Some("Occlusal"), Vec::new())],
            )]);
            let ext = vs.into_extract("http://example/vs/surface");
            let rels = ext.relationships.expect("relationships");
            assert_eq!(rels.len(), 1);
            assert_eq!(rels[0].origin_code, "L");
            assert_eq!(rels[0].relation_name, CHILD_RELATION);
            assert_eq!(rels[0].target_codes.as_deref(), Some(&["O".to_owned()][..]));
            let relations = ext.relations.expect("relations");
            let child = relations.get(CHILD_RELATION).expect("child relation");
            assert_eq!(child.external_code.as_deref(), Some(FHIR_CHILD_PROPERTY));
            assert_eq!(child.local_code, None);
        }

        #[test]
        fn flat_expansion_has_no_relationships() {
            let vs = expansion(&[
                entry(Some("A"), Some("Alpha"), Vec::new()),
                entry(Some("B"), Some("Beta"), Vec::new()),
            ]);
            let ext = vs.into_extract("http://example/vs/flat");
            assert_eq!(ext.terms.expect("terms").len(), 2);
            assert!(ext.relationships.is_none());
            assert!(ext.relations.is_none());
        }

        #[test]
        fn an_abstract_grouper_without_a_code_still_holds_its_members() {
            let vs = expansion(&[entry(
                None,
                Some("Occlusal group"),
                vec![entry(Some("O"), None, Vec::new())],
            )]);
            let ext = vs.into_extract("http://example/vs/grouped");
            let terms = ext.terms.expect("terms");
            assert_eq!(terms.len(), 1);
            assert!(matches!(terms.get("O"), Some(TermEntry::Bare(_))));
            assert!(ext.relationships.is_none());
        }
    }
}
