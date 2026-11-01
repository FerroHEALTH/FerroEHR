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
/// `openehr-adl` is a network-free spec engine whose VETDF check consults a
/// synchronous [`TerminologyResolver`] seam, while a terminology lookup is
/// asynchronous. The service therefore pre-resolves every external term binding
/// of the uploaded archetype against its terminology service
/// ([`FerroEhrService::term_existence`], on the system and code the binding
/// URI names) and hands the validator this memoised map.
///
/// `code_exists` returns `Some(true)`/`Some(false)` for a binding the service
/// could answer, and `None` for one it could not (no external provider
/// configured, a code system the server does not serve, or a transport
/// fault) — matching the
/// VETDF "subject to tool accessibility; … no verification was possible"
/// carve-out (AM ADL2 `master03-archetype_package.adoc` §Validity Rules).
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
    /// Pre-resolve every external term binding of `archetype` against the
    /// terminology service, building the [`AdlTerminologyResolver`] the VETDF
    /// check consults (see its docs — the seam is synchronous, a lookup is not).
    ///
    /// A binding target is a URI in the IHTSDO model (`…/id/<code>`), so it is
    /// taken apart into the FHIR `(system, code)` the server is asked about
    /// ([`code_system_and_code`]); a target of another shape is asked as
    /// written under the binding's own terminology id. Only a definite answer
    /// lands in the resolver: exists, or absent from a code system the server
    /// serves. A binding the service cannot verify — no configured external
    /// provider, a code system the server does not serve, a transport fault —
    /// is left unresolved and logged, so VETDF is not raised for it
    /// (`master03` §Validity Rules: "codes for inaccessible terminologies
    /// should be flagged with a warning indicating that no verification was
    /// possible"). Only genuinely external terminologies are consulted; the
    /// archetype-internal `local`/`openehr` ids are excluded by
    /// [`external_term_bindings`] (their keys are covered by VTTBK/VTCBK).
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
