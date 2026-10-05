// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OPT 1.4 ingestion: canonical XML → [`OperationalTemplate`] parse.
//!
//! An operational template is the compiled, inheritance-flattened, standalone
//! top-level artefact (`AM/docs/OPT2/master02-overview.adoc` §Purpose of the
//! OPT, §Types of OPT; `master03-opt_raw.adoc` §Flattening) and, descending from
//! `AUTHORED_RESOURCE` (`BASE/docs/resource/master02-resource_package.adoc`
//! §Meta-data), carries the original language, translations,
//! `RESOURCE_DESCRIPTION` and revision history.
//!
//! NOTE: OPT 1.4 has no normative prose chapter, the OPT2 masters describing the
//! ADL2 successor, so the canonical XML ingested here is governed by the ITS-XML
//! v1 Template XSD plus AOM 1.4, which are what a structure-conformance claim
//! cites.
//!
//! The tolerant [`openehr_its::opt14`] codec decodes the document; the
//! structural well-formedness gate closing that codec's leniency belongs to the
//! artefact-validity area
//! (`crate::validation::structure::validate_opt_structure`), which the store
//! calls before every ingest.
//!
//! NOTE: the `AUTHORED_RESOURCE` meta-data is parsed by the codec but only
//! `template_id`, `concept` and the root archetype are indexed for lookup and
//! listing, the spec permitting an optional `_description_` (BASE resource
//! master02 §Meta-data).

use openehr_its::opt14::types::OperationalTemplate;

use crate::service::error::ServiceError;

/// Parse OPT 1.4 canonical XML into an [`OperationalTemplate`].
///
/// Failure classification follows the ITS-REST status split: a payload that is
/// not well-formed XML at all is *syntactically invalid content* — the released
/// `400` branch (`docs/specs/openehr/ITS-REST/specifications/responses/400.yaml`:
/// "the request could not be parsed or is invalid (e.g. … syntactically
/// invalid … content)") — while well-formed XML that does not decode as an OPT
/// is a semantic error on the artefact (the overview status table's `422` row,
/// `docs/specs/openehr/ITS-REST/specifications/docs/overview/
/// Requests_and_responses.md` §HTTP status codes; no template operation
/// declares `422`, so the semantic branch is the adjudicated handling).
///
/// # Errors
///
/// - [`ServiceError::BadRequest`] (→ ITS-REST `400`) when the payload is not
///   well-formed XML (including an empty document).
/// - [`ServiceError::Unprocessable`] (→ ITS-REST `422`) when well-formed XML
///   does not decode as an OPT 1.4 `OPERATIONAL_TEMPLATE` document.
pub(crate) fn parse_opt(xml: &str) -> Result<OperationalTemplate, ServiceError> {
    require_well_formed_xml(xml)?;
    openehr_its::opt14::from_xml(xml).map_err(|e| {
        // NOTE: i_definition_adl14.adoc §upload_opt .Errors declares
        // invalid_template for a semantically invalid operational template.
        ServiceError::Unprocessable {
            status: crate::service::status::CallStatusType::InvalidTemplate,
            violation: crate::service::error::Violation::new(format!("invalid OPT 1.4 XML: {e}")),
        }
    })
}

/// The well-formedness gate ahead of the OPT decode: scan the document with a
/// bare `quick_xml::Reader` and reject anything that is not one well-formed
/// XML document with a root element. This isolates the released `400` branch
/// (syntactically invalid content) from the semantic `422` branch the tolerant
/// codec reports past it.
fn require_well_formed_xml(xml: &str) -> Result<(), ServiceError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut saw_root = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(quick_xml::events::Event::Start(_) | quick_xml::events::Event::Empty(_)) => {
                saw_root = true;
            }
            Ok(_) => {}
            Err(e) => {
                return Err(ServiceError::precondition(format!(
                    "syntactically invalid XML content: {e}"
                )));
            }
        }
    }
    if saw_root {
        Ok(())
    } else {
        Err(ServiceError::precondition(
            "syntactically invalid XML content: the document has no root element".to_owned(),
        ))
    }
}
