// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`FhirTerminologyProvider`], a FHIR R4B terminology-server client realizing
//! the SM `I_TERMINOLOGY_SERVICE` calls against a remote server over `reqwest`
//! (rustls).
//!
//! The remote provider is one of the two the routing layer (`super::routing`)
//! selects among; the in-process `openehr-term` bundle (`super::bundle`) is the
//! enumerable local default. `arch-overview master12-terminology.adoc` models
//! this concrete backend as an external "terminology query server". SM contract:
//! `docs/specs/openehr/SM/docs/UML/classes/i_terminology_service.adoc`; the
//! client and the server it points at are our own design. This module owns the
//! HTTP client, the cache and the SM error mapping, while the FHIR resources are
//! decoded by [`ferroehr_ext::fhir::terminology`] over the typed `fhir-model`
//! R4B model (the `fhir` cargo feature; a slim build refuses a configured
//! provider at boot).
//!
//! # SM call → FHIR operation mapping
//!
//! | SM call | FHIR R4B operation |
//! |---|---|
//! | `value_set_validate` | `ValueSet/$validate-code` (or `$expand` + membership) |
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
//! ([`config::ExternalTerminologyConfig::fail_on_error`](super::config::ExternalTerminologyConfig::fail_on_error)).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crate::service::status::{CallStatusType, SmError};
use crate::service::terminology::types::{
    DefinedTerm, TermEntry, TermRelationship, TerminologyDescription, TerminologyExtract,
    TerminologyRelation,
};

use super::config::{FhirOperation, FhirProviderConfig};
use super::oauth2::TokenSource;

/// The `Term_relationship.relation_name` under which a FHIR `$expand`
/// parent→child `contains` nesting is preserved (`terminology_extract.adoc`).
const CHILD_RELATION: &str = "child";
/// The FHIR R4B concept-property URI that defines the parent→child relation,
/// carried as the `Terminology_relation.external_code`
/// (`terminology_relation.adoc`).
const FHIR_CHILD_PROPERTY: &str = "http://hl7.org/fhir/concept-properties#child";

/// A FHIR R4B terminology-server client realizing the SM
/// `I_TERMINOLOGY_SERVICE` lookup/validation calls against a remote server.
#[derive(Debug, Clone)]
pub struct FhirTerminologyProvider {
    client: reqwest::Client,
    /// The FHIR base URL, trailing `/` stripped (e.g. `http://host:8090/fhir`).
    base: String,
    /// The membership operation used by `value_set_validate`.
    operation: FhirOperation,
    /// The configured provider name (for error/log context).
    name: String,
    /// TTL-bounded response cache keyed by the full operation URL (`None`
    /// when disabled). Caches the DECODED response — including a classified
    /// "not found" outcome — so a validation burst over the same codes costs
    /// one remote round trip per TTL window.
    cache: Option<moka::future::Cache<String, Fetched>>,
    /// The `OAuth2` client-credentials token source whose bearer credential
    /// authenticates every request, when the provider configures one. `None` =
    /// unauthenticated requests.
    token: Option<Arc<TokenSource>>,
}

impl FhirTerminologyProvider {
    /// Build a provider from its configuration.
    ///
    /// # Errors
    ///
    /// [`SmError::exception`] if the URL is empty (after trimming), the
    /// configured TLS material is unusable ([`super::tls::TlsMaterialError`]),
    /// or the `reqwest` client cannot be built (e.g. TLS backend init
    /// failure).
    pub fn new(name: &str, cfg: &FhirProviderConfig) -> Result<Self, SmError> {
        let base = cfg.url.expose().trim().trim_end_matches('/').to_owned();
        if base.is_empty() {
            return Err(SmError::exception(format!(
                "terminology provider '{name}' has an empty url"
            )));
        }
        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(cfg.connect_timeout_ms))
            .timeout(Duration::from_millis(cfg.request_timeout_ms));
        // Mutual TLS + the provider's trust anchors, when configured. Bad
        // material is a boot failure here, never a first-request surprise
        // ([`super::tls`]).
        let builder = super::tls::apply(builder, cfg).map_err(|e| {
            SmError::exception(format!(
                "terminology provider '{name}': the configured TLS material is unusable"
            ))
            .with_source(e)
        })?;
        let client = builder.build().map_err(|e| {
            SmError::exception(format!(
                "building the terminology client for provider '{name}' failed"
            ))
            .with_source(e)
        })?;
        let cache = (cfg.cache_ttl_secs > 0).then(|| {
            moka::future::Cache::builder()
                .max_capacity(cfg.cache_capacity)
                .time_to_live(Duration::from_secs(cfg.cache_ttl_secs))
                .build()
        });
        Ok(Self {
            client,
            base,
            operation: cfg.operation,
            name: name.to_owned(),
            cache,
            token: None,
        })
    }

    /// Attach the `OAuth2` client-credentials token source whose bearer
    /// credential every request to this provider carries
    /// (`[terminology.external.providers.<name>] oauth2_client`).
    #[must_use]
    pub fn with_token(mut self, token: Arc<TokenSource>) -> Self {
        self.token = Some(token);
        self
    }

    /// The configured provider name (routing, logging, error context).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
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
        if let Some(cache) = &self.cache
            && let Some(hit) = cache.get(url.as_str()).await
        {
            return Ok(hit);
        }
        let cache_key = url.as_str().to_owned();
        let mut request = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/fhir+json");
        // The bearer is resolved per request (the token source serves it from
        // its cache and re-grants only around expiry), so a rotated credential
        // takes effect without a restart.
        if let Some(token) = &self.token {
            request = request.bearer_auth(token.bearer().await?);
        }
        let response = request
            .send()
            .await
            .map_err(|e| self.transport_error(op_path, &e))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|e| self.transport_error(op_path, &e))?;
        let fetched = if status.is_success() {
            Fetched::Found(decode(kind, &body).map_err(|e| {
                self.provider_fault(op_path, &format_args!("malformed FHIR response: {e}"))
            })?)
        } else {
            let issues = outcome_issue_types(&body);
            let gone =
                status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE;
            if issues.iter().any(|c| c == TX_ISSUE_NOT_FOUND) {
                Fetched::Absent(Absence::UnknownSystem)
            } else if issues.iter().any(|c| c == TX_ISSUE_INVALID_CODE) {
                Fetched::Absent(Absence::UnknownCode)
            } else if gone {
                Fetched::Absent(Absence::Unclassified)
            } else {
                return Err(self.provider_fault(
                    op_path,
                    &format_args!("upstream returned HTTP {}", status.as_u16()),
                ));
            }
        };
        if let Some(cache) = &self.cache {
            cache.insert(cache_key, fetched.clone()).await;
        }
        Ok(fetched)
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
    /// (which provider, which operation, the upstream diagnostic) on the trace
    /// record and NOT on the wire.
    ///
    /// The detail here is the deployment's own configuration — the provider's
    /// configured name and the upstream server's error — which is exactly the
    /// class of fact a client must not be able to read out of a
    /// response body, while the operator still needs it in full. The server
    /// carries no per-request diagnostic surface (`/management` mounts
    /// info/prometheus/metrics/env/loggers only), so the trace record IS the
    /// operator channel. (No openEHR spec governs the content of a `500` body
    /// — our own design/extension; the status itself is the ITS-REST overview
    /// `Requests_and_responses.md` §HTTP status codes row.)
    fn provider_fault(&self, op_path: &str, detail: &dyn std::fmt::Display) -> SmError {
        tracing::error!(
            provider = %self.name,
            operation = op_path,
            error = %detail,
            "terminology provider fault → 500"
        );
        SmError::exception(crate::service::error::INTERNAL_MESSAGE.to_owned())
    }

    /// A `Pre_has_*` precondition failure (`VersionedObjectDoesNotExist`).
    ///
    /// The body carries only what the client can act on, the kind and id it
    /// asked about; which configured provider answered is deployment
    /// configuration and goes to the trace record only.
    fn not_found(&self, what: &str, id: &str) -> SmError {
        tracing::debug!(provider = %self.name, what, id, "terminology lookup: not found");
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

    /// `value_set_validate` → FHIR `ValueSet/$validate-code` (or `$expand` +
    /// membership when the provider is configured for `expand`).
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
        match self.operation {
            FhirOperation::Expand => {
                let expansion = self
                    .expand(value_set_id, at_date.as_deref())
                    .await?
                    .ok_or_else(|| self.not_found("value set", value_set_id))?;
                Ok(expansion.contains_code(candidate_code))
            }
            FhirOperation::ValidateCode => {
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
                    SmError::exception(format!(
                        "terminology provider '{}': $validate-code response had no 'result'",
                        self.name
                    ))
                })
            }
        }
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

    /// Cross-terminology code translation → FHIR `ConceptMap/$translate`
    /// (<https://hl7.org/fhir/R4B/conceptmap-operation-translate.html>):
    /// `system`/`code` name the source coding, `target_system` the FHIR
    /// `targetsystem`, and `concept_map` (when pinned) the `url`.
    ///
    /// `Ok(None)` when no translation exists — the server answered `404`, the
    /// response's `result` is `false`, or no match is STRICTLY equivalent
    /// (`equivalence` ∈ {`equivalent`, `equal`}; the looser relations —
    /// `wider`, `narrower`, `relatedto`, `inexact` — are deliberately not
    /// taken: no openEHR spec governs FHIR conversion, and writing a
    /// non-equivalent code would be a silently wrong clinical value).
    ///
    /// # Errors
    ///
    /// Precondition on an empty `code`; exception on a transport fault or a
    /// malformed response.
    #[cfg(feature = "fhir")]
    pub async fn translate(
        &self,
        system: &str,
        code: &str,
        target_system: &str,
        concept_map: Option<&str>,
    ) -> Result<Option<ferroehr_ext::fhir::mapping::TranslatedCode>, SmError> {
        if code.is_empty() {
            return Err(SmError::precondition("code must not be empty"));
        }
        let mut query = vec![
            ("system", system),
            ("code", code),
            ("targetsystem", target_system),
        ];
        if let Some(url) = concept_map {
            query.push(("url", url));
        }
        let Some(values) = self.parameters("/ConceptMap/$translate", &query).await? else {
            return Ok(None);
        };
        if values.booleans.get("result") != Some(&true) {
            return Ok(None);
        }
        Ok(values
            .matches
            .iter()
            .find(|m| {
                matches!(m.equivalence.as_deref(), Some("equivalent" | "equal")) && m.code.is_some()
            })
            .and_then(|m| {
                m.code
                    .clone()
                    .map(|code| ferroehr_ext::fhir::mapping::TranslatedCode {
                        code,
                        display: m.display.clone(),
                    })
            }))
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
            SmError::exception(format!(
                "terminology provider '{}': $subsumes response had no 'outcome'",
                self.name
            ))
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

// ─── the decoded response the cache holds ─────────────────────────────────────

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

/// What one fetch produced: a decoded resource, or a classified absence. The
/// cache holds this, so a repeated question costs one round trip per TTL.
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
/// The cache is typed on this, never on raw JSON.
#[derive(Debug, Clone)]
#[cfg_attr(
    not(feature = "fhir"),
    expect(
        dead_code,
        reason = "a slim build decodes no FHIR response: the router refuses a configured \
                  provider at boot"
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
    /// `$translate` `match` parameters, in response order.
    #[cfg(feature = "fhir")]
    matches: Vec<TranslateMatchValues>,
}

/// One `$translate` match: the equivalence code + the target concept.
#[cfg(feature = "fhir")]
#[derive(Debug, Clone, Default)]
struct TranslateMatchValues {
    /// The match's `equivalence` (`equivalent`, `equal`, `wider`, …).
    equivalence: Option<String>,
    /// The target concept's code.
    code: Option<String>,
    /// The target concept's display text.
    display: Option<String>,
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
    /// Whether `code` appears anywhere in the (possibly hierarchical)
    /// expansion.
    fn contains_code(&self, code: &str) -> bool {
        self.terms.contains_key(code)
    }

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
    #[error("{0}")]
    Fhir(#[from] ferroehr_ext::fhir::terminology::TerminologyDecodeError),
    /// A slim build has no FHIR model to decode with. Unreachable:
    /// [`FhirTerminologyProvider::new`] refuses a configured provider at boot.
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
                matches: view
                    .matches
                    .into_iter()
                    .map(|m| TranslateMatchValues {
                        equivalence: m.equivalence,
                        code: m.code,
                        display: m.display,
                    })
                    .collect(),
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
/// [`FhirTerminologyProvider::new`] refuses a configured provider at boot.
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
        assert!(vs.contains_code("B"));
        assert!(vs.contains_code("O")); // nested
        assert!(!vs.contains_code("Z"));
        let ext = vs.into_extract("http://example/vs/surface");
        let terms = ext.terms.expect("terms");
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
        let cfg = FhirProviderConfig {
            kind: super::super::config::ProviderKind::Fhir,
            url: crate::config::secret::SecretUrl::new("  "),
            operation: FhirOperation::ValidateCode,
            connect_timeout_ms: 100,
            request_timeout_ms: 100,
            oauth2_client: None,
            client_cert_path: None,
            client_key_path: None,
            ca_bundle_path: None,
            cache_ttl_secs: 0,
            cache_capacity: 1024,
        };
        assert!(FhirTerminologyProvider::new("p", &cfg).is_err());
    }
}
