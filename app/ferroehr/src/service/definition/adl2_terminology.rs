// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The terminology seam of ADL2 validation: the external term bindings of an
//! uploaded archetype, resolved against the terminology service before the
//! synchronous `openehr-adl` VETDF check runs.

use std::collections::HashMap;

use openehr_adl::validate::bindings::{TerminologyResolver, external_term_bindings};
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;

use crate::service::FerroEhrService;
use crate::service::definition::binding_uri::code_system_and_code;
use crate::service::terminology::fhir::TermExistence;

/// A memoised [`TerminologyResolver`] for ADL2 VETDF validation.
///
/// The `openehr-adl` resolver seam is synchronous and a terminology lookup is
/// not, so the service resolves every external term binding first
/// ([`FerroEhrService::term_existence`]) and hands the validator this map.
/// `code_exists` answers `Some` for a binding the service could answer and
/// `None` for one it could not, the VETDF "no verification was possible"
/// case (AM ADL2 `master03-archetype_package.adoc` §Validity Rules).
#[derive(Debug, Default)]
pub(super) struct AdlTerminologyResolver {
    /// `(terminology_id, external target)` → the target's existence, for every
    /// binding the terminology service could answer.
    resolved: HashMap<(String, String), bool>,
}

impl TerminologyResolver for AdlTerminologyResolver {
    fn code_exists(&self, terminology_id: &str, code: &str) -> Option<bool> {
        self.resolved
            .get(&(terminology_id.to_owned(), code.to_owned()))
            .copied()
    }
}

impl FerroEhrService {
    /// Resolves every external term binding of `archetype` against the
    /// terminology service into the [`AdlTerminologyResolver`] the VETDF check
    /// consults.
    ///
    /// A target URI in the IHTSDO form (`…/id/<code>`) is asked as its FHIR
    /// `(system, code)` ([`code_system_and_code`]), any other as written under
    /// the binding's terminology id. Only a definite answer is recorded; a
    /// binding the service cannot verify is logged and left open, so VETDF is not
    /// raised (`master03` §Validity Rules). The `local` and `openehr` ids are
    /// excluded by [`external_term_bindings`].
    pub(super) async fn adl2_terminology_resolver(
        &self,
        archetype: &Archetype,
    ) -> AdlTerminologyResolver {
        let mut resolved = HashMap::new();
        for binding in external_term_bindings(archetype) {
            let (system, code) = code_system_and_code(&binding.target)
                .unwrap_or_else(|| (binding.terminology_id.clone(), binding.target.clone()));
            match self.term_existence(&system, &code).await {
                Ok(TermExistence::Exists) => {
                    resolved.insert((binding.terminology_id, binding.target), true);
                }
                Ok(TermExistence::Absent) => {
                    resolved.insert((binding.terminology_id, binding.target), false);
                }
                Ok(TermExistence::Unverifiable(reason)) => tracing::warn!(
                    terminology = %binding.terminology_id,
                    target = %binding.target,
                    system = %system,
                    reason,
                    "VETDF: no verification was possible for this term binding; accepted unverified"
                ),
                Err(e) => tracing::warn!(
                    terminology = %binding.terminology_id,
                    target = %binding.target,
                    system = %system,
                    error = %e,
                    "VETDF: the terminology server could not be asked; accepted unverified"
                ),
            }
        }
        AdlTerminologyResolver { resolved }
    }
}
