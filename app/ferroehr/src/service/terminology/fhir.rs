// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`FhirTerminologyProvider`], a FHIR R4B terminology-server client realizing
//! the SM `I_TERMINOLOGY_SERVICE` calls against a remote server over `reqwest`
//! (rustls).
//!
//! The one terminology server (`[terminology.server]`) is what the routing
//! layer (`super::routing`) asks for every terminology the in-process
//! `openehr-term` bundle (`super::bundle`) does not carry. BASE
//! `architecture_overview/master12-terminology.adoc` models this backend as an
//! external "terminology query server". SM contract:
//! `docs/specs/openehr/SM/docs/UML/classes/i_terminology_service.adoc`; the
//! client and the server it points at are our own design. This module owns the
//! HTTP client and the SM error mapping, while the FHIR resources are decoded
//! by [`ferroehr_ext::fhir::terminology`] (the `fhir` cargo feature; a slim
//! build refuses a configured server at boot).
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
//! `has_term`/`get_term`/`value_set_validate`) is forwarded as the FHIR `date`
//! parameter of `$lookup`/`$validate-code`/`$expand`.
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
#[cfg(feature = "fhir")]
use std::time::Duration;

use crate::service::status::{CallStatusType, SmError};
use crate::service::terminology::types::{
    DefinedTerm, TermEntry, TermRelationship, TerminologyDescription, TerminologyExtract,
    TerminologyRelation,
};

use super::config::TerminologyServerConfig;
use ferrohealth_smart::client::CallError;
use ferrohealth_smart::client::cache::TokenSource;

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

/// A FHIR R4B terminology-server client realizing the SM
/// `I_TERMINOLOGY_SERVICE` lookup/validation calls against the one configured
/// server.
#[derive(Debug, Clone)]
pub struct FhirTerminologyProvider {
    client: reqwest::Client,
    /// The FHIR base URL, trailing `/` stripped (e.g. `http://host:8090/fhir`).
    base: String,
    /// The base URL with any userinfo masked, the server's name on the trace
    /// record.
    endpoint: String,
    /// The OAuth 2.0 client-credentials token source whose bearer credential
    /// authenticates every request (`[terminology.server.oauth]`). `None` =
    /// unauthenticated requests.
    token: Option<TokenSource>,
    /// Whether a binding this server cannot resolve refuses the composition
    /// ([`TerminologyServerConfig::fail_on_error`]).
    fail_on_error: bool,
}

impl FhirTerminologyProvider {
    /// Build the client from `[terminology.server]`, with the
    /// client-credentials token source attached when `oauth` is set.
    ///
    /// # Errors
    ///
    /// [`SmError::exception`] if the URL is empty (after trimming), the
    /// `oauth` grant is incomplete or refused by the token client, or the
    /// `reqwest` client cannot be built (e.g. TLS backend init failure).
    #[cfg_attr(
        not(feature = "fhir"),
        expect(
            unused_variables,
            reason = "a slim build refuses the server before reading its configuration"
        )
    )]
    pub fn new(cfg: &TerminologyServerConfig) -> Result<Self, SmError> {
        // A slim build cannot decode the server's FHIR responses, so a
        // configured server is refused loudly rather than silently ignored.
        #[cfg(not(feature = "fhir"))]
        return Err(SmError::exception(
            "terminology.server is enabled, but this binary was built without the `fhir` cargo \
             feature (the in-process bundle remains the terminology)",
        ));
        #[cfg(feature = "fhir")]
        Self::build(cfg)
    }

    /// Build the client once the `fhir` feature is known to be present.
    #[cfg(feature = "fhir")]
    fn build(cfg: &TerminologyServerConfig) -> Result<Self, SmError> {
        let base = cfg.url.expose().trim().trim_end_matches('/').to_owned();
        if base.is_empty() {
            return Err(SmError::exception(
                "terminology.server.url must not be empty",
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.request_timeout_ms))
            .build()
            .map_err(|e| {
                SmError::exception("building the terminology client failed").with_source(e)
            })?;
        let token = cfg
            .oauth
            .as_ref()
            .map(|oauth| {
                let grant = oauth.grant(OAUTH_KEY).map_err(SmError::exception)?;
                TokenSource::new(grant).map_err(|e| {
                    SmError::exception(format!("{OAUTH_KEY}: the grant is refused")).with_source(e)
                })
            })
            .transpose()?;
        Ok(Self {
            client,
            base,
            endpoint: cfg.url.redacted(),
            token,
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

    /// GET a FHIR operation with query params, returning the decoded response
    /// or a classified absence.
    ///
    /// A non-2xx answer is read through the `OperationOutcome` it carries: a
    /// `tx-issue-type` `not-found` issue is [`Absence::UnknownSystem`], an
    /// `invalid-code` issue is [`Absence::UnknownCode`], and a bare `404`/`410`
    /// with no classifying issue is [`Absence::Unclassified`] (the resource is
    /// unknown, which of system or code is not stated). Any other failure is a
    /// provider fault. The coding is the HL7 terminology-service vocabulary
    /// (<https://build.fhir.org/ig/HL7/fhir-tools-ig/CodeSystem-tx-issue-type.html>);
    /// the status alone cannot be read, because servers differ on it (`FerroTERM`
    /// answers an unknown code with `400`, an unknown system with `404`).
    async fn fetch(
        &self,
        op_path: &str,
        query: &[(&str, &str)],
        kind: ResponseKind,
    ) -> Result<Fetched, SmError> {
        let mut url = reqwest::Url::parse(&format!("{}{op_path}", self.base))
            .map_err(|e| self.provider_fault(op_path, &format_args!("invalid url: {e}")))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        let request = |bearer: Option<reqwest::header::HeaderValue>| {
            let mut request = self
                .client
                .get(url.clone())
                .header(reqwest::header::ACCEPT, "application/fhir+json");
            if let Some(bearer) = bearer {
                request = request.header(reqwest::header::AUTHORIZATION, bearer);
            }
            request.send()
        };
        // The bearer is resolved per request (the token source serves it from
        // its cache, renews it around expiry, and once more when the server
        // answers `401`), so a rotated credential takes effect without a
        // restart.
        let response = match &self.token {
            None => request(None)
                .await
                .map_err(|e| self.transport_error(op_path, &e))?,
            Some(token) => {
                token
                    .call(|bearer| request(Some(bearer)))
                    .await
                    .map_err(|e| match e {
                        CallError::Call { source } => self.transport_error(op_path, &source),
                        refused => self.token_failure(&refused),
                    })?
            }
        };
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|e| self.transport_error(op_path, &e))?;
        if status.is_success() {
            return Ok(Fetched::Found(decode(kind, &body).map_err(|e| {
                self.provider_fault(op_path, &crate::service::error::ErrorChain::new(&e))
            })?));
        }
        let issues = outcome_issue_types(&body);
        let gone = status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE;
        if issues.iter().any(|c| c == TX_ISSUE_NOT_FOUND) {
            Ok(Fetched::Absent(Absence::UnknownSystem))
        } else if issues.iter().any(|c| c == TX_ISSUE_INVALID_CODE) {
            Ok(Fetched::Absent(Absence::UnknownCode))
        } else if gone {
            Ok(Fetched::Absent(Absence::Unclassified))
        } else {
            Err(self.provider_fault(
                op_path,
                &format_args!("upstream returned HTTP {}", status.as_u16()),
            ))
        }
    }

    /// The named scalar values of a `Parameters` response, or `None` for any
    /// classified absence (the callers that read this map it to a
    /// `VersionedObjectDoesNotExist` precondition).
    async fn parameters(
        &self,
        op_path: &str,
        query: &[(&str, &str)],
    ) -> Result<Option<ParameterValues>, SmError> {
        match self.fetch(op_path, query, ResponseKind::Parameters).await? {
            Fetched::Absent(_) => Ok(None),
            Fetched::Found(DecodedResponse::Parameters(values)) => Ok(Some(values)),
            Fetched::Found(DecodedResponse::Expansion(_) | DecodedResponse::Count(_)) => {
                Err(self.provider_fault(op_path, &format_args!("expected a Parameters response")))
            }
        }
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
        let query = [("system", system), ("code", code)];
        match self
            .fetch("/CodeSystem/$lookup", &query, ResponseKind::Parameters)
            .await?
        {
            Fetched::Found(_) => Ok(TermExistence::Exists),
            Fetched::Absent(Absence::UnknownCode) => Ok(TermExistence::Absent),
            Fetched::Absent(Absence::UnknownSystem) => Ok(TermExistence::Unverifiable(
                "the terminology server does not serve the code system",
            )),
            Fetched::Absent(Absence::Unclassified) => {
                let probe = [("url", system), ("_summary", "count")];
                match self
                    .fetch("/CodeSystem", &probe, ResponseKind::BundleCount)
                    .await
                {
                    Ok(Fetched::Found(DecodedResponse::Count(Some(n)))) if n > 0 => {
                        Ok(TermExistence::Absent)
                    }
                    Ok(Fetched::Found(DecodedResponse::Count(_))) => {
                        Ok(TermExistence::Unverifiable(
                            "the terminology server does not serve the code system",
                        ))
                    }
                    Ok(_) | Err(_) => Ok(TermExistence::Unverifiable(
                        "the terminology server could not say whether it serves the code system",
                    )),
                }
            }
        }
    }

    /// The refusal of a request whose bearer token could not be obtained: the
    /// operator detail (the server and the authorization server's own error)
    /// goes to the trace record, the wire body stays the curated 500 message,
    /// so a client cannot read the credential configuration out of a response.
    /// No openEHR spec governs 500 body content — our own design.
    fn token_failure(&self, error: &CallError<reqwest::Error>) -> SmError {
        tracing::error!(
            server = %self.endpoint,
            error = %crate::service::error::ErrorChain::new(error),
            "terminology oauth2 client-credentials grant failed → 500"
        );
        SmError::exception(crate::service::error::INTERNAL_MESSAGE.to_owned())
    }

    /// A classified transport-fault exception (timeout / connect / other).
    fn transport_error(&self, op_path: &str, e: &reqwest::Error) -> SmError {
        let kind = if e.is_timeout() {
            "timeout"
        } else if e.is_connect() {
            "connect error"
        } else {
            "transport error"
        };
        self.provider_fault(op_path, &format_args!("{kind}: {e}"))
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
    fn provider_fault(&self, op_path: &str, detail: &dyn std::fmt::Display) -> SmError {
        tracing::error!(
            server = %self.endpoint,
            operation = op_path,
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

    /// `ValueSet/$expand` → the decoded [`Expansion`], or `None` when the
    /// value set is unknown (`404`). Forwards `at_date` as the FHIR `date`
    /// parameter.
    async fn expand(
        &self,
        value_set_url: &str,
        at_date: Option<&str>,
    ) -> Result<Option<Expansion>, SmError> {
        let mut query = vec![("url", value_set_url)];
        if let Some(date) = at_date {
            query.push(("date", date));
        }
        match self
            .fetch("/ValueSet/$expand", &query, ResponseKind::ValueSet)
            .await?
        {
            Fetched::Absent(_) => Ok(None),
            Fetched::Found(DecodedResponse::Expansion(expansion)) => Ok(Some(expansion)),
            Fetched::Found(DecodedResponse::Parameters(_) | DecodedResponse::Count(_)) => Err(self
                .provider_fault(
                    "/ValueSet/$expand",
                    &format_args!("expected a ValueSet response"),
                )),
        }
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
        let mut query = vec![
            ("url", value_set_id),
            ("system", terminology_id),
            ("code", candidate_code),
        ];
        if let Some(date) = at_date.as_deref() {
            query.push(("date", date));
        }
        let values = self
            .parameters("/ValueSet/$validate-code", &query)
            .await?
            .ok_or_else(|| self.not_found("value set", value_set_id))?;
        values.booleans.get("result").copied().ok_or_else(|| {
            SmError::exception("the terminology server's $validate-code response had no 'result'")
        })
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
            .expand(value_set_code, None)
            .await?
            .ok_or_else(|| self.not_found("value set", value_set_code))?;
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
        let values = self
            .parameters(
                "/CodeSystem/$subsumes",
                &[
                    ("system", terminology_id),
                    ("codeA", ref_code),
                    ("codeB", candidate_child_code),
                ],
            )
            .await?
            .ok_or_else(|| self.not_found("terminology", terminology_id))?;
        let outcome = values.codes.get("outcome").ok_or_else(|| {
            SmError::exception("the terminology server's $subsumes response had no 'outcome'")
        })?;
        Ok(outcome == "subsumes")
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
        let mut query = vec![("system", terminology_id), ("code", code)];
        if let Some(date) = at_date.as_deref() {
            query.push(("date", date));
        }
        Ok(self
            .parameters("/CodeSystem/$lookup", &query)
            .await?
            .is_some())
    }

    /// `get_term` → FHIR `CodeSystem/$lookup`, mapped to a single-term
    /// [`TerminologyExtract`] (the `display` becomes the term text; a lookup
    /// with no `display` falls back to the code itself). `at_date` → the FHIR
    /// `date` parameter.
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
    /// term); exception on a transport fault / non-2xx / malformed response.
    pub async fn get_term(
        &self,
        terminology_id: &str,
        code: &str,
        _attributes: Option<BTreeMap<String, String>>,
        at_date: Option<String>,
    ) -> Result<TerminologyExtract, SmError> {
        let mut query = vec![("system", terminology_id), ("code", code)];
        if let Some(date) = at_date.as_deref() {
            query.push(("date", date));
        }
        let values = self
            .parameters("/CodeSystem/$lookup", &query)
            .await?
            .ok_or_else(|| self.not_found("term", code))?;
        let text = values
            .strings
            .get("display")
            .cloned()
            .unwrap_or_else(|| code.to_owned());
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
        Ok(self.expand(value_set_code, None).await?.is_some())
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

// ─── the decoded response ──────────────────────────────────────────────────────────────────────────────────────────

/// Which FHIR resource an operation answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseKind {
    /// `$validate-code` / `$subsumes` / `$lookup`.
    Parameters,
    /// `$expand`.
    ValueSet,
    /// A `_summary=count` search: the `Bundle.total` alone.
    BundleCount,
}

/// The `tx-issue-type` code for "the code system is not known to the server"
/// (<https://build.fhir.org/ig/HL7/fhir-tools-ig/CodeSystem-tx-issue-type.html>).
const TX_ISSUE_NOT_FOUND: &str = "not-found";

/// The `tx-issue-type` code for "the code is not in the code system".
const TX_ISSUE_INVALID_CODE: &str = "invalid-code";

/// Why an operation answered "not found", read from its `OperationOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Absence {
    /// The server does not serve the code system asked about (`not-found`).
    UnknownSystem,
    /// The code system is served and the code is not in it (`invalid-code`).
    UnknownCode,
    /// A `404`/`410` with no classifying issue: the resource is unknown, and
    /// whether that is the system or the code is not stated.
    Unclassified,
}

/// What one fetch produced: a decoded resource, or a classified absence.
#[derive(Debug, Clone)]
enum Fetched {
    /// A 2xx answer, decoded.
    Found(DecodedResponse),
    /// A "not found" answer, classified.
    Absent(Absence),
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

/// A decoded FHIR terminology response, reduced to what the SM calls read.
#[derive(Debug, Clone)]
#[cfg_attr(
    not(feature = "fhir"),
    expect(
        dead_code,
        reason = "a slim build decodes no FHIR response: the server wiring refuses a \
                  configured terminology server at boot"
    )
)]
enum DecodedResponse {
    /// The named scalar values of a `Parameters` response.
    Parameters(ParameterValues),
    /// The membership + hierarchy of a `ValueSet` expansion.
    Expansion(Expansion),
    /// The `total` of a `_summary=count` search `Bundle` (absent when the
    /// server does not report one).
    Count(Option<u32>),
}

/// The named scalar values a `Parameters` response carries.
#[derive(Debug, Clone, Default)]
struct ParameterValues {
    /// `valueBoolean` parameters (`$validate-code` `result`).
    booleans: BTreeMap<String, bool>,
    /// `valueCode` parameters (`$subsumes` `outcome`).
    codes: BTreeMap<String, String>,
    /// `valueString` parameters (`$lookup` `display`).
    strings: BTreeMap<String, String>,
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

/// Why a FHIR terminology response body could not be decoded.
#[derive(Debug, thiserror::Error)]
enum DecodeError {
    /// The body is not the FHIR resource the operation returns.
    #[cfg(feature = "fhir")]
    #[error(transparent)]
    Fhir(#[from] ferroehr_ext::fhir::terminology::TerminologyDecodeError),
    /// A slim build has no FHIR model to decode with. Unreachable:
    /// [`FhirTerminologyProvider::new`] refuses a configured terminology server at boot.
    #[cfg(not(feature = "fhir"))]
    #[error("this binary was built without the `fhir` cargo feature")]
    FeatureAbsent,
}

/// Decode a FHIR terminology response body into the reduced form the service
/// consumes, via the typed R4B model in the extension crate.
#[cfg(feature = "fhir")]
fn decode(kind: ResponseKind, body: &[u8]) -> Result<DecodedResponse, DecodeError> {
    match kind {
        ResponseKind::Parameters => {
            let view = ferroehr_ext::fhir::terminology::decode_parameters(body)?;
            Ok(DecodedResponse::Parameters(ParameterValues {
                booleans: view.booleans,
                codes: view.codes,
                strings: view.strings,
            }))
        }
        ResponseKind::ValueSet => {
            let members = ferroehr_ext::fhir::terminology::decode_expansion(body)?;
            let mut expansion = Expansion::default();
            for member in &members {
                collect(member, &mut expansion);
            }
            Ok(DecodedResponse::Expansion(expansion))
        }
        ResponseKind::BundleCount => Ok(DecodedResponse::Count(
            ferroehr_ext::fhir::terminology::decode_bundle_total(body)?,
        )),
    }
}

/// The `tx-issue-type` codes a failing response's `OperationOutcome` carries;
/// empty when the body is not one (a bare status, an HTML error page).
#[cfg(feature = "fhir")]
fn outcome_issue_types(body: &[u8]) -> Vec<String> {
    ferroehr_ext::fhir::terminology::decode_outcome_issue_types(body).unwrap_or_default()
}

/// A slim build reads no `OperationOutcome`; every failure stays unclassified.
#[cfg(not(feature = "fhir"))]
fn outcome_issue_types(_body: &[u8]) -> Vec<String> {
    Vec::new()
}

/// Collect one expansion member (and its descendants) into the flat `terms`
/// map, recording a `Term_relationship` from the member to each direct child
/// so the `$expand` hierarchy is not lost.
#[cfg(feature = "fhir")]
fn collect(member: &ferroehr_ext::fhir::terminology::ExpansionMember, out: &mut Expansion) {
    use crate::service::terminology::types::TermCode;

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
            .children
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
    for child in &member.children {
        collect(child, out);
    }
}

/// A slim build decodes no FHIR responses. Unreachable:
/// [`FhirTerminologyProvider::new`] refuses a configured terminology server at boot.
#[cfg(not(feature = "fhir"))]
fn decode(_kind: ResponseKind, _body: &[u8]) -> Result<DecodedResponse, DecodeError> {
    Err(DecodeError::FeatureAbsent)
}

#[cfg(test)]
#[cfg(feature = "fhir")]
mod tests {
    use super::*;

    fn parameters(json: &str) -> ParameterValues {
        match decode(ResponseKind::Parameters, json.as_bytes()).expect("decode Parameters") {
            DecodedResponse::Parameters(values) => values,
            DecodedResponse::Expansion(_) | DecodedResponse::Count(_) => {
                panic!("expected Parameters")
            }
        }
    }

    fn expansion(json: &str) -> Expansion {
        match decode(ResponseKind::ValueSet, json.as_bytes()).expect("decode ValueSet") {
            DecodedResponse::Expansion(expansion) => expansion,
            DecodedResponse::Parameters(_) | DecodedResponse::Count(_) => {
                panic!("expected ValueSet")
            }
        }
    }

    /// The FHIR-required `ValueSet` members every `$expand` response carries
    /// (`ValueSet.status` and `ValueSet.expansion.timestamp` are both 1..1).
    const VS_HEAD: &str = r#""resourceType":"ValueSet","status":"active""#;
    /// The FHIR-required `ValueSet.expansion.timestamp` (1..1).
    const EXPANSION_HEAD: &str = r#""timestamp":"2026-08-04T00:00:00Z""#;

    #[test]
    fn validate_code_result_extracted() {
        let p = parameters(
            r#"{"resourceType":"Parameters","parameter":[
                {"name":"result","valueBoolean":true},
                {"name":"display","valueString":"Buccal"}]}"#,
        );
        assert_eq!(p.booleans.get("result").copied(), Some(true));
        assert_eq!(p.strings.get("display").map(String::as_str), Some("Buccal"));
    }

    #[test]
    fn subsumes_outcome_extracted() {
        let p = parameters(
            r#"{"resourceType":"Parameters","parameter":[
                {"name":"outcome","valueCode":"subsumes"}]}"#,
        );
        assert_eq!(p.codes.get("outcome").map(String::as_str), Some("subsumes"));
    }

    #[test]
    fn expansion_membership_and_flattening() {
        let vs = expansion(&format!(
            r#"{{{VS_HEAD},"expansion":{{{EXPANSION_HEAD},"contains":[
                {{"system":"s","code":"B","display":"Buccal"}},
                {{"system":"s","code":"L","contains":[
                    {{"system":"s","code":"O","display":"Occlusal"}}]}}]}}}}"#
        ));
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
        let vs = expansion(&format!(
            r#"{{{VS_HEAD},"expansion":{{{EXPANSION_HEAD},"contains":[
                {{"system":"s","code":"L","display":"Lower","contains":[
                    {{"system":"s","code":"O","display":"Occlusal"}}]}}]}}}}"#
        ));
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
        let vs = expansion(&format!(
            r#"{{{VS_HEAD},"expansion":{{{EXPANSION_HEAD},"contains":[
                {{"system":"s","code":"A","display":"Alpha"}},
                {{"system":"s","code":"B","display":"Beta"}}]}}}}"#
        ));
        let ext = vs.into_extract("http://example/vs/flat");
        assert_eq!(ext.terms.expect("terms").len(), 2);
        assert!(ext.relationships.is_none());
        assert!(ext.relations.is_none());
    }

    #[test]
    fn a_response_that_is_not_a_valid_r4b_resource_is_refused() {
        // `ValueSet.status` is 1..1 in R4B; the typed reader refuses the
        // response rather than reading it partially.
        assert!(
            decode(
                ResponseKind::ValueSet,
                br#"{"resourceType":"ValueSet","expansion":{"contains":[]}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn empty_url_rejected() {
        let cfg = TerminologyServerConfig {
            enabled: true,
            url: crate::config::secret::SecretUrl::new("  "),
            ..TerminologyServerConfig::default()
        };
        assert!(FhirTerminologyProvider::new(&cfg).is_err());
    }
}
