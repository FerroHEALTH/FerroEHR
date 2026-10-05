// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Builder-capture tests for the leaf content constraints the `WebTemplate`
//! validation walk enforces at commit time, exercised end-to-end from the
//! vendored CNF content-constraint OPTs (`corpus/
//! templates/`): parse the OPT (`openehr_its::opt14`) → build the `WebTemplate`
//! → assert the leaf node carries the validation-only constraint the walk needs.
//!
//! These pin the builder half of three enforcement rules whose leaf-logic half
//! is unit-tested in `validation_rules.rs`:
//!
//! * `C_DV_QUANTITY.property` (AOM 1.4
//!   `AM/docs/UML/classes/org.openehr.am.aom14.c_quantity.adoc` §`C_QUANTITY`)
//!   → [`WebTemplateNode::quantity_property`];
//! * an explicit-`local` closed `C_CODE_PHRASE` (AOM 1.4
//!   `AM/docs/UML/classes/org.openehr.am.aom14.c_coded_text.adoc`
//!   §`C_CODED_TEXT`) → [`WebTemplateNode::coded_terminology_local`];
//! * a `CONSTRAINT_REF` with no local binding constrains nothing enforceable at
//!   commit time (`AM/docs/AOM1.4/master04-constraint_model_package.adoc`
//!   §Reference Objects) → NO leaf constraint captured.

#![expect(clippy::panic, reason = "test assertions/diagnostics")]

use std::path::PathBuf;

use openehr_its::opt14;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplateNode;

fn templates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/templates")
}

fn build(opt_file: &str) -> WebTemplateNode {
    let xml = std::fs::read_to_string(templates_dir().join(opt_file))
        .unwrap_or_else(|e| panic!("read {opt_file}: {e}"));
    let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("opt14 parse {opt_file}: {e}"));
    build_web_template(&opt)
        .unwrap_or_else(|e| panic!("build_web_template {opt_file}: {e}"))
        .tree
}

/// First node (depth-first) whose `rm_type` starts with `rm`.
fn find_by_rm<'a>(n: &'a WebTemplateNode, rm: &str) -> Option<&'a WebTemplateNode> {
    if n.rm_type.starts_with(rm) {
        return Some(n);
    }
    n.children.iter().find_map(|c| find_by_rm(c, rm))
}

/// Every node (depth-first) whose `rm_type` starts with `rm`.
fn all_by_rm<'a>(n: &'a WebTemplateNode, rm: &str, out: &mut Vec<&'a WebTemplateNode>) {
    if n.rm_type.starts_with(rm) {
        out.push(n);
    }
    for c in &n.children {
        all_by_rm(c, rm, out);
    }
}

#[test]
fn c_dv_quantity_property_captured() {
    // `quantity_property.opt`: C_DV_QUANTITY property=openehr::122 (Length),
    // no enumerated unit list — so the property is the sole units constraint.
    let tree = build("quantity_property.opt");
    let q = find_by_rm(&tree, "DV_QUANTITY").expect("a DV_QUANTITY leaf node");
    assert_eq!(
        q.quantity_property.as_deref(),
        Some("122"),
        "the C_DV_QUANTITY property code must be captured for units enforcement"
    );
}

#[test]
fn c_code_phrase_explicit_local_captured() {
    // `dt_coded_text_c_code_phrase.opt` carries two C_CODE_PHRASE constraints:
    // the COMPOSITION `category` (terminology=openehr, code_list=[433]) and the
    // content ELEMENT value (terminology=local, code_list=[ABC,OPQ]). Only the
    // explicit-local one must flag `coded_terminology_local`.
    let tree = build("dt_coded_text_c_code_phrase.opt");
    let mut coded = Vec::new();
    all_by_rm(&tree, "DV_CODED_TEXT", &mut coded);
    assert!(
        coded.iter().any(|n| n.coded_terminology_local),
        "the explicit-local closed C_CODE_PHRASE must set coded_terminology_local, got \
         {:?}",
        coded
            .iter()
            .map(|n| (&n.rm_type, n.coded_terminology_local))
            .collect::<Vec<_>>()
    );
    // The openehr-terminology `category` constraint (code_list=[433]) must NOT
    // be flagged local — its terminology is `openehr`, not `local`.
    assert!(
        coded.iter().any(|n| !n.coded_terminology_local),
        "the openehr-terminology C_CODE_PHRASE must not be flagged local"
    );
}

#[test]
fn constraint_ref_captures_no_leaf_constraint() {
    // `dt_coded_text_constraint_ref.opt`: CONSTRAINT_REF ac0001 with no
    // constraint_binding. AOM 1.4 defines no accept/reject rule for an unbound
    // CONSTRAINT_REF at data-validation time, and it resolves to an external
    // terminology query, not a local list — so no leaf constraint is captured
    // and any well-formed CODE_PHRASE is admitted (documented no-op).
    let tree = build("dt_coded_text_constraint_ref.opt");
    let mut coded = Vec::new();
    all_by_rm(&tree, "DV_CODED_TEXT", &mut coded);
    // The content value node behind the CONSTRAINT_REF carries no local-list
    // flag and no enumerated code input list.
    assert!(
        coded.iter().all(|n| !n.coded_terminology_local),
        "an unbound CONSTRAINT_REF must not be captured as a local closed list"
    );
}
