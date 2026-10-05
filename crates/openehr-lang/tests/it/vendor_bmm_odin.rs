// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Public-API battery for the `openehr_lang::v1_1::odin` reader over the 38 vendored
//! BMM schema fixtures under `tests/vendor/bmm/`.
//!
//! Every `.bmm` file is an ODIN document (the BMM persistence serialization),
//! mirrored from archie (`e8d92f28`, `bmm/src/test/resources/`). archie's own
//! tests establish the outcome oracle:
//!
//! - `BmmOdinParserTest` parses `testbmm/TestBmm1.bmm` via `BmmOdinParser` (the
//!   ODIN parse + P_BMM object mapping).
//! - `BasicSchemaValidationsTest` / `ClassesValidatorTest` / `IncludesValidatorTest`
//!   / `PropertyValidatorTest` / `CreatedSchemaValidationTest` all first
//!   `BmmOdinParser.convert(...)` the `org/openehr/bmm/v2/persistence/validation/*.bmm`
//!   fixtures **successfully** and then assert BMM *semantic* validation errors
//!   (duplicate class, missing ancestor, unresolved include, …). Those defects
//!   are **above the ODIN layer**: the ODIN parse succeeds; only the BMM
//!   semantic layer rejects them.
//!
//! Accordingly every fixture here must parse as ODIN and yield a schema object.
//! Where a fixture's BMM-semantic verdict is worth pinning beside its ODIN
//! reading, the test states it in this crate's own terms — a typed
//! [`PBmmReadError`] refusal, or a collecting-pass
//! [`openehr_lang::v1_1::bmm_persistence::validate::PBmmValidityFinding`]; archie's
//! `EC_*` codes are named only as the fixtures' provenance, never as an
//! expectation. `vendor_bmm_schema.rs` carries the full outcome tables.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
#![allow(
    clippy::doc_markdown,
    reason = "the module docs name archie Java classes (BmmOdinParser, BmmSchemaConverter, …) and BMM error codes (EC_…) as prose, not code refs"
)]

use openehr_lang::v1_1::bmm_persistence::create_model::create_bmm_model;
use openehr_lang::v1_1::bmm_persistence::error::PBmmReadError;
use openehr_lang::v1_1::bmm_persistence::include_resolution::resolve_includes;
use openehr_lang::v1_1::bmm_persistence::reader::read_schema;
use openehr_lang::v1_1::bmm_persistence::validate::PBmmValidityFinding;
use openehr_lang::v1_1::bmm_persistence::validate::validate_schema;
use openehr_lang::v1_1::odin::{OdinKey, OdinValue, parse};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn read(rel: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vendor");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn parse_ok(rel: &str) -> OdinValue {
    parse(&read(rel)).unwrap_or_else(|e| panic!("{rel} should parse, got error: {e}"))
}

fn field<'a>(v: &'a OdinValue, key: &str) -> &'a OdinValue {
    match v {
        OdinValue::Object(m) => m
            .get(key)
            .unwrap_or_else(|| panic!("attribute {key:?} missing")),
        other => panic!("expected Object, got {other:?}"),
    }
}

fn as_str(v: &OdinValue) -> &str {
    match v {
        OdinValue::String(s) => s,
        other => panic!("expected String, got {other:?}"),
    }
}

fn keyed(v: &OdinValue) -> &[(OdinKey, OdinValue)] {
    match v {
        OdinValue::KeyedList(entries) => entries,
        other => panic!("expected KeyedList, got {other:?}"),
    }
}

fn keyed_str<'a>(v: &'a OdinValue, key: &str) -> &'a OdinValue {
    keyed(v)
        .iter()
        .find(|(k, _)| matches!(k, OdinKey::String(s) if s == key))
        .map_or_else(|| panic!("string key {key:?} missing"), |(_, val)| val)
}

/// The complete claimed set for `tests/vendor/bmm/**`; the coverage gate
/// (`vendor_coverage.rs`) cross-checks this against the filesystem.
const BMM_FIXTURES: &[&str] = &[
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

/// Every BMM fixture parses as an ODIN schema object carrying at least the
/// `rm_publisher` + `schema_name` string headers of the BMM persistence format.
/// (Semantically-malformed fixtures still parse at the ODIN layer — see the
/// module docs; the BMM-semantic verdict is pinned above this one.)
#[test]
fn every_bmm_file_parses_as_odin_schema() {
    for rel in BMM_FIXTURES {
        let v = parse_ok(rel);
        assert!(
            matches!(v, OdinValue::Object(_)),
            "{rel}: expected top-level schema object"
        );
        // rm_publisher + schema_name are string leaves in every BMM schema.
        let _publisher = as_str(field(&v, "rm_publisher"));
        let _schema = as_str(field(&v, "schema_name"));
    }
}

// ---------------------------------------------------------------------------
// TestBmm1 — archie BmmOdinParserTest.parseTestBmm1
// ---------------------------------------------------------------------------

#[test]
fn test_bmm1_structure() {
    let v = parse_ok("bmm/testbmm/TestBmm1.bmm");
    assert_eq!(as_str(field(&v, "bmm_version")), "1.1");
    assert_eq!(as_str(field(&v, "rm_publisher")), "My publisher");
    assert_eq!(as_str(field(&v, "schema_name")), "Test1");
    // schema_contributors is a two-element string list.
    match field(&v, "schema_contributors") {
        OdinValue::List(items) => assert_eq!(items.len(), 2),
        other => panic!("expected list, got {other:?}"),
    }
    // includes is an integer-keyed list of include ids.
    let includes = field(&v, "includes");
    assert_eq!(keyed(includes).len(), 2);
    let inc1 = keyed_str(includes, "1");
    assert_eq!(as_str(field(inc1, "id")), "my_include.2.1.12");
    // packages holds the top-level package keyed list.
    assert!(!keyed(field(&v, "packages")).is_empty());
}

// ---------------------------------------------------------------------------
// Published reference schemas — the real openEHR/CIMI BMM artefacts
// ---------------------------------------------------------------------------

#[test]
fn openehr_rm_102_schema() {
    let v = parse_ok("bmm/openehr/openehr_rm_102.bmm");
    assert_eq!(as_str(field(&v, "rm_publisher")), "openehr");
    assert_eq!(as_str(field(&v, "schema_name")), "rm");
    assert_eq!(as_str(field(&v, "rm_release")), "1.0.2");
    // openehr_rm composes sub-schemas via `includes`.
    assert!(!keyed(field(&v, "includes")).is_empty());
}

#[test]
fn cimi_top_level_rm_schema() {
    let v = parse_ok("bmm/CIMI-RM-3.0.5.bmm");
    assert_eq!(as_str(field(&v, "rm_publisher")), "CIMI");
    assert_eq!(as_str(field(&v, "schema_name")), "RM");
    assert_eq!(as_str(field(&v, "rm_release")), "3.0.5");
}

// ---------------------------------------------------------------------------
// Semantically-malformed fixtures — ODIN parse succeeds; the BMM-semantic
// verdict is above the ODIN layer (archie's BmmSchemaConverter).
// ---------------------------------------------------------------------------

#[test]
fn duplicate_class_parses_at_odin_layer_and_is_reported_by_the_validity_pass() {
    // Fixture provenance: archie BasicSchemaValidationsTest.duplicateClass
    // (its own code is EC_DUPLICATE_CLASS_IN_PACKAGES). Our expectation comes
    // from the spec, not that code: master05-core-model.adoc §Packages — "A
    // model validity checker ensures that every class is contained within
    // exactly one package".
    let rel = "bmm/org/openehr/bmm/v2/persistence/validation/duplicate_class.bmm";
    let v = parse_ok(rel);
    assert_eq!(as_str(field(&v, "schema_name")), "duplicate_class");
    assert!(!keyed(field(&v, "packages")).is_empty());

    // Above the ODIN layer: the schema reads and materialises — a duplicate
    // package listing breaks no BMM_* construction — and the collecting
    // validity pass is what reports it.
    let src = read(rel);
    let schema = read_schema(&src).expect("the fixture reads as a P_BMM schema");
    let model = create_bmm_model(&schema).expect("the fixture materialises a model");
    assert_eq!(
        validate_schema(&schema, &model),
        [PBmmValidityFinding::ClassNotInExactlyOnePackage {
            class: "ParentType1".to_owned(),
            packages: ["ParentPackage".to_owned(), "ParentPackage".to_owned()].to_vec(),
        }]
    );
}

#[test]
fn include_not_found_parses_at_odin_layer_and_is_refused_by_inclusion_resolution() {
    // Fixture provenance: archie IncludesValidatorTest.includeNotFound (its
    // own code is EC_INCLUDE_NOT_FOUND). Our expectation is the existing typed
    // refusal, not a validity finding: `P_BMM_SCHEMA.merge` has precondition
    // `includes_to_process.has (included_schema.schema_id)`
    // (…p_bmm_schema.adoc §Functions), so an unlocatable include leaves
    // nothing to construct and inclusion resolution refuses fail-fast.
    let rel = "bmm/org/openehr/bmm/v2/persistence/validation/include_not_found.bmm";
    let v = parse_ok(rel);
    assert!(!keyed(field(&v, "includes")).is_empty());

    let schema = read_schema(&read(rel)).expect("the fixture reads as a P_BMM schema");
    let requester = schema.schema_id();
    let error =
        resolve_includes(schema, &BTreeMap::new()).expect_err("the included schema is absent");
    assert_eq!(
        error,
        PBmmReadError::MissingInclude {
            requester,
            id: "my_include.2.1.12".to_owned(),
        }
    );
}

#[test]
fn valid_reference_schema_parses() {
    // archie's `valid.bmm` — the baseline schema its BasicSchemaValidationsTest
    // mutates to provoke each error. It parses cleanly and is semantically
    // valid BMM.
    let v = parse_ok("bmm/org/openehr/bmm/v2/persistence/validation/valid.bmm");
    assert_eq!(as_str(field(&v, "rm_publisher")), "My publisher");
    assert!(!keyed(field(&v, "packages")).is_empty());
}
