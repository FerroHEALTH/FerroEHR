// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The example generator answers a COMPOSITION only for a COMPOSITION-rooted
//! template (#3780).
//!
//! The ITS-REST example operation answers an instance of a versioned-object
//! class (`responses/200_Template_example_retrieved.yaml`: COMPOSITION,
//! `EHR_STATUS`, FOLDER and the five parties). A template rooted at an ENTRY or a
//! SECTION admits none of them, so the generator refuses it with the root named
//! instead of writing COMPOSITION members onto an OBSERVATION or a SECTION.
#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]

use std::path::Path;

use openehr_its::opt14;
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;

/// The Web Template of one OPT of the vendored full CKM library.
fn full_pack_template(name: &str) -> WebTemplate {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/templates/ckm/full")
        .join(name);
    let xml =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse {name}: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build {name}: {e}"))
}

/// Asserts every detail level refuses `name` naming `root`.
fn assert_refused(name: &str, root: &str) {
    let wt = full_pack_template(name);
    assert_eq!(wt.tree.rm_type, root, "{name} is rooted at {root}");
    for level in [
        DetailLevel::Required,
        DetailLevel::Medium,
        DetailLevel::Complete,
    ] {
        match example_composition(&wt, level) {
            Err(FlatError::NoExampleForRoot { root: refused }) => {
                assert_eq!(refused, root, "{name}@{level:?}");
            }
            other => panic!("{name}@{level:?}: expected NoExampleForRoot, got {other:?}"),
        }
    }
}

#[test]
fn an_observation_rooted_template_has_no_example_composition() {
    assert_refused(
        "absolute-cardiovascular-risk-assessment-item-r2.opt",
        "OBSERVATION",
    );
}

#[test]
fn a_section_rooted_template_has_no_example_composition() {
    assert_refused("acquisition-and-validation-of-vf-test.opt", "SECTION");
}
