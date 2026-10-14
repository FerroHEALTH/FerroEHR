// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: Apache-2.0

//! The generated strict JSON reader against the `packages` shape openEHR
//! publishes: a leaf package carries no `packages` attribute.
//!
//! The LANG class table declares `P_BMM_PACKAGE_CONTAINER.packages` 1..1, yet
//! `docs/specs/openehr/LANG/docs/bmm_persistence/master04-syntax.adoc`
//! §Package Definition shows a leaf package without it, and every published
//! BMM schema writes leaf packages that way. The generator reads the absence
//! as the empty package map and omits the empty map on write, in both LANG
//! generations.

#![expect(
    clippy::expect_used,
    reason = "test fixtures: a vendored file that does not read is the failure being tested"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use openehr_lang::v1_1::bmm_persistence::p_bmm_package::PBmmPackage;

/// The json BMMs `openehr-codegen` reads for the pinned BASE, RM and TERM
/// generations, relative to its vendored `components/` tree.
const PINNED_JSON_BMMS: &[&str] = &[
    "BASE/json/openehr_base_1.3.0.bmm.json",
    "RM/json/openehr_rm_1.2.0.bmm.json",
    "TERM/json/openehr_term_3.1.0.bmm.json",
];

/// The compact serialization of an empty `packages` map, which no published
/// schema contains.
const EMPTY_PACKAGES: &str = "\"packages\":{}";

/// Reads the top-level `packages` member of one vendored json BMM through the
/// generated strict reader.
#[expect(
    clippy::disallowed_types,
    reason = "the vendored file is raw JSON; this seam only selects its `packages` member for the typed reader"
)]
fn vendored_package_tree(file: &str) -> BTreeMap<String, PBmmPackage> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/openehr-codegen/vendor/bmm/components")
        .join(file);
    let text = std::fs::read_to_string(&path).expect("the vendored json BMM reads");
    let mut schema: serde_json::Value =
        serde_json::from_str(&text).expect("the vendored json BMM is JSON");
    let packages = schema
        .get_mut("packages")
        .map(serde_json::Value::take)
        .expect("a BMM schema carries a top-level packages map");
    serde_json::from_value(packages).expect("the package tree reads")
}

/// Counts the packages in `tree` that hold no nested package.
fn leaf_count(tree: &BTreeMap<String, PBmmPackage>) -> usize {
    tree.values()
        .map(|p| {
            if p.packages.is_empty() {
                1
            } else {
                leaf_count(&p.packages)
            }
        })
        .sum()
}

#[test]
fn the_pinned_json_bmm_package_trees_read_and_round_trip() {
    for file in PINNED_JSON_BMMS {
        let tree = vendored_package_tree(file);
        assert!(
            leaf_count(&tree) > 0,
            "{file}: the published tree has leaf packages without `packages`"
        );
        let written = serde_json::to_string(&tree).expect("the package tree writes");
        assert!(
            !written.contains(EMPTY_PACKAGES),
            "{file}: the writer invented an empty `packages` map"
        );
        let reread: BTreeMap<String, PBmmPackage> =
            serde_json::from_str(&written).expect("the written tree reads back");
        assert_eq!(reread, tree, "{file}: the package tree does not round-trip");
    }
}

#[test]
fn a_leaf_package_reads_absent_packages_as_empty_and_writes_none() {
    let leaf = r#"{"name":"org.openehr.test_pkg","classes":["WHOLE","BOOK"]}"#;
    let package: PBmmPackage = serde_json::from_str(leaf).expect("a leaf package reads");
    assert!(package.packages.is_empty());
    assert_eq!(
        serde_json::to_string(&package).expect("a leaf package writes"),
        r#"{"_type":"P_BMM_PACKAGE","name":"org.openehr.test_pkg","classes":["WHOLE","BOOK"]}"#,
    );
}

#[test]
fn an_explicit_empty_packages_map_reads_like_its_absence() {
    let absent: PBmmPackage =
        serde_json::from_str(r#"{"name":"p"}"#).expect("absent packages reads");
    let empty: PBmmPackage =
        serde_json::from_str(r#"{"name":"p","packages":{}}"#).expect("an empty map reads");
    assert_eq!(absent, empty);
}

#[test]
fn a_null_packages_member_is_still_refused() {
    // A `null` is not a map; only the member's absence takes the empty reading.
    let refused = serde_json::from_str::<PBmmPackage>(r#"{"name":"p","packages":null}"#);
    assert!(refused.is_err(), "a null packages member must not read");
}

#[test]
fn the_released_lang_generation_reads_a_leaf_package_too() {
    let leaf = r#"{"name":"org.openehr.test_pkg","classes":["WHOLE"]}"#;
    let package: openehr_lang::v1_0::bmm_persistence::p_bmm_package::PBmmPackage =
        serde_json::from_str(leaf).expect("a LANG 1.0.0 leaf package reads");
    assert!(package.packages.is_empty());
    let written = serde_json::to_string(&package).expect("a LANG 1.0.0 leaf package writes");
    assert!(
        !written.contains("\"packages\""),
        "the empty package map is omitted: {written}"
    );
}
