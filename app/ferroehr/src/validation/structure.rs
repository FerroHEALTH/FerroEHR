// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Surface A3 — OPT XML well-formedness (`upload_opt-invalid_opt`).
//!
//! FLAG: this is a **CNF-fixture / ingestion guard, not an AOM constraint
//! kind** — it maps to CNF `I_DEFINITION_ADL14` `invalid_templates/*`
//! (`alien_tags`, `multiple_elements`), not to a rule of the AOM 1.4/2.4
//! constraint catalogue. It closes a leniency gap in the tolerant
//! `openehr_its::opt14` codec before the artefact pass (the sibling `opt`
//! module) runs on the parsed tree, so it lives beside the artefact
//! validators rather than in the constraint-taxonomy modules.

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::service::error::{ServiceError, Violation};

/// The legal direct children of the root `<template>` element (the serialized
/// `OPERATIONAL_TEMPLATE` attributes).
const OPT_TOP_LEVEL: &[&str] = &[
    "language",
    "is_controlled",
    "description",
    "revision_history",
    "uid",
    "template_id",
    "concept",
    "definition",
    "ontology",
    "component_ontologies",
    "annotations",
    "constraints",
    "view",
];

/// The `OPERATIONAL_TEMPLATE` top-level elements that may legitimately repeat.
const OPT_TOP_LEVEL_MULTIPLE: &[&str] = &["component_ontologies", "annotations"];

/// Reject an OPT the tolerant [`openehr_its::opt14`] codec would silently accept:
/// a **foreign** top-level element (CNF `invalid_templates/alien_tags`) or a
/// **duplicated single-valued** top-level element (CNF
/// `invalid_templates/multiple_elements` — `concept`/`definition`/`template_id`
/// twice). Only the direct children of the root `<template>` are inspected, so a
/// legitimate nested reuse of a name (e.g. `<template_id>` inside a term binding)
/// is unaffected. The codec already catches missing mandatory elements; this
/// closes the leniency gap the CNF `upload_opt-invalid_opt` case exercises.
///
/// # Errors
///
/// [`ServiceError::Unprocessable`] naming the foreign or duplicated element,
/// or describing the XML parse failure when the document is malformed.
pub(super) fn validate_opt_structure(xml: &str) -> Result<(), ServiceError> {
    let mut reader = Reader::from_str(xml);
    let mut depth: i32 = 0;
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut check = |raw: &str| -> Result<(), ServiceError> {
        let name = raw.to_owned();
        if !OPT_TOP_LEVEL.contains(&name.as_str()) {
            return Err(ServiceError::content_invalid(
                Violation::new(
                    "is an unexpected top-level element of an operational template \
                     (not an OPERATIONAL_TEMPLATE attribute)",
                )
                .with_path(format!("<{name}>")),
            ));
        }
        let count = seen.entry(name.clone()).or_insert(0);
        *count += 1;
        if *count > 1 && !OPT_TOP_LEVEL_MULTIPLE.contains(&name.as_str()) {
            return Err(ServiceError::content_invalid(
                Violation::new("is a duplicate single-valued element of an operational template")
                    .with_path(format!("<{name}>")),
            ));
        }
        Ok(())
    };

    loop {
        match reader.read_event() {
            // A direct child's Start is seen at depth 2 (root <template> is 1).
            Ok(Event::Start(e)) => {
                depth += 1;
                if depth == 2 {
                    check(e.local_name().as_ref())?;
                }
            }
            // A self-closing direct child is seen while the root (depth 1) is open.
            Ok(Event::Empty(e)) => {
                if depth == 1 {
                    check(e.local_name().as_ref())?;
                }
            }
            Ok(Event::End(_)) => depth -= 1,
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(ServiceError::content_invalid(Violation::new(format!(
                    "operational template XML is malformed: {e}"
                ))));
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_opt_structure;

    fn tmpl(children: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?>\n<template xmlns=\"http://schemas.openehr.org/v1\">{children}</template>"
        )
    }

    #[test]
    fn accepts_a_well_formed_top_level() {
        // Single-valued attributes once each + a repeated multi-valued one.
        let xml = tmpl(
            "<language/><template_id><value>t.v1</value></template_id>\
             <concept>C</concept><definition/>\
             <component_ontologies/><component_ontologies/>",
        );
        assert!(validate_opt_structure(&xml).is_ok());
    }

    #[test]
    fn rejects_a_foreign_top_level_element() {
        // CNF invalid_templates/alien_tags — a <bullfrog> tag at the top level.
        let xml = tmpl("<concept>C</concept><bullfrog>x</bullfrog>");
        let err = validate_opt_structure(&xml).unwrap_err().to_string();
        assert!(err.contains("bullfrog"), "{err}");
    }

    #[test]
    fn rejects_duplicate_single_valued_top_level() {
        // CNF invalid_templates/multiple_elements — concept/definition/template_id twice.
        for name in ["concept", "definition", "template_id"] {
            let xml = tmpl(&format!("<{name}>a</{name}><{name}>b</{name}>"));
            let err = validate_opt_structure(&xml).unwrap_err().to_string();
            assert!(err.contains("duplicate"), "{name}: {err}");
        }
    }

    #[test]
    fn ignores_nested_reuse_of_a_name() {
        // A nested <template_id> deeper in the tree must not count as a duplicate
        // of the single top-level one (valid OPTs carry nested references).
        let xml = tmpl(
            "<template_id><value>t.v1</value></template_id>\
             <definition><attributes><template_id>ref</template_id></attributes></definition>",
        );
        assert!(validate_opt_structure(&xml).is_ok());
    }
}
