// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ConceptMap/$translate` against the one terminology server: the code a
//! foreign coding stands for in a target code system.
//!
//! The access directive exchange translates an action of a FHIR `Consent`
//! coded in another system, such as HL7 `consentaction`, to an operation
//! class of the access control terminology through the deployment's concept
//! maps, never through a table FerroEHR embeds (FHIR R4
//! `conceptmap-operation-translate.html`).

use ferrobridge_term::request::Translation;

use crate::error::{ErrorChain, ServiceError};
use crate::service::terminology::fhir::FhirTerminologyProvider;

impl FhirTerminologyProvider {
    /// The code `code` of `system` translates to in `target_system`: the
    /// first `equivalent` or `equal` match (R4
    /// `valueset-concept-map-equivalence.html`), `None` when the server
    /// answers no such match.
    ///
    /// # Errors
    /// `exception` when the call fails or the server refuses it; a refusal is
    /// never read as a missing translation.
    pub async fn translate_to(
        &self,
        system: &str,
        code: &str,
        target_system: &str,
    ) -> Result<Option<String>, ServiceError> {
        let translation = Translation::new(system, code).with_target_system(target_system);
        let outcome = self.wire.translate(&translation).await.map_err(|error| {
            tracing::error!(
                system,
                code,
                error = %ErrorChain::new(&error),
                "terminology $translate failed"
            );
            ServiceError::fault_caused("the terminology server did not translate a code", error)
        })?;
        Ok(outcome
            .accepted()
            .filter_map(|m| m.concept.as_ref())
            .find(|c| c.system.as_deref().is_none_or(|s| s == target_system))
            .and_then(|c| c.code.clone()))
    }
}
