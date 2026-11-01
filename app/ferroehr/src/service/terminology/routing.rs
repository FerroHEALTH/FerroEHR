// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The 9 terminology calls on [`FerroEhrService`], answered by the in-process
//! bundle or the one terminology server (see the module docs in [`super`]).
//!
//! Enumeration is always the bundle's; lookup/validation goes to the bundle
//! when it knows the terminology, else to the configured terminology server,
//! else falls through to the bundle's unknown-terminology `NotFound`.

use std::collections::BTreeMap;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
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
    /// `Result` is the shape every terminology call shares.
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "every service call shares one method-on-the-service, `Result`-returning \
                  shape the protocol adapter invokes uniformly, so neither is dropped because \
                  this implementation happens to be stateless and infallible"
    )]
    pub fn get_terminology_ids(&self) -> Result<Vec<String>, ServiceError> {
        Ok(bundle::terminology_ids())
    }

    /// `has_terminology` — whether `terminology_id` names a terminology this
    /// server can enumerate (enumeration is the bundle's).
    ///
    /// # Errors
    ///
    /// Infallible in practice; the `Result` is the shape every terminology call shares.
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "every service call shares one method-on-the-service, `Result`-returning \
                  shape the protocol adapter invokes uniformly, so neither is dropped because \
                  this implementation happens to be stateless and infallible"
    )]
    pub fn has_terminology(&self, terminology_id: &str) -> Result<bool, ServiceError> {
        Ok(bundle::has_terminology(terminology_id))
    }

    /// `get_terminology_description` — the descriptor of one terminology
    /// (enumeration is the bundle's).
    ///
    /// # Errors
    ///
    /// `VersionedObjectDoesNotExist` when the bundle does not know
    /// `terminology_id`.
    #[expect(
        clippy::unused_self,
        reason = "every service call is a method on the service, which the protocol \
                  adapter invokes uniformly, so the receiver stays even where this \
                  implementation ignores it"
    )]
    pub fn get_terminology_description(
        &self,
        terminology_id: &str,
    ) -> Result<TerminologyDescription, ServiceError> {
        bundle::terminology_description(terminology_id)
    }

    /// `has_term` — whether `code` is a term of `terminology_id`. Routed to
    /// the bundle when it knows the terminology, else to the configured
    /// terminology server (`CodeSystem/$lookup`).
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology; `at_date` is a no-op on the single pinned
    ///   bundle version (bundle NOTE);
    /// - FHIR path: exception on a transport fault / non-2xx / malformed
    ///   response.
    pub async fn has_term(
        &self,
        terminology_id: &str,
        code: &str,
        at_date: Option<String>,
    ) -> Result<bool, ServiceError> {
        if bundle::has_terminology(terminology_id) {
            bundle::has_term(terminology_id, code)
        } else if let Some(server) = self.terminology_server() {
            server.has_term(terminology_id, code, at_date).await
        } else {
            bundle::has_term(terminology_id, code)
        }
    }

    /// Whether `code` exists in the external code system `system`, as the three
    /// answers ADL2 VETDF distinguishes ([`super::fhir::TermExistence`]): the
    /// code system is asked of the one terminology server; with no server
    /// configured nothing is asked and the binding is unverifiable
    /// (AM ADL2 `master03-archetype_package.adoc` §Validity Rules, VETDF).
    ///
    /// # Errors
    ///
    /// The terminology server's: a transport fault, an unclassified non-2xx status,
    /// or a malformed body.
    pub async fn term_existence(
        &self,
        system: &str,
        code: &str,
    ) -> Result<super::fhir::TermExistence, ServiceError> {
        match self.terminology_server() {
            Some(server) => server.term_existence(system, code).await,
            None => Ok(super::fhir::TermExistence::Unverifiable(
                "no terminology server is configured",
            )),
        }
    }

    /// `get_term` — a single-term `Terminology_extract`. Routed to the bundle
    /// when it knows the terminology (no meta-model `attributes` exist for
    /// the openEHR bundle; `at_date` is a no-op on the pinned version), else to the configured terminology server (`CodeSystem/$lookup`).
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology
    ///   or an unknown code;
    /// - FHIR path: `VersionedObjectDoesNotExist` when `$lookup` answers
    ///   `404`; exception on a transport fault / non-2xx / malformed
    ///   response.
    pub async fn get_term(
        &self,
        terminology_id: &str,
        code: &str,
        attributes: Option<BTreeMap<String, String>>,
        at_date: Option<String>,
    ) -> Result<TerminologyExtract, ServiceError> {
        if bundle::has_terminology(terminology_id) {
            bundle::get_term(terminology_id, code)
        } else if let Some(server) = self.terminology_server() {
            server
                .get_term(terminology_id, code, attributes, at_date)
                .await
        } else {
            bundle::get_term(terminology_id, code)
        }
    }

    /// `subsumes` — whether `candidate_child_code` is in the **strict**
    /// subsumption of `ref_code`. The flat bundle always answers `false`
    /// (bundle NOTE); hierarchical subsumption is the terminology server's
    /// `CodeSystem/$subsumes`.
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology;
    /// - FHIR path: `VersionedObjectDoesNotExist` when the server answers
    ///   `404`; exception on a transport fault / a response with no
    ///   `outcome`.
    pub async fn subsumes(
        &self,
        terminology_id: &str,
        ref_code: &str,
        candidate_child_code: &str,
    ) -> Result<bool, ServiceError> {
        if bundle::has_terminology(terminology_id) {
            bundle::subsumes(terminology_id, ref_code, candidate_child_code)
        } else if let Some(server) = self.terminology_server() {
            server
                .subsumes(terminology_id, ref_code, candidate_child_code)
                .await
        } else {
            bundle::subsumes(terminology_id, ref_code, candidate_child_code)
        }
    }

    /// `value_set_validate` — set membership of `candidate_code` in the value
    /// set. Routed to the bundle when it knows the terminology, else to the
    /// configured terminology server (`ValueSet/$validate-code`).
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology; an unknown value set answers `false` (no
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
    ) -> Result<bool, ServiceError> {
        if bundle::has_terminology(terminology_id) {
            bundle::value_set_validate(terminology_id, value_set_id, candidate_code)
        } else if let Some(server) = self.terminology_server() {
            server
                .value_set_validate(terminology_id, value_set_id, candidate_code, at_date)
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
    ) -> Result<bool, ServiceError> {
        if bundle::has_terminology(terminology_id) {
            Ok(bundle::has_value_set(terminology_id, value_set_code))
        } else if let Some(server) = self.terminology_server() {
            server.has_value_set(terminology_id, value_set_code).await
        } else {
            Ok(bundle::has_value_set(terminology_id, value_set_code))
        }
    }

    /// `get_value_set` — the value set's `Terminology_extract`. Routed to the
    /// bundle when it knows the terminology, else to the configured
    /// terminology server (`ValueSet/$expand`).
    ///
    /// # Errors
    ///
    /// - bundle path: `VersionedObjectDoesNotExist` on an unknown terminology
    ///   or an unknown value set;
    /// - FHIR path: `VersionedObjectDoesNotExist` when `$expand` answers
    ///   `404`; exception on a transport fault / non-2xx / malformed
    ///   response.
    pub async fn get_value_set(
        &self,
        terminology_id: &str,
        value_set_code: &str,
    ) -> Result<TerminologyExtract, ServiceError> {
        if bundle::has_terminology(terminology_id) {
            bundle::get_value_set(terminology_id, value_set_code)
        } else if let Some(server) = self.terminology_server() {
            server.get_value_set(terminology_id, value_set_code).await
        } else {
            bundle::get_value_set(terminology_id, value_set_code)
        }
    }
}
