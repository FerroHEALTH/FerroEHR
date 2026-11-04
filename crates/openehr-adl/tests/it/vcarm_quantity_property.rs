// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! VCARM on `DV_QUANTITY.property` (#4964): refused, with a message naming
//! the ADL 1.4 route.
//!
//! VCARM (`AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`):
//! an attribute "must be defined in the underlying information model". No RM
//! release defines `DV_QUANTITY.property`
//! (`RM/docs/UML/classes/org.openehr.rm.data_types.dv_quantity.adoc`); it is
//! the AOM 1.4 archetype-profile attribute
//! (`AM/docs/UML/classes/org.openehr.am.aom14.c_quantity.adoc`).
#![allow(
    clippy::expect_used,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::Path;

use openehr_adl::validate::ValidationIssue;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::validate_source;

fn height() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../corpus/archetypes/adl2/ckm-2013-12-09/entry/observation/openEHR-EHR-OBSERVATION.height.v1.0.0.adls",
    ))
    .expect("the vendored archetype")
}

fn vcarm(src: &str) -> Vec<ValidationIssue> {
    validate_source(src, None, &ProductionRmModel, &NoTerminologyResolver)
        .expect("the source parses")
        .into_iter()
        .filter(|i| i.code == ValidationCode::Vcarm)
        .collect()
}

#[test]
fn a_quantity_property_is_refused_with_the_adl14_route() {
    let issues = vcarm(&height());
    assert!(!issues.is_empty(), "height constrains DV_QUANTITY.property");
    for issue in &issues {
        assert_eq!(issue.severity, Severity::Error);
        assert!(
            issue
                .path
                .as_deref()
                .is_some_and(|p| p.ends_with("/property")),
            "{issue:?}"
        );
        assert_eq!(
            issue.message,
            "attribute \"property\" is not defined on reference-model type \"DV_QUANTITY\": \
             `property` is the ADL 1.4 archetype-profile attribute C_DV_QUANTITY.property, \
             which no ADL 2 or RM release defines; upload the archetype's ADL 1.4 source \
             instead, which converts to ADL 2 without it"
        );
    }
}

/// A misspelled attribute is the error class the refusal exists for: it keeps
/// the plain message.
#[test]
fn another_undefined_attribute_keeps_the_plain_message() {
    let src = height().replace("property matches", "proprety matches");
    let issues = vcarm(&src);
    assert!(!issues.is_empty(), "the misspelled attribute is refused");
    for issue in &issues {
        assert_eq!(
            issue.message,
            "attribute \"proprety\" is not defined on reference-model type \"DV_QUANTITY\""
        );
    }
}
