// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Spec-example vector suite for the Simplified Formats.
//!
//! Every JSON example block in the STABLE ITS-REST Simplified Formats
//! chapters is checked in here as a test vector, each test citing the spec
//! file + section it encodes:
//!
//! - `docs/specs/openehr/ITS-REST/docs/simplified_formats/master04-basic_concepts.adoc`
//!   — the FLAT / STRUCTURED examples, the field-identifier / node-id
//!   examples, the `_normal_range`/`_link` RM-attribute examples, `|raw`,
//!   the Level-Removal worked example, and the Open Value-Sets `|other`
//!   rules.
//! - `master05-rm_mapping.adoc` — the per-RM-type example blocks.
//! - `master06-context_information.adoc` — the `ctx/` vocabulary examples.
//!
//! What is asserted at the public seam (no template needed for the bulk):
//!
//! - **FlatKey parse/print** (`openehr_sdt::flat::path::FlatKey`) round-trips
//!   every key that appears in the examples (master04 §Field Identifiers).
//! - **FLAT parse/emit is lossless & stable** and **FLAT⇄STRUCTURED is a
//!   stable normal form** for each example (master04 §Flat format,
//!   §Structured format, §Conversion Between Formats), via
//!   `openehr_sdt::flat::convert::{flat_to_structured, structured_to_flat}` and
//!   `openehr_sdt::flat::sim::flat::{parse_flat, emit_flat}`.
//! - **Reject rules** via `openehr_sdt::flat::convert::composition_from_flat`
//!   against a minimal hand-built Web Template (unknown path/suffix,
//!   malformed keys, `|raw` without `_type`, `|other` conflicts/closed set).
//! - **Mandatory context** via `openehr_sdt::flat::validation::validate_context`
//!   (master04 §Validation, §Context).
//!
//! Values are transcribed byte-identical to the spec (incl. `°C`, big
//! numbers, and timestamps). Tests are deterministic — the one building
//! direction uses a fixed `NOW`.
#![expect(
    clippy::panic,
    clippy::expect_used,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
#![expect(
    clippy::doc_markdown,
    reason = "the module docs quote openEHR spec prose and Simplified-Formats key names as text, not as Rust code references"
)]

use indexmap::IndexMap;
use serde_json::{Map, Value};

use openehr_sdt::flat::convert::{
    composition_from_flat, flat_to_structured, structured_to_flat, submitted_composition_from_flat,
};
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::path::FlatKey;
use openehr_sdt::flat::sim::flat::{emit_flat, parse_flat};
use openehr_sdt::flat::validation::{validate_context, validate_flat_other};
use openehr_sdt::flat::webtemplate::model::{
    WebTemplate, WebTemplateCodedValue, WebTemplateInput, WebTemplateInputType, WebTemplateNode,
};
use openehr_sdt::rm_instance::ValidationKind;

// ── fixed inputs ─────────────────────────────────────────────────────────────

/// Deterministic `now` for the one building direction (`master04 §Context`:
/// `ctx/time` defaults to `now()`). Fixed so tests never depend on the clock.
const NOW: &str = "2024-01-15T10:30:00Z";

/// Parse a spec FLAT example (a single-level JSON object) verbatim.
fn flat_of(json: &str) -> Map<String, Value> {
    serde_json::from_str(json).expect("spec FLAT block must be a JSON object")
}

/// Parse a spec STRUCTURED example verbatim.
fn json_of(json: &str) -> Value {
    serde_json::from_str(json).expect("spec block must be valid JSON")
}

// ── shared assertions ────────────────────────────────────────────────────────

/// A FLAT-example vector (master04 §Flat format, §Conversion Between Formats):
///
/// 1. every key is a syntactically valid [`FlatKey`] whose `Display` reproduces
///    it byte-for-byte (master04 §Field Identifiers);
/// 2. FLAT parse→emit is lossless and stable (no key added, dropped, or
///    re-spelled — comparison is order-independent `Map` equality);
/// 3. FLAT→STRUCTURED→FLAT→STRUCTURED reaches a stable STRUCTURED normal form
///    (the two §Conversion algorithms compose to a fixed point).
fn assert_flat_vector(json: &str) {
    let flat = flat_of(json);
    for key in flat.keys() {
        let parsed = FlatKey::parse(key).unwrap_or_else(|e| panic!("key {key:?} must parse: {e}"));
        assert_eq!(
            &parsed.to_string(),
            key,
            "FlatKey Display must reproduce {key:?} verbatim"
        );
    }
    assert_eq!(
        emit_flat(&parse_flat(&flat).expect("parse_flat")),
        flat,
        "FLAT parse/emit must be lossless & stable"
    );
    let s1 = flat_to_structured(&flat).expect("flat→structured");
    let s2 = flat_to_structured(&structured_to_flat(&s1).expect("structured→flat"))
        .expect("flat→structured");
    assert_eq!(s1, s2, "FLAT⇄STRUCTURED must reach a stable normal form");
}

/// A STRUCTURED-example vector (master04 §Structured format): STRUCTURED→FLAT→
/// STRUCTURED reproduces the input (order-independent `Value` equality).
fn assert_structured_vector(json: &str) {
    let s = json_of(json);
    let f = structured_to_flat(&s).expect("structured→flat");
    let s2 = flat_to_structured(&f).expect("flat→structured");
    assert_eq!(s2, s, "STRUCTURED→FLAT→STRUCTURED must be stable");
}

// ── minimal Web Template for the reject-path vectors ─────────────────────────

/// A minimal Web Template: a `COMPOSITION` root (`id = "test"`) with a single
/// `DV_CODED_TEXT` leaf child (`id = "coded"`) whose input carries a coded
/// `list` and the given `listOpen` flag. Enough to drive the master04
/// §Validation / §Open Value-Sets reject paths through the public
/// `composition_from_flat` seam without a full OPT.
fn coded_leaf_wt(list_open: bool) -> WebTemplate {
    let mut root = WebTemplateNode::new("COMPOSITION".to_owned(), String::new());
    "test".clone_into(&mut root.id);
    root.min = Some(1);
    root.max = 1;

    let mut leaf = WebTemplateNode::new(
        "DV_CODED_TEXT".to_owned(),
        "/content[openEHR-EHR-OBSERVATION.x.v1]/data[at0001]/events[at0002]\
         /data[at0003]/items[at0004]/value"
            .to_owned(),
    );
    "coded".clone_into(&mut leaf.id);
    leaf.min = Some(0);
    leaf.max = 1;
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
    input.list = vec![WebTemplateCodedValue::new(
        "at0001",
        Some("Option one".to_owned()),
    )];
    input.list_open = Some(list_open);
    leaf.inputs = vec![input];
    root.children = vec![leaf];

    WebTemplate {
        template_id: "reject.v0".to_owned(),
        sem_ver: None,
        version: "2.3".to_owned(),
        default_language: "en".to_owned(),
        languages: vec!["en".to_owned()],
        tree: root,
        other_details: IndexMap::new(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// master04 — basic concepts
// ═══════════════════════════════════════════════════════════════════════════

/// master04 §Format variants — Flat format (the worked FLAT example: two
/// observations with a `_normal_range` RM-attribute subtree, incl. `°C`).
#[test]
fn master04_flat_format_example() {
    assert_flat_vector(
        r#"{
          "ctx/language": "en",
          "ctx/territory": "US",
          "ctx/composer_name": "Dr. Smith",
          "ctx/time": "2024-01-15T10:30:00Z",
          "vital_signs/body_temperature:0/any_event:0/temperature|magnitude": 37.5,
          "vital_signs/body_temperature:0/any_event:0/temperature|unit": "°C",
          "vital_signs/body_temperature:0/any_event:0/temperature/_normal_range/lower|magnitude": 36.0,
          "vital_signs/body_temperature:0/any_event:0/temperature/_normal_range/lower|unit": "°C",
          "vital_signs/body_temperature:0/any_event:0/temperature/_normal_range/upper|magnitude": 37.8,
          "vital_signs/body_temperature:0/any_event:0/temperature/_normal_range/upper|unit": "°C",
          "vital_signs/body_temperature:0/any_event:0/time": "2024-01-15T10:30:00Z",
          "vital_signs/blood_pressure:0/any_event:0/systolic|magnitude": 120,
          "vital_signs/blood_pressure:0/any_event:0/systolic|unit": "mm[Hg]",
          "vital_signs/blood_pressure:0/any_event:0/diastolic|magnitude": 80,
          "vital_signs/blood_pressure:0/any_event:0/diastolic|unit": "mm[Hg]",
          "vital_signs/blood_pressure:0/any_event:0/time": "2024-01-15T10:30:00Z"
        }"#,
    );
}

/// master04 §Format variants — Structured format (the same data nested; note
/// the spec's STRUCTURED block omits the `_normal_range` subtree the FLAT block
/// carries, so the two are asserted independently, not cross-equated).
#[test]
fn master04_structured_format_example() {
    assert_structured_vector(
        r#"{
          "ctx": {
            "language": "en",
            "territory": "US",
            "composer_name": "Dr. Smith",
            "time": "2024-01-15T10:30:00Z"
          },
          "vital_signs": {
            "body_temperature": [
              {
                "any_event": [
                  {
                    "temperature": [
                      {
                        "|magnitude": 37.5,
                        "|unit": "°C"
                      }
                    ],
                    "time": [
                      "2024-01-15T10:30:00Z"
                    ]
                  }
                ]
              }
            ],
            "blood_pressure": [
              {
                "any_event": [
                  {
                    "systolic": [
                      {
                        "|magnitude": 120,
                        "|unit": "mm[Hg]"
                      }
                    ],
                    "diastolic": [
                      {
                        "|magnitude": 80,
                        "|unit": "mm[Hg]"
                      }
                    ],
                    "time": [
                      "2024-01-15T10:30:00Z"
                    ]
                  }
                ]
              }
            ]
          }
        }"#,
    );
}

/// master04 §Field Identifiers — the example identifier structure (a leaf
/// magnitude and a `_normal_range` RM-attribute leaf) parse to the expected
/// segment/suffix shapes.
#[test]
fn master04_field_identifier_structure() {
    let k =
        FlatKey::parse("vital_signs/body_temperature:0/any_event:0/temperature|magnitude").unwrap();
    assert_eq!(k.segments.len(), 4);
    assert_eq!(k.segments[1].name, "body_temperature");
    assert_eq!(k.segments[1].index, Some(0));
    assert_eq!(k.suffixes.len(), 1);
    assert_eq!(k.suffixes[0].name, "magnitude");

    let k = FlatKey::parse(
        "vital_signs/body_temperature:0/any_event:0/temperature/_normal_range/lower|magnitude",
    )
    .unwrap();
    assert!(k.segments.iter().any(|s| s.name == "_normal_range"));
    assert!(
        k.segments
            .iter()
            .any(|s| s.name == "_normal_range" && s.is_rm_attribute())
    );
    assert_eq!(k.suffixes[0].name, "magnitude");
}

/// master04 §Node ID Generation Rules — every generated node id in the worked
/// table is a legal single-segment FLAT key that Displays back verbatim (the
/// syntactic contract between the node-id generator and the path layer). The
/// generation *algorithm* itself is exercised by `webtemplate::id`'s own tests
/// (the generator is `pub(crate)` — see the seam-gap note in the task report).
#[test]
fn master04_node_id_table_ids_are_legal_flat_segments() {
    for id in [
        "body_temperature",  // Body temperature
        "problem_diagnosis", // Problem/diagnosis
        "tests_1_2_3",       // Tests (1, 2, 3)
        "a1st_visit",        // 1st visit
        "blood_pressure",    // Blood Pressure
        "blood_pressure_1",  // Blood Pressure (duplicate)
    ] {
        let k = FlatKey::parse(id).unwrap_or_else(|e| panic!("node id {id:?} must parse: {e}"));
        assert_eq!(k.segments.len(), 1);
        assert_eq!(k.segments[0].name, id);
        assert!(k.suffixes.is_empty());
        assert_eq!(&k.to_string(), id);
    }
}

/// master04 §RM Attributes prefix — the `_uid` example (an optional RM
/// attribute on a repeating observation).
#[test]
fn master04_rm_attr_uid_example() {
    assert_flat_vector(
        r#"{
          "conformance/observation:0/_uid": "9fcc1c70-9349-444d-b9cb-8fa817697f5e"
        }"#,
    );
}

/// master04 §RM Attributes prefix — the `_link` example (a `_link:0` RM
/// attribute with a chained `|meaning|code` suffix).
#[test]
fn master04_rm_attr_link_example() {
    assert_flat_vector(
        r#"{
          "path/observation:0/_link:0|type": "problem",
          "path/observation:0/_link:0|target": "ehr://problem-123",
          "path/observation:0/_link:0|meaning|code": "related_to",
          "path/observation:0/_link:0|meaning|value": "Related to"
        }"#,
    );
    // The chained suffix parses to a two-part suffix chain on an indexed
    // RM-attribute segment.
    let k = FlatKey::parse("path/observation:0/_link:0|meaning|code").unwrap();
    assert_eq!(k.segments[2].name, "_link");
    assert_eq!(k.segments[2].index, Some(0));
    assert_eq!(k.suffixes.len(), 2);
    assert_eq!(k.suffixes[0].name, "meaning");
    assert_eq!(k.suffixes[1].name, "code");
}

/// master04 §RM Attributes prefix — the `_normal_range` example (`°C`).
#[test]
fn master04_rm_attr_normal_range_example() {
    assert_flat_vector(
        r#"{
          "vital_signs/temperature:0/value|magnitude": 37.5,
          "vital_signs/temperature:0/value|unit": "°C",
          "vital_signs/temperature:0/value/_normal_range/lower|magnitude": 36.0,
          "vital_signs/temperature:0/value/_normal_range/lower|unit": "°C",
          "vital_signs/temperature:0/value/_normal_range/upper|magnitude": 37.8,
          "vital_signs/temperature:0/value/_normal_range/upper|unit": "°C"
        }"#,
    );
}

/// master04 §Raw canonical JSON — the `|raw` bypass round-trips as a pure
/// transform (the embedded canonical-JSON object is carried verbatim through
/// FLAT⇄STRUCTURED). The *building* semantics of `|raw` (must carry `_type`)
/// are covered by `rejects_raw_without_type`.
#[test]
fn master04_raw_canonical_json_example() {
    assert_flat_vector(
        r#"{
          "ctx/language": "en",
          "ctx/territory": "US",
          "ctx/composer_name": "Dr. Smith",
          "ctx/time": "2024-01-15T10:30:00Z",
          "vital_signs/blood_pressure:0/any_event:0/systolic|raw": {
            "_type": "DV_QUANTITY",
            "magnitude": 120,
            "unit": "mm[Hg]"
          }
        }"#,
    );
}

/// master04 §Level Removal — the worked FLAT key (a lab-panel result magnitude)
/// parses to the collapsed segment shape the chapter describes (container
/// attributes and wrapper node-ids elided; `ELEMENT.value` → `|magnitude`; the
/// inner CLUSTER repeats, hence `:0`). The canonical-RM-path ↔ FLAT-key
/// *collapse* itself is template-driven (see the seam-gap note in the report).
#[test]
fn master04_level_removal_flat_key() {
    let k = FlatKey::parse(
        "laboratory_test_report/laboratory_test/laboratory_test_panel/laboratory_result:0/result_value|magnitude",
    )
    .unwrap();
    let names: Vec<&str> = k.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "laboratory_test_report",
            "laboratory_test",
            "laboratory_test_panel",
            "laboratory_result",
            "result_value",
        ]
    );
    assert_eq!(
        k.segments[3].index,
        Some(0),
        "the repeating inner CLUSTER carries :0"
    );
    // master04 §Level Removal: "`ELEMENT.value` is replaced by the
    // `|magnitude` attribute suffix" — the key carries exactly that suffix.
    assert_eq!(k.suffixes.len(), 1);
    assert_eq!(k.suffixes[0].name, "magnitude");
    assert_eq!(
        &k.to_string(),
        "laboratory_test_report/laboratory_test/laboratory_test_panel/laboratory_result:0/result_value|magnitude"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// master04 — reject rules (public seam: composition_from_flat + minimal WT)
// ═══════════════════════════════════════════════════════════════════════════

/// master04 §Validation ("Field identifiers match WT metadata structure") — a
/// top-level segment that is neither `ctx` nor the template root is
/// `UnknownPath`.
#[test]
fn rejects_unknown_root_segment() {
    let wt = coded_leaf_wt(true);
    let err = composition_from_flat(&flat_of(r#"{ "other_root/x": "y" }"#), &wt, NOW).unwrap_err();
    assert!(matches!(err, FlatError::UnknownPath(_)), "got {err:?}");
}

/// master04 §Validation — a data segment matching no template child is
/// `UnknownPath`.
#[test]
fn rejects_unknown_child_segment() {
    let wt = coded_leaf_wt(true);
    let err = composition_from_flat(&flat_of(r#"{ "test/nope": "y" }"#), &wt, NOW).unwrap_err();
    assert!(matches!(err, FlatError::UnknownPath(_)), "got {err:?}");
}

/// master04 §Attribute Suffixes / §Raw canonical JSON — a datum suffix on a
/// container node that is not `|raw` is `UnknownSuffix`.
#[test]
fn rejects_unknown_suffix_on_container() {
    let wt = coded_leaf_wt(true);
    let err = composition_from_flat(&flat_of(r#"{ "test|bogus": "x" }"#), &wt, NOW).unwrap_err();
    assert!(
        matches!(err, FlatError::UnknownSuffix { .. }),
        "got {err:?}"
    );
}

/// master04 §Raw canonical JSON ("The raw JSON value must include the `_type`
/// property") — a `|raw` payload without `_type` is `InvalidRaw`.
#[test]
fn rejects_raw_without_type() {
    let wt = coded_leaf_wt(true);
    let err = composition_from_flat(
        &flat_of(r#"{ "test|raw": { "magnitude": 120 } }"#),
        &wt,
        NOW,
    )
    .unwrap_err();
    assert!(matches!(err, FlatError::InvalidRaw { .. }), "got {err:?}");
}

/// master04 §Flat format syntax rules — malformed keys are `MalformedPath`,
/// both at the `FlatKey` parser and through the `composition_from_flat` seam.
#[test]
fn rejects_malformed_keys() {
    for bad in [
        "",             // empty key
        "a//b",         // empty path segment
        "a:x",          // non-numeric instance index
        "a/leaf:70000", // instance index above the resource-safety bound
        "a|",           // empty attribute suffix
        "a/:0",         // empty name before ':'
        "a|code:x",     // non-numeric suffix index
    ] {
        assert!(
            matches!(FlatKey::parse(bad), Err(FlatError::MalformedPath { .. })),
            "FlatKey::parse should reject {bad:?}"
        );
    }
    // And the same rejection surfaces through the conversion seam.
    let wt = coded_leaf_wt(true);
    let err = composition_from_flat(&flat_of(r#"{ "test//x": "y" }"#), &wt, NOW).unwrap_err();
    assert!(
        matches!(err, FlatError::MalformedPath { .. }),
        "got {err:?}"
    );
}

/// master04 §Open Value-Sets and the `|other` Suffix ("`|other` is mutually
/// exclusive with `|code`, `|value` and `|terminology` … servers MUST reject
/// combinations") — asserted both as a hard `composition_from_flat` error and
/// as a `validate_flat_other` diagnostic.
#[test]
fn rejects_other_combined_with_code() {
    let wt = coded_leaf_wt(true);
    let doc = flat_of(r#"{ "test/coded|other": "free", "test/coded|code": "at0001" }"#);
    let err = composition_from_flat(&doc, &wt, NOW).unwrap_err();
    assert!(
        matches!(err, FlatError::OtherSuffixConflict(_)),
        "got {err:?}"
    );

    let msgs = validate_flat_other(&doc, &wt);
    assert!(
        msgs.iter().any(|m| m.kind == ValidationKind::CodedValue),
        "validate_flat_other should flag the |other/|code conflict, got {msgs:?}"
    );
}

/// master04 §Open Value-Sets and the `|other` Suffix ("`|other` MUST be
/// rejected when the constraint is closed (`listOpen: false`)") — hard error
/// plus diagnostic.
#[test]
fn rejects_other_on_closed_value_set() {
    let wt = coded_leaf_wt(false);
    let doc = flat_of(r#"{ "test/coded|other": "free-text value" }"#);
    let err = composition_from_flat(&doc, &wt, NOW).unwrap_err();
    assert!(
        matches!(err, FlatError::OtherOnClosedValueSet(_)),
        "got {err:?}"
    );

    let msgs = validate_flat_other(&doc, &wt);
    assert!(
        msgs.iter().any(|m| m.kind == ValidationKind::CodedValue),
        "validate_flat_other should flag the closed value-set, got {msgs:?}"
    );
}

/// master04 §Validation ("Mandatory context fields (language, territory) are
/// present") — the submission seam refuses a buildable FLAT document missing
/// `ctx/territory`, while the plain projection stays permissive for
/// fragments and round-trips.
#[test]
fn submission_rejects_missing_mandatory_context() {
    let wt = coded_leaf_wt(true);
    let doc = flat_of(r#"{ "ctx/language": "en", "test/coded|code": "at0001" }"#);
    let err = submitted_composition_from_flat(&doc, &wt, NOW).unwrap_err();
    assert!(
        matches!(err, FlatError::MissingContext("territory")),
        "got {err:?}"
    );
    assert!(composition_from_flat(&doc, &wt, NOW).is_ok());
}

/// master04 §Validation ("Terminology bindings are valid") — a `|code`
/// outside a CLOSED value set with no `|value` cannot resolve to a coded
/// value: the submission seam names the cause instead of emitting a
/// value-less `DV_CODED_TEXT` for the strict reader to refuse downstream.
#[test]
fn submission_rejects_code_outside_closed_value_set() {
    let doc =
        flat_of(r#"{ "ctx/language": "en", "ctx/territory": "NL", "test/coded|code": "at9999" }"#);
    let err = submitted_composition_from_flat(&doc, &coded_leaf_wt(false), NOW).unwrap_err();
    assert!(
        matches!(err, FlatError::CodeNotInValueSet { .. }),
        "got {err:?}"
    );
    // An OPEN list admits the unlisted code (master04 §Open Value-Sets).
    assert!(submitted_composition_from_flat(&doc, &coded_leaf_wt(true), NOW).is_ok());
}

// ═══════════════════════════════════════════════════════════════════════════
// master04 — mandatory context (public seam: validate_context)
// ═══════════════════════════════════════════════════════════════════════════

/// master04 §Validation / §Context ("Mandatory: language, territory") — both
/// present via `ctx/` produces no message.
#[test]
fn context_present_via_ctx_passes() {
    let doc = parse_flat(&flat_of(
        r#"{ "ctx/language": "de", "ctx/territory": "US" }"#,
    ))
    .unwrap();
    assert!(validate_context(&doc).is_empty());
}

/// master04 §Validation — both mandatory fields absent yields two `Required`
/// messages.
#[test]
fn context_missing_both_flagged() {
    let doc = parse_flat(&flat_of(r#"{ "ctx/composer_name": "Dr. Smith" }"#)).unwrap();
    let msgs = validate_context(&doc);
    assert_eq!(msgs.len(), 2, "got {msgs:?}");
    assert!(msgs.iter().all(|m| m.kind == ValidationKind::Required));
}

/// master04 §Validation — a single missing field is reported on its own key.
#[test]
fn context_missing_territory_flagged() {
    let doc = parse_flat(&flat_of(r#"{ "ctx/language": "en" }"#)).unwrap();
    let msgs = validate_context(&doc);
    assert_eq!(msgs.len(), 1, "got {msgs:?}");
    assert_eq!(msgs[0].path, "ctx/territory");
    assert_eq!(msgs[0].kind, ValidationKind::Required);
}

/// master05 §COMPOSITION permits the root-level path spelling of the mandatory
/// fields (`<root>/language|code`, `<root>/territory|code`); those satisfy the
/// mandatory-context rule too (master04 §Validation).
#[test]
fn context_present_via_root_path_spelling_passes() {
    let doc = parse_flat(&flat_of(
        r#"{
          "conformance-ehrbase.de.v0/language|code": "en",
          "conformance-ehrbase.de.v0/territory|code": "US"
        }"#,
    ))
    .unwrap();
    assert!(validate_context(&doc).is_empty());
}
// ═══════════════════════════════════════════════════════════════════════════
// master05 — RM mappings: structural (ENTRY / structure) types
// ═══════════════════════════════════════════════════════════════════════════

/// master05 §COMPOSITION — minimal and full example blocks.
#[test]
fn master05_composition() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/language|code": "en",
          "conformance-ehrbase.de.v0/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/territory|code": "US",
          "conformance-ehrbase.de.v0/territory|terminology": "ISO_3166-1",
          "conformance-ehrbase.de.v0/category|code": "433",
          "conformance-ehrbase.de.v0/category|value": "event",
          "conformance-ehrbase.de.v0/category|terminology": "openehr",
          "conformance-ehrbase.de.v0/context/start_time": "2021-12-21T14:19:31.649613+01:00",
          "conformance-ehrbase.de.v0/context/setting|code": "238",
          "conformance-ehrbase.de.v0/context/setting|value": "other care",
          "conformance-ehrbase.de.v0/context/setting|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/time": "2021-12-21T16:02:58.0094262+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/encoding|terminology": "IANA_character-sets",
          "conformance-ehrbase.de.v0/composer|name": "Silvia Blake"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/_uid": "6e3a9506-b81c-4d74-a37f-1464fb7106b2::ehrbase.org::1",
          "conformance-ehrbase.de.v0/language|code": "en",
          "conformance-ehrbase.de.v0/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/territory|code": "US",
          "conformance-ehrbase.de.v0/territory|terminology": "ISO_3166-1",
          "conformance-ehrbase.de.v0/category|code": "433",
          "conformance-ehrbase.de.v0/category|value": "event",
          "conformance-ehrbase.de.v0/category|terminology": "openehr",
          "conformance-ehrbase.de.v0/context/_health_care_facility|id": "9091",
          "conformance-ehrbase.de.v0/context/_health_care_facility|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/_health_care_facility|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/_health_care_facility|name": "Hospital",
          "conformance-ehrbase.de.v0/context/_participation:0|function": "requester",
          "conformance-ehrbase.de.v0/context/_participation:0|mode": "face-to-face communication",
          "conformance-ehrbase.de.v0/context/_participation:0|name": "Dr. Marcus Johnson",
          "conformance-ehrbase.de.v0/context/_participation:0|id": "199",
          "conformance-ehrbase.de.v0/context/_participation:0|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/_participation:0|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/start_time": "2021-12-21T14:19:31.649613+01:00",
          "conformance-ehrbase.de.v0/context/_end_time": "2021-12-21T15:19:31.649613+01:00",
          "conformance-ehrbase.de.v0/context/_location": "microbiology lab 2",
          "conformance-ehrbase.de.v0/context/setting|code": "238",
          "conformance-ehrbase.de.v0/context/setting|value": "other care",
          "conformance-ehrbase.de.v0/context/setting|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/time": "2021-12-21T16:02:58.0094262+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/encoding|terminology": "IANA_character-sets",
          "conformance-ehrbase.de.v0/_link:0|type": "problem",
          "conformance-ehrbase.de.v0/_link:0|meaning": "problem related note",
          "conformance-ehrbase.de.v0/_link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e",
          "conformance-ehrbase.de.v0/composer|name": "Silvia Blake",
          "conformance-ehrbase.de.v0/composer|id": "1234-5678",
          "conformance-ehrbase.de.v0/composer|id_scheme": "UUID",
          "conformance-ehrbase.de.v0/composer|id_namespace": "EHR.NETWORK"
        }"#,
    );
}

/// master05 §ADMIN_ENTRY — minimal and full example blocks.
#[test]
fn master05_admin_entry() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/dv_text": "DV_TEXT 56",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/encoding|terminology": "IANA_character-sets"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/dv_text": "DV_TEXT 56",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/encoding|terminology": "IANA_character-sets",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:0|function": "requester",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:0|mode": "face-to-face communication",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:0|name": "Dr. Marcus Johnson",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:0|id": "199",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:0|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:0|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:1|function": "performer",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:1|mode": "not specified",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:1|name": "Lara Markham",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:1|id": "198",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:1|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_other_participation:1|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_work_flow_id|type": "WORKFLOW",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_work_flow_id|namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_work_flow_id|id": "335645",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_work_flow_id|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_uid":"9fcc1c70-9349-444d-b9cb-8fa817697f5e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_link:0|type": "problem",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_link:0|meaning": "problem related note",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_feeder_audit/originating_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_admin_entry/_feeder_audit/originating_system_audit|system_id": "orig"
        }"#,
    );
}

/// master05 §INSTRUCTION — minimal and full example blocks.
#[test]
fn master05_instruction() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/dv_text": "DV_TEXT 45",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/timing": "R4/2022-01-31T10:00:00+01:00/P3M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/timing|formalism": "timing",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/dv_text": "DV_TEXT 91",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/expiry_time": "2022-01-31T10:33:28.724259+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/narrative": "Human readable instruction narrative",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/encoding|terminology": "IANA_character-sets"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/dv_text": "DV_TEXT 45",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/timing": "R4/2022-01-31T10:00:00+01:00/P3M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/timing|formalism": "timing",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/action_archetype_id": "/openEHR-EHR-CLUSTER.conformance_action.v0/",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/dv_text": "DV_TEXT 91",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/expiry_time": "2022-01-31T10:33:28.724259+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/narrative": "Human readable instruction narrative",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_wf_definition|value": "wf_definition",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_wf_definition|formalism": "formalism",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/encoding|terminology": "IANA_character-sets",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_other_participation:0|function": "requester",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_other_participation:0|mode": "face-to-face communication",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_other_participation:0|name": "Dr. Marcus Johnson",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_other_participation:0|id": "199",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_other_participation:0|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_other_participation:0|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_guideline_id|type": "GUIDELINE",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_guideline_id|namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_guideline_id|id": "3445",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_guideline_id|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_work_flow_id|type": "WORKFLOW",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_work_flow_id|namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_work_flow_id|id": "335645",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_work_flow_id|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_uid":"9fcc1c70-9349-444d-b9cb-8fa817697f5e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_link:0|type": "problem",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_link:0|meaning": "problem related note",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_feeder_audit/originating_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_feeder_audit/originating_system_audit|system_id": "orig"
        }"#,
    );
}

/// master05 §ACTION — minimal and full example blocks.
#[test]
fn master05_action() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/dv_text": "dv_text in description",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/dv_text2": "dv_text in protocol",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|code": "532",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|value": "completed",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/time": "2022-01-31T10:33:28.72414+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/encoding|terminology": "IANA_character-sets"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/dv_text": "dv_text in description",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/dv_text2": "dv_text in protocol",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|code": "532",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|value": "completed",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_instruction_details|path": "/content[openEHR-EHR-SECTION.conformance_section.v0]/items[openEHR-EHR-INSTRUCTION.conformance_instruction.v0]",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_instruction_details|composition_uid": "4cdc3017-d8c5-4cd3-9900-f3bb7171d006",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_instruction_details|activity_id": "activities[at0001]",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/time": "2022-01-31T10:33:28.72414+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/encoding|terminology": "IANA_character-sets",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_other_participation:0|function": "requester",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_other_participation:0|mode": "face-to-face communication",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_other_participation:0|name": "Dr. Marcus Johnson",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_other_participation:0|id": "199",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_other_participation:0|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_other_participation:0|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_uid":"9fcc1c70-9349-444d-b9cb-8fa817697f5e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_link:0|type": "problem",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_link:0|meaning": "problem related note",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_feeder_audit/originating_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_feeder_audit/originating_system_audit|system_id": "orig"
        }"#,
    );
}

/// master05 §EVALUATION — minimal and full example blocks.
#[test]
fn master05_evaluation() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/dv_text": "dv_text in data",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/dv_text2": "dv_text in protocol",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/encoding|terminology": "IANA_character-sets"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/dv_text": "dv_text in data",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/dv_text2": "dv_text in protocol",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/encoding|terminology": "IANA_character-sets",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_other_participation:0|function": "requester",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_other_participation:0|mode": "face-to-face communication",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_other_participation:0|name": "Dr. Marcus Johnson",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_other_participation:0|id": "199",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_other_participation:0|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_other_participation:0|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_guideline_id|type": "GUIDELINE",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_guideline_id|namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_guideline_id|id": "3445",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_guideline_id|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_work_flow_id|type": "WORKFLOW",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_work_flow_id|namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_work_flow_id|id": "335645",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_work_flow_id|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_uid":"9fcc1c70-9349-444d-b9cb-8fa817697f5e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_link:0|type": "problem",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_link:0|meaning": "problem related note",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_feeder_audit/originating_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_evaluation/_feeder_audit/originating_system_audit|system_id": "orig"
        }"#,
    );
}

/// master05 §OBSERVATION — minimal and full example blocks (incl. `subject`
/// PARTY_RELATED, `_provider`, `history_origin`, feeder audit).
#[test]
fn master05_observation() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text_state": "DV_TEXT in State",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/dv_text": "dv_text in protocol",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/time": "2021-12-21T16:02:58.0094262+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/encoding|terminology": "IANA_character-sets"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text_state": "DV_TEXT in State",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/dv_text": "dv_text in protocol",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/time": "2021-12-21T16:02:58.0094262+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/history_origin": "2021-12-20T16:02:58.0094262+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/encoding|terminology": "IANA_character-sets",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject|id": "1234-5678",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject|id_scheme": "UUID",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject|id_namespace": "EHR.NETWORK",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject|name": "Silvia Blake",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject/_identifier:0|id": "122",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject/_identifier:0|issuer": "issuer",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject/_identifier:0|assigner": "assigner",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject/_identifier:0|type": "type",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject/relationship|code": "10",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/subject/relationship|value": "mother",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_provider|name": "Dr. Marcus Johnson",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_other_participation:0|function": "requester",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_other_participation:0|mode": "face-to-face communication",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_other_participation:0|name": "Dr. Marcus Johnson",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_other_participation:0|id": "199",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_other_participation:0|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_other_participation:0|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_uid":"9fcc1c70-9349-444d-b9cb-8fa817697f5e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_link:0|type": "problem",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_link:0|meaning": "problem related note",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/originating_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/originating_system_audit|system_id": "orig",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/original_content": "Hello world!",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/original_content|formalism": "text/plain"
        }"#,
    );
}

/// master05 §ELEMENT — the three example blocks (value; uid/link/feeder audit;
/// null_flavour / null_reason).
#[test]
fn master05_element() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_uid":"9fcc1c70-9349-444d-b9cb-8fa817697f5e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_link:0|type": "problem",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_link:0|meaning": "problem related note",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_feeder_audit/originating_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_feeder_audit/originating_system_audit|system_id": "orig"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_null_flavour|code": "253",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_null_flavour|value": "unknown",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_null_flavour|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_null_reason": "sample reason"
        }"#,
    );
}

/// master05 §CLUSTER — minimal and full example blocks.
#[test]
fn master05_cluster() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/conformance_cluster/labresult/text_value": "labresult 4"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/conformance_cluster/labresult/text_value": "labresult 4",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/conformance_cluster/_feeder_audit/originating_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/conformance_cluster/_feeder_audit/originating_system_audit|system_id": "orig",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/conformance_cluster/_uid":"9fcc1c70-9349-444d-b9cb-8fa817697f5e",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/conformance_cluster/_link:0|type": "problem",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/conformance_cluster/_link:0|meaning": "problem related note",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/conformance_cluster/_link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e"
        }"#,
    );
}

/// master05 §LINK — the example block.
#[test]
fn master05_link() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/_link:0|type": "problem",
          "conformance-ehrbase.de.v0/_link:0|meaning": "problem related note",
          "conformance-ehrbase.de.v0/_link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e"
        }"#,
    );
}

/// master05 §FEEDER_AUDIT — minimal and full example blocks.
#[test]
fn master05_feeder_audit() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit|system_id": "orig"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit|system_id": "orig",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/location|id": "12342341",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/location|id_namespace": "uk.org.nmc",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/location|id_scheme": "NMC",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/location|name": "Org 1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/subject|id": "456",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/subject|id_namespace": "uk.org.nmc",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/subject|id_scheme": "NMC",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/subject|name": "Per 1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/provider|id": "456",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/provider|id_namespace": "uk.org.nmc",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/provider|id_scheme": "NMC",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit/provider|name": "Per 1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_audit|time": "2021-12-21T16:02:58.0094262+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_item_id:0|id": "id1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_item_id:0|issuer": "issuer1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_item_id:0|assigner": "assigner1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_item_id:0|type": "PERSON",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_item_id:1|id": "id2",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_item_id:1|issuer": "issuer2",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_item_id:1|assigner": "assigner2",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/originating_system_item_id:1|type": "PERSON",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/original_content": "Hello world!",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/original_content|formalism": "text/plain",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_item_id:0|id": "id1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_item_id:0|issuer": "issuer1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_item_id:0|assigner": "assigner1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_item_id:0|type": "PERSON",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_item_id:1|id": "id2",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_item_id:1|issuer": "issuer2",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_item_id:1|assigner": "assigner2",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_item_id:1|type": "PERSON",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit|system_id": "orig",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/location|id": "12342341",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/location|id_namespace": "uk.org.nmc",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/location|id_scheme": "NMC",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/location|name": "Org 1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/subject|id": "456",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/subject|id_namespace": "uk.org.nmc",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/subject|id_scheme": "NMC",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/subject|name": "Per 1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/provider|id": "456",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/provider|id_namespace": "uk.org.nmc",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/provider|id_scheme": "NMC",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit/provider|name": "Per 1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/_feeder_audit/feeder_system_audit|time": "2021-12-21T16:02:58.0094262+01:00"
        }"#,
    );
}

/// master05 §FEEDER_AUDIT_DETAILS — minimal and full example blocks.
#[test]
fn master05_feeder_audit_details() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit|system_id": "orig"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/subject|id": "1234-5678",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/subject|id_scheme": "UUID",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/subject|id_namespace": "EHR.NETWORK",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/subject|name": "Silvia Blake",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/subject/_identifier:0|id": "122",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/subject/_identifier:0|issuer": "issuer",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/subject/_identifier:0|assigner": "assigner",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/subject/_identifier:0|type": "type",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/provider|id": "1234-5678",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/provider|id_scheme": "UUID",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/provider|id_namespace": "EHR.NETWORK",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/provider|name": "Silvia Blake",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/provider/_identifier:0|id": "122",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/provider/_identifier:0|issuer": "issuer",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/provider/_identifier:0|assigner": "assigner",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/provider/_identifier:0|type": "type",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/location|id": "12342341",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/location|id_scheme": "NMC",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/location|id_namespace": "uk.org.nmc",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit/location|name": "Org 1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit|system_id": "orig",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit|version_id": "final",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/_feeder_audit/feeder_system_audit|time": "2021-12-21T16:02:58.0094262+01:00"
        }"#,
    );
}

/// master05 §ACTIVITY — minimal and full example blocks.
#[test]
fn master05_activity() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/dv_text": "DV_TEXT 45",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/timing": "R4/2022-01-31T10:00:00+01:00/P3M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/timing|formalism": "timing"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/dv_text": "DV_TEXT 45",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/timing": "R4/2022-01-31T10:00:00+01:00/P3M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/timing|formalism": "timing",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/current_activity/action_archetype_id": "/openEHR-EHR-CLUSTER.conformance_action.v0/"
        }"#,
    );
}

/// master05 §ISM_TRANSITION — minimal and full example blocks.
#[test]
fn master05_ism_transition() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|code": "532",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|value": "completed",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|terminology": "openehr"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|code": "532",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|value": "completed",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/current_state|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/transition|code": "548",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/transition|value": "finish",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/transition|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/careflow_step|code": "at0006",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/careflow_step|value": "transition",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/careflow_step|terminology": "local",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/ism_transition/_reason:0": "reason 1"
        }"#,
    );
}

/// master05 §INSTRUCTION_DETAILS — the example block.
#[test]
fn master05_instruction_details() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_instruction_details|path": "/content[openEHR-EHR-SECTION.conformance_section.v0]/items[openEHR-EHR-INSTRUCTION.conformance_instruction.v0]",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_instruction_details|composition_uid": "4cdc3017-d8c5-4cd3-9900-f3bb7171d006",
          "conformance-ehrbase.de.v0/conformance_section/conformance_action/_instruction_details|activity_id": "activities[at0001]"
        }"#,
    );
}

/// master05 §EVENT_CONTEXT — minimal and full example blocks.
#[test]
fn master05_event_context() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/context/start_time": "2021-12-21T14:19:31.649613+01:00",
          "conformance-ehrbase.de.v0/context/setting|code": "238",
          "conformance-ehrbase.de.v0/context/setting|value": "other care",
          "conformance-ehrbase.de.v0/context/setting|terminology": "openehr"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/context/_health_care_facility|id": "9091",
          "conformance-ehrbase.de.v0/context/_health_care_facility|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/_health_care_facility|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/_health_care_facility|name": "Hospital",
          "conformance-ehrbase.de.v0/context/_participation:0|function": "requester",
          "conformance-ehrbase.de.v0/context/_participation:0|mode": "face-to-face communication",
          "conformance-ehrbase.de.v0/context/_participation:0|name": "Dr. Marcus Johnson",
          "conformance-ehrbase.de.v0/context/_participation:0|id": "199",
          "conformance-ehrbase.de.v0/context/_participation:0|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/_participation:0|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/start_time": "2021-12-21T14:19:31.649613+01:00",
          "conformance-ehrbase.de.v0/context/_end_time": "2021-12-21T15:19:31.649613+01:00",
          "conformance-ehrbase.de.v0/context/_location": "Ward A3, Room 12",
          "conformance-ehrbase.de.v0/context/setting|code": "238",
          "conformance-ehrbase.de.v0/context/setting|value": "other care",
          "conformance-ehrbase.de.v0/context/setting|terminology": "openehr"
        }"#,
    );
}

/// master05 §PARTICIPATION — the example block (performer inlined) and the
/// PARTY_RELATED-performer variant (`/relationship`).
#[test]
fn master05_participation() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/context/_participation:0|function": "requester",
          "conformance-ehrbase.de.v0/context/_participation:0|mode": "face-to-face communication",
          "conformance-ehrbase.de.v0/context/_participation:0|name": "Dr. Marcus Johnson",
          "conformance-ehrbase.de.v0/context/_participation:0|id": "199",
          "conformance-ehrbase.de.v0/context/_participation:0|id_scheme": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/_participation:0|id_namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/context/_participation:0|identifiers_id:0": "122",
          "conformance-ehrbase.de.v0/context/_participation:0|identifiers_issuer:0": "issuer",
          "conformance-ehrbase.de.v0/context/_participation:0|identifiers_assigner:0": "assigner",
          "conformance-ehrbase.de.v0/context/_participation:0|identifiers_type:0": "type"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/context/_participation:0|function": "next of kin",
          "conformance-ehrbase.de.v0/context/_participation:0|name": "Susan Doe",
          "conformance-ehrbase.de.v0/context/_participation:0/relationship|code": "10",
          "conformance-ehrbase.de.v0/context/_participation:0/relationship|value": "mother",
          "conformance-ehrbase.de.v0/context/_participation:0/relationship|terminology": "openehr"
        }"#,
    );
}

/// master05 §OBJECT_REF — the example block.
#[test]
fn master05_object_ref() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_guideline_id|type": "GUIDELINE",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_guideline_id|namespace": "HOSPITAL-NS",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_guideline_id|id": "3445",
          "conformance-ehrbase.de.v0/conformance_section/conformance_instruction/_guideline_id|id_scheme": "HOSPITAL-NS"
        }"#,
    );
}

/// master05 §INTERVAL_EVENT — minimal and full example blocks.
#[test]
fn master05_interval_event() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/width": "P30D",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/math_function|code": "146",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/math_function|value": "mean",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/math_function|terminology": "openehr"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0|sample_count": 5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/width": "P30D",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/math_function|code": "146",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/math_function|value": "mean",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/math_function|terminology": "openehr"
        }"#,
    );
}

/// master05 §POINT_EVENT — the example block.
#[test]
fn master05_point_event() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text": "DV_TEXT value",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/time": "2021-12-21T16:02:58.0094262+01:00"
        }"#,
    );
}

/// master05 §PARTY_SELF — the example block (`composer_self` + external ref).
#[test]
fn master05_party_self() {
    assert_flat_vector(
        r#"{
          "ctx/composer_self": true,
          "conformance-ehrbase.de.v0/composer|id": "1234-5678",
          "conformance-ehrbase.de.v0/composer|id_scheme": "UUID",
          "conformance-ehrbase.de.v0/composer|id_namespace": "EHR.NETWORK"
        }"#,
    );
}

/// master05 §PARTY_IDENTIFIED — minimal and full example blocks.
#[test]
fn master05_party_identified() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/composer|name": "Silvia Blake"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/composer|name": "Silvia Blake",
          "conformance-ehrbase.de.v0/composer|id": "1234-5678",
          "conformance-ehrbase.de.v0/composer|id_scheme": "UUID",
          "conformance-ehrbase.de.v0/composer|id_namespace": "EHR.NETWORK",
          "conformance-ehrbase.de.v0/composer/_identifier:0|id": "122",
          "conformance-ehrbase.de.v0/composer/_identifier:0|issuer": "issuer",
          "conformance-ehrbase.de.v0/composer/_identifier:0|assigner": "assigner",
          "conformance-ehrbase.de.v0/composer/_identifier:0|type": "type"
        }"#,
    );
}

/// master05 §PARTY_RELATED — minimal and full example blocks.
#[test]
fn master05_party_related() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/composer|name": "Silvia Blake",
          "conformance-ehrbase.de.v0/composer/relationship|code": "10",
          "conformance-ehrbase.de.v0/composer/relationship|value": "mother",
          "conformance-ehrbase.de.v0/composer/relationship|terminology": "openehr"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/composer|name": "Silvia Blake",
          "conformance-ehrbase.de.v0/composer|id": "1234-5678",
          "conformance-ehrbase.de.v0/composer|id_scheme": "UUID",
          "conformance-ehrbase.de.v0/composer|id_namespace": "EHR.NETWORK",
          "conformance-ehrbase.de.v0/composer/relationship|code": "10",
          "conformance-ehrbase.de.v0/composer/relationship|value": "mother",
          "conformance-ehrbase.de.v0/composer/relationship|terminology": "openehr",
          "conformance-ehrbase.de.v0/composer/_identifier:0|id": "122",
          "conformance-ehrbase.de.v0/composer/_identifier:0|issuer": "issuer",
          "conformance-ehrbase.de.v0/composer/_identifier:0|assigner": "assigner",
          "conformance-ehrbase.de.v0/composer/_identifier:0|type": "type"
        }"#,
    );
}
// ═══════════════════════════════════════════════════════════════════════════
// master05 — RM mappings: data value (DV_*) types
// ═══════════════════════════════════════════════════════════════════════════

/// master05 §DV_TEXT — minimal and full example blocks (mappings, language,
/// encoding).
#[test]
fn master05_dv_text() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text": "DV_TEXT value"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text": "DV_TEXT value",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text|formatting": "plain",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_language|preferred_term": "English",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_encoding|terminology": "IANA_character-sets",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0|match": "=",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/target|terminology": "SNOMED-CT",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/target|code": "21794005",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/purpose|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/purpose|code": "671",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/purpose|value": "research study"
        }"#,
    );
}

/// master05 §CODE_PHRASE — minimal and full example blocks.
#[test]
fn master05_code_phrase() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_language|terminology": "ISO_639-1"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_language|preferred_term": "English"
        }"#,
    );
}

/// master05 §TERM_MAPPING — minimal and full example blocks.
#[test]
fn master05_term_mapping() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0|match": "=",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/target|terminology": "SNOMED-CT",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/target|code": "21794005"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0|match": "=",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/target|terminology": "SNOMED-CT",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/target|code": "21794005",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/purpose|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/purpose|code": "671",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_text/_mapping:0/purpose|value": "research study"
        }"#,
    );
}

/// master05 §DV_CODED_TEXT — minimal, full, and the `|other` free-text block
/// (the `|other` suffix round-trips as a pure transform; its *semantics* are
/// asserted by `rejects_other_*`).
#[test]
fn master05_dv_coded_text() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text|value": "term1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text|code": "at0006",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text|terminology": "local"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text|value": "term1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text|code": "at0006",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text|terminology": "local",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text|preferred_term": "Term One",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text|formatting": "plain",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_language|preferred_term": "English",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_encoding|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_encoding|terminology": "IANA_character-sets",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_mapping:0|match": "=",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_mapping:0/target|terminology": "SNOMED-CT",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_mapping:0/target|code": "21794005",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_mapping:0/purpose|terminology": "openehr",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_mapping:0/purpose|code": "671",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text/_mapping:0/purpose|value": "research study"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_coded_text|other": "free-text value not in the coded list"
        }"#,
    );
}

/// master05 §DV_ORDINAL — minimal and full (normal/other reference ranges).
#[test]
fn master05_dv_ordinal() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal|code": "at0015",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal|value": "value1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal|ordinal": 1
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal|code": "at0015",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal|value": "value1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal|ordinal": 1,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_normal_range/lower|code": "at0015",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_normal_range/lower|value": "value1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_normal_range/lower|ordinal": 1,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_normal_range/upper|code": "at0015",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_normal_range/upper|value": "value1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_normal_range/upper|ordinal": 1,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_other_reference_ranges:0/lower|code": "at0016",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_other_reference_ranges:0/lower|value": "value2",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_other_reference_ranges:0/lower|ordinal": 2,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_other_reference_ranges:0|upper_unbounded": true,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_other_reference_ranges:0|upper_included": false,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ordinal/_other_reference_ranges:0/meaning": "high"
        }"#,
    );
}

/// master05 §DV_BOOLEAN — the example block.
#[test]
fn master05_dv_boolean() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_boolean": true
        }"#,
    );
}

/// master05 §DV_URI — the example block.
#[test]
fn master05_dv_uri() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_uri": "https://www.google.com/"
        }"#,
    );
}

/// master05 §DV_EHR_URI — the example block.
#[test]
fn master05_dv_ehr_uri() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_ehr_uri": "ehr://766b3873-0762-4921-91e2-838c8546d47f"
        }"#,
    );
}

/// master05 §DV_IDENTIFIER — minimal and full example blocks.
#[test]
fn master05_dv_identifier() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_identifier|id": "A123"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_identifier|id": "A123",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_identifier|issuer": "Issuer",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_identifier|assigner": "Assigner",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_identifier|type": "Prescription"
        }"#,
    );
}

/// master05 §DV_QUANTITY — minimal and full (statuses, accuracy, ranges).
#[test]
fn master05_dv_quantity() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude": 65.9,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|magnitude_status": "~",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|normal_status": "N",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|accuracy": 50.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|accuracy_is_percent": true,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|precision": 1,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|units_system": "units_system",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity|units_display_name": "units_display_name",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_normal_range/lower|magnitude": 20.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_normal_range/lower|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_normal_range/upper|magnitude": 66.6,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_normal_range/upper|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/lower|magnitude": 70.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/lower|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/upper|magnitude": 77.6,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/upper|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/meaning|value": "very high",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/meaning|code": "260360000",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/meaning|terminology": "SNOMED-CT"
        }"#,
    );
}

/// master05 §DV_PROPORTION — minimal and full (calculated magnitude, ranges).
#[test]
fn master05_dv_proportion() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|numerator": 20.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|denominator": 12.4,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|type": 0
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|numerator": 20.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|denominator": 12.4,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|type": 0,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion": 1.6532258064516128,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|magnitude_status": "~",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|normal_status": "N",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|accuracy": 50.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|accuracy_is_percent": true,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion|precision": 1,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_normal_range/lower|numerator": 20.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_normal_range/lower|denominator": 12.4,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_normal_range/lower|type": 0,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_normal_range/lower": 1.6532258064516128,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_normal_range/upper|numerator": 25.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_normal_range/upper|denominator": 12.4,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_normal_range/upper|type": 0,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_normal_range/upper": 2.0564516129032255,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_other_reference_ranges:0/lower|numerator": 20.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_other_reference_ranges:0/lower|denominator": 18.4,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_other_reference_ranges:0/lower|type": 0,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_other_reference_ranges:0/lower": 1.1141304347826089,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_other_reference_ranges:0/upper|numerator": 25.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_other_reference_ranges:0/upper|denominator": 12.4,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_other_reference_ranges:0/upper|type": 0,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_other_reference_ranges:0/upper": 2.0564516129032255,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_proportion/_other_reference_ranges:0/meaning": "high"
        }"#,
    );
}

/// master05 §DV_COUNT — minimal and full example blocks.
#[test]
fn master05_dv_count() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count": 7
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count": 7,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count|magnitude_status": "~",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count|normal_status": "N",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count|accuracy": 50.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count|accuracy_is_percent": true,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count/_normal_range/lower": 1,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count/_normal_range/upper": 8,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count/_other_reference_ranges:0/lower": 8,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count/_other_reference_ranges:0/upper": 10,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_count/_other_reference_ranges:0/meaning": "high"
        }"#,
    );
}

/// master05 §DV_DATE — minimal and full example blocks.
#[test]
fn master05_dv_date() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date": "2022-01-12"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date": "2022-01-12",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date|magnitude_status": "~",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date|normal_status": "N",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date/_accuracy": "P2D",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date/_normal_range/lower": "2022-01-12",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date/_normal_range/upper": "2022-02-12",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date/_other_reference_ranges:0/lower": "2022-02-12",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date/_other_reference_ranges:0/upper": "2022-03-12",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date/_other_reference_ranges:0/meaning": "high"
        }"#,
    );
}

/// master05 §DV_DATE_TIME — minimal and full example blocks.
#[test]
fn master05_dv_date_time() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time": "2022-01-12T13:22:34.000868+01:00"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time": "2022-01-12T13:22:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time|magnitude_status": "~",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time|normal_status": "N",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time/_accuracy": "P2DT9H52M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time/_normal_range/lower": "2022-01-12T13:22:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time/_normal_range/upper": "2022-02-12T13:22:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time/_other_reference_ranges:0/lower": "2022-02-12T13:22:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time/_other_reference_ranges:0/upper": "2022-03-12T13:22:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_date_time/_other_reference_ranges:0/meaning": "high"
        }"#,
    );
}

/// master05 §DV_TIME — minimal and full example blocks.
#[test]
fn master05_dv_time() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time": "13:22:34.000868+01:00"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time": "13:22:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time|magnitude_status": "~",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time|normal_status": "N",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time/_accuracy": "PT9H52M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time/_normal_range/lower": "13:22:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time/_normal_range/upper": "14:22:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time/_other_reference_ranges:0/lower": "14:10:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time/_other_reference_ranges:0/upper": "15:22:34.000868+01:00",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_time/_other_reference_ranges:0/meaning": "high"
        }"#,
    );
}

/// master05 §DV_DURATION — minimal and full example blocks.
#[test]
fn master05_dv_duration() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration": "P2DT11H33M"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration": "P2DT11H33M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration|magnitude_status": "~",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration|normal_status": "N",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration|accuracy": 50.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration|accuracy_is_percent": true,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration/_normal_range/lower": "P2DT11H33M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration/_normal_range/upper": "P2DT12H33M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration/_other_reference_ranges:0/lower": "P2DT11H33M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration/_other_reference_ranges:0/upper": "P2DT15H33M",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_duration/_other_reference_ranges:0/meaning": "high"
        }"#,
    );
}

/// master05 §REFERENCE_RANGE — minimal and full example blocks (bounded /
/// unbounded endpoints, coded meaning).
#[test]
fn master05_reference_range() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/lower|magnitude": 70.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/lower|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/upper|magnitude": 77.6,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/upper|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/meaning|value": "high"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/lower|magnitude": 70.5,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/lower|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0|upper_unbounded": true,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0|upper_included": false,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/meaning|value": "very high",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/meaning|code": "260360000",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:0/meaning|terminology": "SNOMED-CT",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:1|lower_unbounded": true,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:1|lower_included": false,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:1/upper|magnitude": 77.6,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:1/upper|unit": "unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:1/meaning|value": "very high",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:1/meaning|code": "260360000",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_quantity/_other_reference_ranges:1/meaning|terminology": "SNOMED-CT"
        }"#,
    );
}

/// master05 §DV_PARSABLE — minimal and full example blocks.
#[test]
fn master05_dv_parsable() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_parsable": "Formal instructions on carrying out the procedure...",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_parsable|formalism": "GLIF 1.0"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_parsable": "Formal instructions on carrying out the procedure...",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_parsable|formalism": "GLIF 1.0",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_parsable/_language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_parsable/_language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_parsable/_charset|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_parsable/_charset|terminology": "IANA_character-sets"
        }"#,
    );
}

/// master05 §DV_MULTIMEDIA — minimal and full (thumbnail, language, charset).
#[test]
fn master05_dv_multimedia() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia": "http://med.tube.com/sample",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia|mediatype": "video/H261",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia|size": 504903212
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia": "http://med.tube.com/sample",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia|mediatype": "video/H261",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia|size": 504903212,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia|compression_algorithm": "zlib",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia|alternatetext": "alternate text",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia|integrity_check": "b90360558e5420cef47015b1afbd70a156f940afa470b0515f95eacc2edcef6a",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia|integrity_check_algorithm": "SHA-256",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia/_thumbnail|data": "Z2hnZ2pnamdnag==",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia/_thumbnail|mediatype": "image/png",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia/_thumbnail|size": 504,
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia/_language|code": "en",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia/_language|terminology": "ISO_639-1",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia/_charset|code": "UTF-8",
          "conformance-ehrbase.de.v0/conformance_section/conformance_observation/any_event:0/dv_multimedia/_charset|terminology": "IANA_character-sets"
        }"#,
    );
}

/// master05 §DV_INTERVAL — minimal and full example blocks.
#[test]
fn master05_dv_interval() {
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_interval/any_event:0/interval_dv_quantity/lower|magnitude": 72.83,
          "conformance-ehrbase.de.v0/conformance_section/conformance_interval/any_event:0/interval_dv_quantity/lower|unit": "Unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_interval/any_event:0/interval_dv_quantity/upper|magnitude": 80.83,
          "conformance-ehrbase.de.v0/conformance_section/conformance_interval/any_event:0/interval_dv_quantity/upper|unit": "Unit"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "conformance-ehrbase.de.v0/conformance_section/conformance_interval/any_event:0/interval_dv_quantity/lower|magnitude": 72.83,
          "conformance-ehrbase.de.v0/conformance_section/conformance_interval/any_event:0/interval_dv_quantity/lower|unit": "Unit",
          "conformance-ehrbase.de.v0/conformance_section/conformance_interval/any_event:0/interval_dv_quantity|lower_included": false,
          "conformance-ehrbase.de.v0/conformance_section/conformance_interval/any_event:0/interval_dv_quantity|upper_unbounded": true,
          "conformance-ehrbase.de.v0/conformance_section/conformance_interval/any_event:0/interval_dv_quantity|upper_included": false
        }"#,
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// master06 — context information
// ═══════════════════════════════════════════════════════════════════════════

/// master06 (intro) — the full `ctx/` example (composer, ids, workflow id,
/// participations in compact and non-compact forms, health-care facility).
#[test]
fn master06_full_ctx_example() {
    assert_flat_vector(
        r#"{
          "ctx/language": "de",
          "ctx/territory": "US",
          "ctx/time": "2021-04-01T12:40:31.418954+02:00",
          "ctx/composer_name": "Silvia Blake",
          "ctx/composer_id": "123",
          "ctx/id_namespace": "HOSPITAL-NS",
          "ctx/id_scheme": "HOSPITAL-NS",
          "ctx/work_flow_id|id": "567",
          "ctx/work_flow_id|id_scheme": "HOSPITAL-NS",
          "ctx/work_flow_id|namespace": "HOSPITAL-NS",
          "ctx/work_flow_id|type": "ORGANISATION",
          "ctx/participation_name:0": "Dr. Marcus Johnson",
          "ctx/participation_function:0": "requester",
          "ctx/participation_mode:0": "face-to-face communication",
          "ctx/participation_id:0": "199",
          "ctx/participation_identifiers:0": "issuer1::assigner1::id1::PERSON;issuer2::assigner2::id2::PERSON",
          "ctx/participation_name:1": "Lara Markham",
          "ctx/participation_function:1": "performer",
          "ctx/participation_id:1": "198",
          "ctx/participation_identifiers:1|issuer:0": "issuer3",
          "ctx/participation_identifiers:1|assigner:0": "assigner3",
          "ctx/participation_identifiers:1|id:0": "id3",
          "ctx/participation_identifiers:1|type:0": "PERSON",
          "ctx/participation_identifiers:1|issuer:1": "issuer4",
          "ctx/participation_identifiers:1|assigner:1": "assigner4",
          "ctx/participation_identifiers:1|id:1": "id4",
          "ctx/participation_identifiers:1|type:1": "PERSON",
          "ctx/health_care_facility|name": "Hospital",
          "ctx/health_care_facility|id": "9091"
        }"#,
    );
}

/// master06 §Composer — the two composer forms (`composer_name`;
/// `composer_self`).
#[test]
fn master06_composer() {
    assert_flat_vector(
        r#"{
          "ctx/composer_name": "Silvia Blake",
          "ctx/composer_id": "123",
          "ctx/id_namespace": "HOSPITAL-NS",
          "ctx/id_scheme": "HOSPITAL-NS"
        }"#,
    );
    assert_flat_vector(
        r#"{
          "ctx/composer_self": true,
          "ctx/composer_id": "123",
          "ctx/id_namespace": "HOSPITAL-NS",
          "ctx/id_scheme": "HOSPITAL-NS"
        }"#,
    );
}

/// master06 §ID Namespace and Scheme — the example block.
#[test]
fn master06_id_namespace_and_scheme() {
    assert_flat_vector(
        r#"{
          "ctx/composer_id": "123",
          "ctx/id_namespace": "HOSPITAL-NS",
          "ctx/id_scheme": "HOSPITAL-NS"
        }"#,
    );
}

/// master06 §Language and Territory — the mandatory-context example block.
#[test]
fn master06_language_and_territory() {
    let json = r#"{ "ctx/language": "de", "ctx/territory": "US" }"#;
    assert_flat_vector(json);
    // These are the mandatory fields; validate_context accepts them.
    let doc = parse_flat(&flat_of(json)).unwrap();
    assert!(validate_context(&doc).is_empty());
}

/// master06 §Workflow ID — the example block.
#[test]
fn master06_workflow_id() {
    assert_flat_vector(
        r#"{
          "ctx/work_flow_id|id": "567",
          "ctx/work_flow_id|id_scheme": "HOSPITAL-NS",
          "ctx/work_flow_id|namespace": "HOSPITAL-NS",
          "ctx/work_flow_id|type": "ORGANISATION"
        }"#,
    );
}

/// master06 §Participation — the example block (compact + non-compact
/// identifier forms).
#[test]
fn master06_participation() {
    assert_flat_vector(
        r#"{
          "ctx/participation_name:0": "Dr. Marcus Johnson",
          "ctx/participation_function:0": "requester",
          "ctx/participation_mode:0": "face-to-face communication",
          "ctx/participation_id:0": "199",
          "ctx/participation_identifiers:0": "issuer1::assigner1::id1::PERSON;issuer2::assigner2::id2::PERSON",

          "ctx/participation_name:1": "Lara Markham",
          "ctx/participation_function:1": "performer",
          "ctx/participation_id:1": "198",
          "ctx/participation_identifiers:1|issuer:0": "issuer3",
          "ctx/participation_identifiers:1|assigner:0": "assigner3",
          "ctx/participation_identifiers:1|id:0": "id3",
          "ctx/participation_identifiers:1|type:0": "PERSON",
          "ctx/participation_identifiers:1|issuer:1": "issuer4",
          "ctx/participation_identifiers:1|assigner:1": "assigner4",
          "ctx/participation_identifiers:1|id:1": "id4",
          "ctx/participation_identifiers:1|type:1": "PERSON"
        }"#,
    );
}

/// master06 §health_care_facility — the example block.
#[test]
fn master06_health_care_facility() {
    assert_flat_vector(
        r#"{
          "ctx/health_care_facility|name": "Hospital",
          "ctx/health_care_facility|id": "9091",
          "ctx/id_namespace": "HOSPITAL-NS",
          "ctx/id_scheme": "HOSPITAL-NS"
        }"#,
    );
}

/// master06 §§time / end_time / history_origin / action_time / activity_timing
/// — the single-key temporal-default example blocks.
#[test]
fn master06_time_defaults() {
    assert_flat_vector(r#"{ "ctx/time": "2021-04-01T12:40:31.418954+02:00" }"#);
    assert_flat_vector(r#"{ "ctx/end_time": "2021-05-01T12:40:31.418954+02:00" }"#);
    assert_flat_vector(r#"{ "ctx/history_origin": "2021-05-01T12:40:31.418954+02:00" }"#);
    assert_flat_vector(r#"{ "ctx/action_time": "2021-05-01T12:40:31.418954+02:00" }"#);
    assert_flat_vector(r#"{ "ctx/activity_timing": "R4/2022-01-31T10:00:00+01:00/P3M" }"#);
}

/// master06 §provider — the example block.
#[test]
fn master06_provider() {
    assert_flat_vector(
        r#"{
          "ctx/provider_name": "Silvia Blake",
          "ctx/provider_id": "123",
          "ctx/id_namespace": "HOSPITAL-NS",
          "ctx/id_scheme": "HOSPITAL-NS"
        }"#,
    );
}

/// master06 §action_ism_transition_current_state — the two forms (value / code).
#[test]
fn master06_action_ism_transition_current_state() {
    assert_flat_vector(r#"{ "ctx/action_ism_transition_current_state": "completed" }"#);
    assert_flat_vector(r#"{ "ctx/action_ism_transition_current_state": "532" }"#);
}

/// master06 §instruction_narrative / §location — the single-key example blocks.
#[test]
fn master06_instruction_narrative_and_location() {
    assert_flat_vector(
        r#"{ "ctx/instruction_narrative": "Human readable instruction narrative" }"#,
    );
    assert_flat_vector(r#"{ "ctx/location": "Lab B2" }"#);
}

/// master06 §setting — the two forms (value / code).
#[test]
fn master06_setting() {
    assert_flat_vector(r#"{ "ctx/setting": "other care" }"#);
    assert_flat_vector(r#"{ "ctx/setting": "238" }"#);
}

/// master06 §link — the `ctx/link:i|…` default LINK example block.
#[test]
fn master06_link() {
    assert_flat_vector(
        r#"{
          "ctx/link:0|type": "problem",
          "ctx/link:0|meaning": "problem related note",
          "ctx/link:0|target": "ehr://ehr.network/347a5490-55ee-4da9-b91a-9bba710f730e"
        }"#,
    );
}
