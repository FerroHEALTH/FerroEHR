// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Public-API conformance battery for the hand-written `openehr_lang::v1_1::odin`
//! reader against the 17 vendored ODIN fixtures under `tests/vendor/odin/`.
//!
//! The fixtures are mirrored from the openEHR reference implementation
//! (archie, commit `e8d92f28`, `odin/src/test/resources/odin/`); archie's own
//! `OdinBaseVisitorTest` / `OdinBaseVisitorTest2` / `OdinBaseVisitorReferencingTest`
//! are the outcome oracle for what each fixture parses to. The ODIN grammar
//! (`crates/openehr-lang/vendor/grammar/v1_1/{odin.g4,odin_values.g4,base_lexer.g4}`)
//! and the normative spec text (`docs/specs/openehr/LANG/docs/odin/`) are the
//! syntax authority.
//!
//! Four fixtures are adjudicated as expected-error (see the individual tests
//! for the citation): `log4j2.xml` is a log4j configuration, not ODIN; the two
//! illustrative `master`-style example documents use bareword / meta-id
//! placeholders and a top-level keyed-object list that the `odin_text` start
//! rule does not accept; and `odin_test.txt` declares one attribute name
//! twice, which rule *VDATU* forbids.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_lang::v1_1::odin::{OdinErrorKind, OdinInterval, OdinKey, OdinValue, parse};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn read(rel: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vendor");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn parse_ok(rel: &str) -> OdinValue {
    parse(&read(rel)).unwrap_or_else(|e| panic!("{rel} should parse, got error: {e}"))
}

fn attr_count(v: &OdinValue) -> usize {
    match v {
        OdinValue::Object(m) => m.len(),
        other => panic!("expected Object, got {other:?}"),
    }
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

fn list(v: &OdinValue) -> &[OdinValue] {
    match v {
        OdinValue::List(items) => items,
        other => panic!("expected List, got {other:?}"),
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

/// The complete claimed set for `tests/vendor/odin/**`; the coverage gate
/// (`vendor_coverage.rs`) cross-checks this against the filesystem.
const ODIN_FIXTURES: &[(&str, bool)] = &[
    ("odin/log4j2.xml", false),
    ("odin/odin/CIMI-RM-3.0.5.bmm", true),
    ("odin/odin/CIMI-RM-3.0.5_tweaked.bmm", true),
    ("odin/odin/CIMI_RM_CLINICAL.v.0.0.1.bmm", true),
    ("odin/odin/CIMI_RM_CORE.v.0.0.1.bmm", true),
    ("odin/odin/CIMI_RM_FOUNDATION.v.0.0.1.bmm", true),
    ("odin/odin/anonymous_odin.txt", false),
    ("odin/odin/identified_object_document.txt", false),
    ("odin/odin/odin_keyed_object.txt", true),
    ("odin/odin/odin_nested_attribute_structure1.txt", true),
    ("odin/odin/odin_nested_keyed_object.txt", true),
    ("odin/odin/odin_primitive_intervals.txt", true),
    ("odin/odin/odin_primitive_lists.txt", true),
    ("odin/odin/odin_primitive_types.txt", true),
    ("odin/odin/odin_term_binding_test.txt", true),
    // Expected-error: the fixture declares the top-level attribute `people`
    // twice — see `referencing_document_is_refused_for_duplicate_attribute`.
    ("odin/odin/odin_test.txt", false),
    ("odin/odin/odin_types.txt", true),
];

/// Every claimed ODIN fixture exists and matches its coarse parse verdict
/// (Ok/Err). Structure is asserted per-fixture below.
#[test]
fn all_odin_fixtures_parse_verdict() {
    for (rel, should_parse) in ODIN_FIXTURES {
        let src = read(rel);
        let result = parse(&src);
        assert_eq!(
            result.is_ok(),
            *should_parse,
            "{rel}: expected parse ok={should_parse}, got {result:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// primitive leaf values — archie OdinBaseVisitorTest.testOdinPrimitives
// ---------------------------------------------------------------------------

#[test]
fn primitive_types() {
    let v = parse_ok("odin/odin/odin_primitive_types.txt");
    assert_eq!(attr_count(&v), 11);
    assert_eq!(as_str(field(&v, "a_string_attribute")), "a string value");
    assert_eq!(field(&v, "a_boolean_attribute"), &OdinValue::Boolean(false));
    assert_eq!(field(&v, "a_integer_attribute"), &OdinValue::Integer(1));
    assert_eq!(field(&v, "a_real_attribute"), &OdinValue::Real(-3.05e-10));
    assert_eq!(field(&v, "a_char_attribute"), &OdinValue::Character('c'));
    assert_eq!(
        field(&v, "a_term_code_attribute"),
        &OdinValue::TermCode("[ISO_639-1::en]".to_owned())
    );
    assert_eq!(
        field(&v, "a_date_attribute"),
        &OdinValue::Date("2007-11-31".to_owned())
    );
    // comma-as-decimal-separator time with a numeric timezone (`16:23:54,5+2221`).
    assert_eq!(
        field(&v, "a_time_attribute"),
        &OdinValue::Time("16:23:54,5+2221".to_owned())
    );
    assert_eq!(
        field(&v, "a_datetime_attribute"),
        &OdinValue::DateTime("2007-11-31T16:23:54,5Z".to_owned())
    );
    assert_eq!(
        field(&v, "a_duration_attribute"),
        &OdinValue::Duration("P5Y2M4W5DT34H34M63.276S".to_owned())
    );
    // EMBEDDED_URI leaf (`base_lexer.g4` EMBEDDED_URI): the `<>` are kept.
    match field(&v, "a_uri_attribute") {
        OdinValue::Uri(u) => assert!(u.contains("www.domain.com/some/path?attr1"), "{u}"),
        other => panic!("expected Uri, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// primitive lists — archie testOdinLists (asserts element counts)
// ---------------------------------------------------------------------------

#[test]
fn primitive_lists() {
    let v = parse_ok("odin/odin/odin_primitive_lists.txt");
    assert_eq!(attr_count(&v), 9);
    // (attribute, expected element count) mirrored from archie's assertions;
    // the comma-as-decimal-separator time/datetime lists exercise the lexer's
    // longest-match vs the list separator.
    let counts = [
        ("a_string_list_attribute", 3),
        ("a_boolean_list_attribute", 4),
        ("a_integer_list_attribute", 5),
        ("a_real_list_attribute", 4),
        ("a_char_list_attribute", 3),
        ("a_term_code_list_attribute", 2),
        ("a_date_list_attribute", 3),
        ("a_time_list_attribute", 3),
        ("a_datetime_list_attribute", 2),
    ];
    for (name, want) in counts {
        assert_eq!(list(field(&v, name)).len(), want, "{name}");
    }
    // spot the signed-integer list `1,+2,6,-3,2`.
    assert_eq!(
        list(field(&v, "a_integer_list_attribute")),
        &[
            OdinValue::Integer(1),
            OdinValue::Integer(2),
            OdinValue::Integer(6),
            OdinValue::Integer(-3),
            OdinValue::Integer(2),
        ]
    );
    // term-code list `[ISO_639-1::en],[ICD10AM(1998)::F23]` (the second carries
    // a `(version)` qualifier — `base_lexer.g4` TERM_CODE_REF).
    assert_eq!(
        list(field(&v, "a_term_code_list_attribute")),
        &[
            OdinValue::TermCode("[ISO_639-1::en]".to_owned()),
            OdinValue::TermCode("[ICD10AM(1998)::F23]".to_owned()),
        ]
    );
}

// ---------------------------------------------------------------------------
// intervals — archie testOdinIntervals
// ---------------------------------------------------------------------------

#[test]
fn primitive_intervals() {
    let v = parse_ok("odin/odin/odin_primitive_intervals.txt");
    assert_eq!(attr_count(&v), 7);

    // |1..2| — closed integer range.
    match field(&v, "a_integer_interval_attribute1") {
        OdinValue::Interval(OdinInterval::Range {
            lower,
            lower_included,
            upper,
            upper_included,
        }) => {
            assert_eq!(lower.as_deref(), Some(&OdinValue::Integer(1)));
            assert_eq!(upper.as_deref(), Some(&OdinValue::Integer(2)));
            assert!(*lower_included && *upper_included);
        }
        other => panic!("expected range, got {other:?}"),
    }

    // |>=6| — lower-bounded (inclusive), open above.
    match field(&v, "a_integer_interval_attribute2") {
        OdinValue::Interval(OdinInterval::Range {
            lower,
            lower_included,
            upper,
            ..
        }) => {
            assert_eq!(lower.as_deref(), Some(&OdinValue::Integer(6)));
            assert!(*lower_included);
            assert!(upper.is_none());
        }
        other => panic!("expected range, got {other:?}"),
    }

    // |>6.7..<23.24e2| — both bounds exclusive; `23.24e2` = 2324.0.
    match field(&v, "a_real_interval_attribute") {
        OdinValue::Interval(OdinInterval::Range {
            lower,
            lower_included,
            upper,
            upper_included,
        }) => {
            assert_eq!(lower.as_deref(), Some(&OdinValue::Real(6.7)));
            assert_eq!(upper.as_deref(), Some(&OdinValue::Real(2324.0)));
            assert!(!*lower_included && !*upper_included);
        }
        other => panic!("expected range, got {other:?}"),
    }

    // |<16:23:54,5-0524| — upper-bounded (exclusive), open below.
    match field(&v, "a_time_interval_attribute") {
        OdinValue::Interval(OdinInterval::Range {
            lower,
            upper,
            upper_included,
            ..
        }) => {
            assert!(lower.is_none());
            assert_eq!(
                upper.as_deref(),
                Some(&OdinValue::Time("16:23:54,5-0524".to_owned()))
            );
            assert!(!*upper_included);
        }
        other => panic!("expected range, got {other:?}"),
    }

    // duration range — both bounds inclusive.
    match field(&v, "a_duration_interval_attribute") {
        OdinValue::Interval(OdinInterval::Range {
            lower,
            upper,
            lower_included,
            upper_included,
        }) => {
            assert_eq!(
                lower.as_deref(),
                Some(&OdinValue::Duration("P5Y2M4W5DT34H34M63.276S".to_owned()))
            );
            assert_eq!(
                upper.as_deref(),
                Some(&OdinValue::Duration("P6Y2M4W5DT34H34M63.276S".to_owned()))
            );
            assert!(*lower_included && *upper_included);
        }
        other => panic!("expected range, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// keyed objects & nested structures — archie validateKeyedObjects /
// validateNestedAttributeStructures / validateOdinNestedKeyedObject
// ---------------------------------------------------------------------------

#[test]
fn keyed_object() {
    let v = parse_ok("odin/odin/odin_keyed_object.txt");
    assert_eq!(attr_count(&v), 1);
    let entries = keyed(field(&v, "attribute1"));
    assert_eq!(entries.len(), 2);
    // keys are quoted strings (`["1"]`, `["2"]`) → OdinKey::String.
    assert_eq!(entries[0].0, OdinKey::String("1".to_owned()));
    assert_eq!(as_str(&entries[0].1), "One");
    assert_eq!(entries[1].0, OdinKey::String("2".to_owned()));
    assert_eq!(as_str(&entries[1].1), "Two");
}

#[test]
fn nested_attribute_structure() {
    let v = parse_ok("odin/odin/odin_nested_attribute_structure1.txt");
    assert_eq!(attr_count(&v), 1);
    let a1 = field(&v, "attribute1");
    assert_eq!(attr_count(a1), 3);
    assert_eq!(as_str(field(a1, "attribute1_1")), "attribute1_1");
    assert_eq!(as_str(field(a1, "attribute1_2")), "attribute1_2");
    let a13 = field(a1, "attribute1_3");
    assert_eq!(attr_count(a13), 2);
    assert_eq!(as_str(field(a13, "attribute1_3_1")), "attribute1_3_1");
    assert_eq!(as_str(field(a13, "attribute1_3_2")), "attribute1_3_2");
}

#[test]
fn nested_keyed_object() {
    let v = parse_ok("odin/odin/odin_nested_keyed_object.txt");
    let td = field(&v, "term_definitions");
    assert_eq!(keyed(td).len(), 1);
    let en = keyed_str(td, "en");
    // `id0.2` etc. — dotted id codes are legal string keys.
    assert_eq!(keyed(en).len(), 4);
    assert_eq!(as_str(field(keyed_str(en, "id1.1"), "text")), "Actor");
    assert_eq!(as_str(field(keyed_str(en, "id0.2"), "text")), "Language");
    assert_eq!(as_str(field(keyed_str(en, "id0.4"), "text")), "Actor type");
}

#[test]
fn term_binding_uris() {
    let v = parse_ok("odin/odin/odin_term_binding_test.txt");
    let bindings = field(&v, "term_bindings");
    let snomed = keyed_str(bindings, "snomed-ct");
    let items = field(snomed, "items");
    assert_eq!(keyed(items).len(), 4);
    // each keyed value is an EMBEDDED_URI leaf.
    match keyed_str(items, "id1.1") {
        OdinValue::Uri(u) => assert!(u.contains("snomed.info/id/138875005"), "{u}"),
        other => panic!("expected Uri, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// typed casts & references — archie validateOdinTypes / referencing test
// ---------------------------------------------------------------------------

#[test]
fn typed_casts_and_generic_types() {
    let v = parse_ok("odin/odin/odin_types.txt");
    assert_eq!(attr_count(&v), 2);

    // person = (List<PERSON>) < [01234] = <name = <...> address = <...>> >
    let person = field(&v, "person");
    let OdinValue::Typed { rm_type, value } = person else {
        panic!("expected typed cast, got {person:?}");
    };
    assert_eq!(rm_type, "List<PERSON>");
    // `[01234]` is an integer key (leading zeros preserved as the value 1234).
    let entries = keyed(value);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, OdinKey::Integer(1234));
    let holmes = &entries[0].1;
    assert_eq!(attr_count(holmes), 2);
    let name = field(holmes, "name");
    assert_eq!(as_str(field(name, "forenames")), "Sherlock");
    assert_eq!(as_str(field(name, "family_name")), "Holmes");
    let address = field(holmes, "address");
    assert_eq!(as_str(field(address, "habitation_number")), "221B");
    assert_eq!(as_str(field(address, "country")), "England");

    // parent → ["ELEMENT"] with an open `ancestors` list and typed properties.
    let parent = field(&v, "parent");
    let element = keyed_str(parent, "ELEMENT");
    assert_eq!(as_str(field(element, "name")), "ELEMENT");
    let ancestors = list(field(element, "ancestors"));
    // `<"ITEM", ...>` — open one-element list: string then continuation marker.
    assert_eq!(
        ancestors,
        &[
            OdinValue::String("ITEM".to_owned()),
            OdinValue::ListContinue
        ]
    );
    let props = field(element, "properties");
    let null_flavor = keyed_str(props, "null_flavor");
    let OdinValue::Typed { rm_type, value } = null_flavor else {
        panic!("expected typed property, got {null_flavor:?}");
    };
    assert_eq!(rm_type, "P_BMM_SINGLE_PROPERTY");
    assert_eq!(as_str(field(value, "bmmType")), "CODED_TEXT");
}

/// The INVALID twin of [`referencing_document_parses`]: the vendored fixture
/// itself, which declares the top-level attribute `people` twice (once with
/// integer keys, once with string keys — archie's fixture concatenates the two
/// container examples of `AM/docs/ADL1.4/master04-dadl` §Container Objects
/// under the same attribute name).
///
/// This assertion was MOVED TOWARD THE SPEC: it previously pinned a
/// last-one-wins overwrite (6 surviving attributes). Sibling attribute names
/// must be unique — `LANG/docs/odin/master05-content` §General Structure rule
/// *VDATU*, and the principle "Sibling attribute names must be unique" of
/// `AM/docs/ADL1.4/master04-dadl` §General Form — so the document is
/// spec-invalid and the reader now refuses it with the typed error naming the
/// repeated attribute. The vendored fixture is unchanged; only the expectation
/// moved, from an implementation behaviour to the spec rule.
#[test]
fn referencing_document_is_refused_for_duplicate_attribute() {
    let err = parse(&read("odin/odin/odin_test.txt")).expect_err("duplicate `people` must refuse");
    assert_eq!(
        err.kind,
        OdinErrorKind::DuplicateAttribute("people".to_owned())
    );
}

/// The spec-valid twin of the fixture above, with the duplicated attribute
/// renamed and nothing else changed — so every construct archie's
/// `OdinBaseVisitorReferencingTest` exercises stays asserted: `;`-separated
/// attributes (`term = <text = <"plan">; …>`), object-reference paths
/// (`</hotels["gran sevilla"]>`), and typed keyed values
/// (`(HISTORIC_HOTEL) <…>`).
const REFERENCING_DOCUMENT_VALID_TWIN: &str = r#"
term = <text = <"plan">; description = <"The clinician's advice">>

people_by_index = <
    [1] = <name = <"akmal"> birth_date = <1975-02-21> interests = <"painting", "running"> >
>

people = <
    ["akmal:1975-04-22"] = <name = <"akmal"> birth_date = <1975-04-22> >
>

destinations = <
    ["seville"] = <
        hotels = <
            ["gran sevilla"] = </hotels["gran sevilla"]>
        >
    >
>

hotels = <
    ["gran sevilla"] = (HISTORIC_HOTEL) <name=<"Gran Sevilla Hotel">>
    ["sofitel"] = (LUXURY_HOTEL) <name=<"Sofitel">>
>
"#;

#[test]
fn referencing_document_parses() {
    let v = parse(REFERENCING_DOCUMENT_VALID_TWIN).expect("valid twin parses");
    assert_eq!(attr_count(&v), 5);
    assert_eq!(as_str(field(field(&v, "term"), "text")), "plan");
    // hotels holds typed keyed objects.
    let hotels = field(&v, "hotels");
    let gran = keyed_str(hotels, "gran sevilla");
    let OdinValue::Typed { rm_type, .. } = gran else {
        panic!("expected typed hotel, got {gran:?}");
    };
    assert_eq!(rm_type, "HISTORIC_HOTEL");
    // destinations references hotels by object-reference path list.
    let dest = keyed_str(field(&v, "destinations"), "seville");
    let path_hotels = field(dest, "hotels");
    match keyed_str(path_hotels, "gran sevilla") {
        OdinValue::PathList(paths) => {
            assert_eq!(paths, &["/hotels[\"gran sevilla\"]".to_owned()]);
        }
        other => panic!("expected path list, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// BMM schemas serialized as ODIN — archie OdinBaseVisitorTest.loadReferenceModel
// (CIMI-RM-3.0.5) + OdinBaseVisitorTest2.loadReferenceModel2 (CLINICAL)
// ---------------------------------------------------------------------------

/// Assert the eight leaf schema-header attributes archie's
/// `validateRootLevelAttributes` pins.
fn assert_schema_header(v: &OdinValue, bmm_version: &str, schema_name: &str, rm_release: &str) {
    assert_eq!(attr_count(v), 11, "{schema_name}: root attribute count");
    assert_eq!(
        as_str(field(v, "bmm_version")),
        bmm_version,
        "{schema_name}: bmm_version"
    );
    assert_eq!(
        as_str(field(v, "rm_publisher")),
        "CIMI",
        "{schema_name}: rm_publisher"
    );
    assert_eq!(
        as_str(field(v, "schema_name")),
        schema_name,
        "{schema_name}: schema_name"
    );
    assert_eq!(
        as_str(field(v, "rm_release")),
        rm_release,
        "{schema_name}: rm_release"
    );
    assert_eq!(
        as_str(field(v, "schema_lifecycle_state")),
        "dstu",
        "{schema_name}: schema_lifecycle_state"
    );
    // packages + class_definitions are the two keyed-list bodies.
    assert!(
        !keyed(field(v, "packages")).is_empty(),
        "{schema_name}: packages must be a non-empty keyed list"
    );
    assert!(
        !keyed(field(v, "class_definitions")).is_empty(),
        "{schema_name}: class_definitions must be a non-empty keyed list"
    );
}

#[test]
fn cimi_rm_reference_model() {
    // archie loadReferenceModel: 11 root attributes; publisher CIMI, schema RM.
    let v = parse_ok("odin/odin/CIMI-RM-3.0.5.bmm");
    assert_schema_header(&v, "2.0", "RM", "3.0.5");
    assert_eq!(
        as_str(field(&v, "schema_description")),
        "CIMI_Reference_Model v3.0.5 schema generated from UML"
    );
    // single-element open list `<"CIMI_Reference_Model.Core", ...>`.
    let closure = list(field(&v, "archetype_rm_closure_packages"));
    assert_eq!(closure.len(), 2);
    assert_eq!(as_str(&closure[0]), "CIMI_Reference_Model.Core");
    assert_eq!(closure[1], OdinValue::ListContinue);
}

#[test]
fn cimi_rm_reference_model_tweaked_is_identical() {
    // `CIMI-RM-3.0.5_tweaked.bmm` is byte-identical to the original (archie's
    // `loadReferenceModel1` is commented out); it is valid ODIN and parses to
    // the same tree.
    let a = read("odin/odin/CIMI-RM-3.0.5.bmm");
    let b = read("odin/odin/CIMI-RM-3.0.5_tweaked.bmm");
    assert_eq!(
        a, b,
        "tweaked fixture is expected to be identical to the original"
    );
    let v = parse_ok("odin/odin/CIMI-RM-3.0.5_tweaked.bmm");
    assert_schema_header(&v, "2.0", "RM", "3.0.5");
}

#[test]
fn cimi_rm_clinical() {
    // archie OdinBaseVisitorTest2.loadReferenceModel2 (parse-only).
    let v = parse_ok("odin/odin/CIMI_RM_CLINICAL.v.0.0.1.bmm");
    assert_schema_header(&v, "2.1", "RM_CLINICAL", "0.0.1");
}

#[test]
fn cimi_rm_core_open_multi_list() {
    // Regression guard for the open multi-element list fix (`master07`
    // §"Lists of Built-in Types"): archie loads this file as a builtin
    // reference model, but the pre-fix parser rejected the trailing `, ...`
    // after a multi-element string list.
    let v = parse_ok("odin/odin/CIMI_RM_CORE.v.0.0.1.bmm");
    assert_schema_header(&v, "2.1", "RM_CORE", "0.0.1");
    let closure = list(field(&v, "archetype_rm_closure_packages"));
    assert_eq!(closure.len(), 4);
    assert_eq!(as_str(&closure[0]), "CIMI_Reference_Model.Core");
    assert_eq!(as_str(&closure[2]), "CIMI_Reference_Model.Primitive_Types");
    assert_eq!(closure[3], OdinValue::ListContinue);
}

#[test]
fn cimi_rm_foundation_open_multi_list() {
    let v = parse_ok("odin/odin/CIMI_RM_FOUNDATION.v.0.0.1.bmm");
    assert_schema_header(&v, "2.1", "RM_FOUNDATION", "0.0.1");
    let closure = list(field(&v, "archetype_rm_closure_packages"));
    assert_eq!(closure.len(), 3);
    assert_eq!(as_str(&closure[0]), "CIMI_Foundation_RM.Foundation");
    assert_eq!(as_str(&closure[1]), "CIMI_Foundation_RM.Party");
    assert_eq!(closure[2], OdinValue::ListContinue);
}

// ---------------------------------------------------------------------------
// adjudicated expected-error fixtures
// ---------------------------------------------------------------------------

#[test]
fn log4j2_xml_is_not_odin() {
    // Adjudication: `odin/log4j2.xml` mirrors archie's log4j2 *logging* config
    // resource, not an ODIN fixture (archie has no ODIN test referencing it).
    // It is XML; the leading `<?xml` is not lexable ODIN → Err at the start.
    let err = parse(&read("odin/log4j2.xml")).expect_err("XML is not ODIN");
    assert_eq!(err.line, 1);
}

#[test]
fn anonymous_document_placeholder_leaf_is_refused() {
    // Adjudication: the Anonymous Object Document FORM — an outer `<>` around a
    // fragment — is normative and "should be supported by parsers"
    // (`LANG/docs/odin/master04-odin_artefacts` §Anonymous Object Document);
    // its materialized twin parses in `odin_spec_examples.rs`. What keeps THIS
    // fixture refused is only its illustrative bareword meta-placeholder
    // `leaf_value`: a bareword is neither a `primitive_object`, an `attr_vals`,
    // a `keyed_object`, nor an `object_reference_block` path (`base_lexer.g4`
    // ADL_PATH requires a `/`).
    let err = parse(&read("odin/odin/anonymous_odin.txt")).expect_err("bareword leaf is invalid");
    assert!(err.line >= 1 && err.column >= 1);
}

#[test]
fn identified_document_placeholder_ellipsis_is_refused() {
    // Adjudication (re-grounded by the #854 ch.4 audit / #1374): the
    // Identified Object Document FORM — top-level `["id"] = <…>` keyed
    // objects — is normative (`LANG/docs/odin/master04-odin_artefacts`
    // §Identified Object Document) and parses since #1374; its materialized
    // twin is pinned in `odin_spec_examples.rs`. What keeps THIS fixture
    // refused is only the illustrative bare `...` ellipsis line standing
    // BETWEEN two keyed objects at the top level, which no production
    // admits. archie has no test referencing it.
    let err = parse(&read("odin/odin/identified_object_document.txt"))
        .expect_err("a bare top-level `...` between keyed objects is invalid");
    assert!(err.line >= 1 && err.column >= 1);
}
