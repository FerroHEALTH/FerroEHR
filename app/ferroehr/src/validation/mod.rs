// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Archetype / template artefact validation (the ingestion surface).
//!
//! The platform crate has three validation surfaces; this module owns the
//! artefact validators and names the seams to the other two.
//!
//! - Surface A, artefact validity (owned here): the AOM 1.4/2.4 constraint
//!   catalogue applied to an uploaded artefact before it becomes an operational
//!   template. `opt` carries OPT 1.4 artefact validity, the AOM2/08 catalogue
//!   over a flattened `openehr_its::opt14::types::OperationalTemplate`;
//!   `structure` carries OPT XML well-formedness, a CNF ingestion guard rather
//!   than an AOM constraint kind. ADL2 source validation is the full
//!   `openehr-adl` engine (parse, then AOM2 phases 1-3), invoked from
//!   `service::definition::{adl2,wire}`.
//! - Surface B, the instance `valid_value` cascade (a seam, not here): the
//!   recursive top-down data-conformance function
//!   (`AM/docs/AOM1.4/master04-constraint_model_package.adoc` §`Valid_value`)
//!   runs at commit time over the compacted `WebTemplate`, lives in
//!   `openehr_sdt::flat`, and is invoked from the EHR commit choke point
//!   `service::ehr::composition_validate`.
//! - Surface C, per-kind RM structural validators (a seam, not here): the
//!   RM-invariant checks on template-less commit bodies (`EHR_STATUS`,
//!   `EHR_ACCESS`, `FOLDER`, party roots) live with their kinds under
//!   `service/`, routed by `service::ehr`'s `validate_for_commit` so no commit
//!   path bypasses validation.
//!
//! The files under `opt` follow the AM constraint taxonomy (structural
//! invariant, RM conformance, primitive, terminology, interval), the axis the
//! AOM2/08 catalogue itself uses, so a file traces to a catalogue section. The
//! functions below are the crate-facing contract (`crate::validation::<fn>`);
//! the machinery behind them is module-private.
//!
//! NOTE: AOM 1.4 defines only the positive `valid_value` cascade and is silent
//! on present-but-unmatched nodes, so surface B's closed-world check follows the
//! AOM2 `c_conforms_to` VSONCT/VSONCO formalization
//! (`AM/docs/AOM2/master08-validation.adoc` §Phase 2).
//!
//! The VTTBK/VTCBK rules here are artefact-side: a binding's key must be a
//! defined at- or ac-code (`AM/docs/AOM2/master08-validation.adoc`
//! §Terminology). The data-side consequence, that an instance code belongs to
//! the value set the ac-code's `CONSTRAINT_REF` is bound to (BASE
//! `architecture_overview/master12-terminology.adoc` §"Binding Terminology
//! Value-sets to Archetypes"), needs the external terminology query server and
//! is resolved at commit by `service::terminology::binding`.

mod opt;
mod structure;

use openehr_its::opt14::types::OperationalTemplate;

use crate::service::error::ServiceError;

/// Reject an OPT the tolerant `openehr_its::opt14` codec would silently
/// accept: a foreign top-level element (CNF `invalid_templates/alien_tags`)
/// or a duplicated single-valued top-level element (CNF
/// `invalid_templates/multiple_elements`).
///
/// # Errors
///
/// [`ServiceError::Unprocessable`] (→ ITS-REST `422`) naming the offending
/// element, or describing the XML parse failure when the document is
/// malformed.
pub(crate) fn validate_opt_structure(xml: &str) -> Result<(), ServiceError> {
    structure::validate_opt_structure(xml)
}

/// Validate an uploaded OPT 1.4 artefact against the AOM2/08
/// standalone-artefact validity rules
/// (`AM/docs/AOM2/master08-validation.adoc`; rule map in the `opt` module
/// doc).
///
/// A fully valid artefact returns `Ok`.
///
/// # Errors
///
/// The first violation found, as [`ServiceError::ValidationFailed`] (→
/// ITS-REST `422` rendering the `Error` object with the AOM2 rule code in
/// `validationErrors[]`): a rule violation on a successfully parsed artefact
/// is a semantic error (the overview status table's `422` row), never the
/// syntactic `400` branch of `responses/400.yaml`.
pub fn validate_opt_artefact(opt: &OperationalTemplate) -> Result<(), ServiceError> {
    opt::validate_opt_artefact(opt)
}
