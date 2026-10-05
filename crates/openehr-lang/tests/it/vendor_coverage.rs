// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Coverage gate for the vendored ODIN + BMM fixtures.
//!
//! Owner hard rule: every vendored corpus is 100% exercised with asserted
//! expected outcomes. This test walks `tests/vendor/**` and fails if any file
//! (other than `PROVENANCE.md`) is not claimed by the fixture batteries, or if
//! any claim has gone stale (names a file that no longer exists). Keyed on the
//! full relative path.
//!
//! `CLAIMED` is the union of the fixture tables in `vendor_odin.rs`
//! (`ODIN_FIXTURES`, 17 files) and `vendor_bmm_odin.rs` (`BMM_FIXTURES`, 38
//! files) — every claimed path is genuinely parsed and asserted there.
//!
//! `vendor_bmm_schema.rs` carries a SECOND, independent table over the same
//! files: it claims every `.bmm` schema (the 38 above plus the five under
//! `odin/odin/`) with an adjudicated `P_BMM` read → resolve → `BMM_MODEL` outcome,
//! and gates its own completeness against the filesystem. It adds no paths, so
//! `CLAIMED` is unaffected.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The 55 claimed fixture paths (17 ODIN + 38 BMM). Mirror of the fixture
/// tables in `vendor_odin.rs` and `vendor_bmm_odin.rs`.
const CLAIMED: &[&str] = &[
    // --- vendor_odin.rs :: ODIN_FIXTURES (17) ---
    "odin/log4j2.xml",
    "odin/odin/CIMI-RM-3.0.5.bmm",
    "odin/odin/CIMI-RM-3.0.5_tweaked.bmm",
    "odin/odin/CIMI_RM_CLINICAL.v.0.0.1.bmm",
    "odin/odin/CIMI_RM_CORE.v.0.0.1.bmm",
    "odin/odin/CIMI_RM_FOUNDATION.v.0.0.1.bmm",
    "odin/odin/anonymous_odin.txt",
    "odin/odin/identified_object_document.txt",
    "odin/odin/odin_keyed_object.txt",
    "odin/odin/odin_nested_attribute_structure1.txt",
    "odin/odin/odin_nested_keyed_object.txt",
    "odin/odin/odin_primitive_intervals.txt",
    "odin/odin/odin_primitive_lists.txt",
    "odin/odin/odin_primitive_types.txt",
    "odin/odin/odin_term_binding_test.txt",
    "odin/odin/odin_test.txt",
    "odin/odin/odin_types.txt",
    // --- vendor_bmm_odin.rs :: BMM_FIXTURES (38) ---
    "bmm/CIMI-RM-3.0.5.bmm",
    "bmm/cimi/CIMI-RM-3.0.5.bmm",
    "bmm/cimi/CIMI_RM_CLINICAL.v.0.0.2.bmm",
    "bmm/cimi/CIMI_RM_CORE.v.0.0.2.bmm",
    "bmm/cimi/CIMI_RM_FOUNDATION.v.0.0.2.bmm",
    "bmm/openehr/openEHR_aom_206.bmm",
    "bmm/openehr/openehr_adltest_100.bmm",
    "bmm/openehr/openehr_base_110.bmm",
    "bmm/openehr/openehr_base_for_aom.bmm",
    "bmm/openehr/openehr_basic_types_102.bmm",
    "bmm/openehr/openehr_demographic_102.bmm",
    "bmm/openehr/openehr_ehr_102.bmm",
    "bmm/openehr/openehr_primitive_types_102.bmm",
    "bmm/openehr/openehr_rm_102.bmm",
    "bmm/openehr/openehr_structures_102.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/ancestor_def_doesnt_exist.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/ancestor_doesnt_exist.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/ancestor_name_empty.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/class_not_in_definition.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/class_not_in_packages.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/container_target_type_empty.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/container_target_type_not_found.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/container_type_not_found.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/duplicate_class.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/generic_container_property_not_found.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/generic_parameter_not_found.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/generic_parameter_type_missing.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/generic_property_type_def_undefined.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/generic_root_type_not_found.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/illegal_sibling_packages.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/include_not_found.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/overridden_property_non_conformance.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/package_class_name_empty.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/package_illegal_qualified_name.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/single_open_property_type_not_found.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/single_property_type_not_found.bmm",
    "bmm/org/openehr/bmm/v2/persistence/validation/valid.bmm",
    "bmm/testbmm/TestBmm1.bmm",
];

fn vendor_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vendor")
}

fn walk(dir: &Path, root: &Path, out: &mut BTreeSet<String>) {
    for entry in
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
    {
        let path = entry.unwrap_or_else(|e| panic!("dir entry: {e}")).path();
        if path.is_dir() {
            walk(&path, root, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap_or_else(|e| panic!("strip_prefix: {e}"));
            out.insert(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn every_vendor_fixture_is_claimed_and_no_claim_is_stale() {
    let root = vendor_root();
    let mut on_disk = BTreeSet::new();
    walk(&root, &root, &mut on_disk);
    // PROVENANCE.md documents the corpus and is not a fixture.
    on_disk.remove("PROVENANCE.md");

    let claimed: BTreeSet<String> = CLAIMED.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        claimed.len(),
        CLAIMED.len(),
        "duplicate entry in CLAIMED registry"
    );

    let unclaimed: Vec<String> = on_disk.difference(&claimed).cloned().collect();
    assert!(
        unclaimed.is_empty(),
        "vendor fixtures not claimed by any test (add them to vendor_odin.rs / vendor_bmm_odin.rs): {}",
        unclaimed.join(", ")
    );

    let stale: Vec<String> = claimed.difference(&on_disk).cloned().collect();
    assert!(
        stale.is_empty(),
        "CLAIMED names files that are not present on disk: {}",
        stale.join(", ")
    );

    // 17 ODIN + 38 BMM = 55 fixtures.
    assert_eq!(on_disk.len(), 55, "expected 55 vendored fixtures");
}
