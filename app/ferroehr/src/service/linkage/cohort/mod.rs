// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Cohort query: select a population in the DEMOGRAPHIC domain, resolve it to
//! EHRs through the LINKAGE domain, and run the caller's AQL over exactly those
//! EHRs on the clinical pool.
//!
//! **No openEHR spec governs this — our own design/extension.** AQL is defined
//! over the clinical domain alone (QUERY `master03-syntax.adoc` names EHR,
//! COMPOSITION and the RM structures below them; it has no demographic source),
//! and the SM publishes no cohort operation, so nothing released describes this
//! call. What it exists for is the question a research or public-health caller
//! actually asks — "the blood pressures of everyone in this city" — which today
//! is answered by handing someone a list of patient identifiers.
//!
//! ## Three steps, three credentials, identifiers only
//!
//! 1. **The predicate** ([`predicate`]) runs on the demographic pool against a
//!    binding the deployment declared in `[cohort.predicates]`. It selects
//!    party ids and nothing else — no name, no address, no identifier leaves
//!    the domain.
//! 2. **The crossing** (`store::open_mappings`) runs on the linkage pool:
//!    those party ids in, the EHR ids they are the subject of out. One
//!    statement, one round trip, no attribute.
//! 3. **The query** runs on the clinical pool, scoped to those EHR ids through
//!    the ordinary [`AqlQueryRequest::ehr_ids`] path, so the whole AQL engine —
//!    the population gate, the profile gate, paging, the access columns —
//!    applies unchanged.
//!
//! The three statements never meet: under `[db].demographic_url` +
//! `[db].linkage_url` each pool authenticates as its own role, and
//! [`crate::db::verify_domain_isolation`] refuses to boot a database where one
//! can read another's schema. The caller never learns a party id either — the
//! result set carries whatever the AQL projected, and the cohort metadata
//! carries counts and a digest of the definition.
//!
//! ## Small-cell suppression
//!
//! A cohort that resolves to fewer served EHRs than
//! [`config::CohortConfig::small_cell_threshold`] has its rows withheld. A
//! result set narrow enough to name one person re-identifies that person out of
//! content the caller holds no individual entitlement to, and the count is the
//! standard disclosure control for exactly that
//! (Eurostat, *Handbook on Statistical Disclosure Control*,
//! <https://ec.europa.eu/eurostat/cros/content/handbook-statistical-disclosure-control_en>).
//! The response says it was suppressed rather than pretending the cohort was
//! empty: a caller who cannot tell the two apart will keep re-running the
//! query with a wider predicate until the boundary leaks.
//!
//! ## Under an IHE patient identity source
//!
//! With `[ehr_index] provider = "ihe"` the server holds no party-to-EHR map:
//! it knows each patient only by the pseudonym `EHR_STATUS` carries in its
//! own clinical domain, and translates a real identifier forward through the
//! PIX Manager (ITI-83 with `targetSystem` set to that domain), never a
//! pseudonym back (RM ehr master04 §EHR Status, `EHR_STATUS.subject`; GDPR
//! Art. 4(5) and Art. 25). So a cohort that names its subjects by identifier
//! is resolved one subject at a time through that forward translation, and a
//! cohort that selects on demographic attributes, which would need the map
//! or the reverse translation, is refused as [`CohortError::IdentityNotHeld`].
//!
//! ## Every execution is recorded
//!
//! One `linkage`-domain access event per execution, suppressed and failed ones
//! included, naming the definition digest rather than the predicate values —
//! the values are the selection criterion, and a trail that carried them would
//! be a second copy of the cohort. A crossing nobody can reconstruct afterwards
//! is what the access log exists to prevent.

pub mod config;
pub mod predicate;

use std::collections::BTreeSet;

use sha2::Digest;

use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::ehr_index::IndexError;
use crate::service::ehr_index::provider::EhrIndexProvider;
use crate::service::ehr_index::types::SubjectRef;
use crate::service::query::request::{AqlQueryRequest, QueryOutcome};
use crate::service::status::{CallStatusType, SmError};
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};

/// One predicate a caller applies, by the name the deployment bound it under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CohortPredicate {
    /// The `[cohort.predicates]` key.
    pub name: String,
    /// The value to match, interpreted by the binding's
    /// [`config::PredicateKind`].
    pub value: String,
}

/// A cohort query: the demographic selection, and the AQL to run over what it
/// selects.
#[derive(Debug, Clone, Default)]
pub struct CohortQueryRequest {
    /// The AQL text, executed exactly as an ad-hoc query would be.
    pub aql: String,
    /// The demographic predicates, intersected.
    pub predicates: Vec<CohortPredicate>,
    /// The subjects the cohort names by identifier, in any identifier domain;
    /// their EHRs intersect with what the predicates select.
    pub subjects: Vec<SubjectRef>,
    /// The caller's declared purpose of use for the access record; falls back
    /// to the request scope's own purpose
    /// ([`crate::system_log::access_context`]).
    pub purpose: Option<String>,
    /// The paging window, `$parameter` binds and ABAC scope of the inner
    /// execution. Its `ehr_ids` MUST be empty — the cohort is the scope.
    pub query: AqlQueryRequest,
}

/// What a cohort execution served.
#[derive(Debug, Clone)]
pub struct CohortOutcome {
    /// The inner AQL outcome, with the rows withheld when suppressed.
    pub query: QueryOutcome,
    /// How many EHRs the cohort resolved to, before the query ran.
    pub cohort_size: u64,
    /// How many EHRs the served rows came from; `0` when suppressed.
    pub served_ehrs: u64,
    /// Whether small-cell suppression withheld the rows.
    pub suppressed: bool,
    /// The hex SHA-256 of the canonical predicate list — the cohort's identity
    /// in the access record, carrying no value.
    pub definition_hash: String,
}

/// What went wrong executing a cohort query.
#[derive(Debug, thiserror::Error)]
pub enum CohortError {
    /// This deployment binds no predicate, so the surface is off.
    #[error(
        "no cohort predicate is bound ([cohort.predicates]), so this deployment serves no \
         cohort queries"
    )]
    NotConfigured,
    /// The request names a predicate this deployment did not bind.
    #[error("`{0}` is not a bound cohort predicate")]
    UnknownPredicate(String),
    /// A predicate's value does not parse for its bound kind.
    #[error("the value of cohort predicate `{name}` is invalid: {reason}")]
    InvalidValue {
        /// The predicate whose value was refused.
        name: String,
        /// Why it was refused.
        reason: String,
    },
    /// The predicate matches more parties than the deployment serves.
    ///
    /// Refused rather than truncated: a silently shortened cohort is a wrong
    /// denominator with nothing on the wire to say so.
    #[error("the cohort matches {size} parties, above the configured maximum of {max}")]
    CohortTooLarge {
        /// How many parties matched.
        size: u64,
        /// The configured ceiling.
        max: u64,
    },
    /// The request carries no predicate and no subject at all.
    ///
    /// An unfiltered cohort is the whole population, which is what the ordinary
    /// ad-hoc query already serves under the population gate; routing it
    /// through here would cross the boundary for nothing.
    #[error("a cohort query names at least one predicate or subject")]
    NoPredicates,
    /// The request selects on demographic predicates while an IHE patient
    /// identity source holds the subject-to-EHR cross-reference: resolving
    /// the selected parties to EHRs would need the local map, which that
    /// provider does not keep, or a translation of the clinical pseudonym
    /// back to another domain, which the server never asks for.
    #[error(
        "a cohort predicate selects parties, and under the IHE patient identity source this \
         server holds no party-to-EHR map and never translates a pseudonym back; name the \
         cohort's subjects by identifier instead"
    )]
    IdentityNotHeld,
    /// A subject the cohort names could not be resolved: no provider is
    /// installed, or the identity source failed or answered no pseudonym.
    #[error("a cohort subject could not be resolved")]
    Subject(#[source] IndexError),
    /// The EHR index answered an `ehr_id` that is no UUID.
    #[error("the EHR index answered an ehr_id that is no UUID")]
    IndexAnswer(#[source] uuid::Error),
    /// The request pre-scopes the inner query to EHRs of its own.
    #[error("a cohort query carries no `ehr_ids` of its own — the cohort is the scope")]
    ScopeNotAllowed,
    /// The inner AQL execution failed.
    #[error("the cohort query failed")]
    Query(#[source] SmError),
    /// A demographic or linkage read failed.
    #[error("the cohort store is unavailable")]
    Database(#[source] sqlx::Error),
    /// No connection to the demographic or linkage store could be obtained,
    /// so nothing ran.
    #[error("the cohort store is unavailable")]
    Acquire(#[from] crate::db::acquire::AcquireError),
    /// The execution ran but its access record could not be taken, and the
    /// deployment fails closed, so the result is withheld.
    #[error("the access record could not be taken and the deployment fails closed")]
    Unrecorded(#[source] SmError),
}

impl From<super::store::StoreError> for CohortError {
    fn from(error: super::store::StoreError) -> Self {
        match error {
            super::store::StoreError::Acquire(e) => CohortError::Acquire(e),
            super::store::StoreError::Database(e) => CohortError::Database(e),
        }
    }
}

impl From<CohortError> for SmError {
    fn from(error: CohortError) -> Self {
        match error {
            CohortError::Query(inner) | CohortError::Unrecorded(inner) => inner,
            CohortError::Subject(inner) => SmError::from(inner),
            CohortError::NotConfigured
            | CohortError::UnknownPredicate(_)
            | CohortError::InvalidValue { .. }
            | CohortError::NoPredicates
            | CohortError::IdentityNotHeld
            | CohortError::ScopeNotAllowed => {
                let message = error.to_string();
                Self::new(CallStatusType::PreconditionViolation, message).with_source(error)
            }
            CohortError::CohortTooLarge { .. } => {
                let message = error.to_string();
                Self::new(CallStatusType::ContentInvalid, message).with_source(error)
            }
            CohortError::Database(_) | CohortError::IndexAnswer(_) => {
                crate::service::error::internal_fault_caused("execute a cohort query", &error)
                    .with_source(error)
            }
            CohortError::Acquire(e) => SmError::from(e),
        }
    }
}

impl FerroEhrService {
    /// Whether this deployment serves cohort queries
    /// ([`config::CohortConfig::is_enabled`]).
    #[must_use]
    pub fn cohort_enabled(&self) -> bool {
        self.cohort.is_enabled()
    }

    /// Select a cohort in the demographic domain, resolve it to EHRs through
    /// the linkage domain, and run `request.aql` over exactly those EHRs.
    ///
    /// The three steps and what each one may carry are the module
    /// documentation. An empty cohort runs no AQL at all — there is nothing to
    /// scope a query to — and answers with an empty `RESULT_SET` for the
    /// submitted text.
    ///
    /// # Errors
    /// [`CohortError::NotConfigured`] when the deployment binds no predicate,
    /// [`CohortError::NoPredicates`] / [`CohortError::ScopeNotAllowed`] /
    /// [`CohortError::IdentityNotHeld`] /
    /// [`CohortError::UnknownPredicate`] / [`CohortError::InvalidValue`] for a
    /// malformed request, [`CohortError::CohortTooLarge`] above the configured
    /// ceiling, [`CohortError::Acquire`] when no domain connection can be
    /// obtained, [`CohortError::Database`] when a domain read fails,
    /// [`CohortError::Subject`] when a named subject cannot be resolved,
    /// [`CohortError::IndexAnswer`] when the index answers no UUID,
    /// [`CohortError::Query`] when the AQL execution fails, and
    /// [`CohortError::Unrecorded`] when the access record could not be taken
    /// under `fail_mode = "closed"`.
    pub async fn execute_cohort_query(
        &self,
        request: CohortQueryRequest,
    ) -> Result<CohortOutcome, CohortError> {
        let hash = definition_hash(&request.predicates, &request.subjects);
        let purpose = request
            .purpose
            .clone()
            .or_else(crate::system_log::access_context::current_purpose);
        let outcome = self.cohort_query_inner(&request, &hash).await;
        let served = outcome.as_ref().map_or(0, |served| served.served_ehrs);
        self.emit_cohort_access(&hash, purpose, served, outcome.is_ok())?;
        outcome
    }

    /// The fallible body of [`Self::execute_cohort_query`], so the access
    /// record is taken on every path including the failures.
    async fn cohort_query_inner(
        &self,
        request: &CohortQueryRequest,
        hash: &str,
    ) -> Result<CohortOutcome, CohortError> {
        if !self.cohort.is_enabled() {
            return Err(CohortError::NotConfigured);
        }
        if request.predicates.is_empty() && request.subjects.is_empty() {
            return Err(CohortError::NoPredicates);
        }
        if !request.query.ehr_ids.is_empty() {
            return Err(CohortError::ScopeNotAllowed);
        }
        let max = u64::from(self.cohort.max_cohort_size);
        let selected = match (request.predicates.is_empty(), request.subjects.is_empty()) {
            (false, true) => self.predicate_ehrs(&request.predicates, max).await?,
            (true, false) => self.named_subject_ehrs(&request.subjects, max).await?,
            (false, false) => {
                let by_predicate = self.predicate_ehrs(&request.predicates, max).await?;
                let named = self.named_subject_ehrs(&request.subjects, max).await?;
                by_predicate.intersection(&named).copied().collect()
            }
            (true, true) => return Err(CohortError::NoPredicates),
        };
        let ehr_ids: Vec<EhrId> = selected.into_iter().collect();
        let cohort_size = u64::try_from(ehr_ids.len()).unwrap_or(u64::MAX);

        // Step three. An empty cohort scopes nothing, and an unscoped AQL is a
        // population query — the opposite of what was asked — so it never runs.
        if ehr_ids.is_empty() {
            return Ok(self.suppress(
                CohortOutcome {
                    query: QueryOutcome::plain(
                        crate::service::query::result_set::empty_result_set(&request.aql),
                    ),
                    cohort_size,
                    served_ehrs: 0,
                    suppressed: false,
                    definition_hash: hash.to_owned(),
                },
                hash,
            ));
        }
        let inner = AqlQueryRequest {
            ehr_ids: ehr_ids.iter().map(EhrId::to_string).collect(),
            ..request.query.clone()
        };
        let query = self
            .execute_aql(&request.aql, None, &inner)
            .await
            .map_err(CohortError::Query)?;
        let served_ehrs = u64::try_from(query.served_ehrs.len()).unwrap_or(u64::MAX);
        Ok(self.suppress(
            CohortOutcome {
                query,
                cohort_size,
                served_ehrs,
                suppressed: false,
                definition_hash: hash.to_owned(),
            },
            hash,
        ))
    }

    /// Steps one and two for the predicates: the parties they select, through
    /// the local map to the EHRs those parties are the subject of.
    ///
    /// # Errors
    /// [`CohortError::IdentityNotHeld`] under an IHE patient identity source,
    /// [`CohortError::CohortTooLarge`] above `max`, and the predicate and
    /// store failures.
    async fn predicate_ehrs(
        &self,
        predicates: &[CohortPredicate],
        max: u64,
    ) -> Result<BTreeSet<EhrId>, CohortError> {
        if matches!(self.ehr_index_provider(), EhrIndexProvider::Ihe(_)) {
            return Err(CohortError::IdentityNotHeld);
        }
        let parties = self.cohort_parties(predicates).await?;
        let size = u64::try_from(parties.len()).unwrap_or(u64::MAX);
        if size > max {
            return Err(CohortError::CohortTooLarge { size, max });
        }
        // Step two: the crossing. Identifiers in, identifiers out.
        let party_ids: Vec<VoId> = parties.into_iter().collect();
        Ok(super::store::open_mappings(&self.linkage_pool, &party_ids)
            .await
            .map_err(CohortError::from)?
            .into_iter()
            .collect())
    }

    /// The EHRs of the subjects the cohort names, each resolved by the
    /// installed provider: under an IHE identity source through the forward
    /// PIXm translation into the clinical domain, under the local map
    /// through the map. A subject no EHR belongs to adds nothing.
    ///
    /// # Errors
    /// [`CohortError::CohortTooLarge`] for more than `max` subjects,
    /// [`CohortError::Subject`] when a subject cannot be resolved, and
    /// [`CohortError::IndexAnswer`] for an `ehr_id` that is no UUID.
    async fn named_subject_ehrs(
        &self,
        subjects: &[SubjectRef],
        max: u64,
    ) -> Result<BTreeSet<EhrId>, CohortError> {
        let size = u64::try_from(subjects.len()).unwrap_or(u64::MAX);
        if size > max {
            return Err(CohortError::CohortTooLarge { size, max });
        }
        let mut ehrs = BTreeSet::new();
        for subject in subjects {
            let entries = self
                .provider_subject_ehrs(subject)
                .await
                .map_err(CohortError::Subject)?;
            for entry in entries {
                let id = entry
                    .ehr_id
                    .parse::<uuid::Uuid>()
                    .map_err(CohortError::IndexAnswer)?;
                ehrs.insert(EhrId(id));
            }
        }
        Ok(ehrs)
    }

    /// Step one: the intersection of every predicate's matching parties.
    ///
    /// Intersected in the application rather than in one `INTERSECT`: each
    /// predicate is its own constant statement, and composing them in SQL would
    /// make the statement text a function of how many predicates a caller sent.
    async fn cohort_parties(
        &self,
        predicates: &[CohortPredicate],
    ) -> Result<BTreeSet<VoId>, CohortError> {
        let mut selected: Option<BTreeSet<VoId>> = None;
        for requested in predicates {
            let binding = self
                .cohort
                .predicates
                .get(&requested.name)
                .ok_or_else(|| CohortError::UnknownPredicate(requested.name.clone()))?;
            let matched = predicate::matching_parties(
                &self.demographic_pool,
                &requested.name,
                binding,
                &requested.value,
            )
            .await?;
            selected = Some(match selected {
                None => matched,
                Some(previous) => previous.intersection(&matched).copied().collect(),
            });
            if selected.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        Ok(selected.unwrap_or_default())
    }

    /// Apply small-cell suppression and stamp `meta.cohort` on the result set.
    ///
    /// Suppression clears the rows rather than the whole document, so the
    /// caller still receives the query it submitted, its columns, and the
    /// statement that the cell was too small to serve.
    fn suppress(&self, mut outcome: CohortOutcome, hash: &str) -> CohortOutcome {
        let threshold = u64::from(self.cohort.small_cell_threshold);
        if threshold > 0 && outcome.served_ehrs > 0 && outcome.served_ehrs < threshold {
            outcome.suppressed = true;
            outcome.served_ehrs = 0;
            outcome.query.served_ehrs.clear();
            outcome.query.served_rows = 0;
            if let Some(map) = outcome.query.result_set.as_object_mut() {
                map.insert("rows".to_owned(), serde_json::Value::Array(Vec::new()));
            }
        }
        // NOTE: no openEHR spec governs this key — the released `ResultSetMetadata`
        // declares `additionalProperties: true`, so the cohort facts ride there.
        if let Some(meta) = outcome
            .query
            .result_set
            .get_mut("meta")
            .and_then(serde_json::Value::as_object_mut)
        {
            meta.insert(
                "cohort".to_owned(),
                serde_json::json!({
                    "size": outcome.cohort_size,
                    "served_ehrs": outcome.served_ehrs,
                    "suppressed": outcome.suppressed,
                    "definition": hash,
                }),
            );
        }
        outcome
    }

    /// Record one cohort execution as a linkage-domain access.
    ///
    /// The record names the cohort by its definition DIGEST, never by the
    /// values selected on: the values are the cohort, and a trail carrying them
    /// would hold the very selection the boundary exists to keep apart from the
    /// clinical content it reached.
    ///
    /// # Errors
    /// [`CohortError::Unrecorded`] when the sender rejected the record under
    /// `fail_mode = "closed"`; the caller withholds its result.
    fn emit_cohort_access(
        &self,
        hash: &str,
        purpose: Option<String>,
        served: u64,
        succeeded: bool,
    ) -> Result<(), CohortError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let outcome = if succeeded {
            EventOutcome::Success
        } else {
            EventOutcome::MinorFailure
        };
        let mut event = AuditEvent::new(EventActionCode::Execute, ObjectClass::Query, outcome);
        event.domain = AccessDomain::Linkage;
        event.object_id = Some(format!("cohort:{hash}"));
        event.result_count = Some(served);
        super::stamp_requester(&mut event);
        event.purpose = purpose;
        event.permit = crate::system_log::access_context::current_permit();
        event.legal_basis = self.audit_legal_basis().map(str::to_owned);
        self.record_access(event).map_err(CohortError::Unrecorded)
    }
}

/// The hex SHA-256 of the canonical predicate list: sorted by name, each
/// rendered `name=value`, joined by newlines, then one `subjects@<namespace>`
/// line per named subject, sorted.
///
/// Canonicalized so the same cohort has the same identity whichever order a
/// caller sent it in, and digested so the access record can name the cohort
/// without carrying what it selected on. A subject enters by its namespace
/// alone: a digest over a national identifier's value could be reversed by
/// trying every value, so it would carry the identifier after all.
fn definition_hash(predicates: &[CohortPredicate], subjects: &[SubjectRef]) -> String {
    let mut sorted: Vec<&CohortPredicate> = predicates.iter().collect();
    sorted.sort_by(|a, b| (&a.name, &a.value).cmp(&(&b.name, &b.value)));
    let mut namespaces: Vec<&str> = subjects.iter().map(|s| s.namespace.as_str()).collect();
    namespaces.sort_unstable();
    let canonical = sorted
        .iter()
        .map(|p| format!("{}={}", p.name, p.value))
        .chain(namespaces.iter().map(|ns| format!("subjects@{ns}")))
        .collect::<Vec<_>>()
        .join("\n");
    sha2::Sha256::digest(canonical.as_bytes()).iter().fold(
        String::with_capacity(64),
        |mut out, byte| {
            use std::fmt::Write as _;
            // NOTE: `fmt::Write` on a `String` never fails — its `write_str`
            // returns `Ok` unconditionally
            // (<https://doc.rust-lang.org/std/fmt/trait.Write.html#impl-Write-for-String>).
            let _outcome = write!(out, "{byte:02x}");
            out
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition_hash_of(predicates: &[CohortPredicate]) -> String {
        definition_hash(predicates, &[])
    }

    fn predicate(name: &str, value: &str) -> CohortPredicate {
        CohortPredicate {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn the_definition_hash_is_order_independent_and_value_sensitive() {
        let a = definition_hash_of(&[predicate("city", "Groningen"), predicate("sex", "F")]);
        let b = definition_hash_of(&[predicate("sex", "F"), predicate("city", "Groningen")]);
        assert_eq!(a, b, "the same cohort has one identity");
        assert_ne!(
            a,
            definition_hash_of(&[predicate("city", "Assen"), predicate("sex", "F")]),
            "a different cohort has a different identity"
        );
        assert_eq!(a.len(), 64, "hex SHA-256");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// The digest carries no value, which is the property that makes it safe in
    /// the access record.
    #[test]
    fn the_definition_hash_carries_no_predicate_value() {
        let hash = definition_hash_of(&[predicate("city", "Groningen")]);
        assert!(!hash.contains("Groningen") && !hash.contains("city"));
    }

    /// A named subject enters the digest by its namespace, never its value.
    #[test]
    fn the_definition_hash_carries_no_subject_value() {
        let subject = |id: &str| SubjectRef::person(id, "urn:oid:2.16.840.1.113883.2.4.6.3");
        let one = definition_hash(&[], &[subject("synthetic-0019")]);
        assert_eq!(one, definition_hash(&[], &[subject("synthetic-0020")]));
        assert_ne!(
            one,
            definition_hash(&[], &[]),
            "a subject cohort is not the empty one"
        );
        assert!(!one.contains("synthetic-0019"));
    }

    #[test]
    fn the_refusals_map_onto_their_call_statuses() {
        for (error, expected) in [
            (
                CohortError::NotConfigured,
                CallStatusType::PreconditionViolation,
            ),
            (
                CohortError::NoPredicates,
                CallStatusType::PreconditionViolation,
            ),
            (
                CohortError::ScopeNotAllowed,
                CallStatusType::PreconditionViolation,
            ),
            (
                CohortError::IdentityNotHeld,
                CallStatusType::PreconditionViolation,
            ),
            (
                CohortError::UnknownPredicate("nope".to_owned()),
                CallStatusType::PreconditionViolation,
            ),
            (
                CohortError::InvalidValue {
                    name: "age_band".to_owned(),
                    reason: "bad".to_owned(),
                },
                CallStatusType::PreconditionViolation,
            ),
            (
                CohortError::CohortTooLarge {
                    size: 200_000,
                    max: 100_000,
                },
                CallStatusType::ContentInvalid,
            ),
            (
                CohortError::Unrecorded(SmError::new(
                    CallStatusType::ServiceOverloaded,
                    "audit trail unavailable (fail-closed)",
                )),
                CallStatusType::ServiceOverloaded,
            ),
        ] {
            assert_eq!(SmError::from(error).status, expected);
        }
    }
}
