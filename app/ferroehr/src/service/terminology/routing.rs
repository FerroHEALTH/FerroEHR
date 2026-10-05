// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The 9 SM `I_TERMINOLOGY_SERVICE` calls on
//! [`FerroEhrService`], routing between the two providers
//! (`i_terminology_service.adoc`; the routing rule — see the module docs in
//! [`super`]).
//!
//! Enumeration is always the bundle's; lookup/validation goes to the bundle
//! when it knows the terminology, else to the FHIR provider **the
//! `terminology_id` routes to** ([`super::router::TerminologyRouter`] — several
//! servers may be configured at once, BASE
//! `docs/architecture_overview/master12-terminology.adoc` §Overview), else
//! falls through to the bundle's `Pre_has_terminology` → `NotFound`.

use std::collections::BTreeMap;

use crate::service::FerroEhrService;
use crate::service::status::SmError;
use crate::service::terminology::types::{TerminologyDescription, TerminologyExtract};

use super::bundle;

impl FerroEhrService {
    /// `get_terminology_ids` — every terminology id this server knows:
    /// `"openehr"` plus the bundle's external code-set ids (enumeration is
    /// the bundle's).
    ///
    /// # Errors
    ///
    /// Infallible in practice (the bundle is compile-time-embedded); the
    /// `Result` is the SM call shape.
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "the SM interface declares this call on the service and in the \
                  SM call-status `Result` shape; the protocol adapter invokes \
                  every SM call uniformly, so neither is dropped because this \
                  particular realization happens to be stateless and infallible"
    )]
    pub fn get_terminology_ids(&self) -> Result<Vec<String>, SmError> {
        Ok(bundle::terminology_ids())
    }

    /// `has_terminology` — whether `terminology_id` names a terminology this
    /// server can enumerate (enumeration is the bundle's).
    ///
    /// # Errors
    ///
    /// Infallible in practice; the `Result` is the SM call shape.
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "the SM interface declares this call on the service and in the \
                  SM call-status `Result` shape; the protocol adapter invokes \
                  every SM call uniformly, so neither is dropped because this \
                  particular realization happens to be stateless and infallible"
    )]
    pub fn has_terminology(&self, terminology_id: &str) -> Result<bool, SmError> {
        Ok(bundle::has_terminology(terminology_id))
    }

    /// `get_terminology_description` — the descriptor of one terminology
    /// (enumeration is the bundle's).
    ///
    /// # Errors
    ///
    /// `VersionedObjectDoesNotExist` when the bundle does not know
    /// `terminology_id` (`Pre_has_terminology`).
    #[expect(
        clippy::unused_self,
        reason = "the SM interface declares this call on the service; the \
                  protocol adapter invokes every SM call uniformly, so the \
                  receiver stays even where this realization ignores it"
    )]
    pub fn get_terminology_description(
        &self,
        terminology_id: &str,
    ) -> Result<TerminologyDescription, SmError> {
        bundle::terminology_description(terminology_id)
    }

    /// `has_term` — whether `code` is a term of `terminology_id`. Routed to
    /// the bundle when it knows the terminology, else to the configured FHIR
    /// provider (`CodeSystem/$lookup`).
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology
    ///   (`Pre_has_terminology`); `at_date` is a no-op on the single pinned
    ///   bundle version (bundle NOTE);
    /// - FHIR path: exception on a transport fault / non-2xx / malformed
    ///   response.
    pub async fn has_term(
        &self,
        terminology_id: &str,
        code: &str,
        at_date: Option<String>,
    ) -> Result<bool, SmError> {
        if bundle::has_terminology(terminology_id) {
            bundle::has_term(terminology_id, code)
        } else if let Some(p) = self.terminology_provider(terminology_id) {
            p.has_term(terminology_id, code, at_date).await
        } else {
            bundle::has_term(terminology_id, code)
        }
    }

    /// Whether `code` exists in the external code system `system`, as the three
    /// answers ADL2 VETDF distinguishes ([`super::fhir::TermExistence`]): the
    /// binding's outer key `terminology_id` only helps pick the provider (its
    /// own route, then the system's route, then the default); with no external
    /// provider configured nothing is asked and the binding is unverifiable
    /// (AM ADL2 `master03-archetype_package.adoc` §Validity Rules, VETDF).
    ///
    /// # Errors
    ///
    /// The FHIR provider's: a transport fault, an unclassified non-2xx status,
    /// or a malformed body.
    pub async fn term_existence(
        &self,
        terminology_id: &str,
        system: &str,
        code: &str,
    ) -> Result<super::fhir::TermExistence, SmError> {
        let provider = self
            .terminology_route(system)
            .or_else(|| self.terminology_provider(terminology_id));
        match provider {
            Some(p) => p.term_existence(system, code).await,
            None => Ok(super::fhir::TermExistence::Unverifiable(
                "no external terminology provider is configured",
            )),
        }
    }

    /// `get_term` — a single-term `Terminology_extract`. Routed to the bundle
    /// when it knows the terminology (no meta-model `attributes` exist for
    /// the openEHR bundle; `at_date` is a no-op on the pinned version), else to the configured FHIR provider (`CodeSystem/$lookup`).
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology
    ///   or an unknown code (`Pre_has_terminology` + `Pre_has_term`);
    /// - FHIR path: `VersionedObjectDoesNotExist` when `$lookup` answers
    ///   `404`; exception on a transport fault / non-2xx / malformed
    ///   response.
    pub async fn get_term(
        &self,
        terminology_id: &str,
        code: &str,
        attributes: Option<BTreeMap<String, String>>,
        at_date: Option<String>,
    ) -> Result<TerminologyExtract, SmError> {
        if bundle::has_terminology(terminology_id) {
            bundle::get_term(terminology_id, code)
        } else if let Some(p) = self.terminology_provider(terminology_id) {
            p.get_term(terminology_id, code, attributes, at_date).await
        } else {
            bundle::get_term(terminology_id, code)
        }
    }

    /// `subsumes` — whether `candidate_child_code` is in the **strict**
    /// subsumption of `ref_code`. The flat bundle always answers `false`
    /// (bundle NOTE); hierarchical subsumption is the FHIR provider's
    /// `CodeSystem/$subsumes`.
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology
    ///   (`Pre_has_terminology`);
    /// - FHIR path: `VersionedObjectDoesNotExist` when the server answers
    ///   `404`; exception on a transport fault / a response with no
    ///   `outcome`.
    pub async fn subsumes(
        &self,
        terminology_id: &str,
        ref_code: &str,
        candidate_child_code: &str,
    ) -> Result<bool, SmError> {
        if bundle::has_terminology(terminology_id) {
            bundle::subsumes(terminology_id, ref_code, candidate_child_code)
        } else if let Some(p) = self.terminology_provider(terminology_id) {
            p.subsumes(terminology_id, ref_code, candidate_child_code)
                .await
        } else {
            bundle::subsumes(terminology_id, ref_code, candidate_child_code)
        }
    }

    /// `value_set_validate` — set membership of `candidate_code` in the value
    /// set. Routed to the bundle when it knows the terminology, else to the
    /// configured FHIR provider (`ValueSet/$validate-code` or `$expand` +
    /// membership).
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology
    ///   (`Pre_has_terminology`); an unknown value set answers `false` (no
    ///   precondition on the membership test itself);
    /// - FHIR path: precondition on an empty `candidate_code`;
    ///   `VersionedObjectDoesNotExist` on an unknown value set; exception on
    ///   a transport fault / a `$validate-code` response with no `result`.
    pub async fn value_set_validate(
        &self,
        terminology_id: &str,
        value_set_id: &str,
        candidate_code: &str,
        at_date: Option<String>,
    ) -> Result<bool, SmError> {
        if bundle::has_terminology(terminology_id) {
            bundle::value_set_validate(terminology_id, value_set_id, candidate_code)
        } else if let Some(p) = self.terminology_provider(terminology_id) {
            p.value_set_validate(terminology_id, value_set_id, candidate_code, at_date)
                .await
        } else {
            bundle::value_set_validate(terminology_id, value_set_id, candidate_code)
        }
    }

    /// `has_value_set` — whether the value set exists (total: no
    /// precondition; an unknown terminology answers `false` on the bundle
    /// path).
    ///
    /// # Errors
    ///
    /// FHIR path only: exception on a transport fault / non-2xx response
    /// (the `$expand` probe).
    pub async fn has_value_set(
        &self,
        terminology_id: &str,
        value_set_code: &str,
    ) -> Result<bool, SmError> {
        if bundle::has_terminology(terminology_id) {
            Ok(bundle::has_value_set(terminology_id, value_set_code))
        } else if let Some(p) = self.terminology_provider(terminology_id) {
            p.has_value_set(terminology_id, value_set_code).await
        } else {
            Ok(bundle::has_value_set(terminology_id, value_set_code))
        }
    }

    /// `get_value_set` — the value set's `Terminology_extract`. Routed to the
    /// bundle when it knows the terminology, else to the configured FHIR
    /// provider (`ValueSet/$expand`).
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology
    ///   or an unknown value set (`Pre_has_terminology` + `Pre_has_value_set`);
    /// - FHIR path: `VersionedObjectDoesNotExist` when `$expand` answers
    ///   `404`; exception on a transport fault / non-2xx / malformed
    ///   response.
    pub async fn get_value_set(
        &self,
        terminology_id: &str,
        value_set_code: &str,
    ) -> Result<TerminologyExtract, SmError> {
        if bundle::has_terminology(terminology_id) {
            bundle::get_value_set(terminology_id, value_set_code)
        } else if let Some(p) = self.terminology_provider(terminology_id) {
            p.get_value_set(terminology_id, value_set_code).await
        } else {
            bundle::get_value_set(terminology_id, value_set_code)
        }
    }
}
