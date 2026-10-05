// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Public-API battery for the `rm_access` schema-loading facade
//! (`openehr_lang::v1_1::bmm::rm_access`): a schema DIRECTORY scanned, selected,
//! loaded, validated and materialised.
//!
//! Spec oracle: `docs/specs/openehr/LANG/docs/bmm/master04-rm_access.adoc`
//! §Overview (the package "provides an interface for the application to load and
//! access BMM schemas") plus the class docs
//! `docs/specs/openehr/LANG/docs/UML/classes/org.openehr.lang.bmm.reference_model_access.adoc`
//! and `…bmm.schema_descriptor.adoc`.
//!
//! The fixtures are copies of the vendored openEHR RM 1.0.2 inclusion chain
//! (`tests/vendor/bmm/openehr/`, whose adjudicated class counts are pinned by
//! `vendor_bmm_schema.rs`) placed in a temp directory, because a directory scan
//! is exactly what this facade adds over the `bmm_persistence` pipeline. The
//! vendored files themselves are never written to.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use openehr_lang::v1_1::bmm::rm_access::error::RmAccessError;
use openehr_lang::v1_1::bmm::rm_access::reference_model_access::ReferenceModelAccess;
use openehr_lang::v1_1::bmm::rm_access::schema_descriptor::SchemaDescriptor;

/// The four schemas of the vendored RM 1.0.2 inclusion chain, deepest first:
/// `ehr` → `structures` → `basic_types` → `primitive_types`.
const CHAIN: &[(&str, &str)] = &[
    (
        "openehr_primitive_types_102.bmm",
        "openehr_primitive_types_1.0.2",
    ),
    ("openehr_basic_types_102.bmm", "openehr_basic_types_1.0.2"),
    ("openehr_structures_102.bmm", "openehr_structures_1.0.2"),
    ("openehr_ehr_102.bmm", "openehr_ehr_1.0.2"),
];

/// The vendored source of one chain file.
fn vendored(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vendor/bmm/openehr")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// A temp directory holding `files` (copies of the vendored chain).
fn repository(files: &[&str]) -> assert_fs::TempDir {
    let dir = assert_fs::TempDir::new().expect("a temp schema directory");
    for file in files {
        std::fs::write(dir.path().join(file), vendored(file))
            .unwrap_or_else(|e| panic!("write {file}: {e}"));
    }
    dir
}

/// The directory path, as `schema_directories` takes it.
fn directories(dir: &assert_fs::TempDir) -> Vec<String> {
    vec![dir.path().display().to_string()]
}

/// The class count of the model published under `logical_name`.
fn classes(access: &ReferenceModelAccess, logical_name: &str) -> usize {
    let models = access.valid_models().expect("materialised models");
    let model = models
        .get(logical_name)
        .unwrap_or_else(|| panic!("no model published as {logical_name}"));
    model.class_definitions.as_ref().map_or(0, BTreeMap::len)
}

#[test]
fn initialise_all_loads_the_whole_inclusion_chain_and_materialises_its_root() {
    // `initialise_all`: "Initialise with all schemas found in the schema
    // directories" (…reference_model_access.adoc §Functions).
    let files: Vec<&str> = CHAIN.iter().map(|(file, _)| *file).collect();
    let dir = repository(&files);
    let mut access = ReferenceModelAccess::new(directories(&dir));
    access.initialise_all().expect("the chain loads");

    let all = access.all_schemas().expect("all_schemas is populated");
    let ids: Vec<&str> = all.keys().map(String::as_str).collect();
    let mut expected: Vec<&str> = CHAIN.iter().map(|(_, id)| *id).collect();
    expected.sort_unstable();
    assert_eq!(ids, expected, "all_schemas is keyed by schema_id");
    for descriptor in all.values() {
        // Every descriptor completed `load` and carries its metadata table.
        assert!(
            descriptor.p_schema().is_some(),
            "{}",
            descriptor.schema_id()
        );
        assert!(
            descriptor.schema_path().is_some(),
            "{}",
            descriptor.schema_id()
        );
        assert!(descriptor.is_bmm_compatible(), "{}", descriptor.schema_id());
        descriptor.validate().expect("the descriptor validates");
    }

    // `is_top_level`: "True if this is a top-level schema, i.e. not included by
    // some other schema" — only `ehr` is, in this chain.
    let top_level: Vec<&str> = all
        .values()
        .filter(|descriptor| descriptor.is_top_level(all))
        .map(SchemaDescriptor::schema_id)
        .collect();
    assert_eq!(top_level, ["openehr_ehr_1.0.2"]);

    // `valid_models`: "Top-level (root) models in use. Keyed by logical
    // schema_name, i.e. model_publisher '_' model_name".
    let models = access.valid_models().expect("materialised models");
    assert_eq!(models.keys().collect::<Vec<&String>>(), ["openehr_ehr"]);
    // The adjudicated class count of the fully resolved RM 1.0.2 ehr schema
    // (pinned identically by `vendor_bmm_schema.rs`).
    assert_eq!(classes(&access, "openehr_ehr"), 124);
}

#[test]
fn a_load_list_pulls_in_the_transitive_includes_of_what_it_names() {
    // `initialise_with_load_list`: "Initialise with a specific schema load list,
    // usually a sub-set of schemas that will be found in the directories".
    let files: Vec<&str> = CHAIN.iter().map(|(file, _)| *file).collect();
    let dir = repository(&files);
    let mut access = ReferenceModelAccess::new(Vec::new());
    access
        .initialise_with_load_list(directories(&dir), &["openehr_structures_1.0.2".to_owned()])
        .expect("the sub-set loads");

    let all = access.all_schemas().expect("all_schemas is populated");
    assert_eq!(
        all.keys().map(String::as_str).collect::<Vec<&str>>(),
        [
            "openehr_basic_types_1.0.2",
            "openehr_primitive_types_1.0.2",
            "openehr_structures_1.0.2",
        ],
        "the load list's transitive include closure, and nothing else",
    );
    // `ehr` is in the directory but outside the closure, so it is not published.
    let models = access.valid_models().expect("materialised models");
    assert_eq!(
        models.keys().collect::<Vec<&String>>(),
        ["openehr_structures"]
    );
    assert_eq!(classes(&access, "openehr_structures"), 105);
}

#[test]
fn a_load_list_entry_naming_no_file_is_refused() {
    let dir = repository(&["openehr_primitive_types_102.bmm"]);
    let mut access = ReferenceModelAccess::new(Vec::new());
    let error = access
        .initialise_with_load_list(directories(&dir), &["openehr_absent_9.9.9".to_owned()])
        .expect_err("the load list names no schema in the directory");
    assert!(
        matches!(
            error,
            RmAccessError::UnknownLoadListEntry { ref id } if id == "openehr_absent_9.9.9"
        ),
        "expected an unknown-load-list-entry refusal, got {error:?}",
    );
}

#[test]
fn a_missing_include_surfaces_as_the_typed_refusal() {
    // `validate_includes`: "Validate includes list for this schema, to see if
    // each mentioned schema exists in read schemas" — `basic_types` includes
    // `primitive_types`, which is not in this directory.
    let dir = repository(&["openehr_basic_types_102.bmm"]);
    let mut access = ReferenceModelAccess::new(directories(&dir));
    let error = access
        .initialise_all()
        .expect_err("the include is not among the read schemas");
    assert!(
        matches!(
            error,
            RmAccessError::MissingInclude { ref requester, ref id }
                if requester == "openehr_basic_types_1.0.2"
                    && id == "openehr_primitive_types_1.0.2"
        ),
        "expected a missing-include refusal, got {error:?}",
    );
}

#[test]
fn a_duplicate_schema_id_in_one_directory_is_refused() {
    let dir = repository(&["openehr_primitive_types_102.bmm"]);
    std::fs::write(
        dir.path().join("a_copy.bmm"),
        vendored("openehr_primitive_types_102.bmm"),
    )
    .expect("write the duplicate");
    let mut access = ReferenceModelAccess::new(directories(&dir));
    let error = access
        .initialise_all()
        .expect_err("two files, one schema id");
    assert!(
        matches!(
            error,
            RmAccessError::DuplicateSchemaId { ref id, .. } if id == "openehr_primitive_types_1.0.2"
        ),
        "expected a duplicate-schema-id refusal, got {error:?}",
    );
}

#[test]
fn reload_schemas_picks_up_an_edited_file() {
    // `reload_schemas`: "Reload all schemas" — re-read from disk.
    let dir = repository(&[]);
    let path = dir.path().join("edited.bmm");
    std::fs::write(&path, one_class_schema("")).expect("write the schema");
    let mut access = ReferenceModelAccess::new(directories(&dir));
    access.initialise_all().expect("the schema loads");
    assert_eq!(classes(&access, "openehr_edited"), 1);

    std::fs::write(&path, one_class_schema("SECOND")).expect("rewrite the schema");
    access.reload_schemas().expect("the reload succeeds");
    assert_eq!(
        classes(&access, "openehr_edited"),
        2,
        "the reload did not re-read the edited file",
    );
}

#[test]
fn a_foreign_bmm_generation_is_not_compatible() {
    // `is_bmm_compatible`: "True if the BMM version found in the schema (or
    // assumed, if none) is compatible with that in this software" — a different
    // MAJOR P_BMM generation is not.
    let dir = repository(&[]);
    let path = dir.path().join("future.bmm");
    std::fs::write(
        &path,
        one_class_schema("").replace(r#"bmm_version = <"2.4">"#, r#"bmm_version = <"9.0">"#),
    )
    .expect("write the schema");
    let descriptor = SchemaDescriptor::from_schema_file(&path).expect("the header reads");
    assert!(!descriptor.is_bmm_compatible());

    let mut access = ReferenceModelAccess::new(directories(&dir));
    let error = access
        .initialise_all()
        .expect_err("a foreign BMM generation is refused");
    assert!(
        matches!(
            error,
            RmAccessError::IncompatibleBmmVersion { ref found, .. } if found == "9.0"
        ),
        "expected an incompatible-bmm-version refusal, got {error:?}",
    );
}

/// A self-contained one- or two-class schema named `edited`; `extra` names a
/// second class when non-empty.
fn one_class_schema(extra: &str) -> String {
    let (packages, definitions) = if extra.is_empty() {
        ("\"FIRST\"".to_owned(), String::new())
    } else {
        (
            format!("\"FIRST\", \"{extra}\""),
            format!("[\"{extra}\"] = < name = <\"{extra}\"> >\n"),
        )
    };
    format!(
        r#"
        bmm_version = <"2.4">
        rm_publisher = <"openehr">
        schema_name = <"edited">
        rm_release = <"1.0.0">
        packages = <
            ["org.openehr.edited"] = <
                name = <"org.openehr.edited">
                classes = <{packages}>
            >
        >
        class_definitions = <
            ["FIRST"] = < name = <"FIRST"> >
            {definitions}
        >
    "#
    )
}
