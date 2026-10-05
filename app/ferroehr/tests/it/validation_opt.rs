// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Ingestion-side artefact-validity tests (surface A1).
//!
//! Each per-code test takes the vendored valid `minimal_evaluation.opt` and
//! breaks exactly one aspect (either as a targeted XML mutation or as a typed
//! model mutation for the terminology-side sets the minimal fixture does not
//! carry), asserting the matching AOM2 rule code surfaces. The corpus guard
//! asserts every vendored valid OPT still passes unchanged.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::string_slice,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::path::{Path, PathBuf};

use openehr_base::prelude::{CodePhrase, TerminologyId};
use openehr_its::opt14;
use openehr_its::opt14::types::{
    ArchetypeTerm, CAttribute, CObject, Codedefinitionset, ConstraintBindingItem,
    Constraintbindingset, FlatArchetypeOntology, Intervalofinteger, OperationalTemplate,
    TermBindingItem, Termbindingset,
};

use ferroehr::service::error::ServiceError;
use ferroehr::validation::validate_opt_artefact;

fn manifest() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

/// The vendored valid minimal OPT (the mutation base).
fn minimal_xml() -> String {
    let p = Path::new(manifest()).join(
        "../../docs/specs/openehr/CNF/tests/platform/robot/\
         _resources/test_data_sets/valid_templates/minimal/minimal_evaluation.opt",
    );
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn parse(xml: &str) -> OperationalTemplate {
    opt14::from_xml(xml).expect("minimal OPT parses")
}

/// Assert the artefact is rejected with the given AOM2 rule code carried in the
/// `ValidationFailed` payload (the ITS-REST 422 `validationErrors[]` rendering).
fn expect_code(opt: &OperationalTemplate, code: &str) {
    let err = validate_opt_artefact(opt).expect_err("expected a violation");
    let ServiceError::ValidationFailed(violations) = &err else {
        panic!("expected ValidationFailed (422), got {err:?}");
    };
    assert!(
        violations
            .iter()
            .any(|v| v.path == code || v.message.contains(code)),
        "expected `{code}` in violations, got: {violations:?}"
    );
}

/// Replace the first `from` occurring *after* `marker` with `to`.
fn mutate_after(xml: &str, marker: &str, from: &str, to: &str) -> String {
    let idx = xml
        .find(marker)
        .unwrap_or_else(|| panic!("marker not found: {marker}"));
    let (head, tail) = xml.split_at(idx);
    let new_tail = tail.replacen(from, to, 1);
    assert_ne!(new_tail, tail, "no `{from}` found after `{marker}`");
    format!("{head}{new_tail}")
}

fn code_phrase(terminology: &str, code: &str) -> CodePhrase {
    CodePhrase {
        terminology_id: TerminologyId {
            value: terminology.to_owned(),
        },
        code_string: code.to_owned(),
        preferred_term: None,
    }
}

// ── sanity + corpus guard ───────────────────────────────────────────────────

#[test]
fn valid_minimal_passes() {
    let opt = parse(&minimal_xml());
    validate_opt_artefact(&opt).expect("the vendored minimal OPT is valid");
}

/// Every vendored valid OPT under this crate's own resources must parse and
/// must still upload — the ingestion checks may never mis-reject a legitimate
/// template.
///
/// Parseability is asserted here as well as in the `openehr-its` `opt14_corpus`
/// gate: a template this sweep cannot read is a finding either way, and
/// skipping it silently would make the upload assertion vacuous for that file.
#[test]
fn corpus_all_valid_opts_pass() {
    // The service knowledge corpus is the upload-surface oracle. (The
    // openehr-its Better/SDK fixtures are deliberately NOT swept: they are
    // serialization-test material and include artefacts that genuinely violate
    // the AOM validity rules — e.g. `sdk/section_cardinality.opt` breaks VCOC
    // with mandatory-occurrence sums exceeding the container cardinality — so
    // a conformant server MUST reject them at upload.)
    let dir = PathBuf::from(manifest()).join("../../corpus/fixtures/service");
    let mut files = Vec::new();
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)
            .unwrap_or_else(|e| panic!("read {}: {e}", d.display()))
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "opt") {
                files.push(path);
            }
        }
    }
    files.sort();
    assert!(
        files.len() >= 90,
        "expected the full OPT corpus (~91 files), found {}",
        files.len()
    );

    let mut failures = Vec::new();
    for path in &files {
        let xml = std::fs::read_to_string(path).expect("read opt");
        let opt = match opt14::from_xml(&xml) {
            Ok(opt) => opt,
            Err(e) => {
                failures.push(format!("{}: does not parse: {e}", path.display()));
                continue;
            }
        };
        if let Err(e) = validate_opt_artefact(&opt) {
            failures.push(format!("{}: {e}", path.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "these valid vendored OPTs were mis-rejected by the ingestion checks:\n{}",
        failures.join("\n")
    );
}

#[test]
fn rejection_is_422_validation_failed() {
    // VCARM path — an AOM2 rule violation on a successfully PARSED artefact is
    // a semantic error: the ITS-REST overview status table's 422 row
    // (`Requests_and_responses.md` §HTTP status codes, "well-formed but …
    // semantic errors"), rendered as the `Error` object with the rule code in
    // `validationErrors[]`. The syntactic 400 branch (`responses/400.yaml`)
    // owns only content that fails to parse.
    let xml = minimal_xml().replace(
        "<rm_attribute_name>category</rm_attribute_name>",
        "<rm_attribute_name>bogus_attr</rm_attribute_name>",
    );
    let opt = parse(&xml);
    let err = validate_opt_artefact(&opt).unwrap_err();
    assert!(
        matches!(err, ServiceError::ValidationFailed(_)),
        "expected ValidationFailed (422), got {err:?}"
    );
}

// ── RM conformance (VCORM/VCARM/VCAEX/VCACA/VCAM) ────────────────────────────

#[test]
fn vcarm_unknown_rm_attribute() {
    // COMPOSITION has no `bogus_attr` attribute.
    let xml = minimal_xml().replace(
        "<rm_attribute_name>category</rm_attribute_name>",
        "<rm_attribute_name>bogus_attr</rm_attribute_name>",
    );
    expect_code(&parse(&xml), "VCARM");
}

#[test]
fn vcorm_unknown_rm_type() {
    // `NOT_A_TYPE` is not a reference-model class.
    let xml = minimal_xml().replace(
        "<rm_type_name>DV_QUANTITY</rm_type_name>",
        "<rm_type_name>NOT_A_TYPE</rm_type_name>",
    );
    expect_code(&parse(&xml), "VCORM");
}

#[test]
fn vcam_container_on_single_valued_attribute() {
    // `content` is a C_MULTIPLE_ATTRIBUTE; `language` is single-valued in the RM.
    let xml = minimal_xml().replace(
        "<rm_attribute_name>content</rm_attribute_name>",
        "<rm_attribute_name>language</rm_attribute_name>",
    );
    expect_code(&parse(&xml), "VCAM");
}

#[test]
fn vcaex_widened_existence_on_mandatory_attribute() {
    // `category` is mandatory in COMPOSITION; drop its existence lower to 0.
    let xml = mutate_after(
        &minimal_xml(),
        "<rm_attribute_name>category</rm_attribute_name>",
        "<lower>1</lower>",
        "<lower>0</lower>",
    );
    expect_code(&parse(&xml), "VCAEX");
}

// ── VCACA (cardinality conformance to the RM) ───────────────────────────────

/// Retype the minimal fixture's `ITEM_TREE` node as a `CLUSTER` — whose RM
/// `items` is `List<ITEM> [1..*]`
/// (`RM/docs/UML/classes/org.openehr.rm.data_structures.cluster.adoc`
/// §Attributes), i.e. an RM cardinality lower bound of 1 — and restate that
/// attribute's cardinality interval. Everything else is the vendored valid OPT.
fn cluster_items_cardinality(lower: i32, upper: Option<i32>) -> OperationalTemplate {
    let mut opt = parse(&minimal_xml());
    let mut roots = vec![CObject::CArchetypeRoot(opt.definition.clone())];
    retype(&mut roots, lower, upper);
    let [CObject::CArchetypeRoot(root)] = &*roots else {
        panic!("the retyped root is still a C_ARCHETYPE_ROOT");
    };
    opt.definition = root.clone();
    opt
}

/// Retypes every `ITEM_TREE` in `objects` to `CLUSTER`, restating the stated
/// cardinality of each `items` attribute it reaches.
fn retype(objects: &mut [CObject], lower: i32, upper: Option<i32>) {
    for object in objects {
        let attributes = match object {
            CObject::CComplexObject(c) => {
                if c.rm_type_name == "ITEM_TREE" {
                    "CLUSTER".clone_into(&mut c.rm_type_name);
                }
                &mut c.attributes
            }
            CObject::CArchetypeRoot(c) => &mut c.attributes,
            _ => continue,
        };
        for attribute in attributes.iter_mut() {
            retype_attribute(attribute, lower, upper);
        }
    }
}

/// Restates one attribute's cardinality when it is `items`, then descends.
fn retype_attribute(attribute: &mut CAttribute, lower: i32, upper: Option<i32>) {
    match attribute {
        CAttribute::CMultipleAttribute(multiple) => {
            if multiple.rm_attribute_name == "items" {
                // `CLUSTER.items` is RM-mandatory, so the fixture's `{0..1}`
                // existence must rise with the retype or VCAEX (not VCACA) is
                // what fires.
                multiple.existence.lower = Some(1);
                multiple.cardinality.interval = Intervalofinteger {
                    lower_included: Some(true),
                    upper_included: upper.map(|_| true),
                    lower_unbounded: false,
                    upper_unbounded: upper.is_none(),
                    lower: Some(lower),
                    upper,
                };
            }
            retype(&mut multiple.children, lower, upper);
        }
        CAttribute::CSingleAttribute(single) => retype(&mut single.children, lower, upper),
    }
}

/// VCACA: "the cardinality of an attribute must conform, i.e. be the same or
/// narrower, to the cardinality of the corresponding attribute in the
/// underlying information model" (`AOM2/master04.5-constraint_model-class_definitions.adoc`
/// line 162; `master08-validation.adoc` line 74). A STATED `{0..3}` on
/// `CLUSTER.items` admits an empty CLUSTER the RM's `[1..*]` forbids.
#[test]
fn vcaca_stated_cardinality_wider_than_the_rm_is_rejected() {
    expect_code(&cluster_items_cardinality(0, Some(3)), "VCACA");
}

/// The valid twin of [`vcaca_stated_cardinality_wider_than_the_rm_is_rejected`]:
/// a stated interval INSIDE the RM's (`{2..3}` ⊂ `{1..*}`) conforms.
#[test]
fn vcaca_stated_cardinality_inside_the_rm_is_accepted() {
    validate_opt_artefact(&cluster_items_cardinality(2, Some(3)))
        .expect("a narrower stated cardinality conforms");
}

/// The fully-open `{0..*}` states NO cardinality override and defers to the RM,
/// so it is not a widening: AOM 1.4 makes the field MANDATORY
/// (`org.openehr.am.aom14.c_multiple_attribute.adoc` §Attributes,
/// `cardinality 1..1`) where AOM2 makes it optional and "only set if it
/// overrides the underlying reference model"
/// (`org.openehr.am.aom2.c_attribute.adoc` §Attributes), and cADL's open
/// constraint means "any value permitted by the underlying information model"
/// (`ADL1.4/master05-cadl.adoc` §"'Any' Constraints"). Real published OPTs in
/// the vendored corpus carry exactly this shape on `CLUSTER.items`.
#[test]
fn vcaca_open_cardinality_defers_to_the_rm() {
    validate_opt_artefact(&cluster_items_cardinality(0, None))
        .expect("an open cardinality states no override");
}

// ── VACMCO / VCOC (occurrences vs cardinality) ──────────────────────────────

#[test]
fn vacmco_children_exceed_cardinality_upper() {
    // Bound the `items` container cardinality to an upper of 1, and require the
    // single ELEMENT child to occur at least twice → the mandatory children
    // cannot fit within the cardinality range.
    let xml = minimal_xml();
    let xml = mutate_after(
        &xml,
        "<rm_type_name>ELEMENT</rm_type_name>",
        "<lower>0</lower>",
        "<lower>2</lower>",
    );
    // The first open cardinality upper after ELEMENT is the `items` container's.
    let xml = mutate_after(
        &xml,
        "<rm_type_name>ELEMENT</rm_type_name>",
        "<upper_unbounded>true</upper_unbounded>",
        "<upper_unbounded>false</upper_unbounded><upper>1</upper>",
    );
    expect_code(&parse(&xml), "VACMCO");
}

// ── VATID (node-id codes defined in terminology) ────────────────────────────

/// THE MALFORMED MIDDLE CLASS (#1691): a node id carrying the at/id leader
/// but failing the code-body grammar is a malformed CLAIMED code — AOM2's
/// own predicate is leader-based (`adl_code_definitions.adoc` §`is_at_code`),
/// so `at0abc` is neither a valid code (grammar) nor free text (leader) and
/// must be refused, never fall between the two families. The valid twin is
/// every `at0001`-formed id the corpus accepts.
#[test]
fn vatid_leader_carrying_malformed_node_id() {
    let xml = minimal_xml().replace("<node_id>at0001</node_id>", "<node_id>at0abc</node_id>");
    expect_code(&parse(&xml), "VATID");
}

#[test]
fn vatid_undefined_node_id() {
    // Point a node at an at-code that no term definition defines.
    let xml = minimal_xml().replace("<node_id>at0001</node_id>", "<node_id>at9999</node_id>");
    expect_code(&parse(&xml), "VATID");
}

// ── VTTBK / VTCBK / VTLC (terminology sets — typed mutations) ────────────────

#[test]
fn vttbk_undefined_term_binding_key() {
    let mut opt = parse(&minimal_xml());
    opt.definition.term_bindings.push(Termbindingset {
        terminology: "SNOMED-CT".to_owned(),
        items: vec![TermBindingItem {
            // at7777 is not a defined archetype term, nor a path.
            code: "at7777".to_owned(),
            value: code_phrase("SNOMED-CT", "1234"),
        }],
    });
    expect_code(&opt, "VTTBK");
}

#[test]
fn vtcbk_undefined_constraint_binding_key() {
    let mut opt = parse(&minimal_xml());
    opt.ontology = Some(FlatArchetypeOntology {
        archetype_id: "openEHR-EHR-COMPOSITION.minimal.v1".to_owned(),
        term_definitions: Vec::new(),
        constraint_definitions: Vec::new(), // no ac-codes defined
        term_bindings: Vec::new(),
        constraint_bindings: vec![Constraintbindingset {
            terminology: "SNOMED-CT".to_owned(),
            items: vec![ConstraintBindingItem {
                code: "ac9999".to_owned(),
                value: "http://example.org/vs".to_owned(),
            }],
        }],
    });
    expect_code(&opt, "VTCBK");
}

// ── AOM 1.4 constraint-model invariants ─────────────────────────────────────

#[test]
fn existence_set_upper_above_one() {
    // C_ATTRIBUTE invariant Existence_set: existence.upper <= 1.
    let xml = mutate_after(
        &minimal_xml(),
        "<rm_attribute_name>category</rm_attribute_name>",
        "<upper>1</upper>",
        "<upper>2</upper>",
    );
    expect_code(&parse(&xml), "Existence_set");
}

#[test]
fn members_valid_occurrences_above_one_under_single_attribute() {
    // `category` is a C_SINGLE_ATTRIBUTE; give its DV_CODED_TEXT child
    // occurrences 0..5.
    let xml = minimal_xml();
    let idx = xml
        .find("<rm_attribute_name>category</rm_attribute_name>")
        .expect("category attribute present");
    let tail = &xml[idx..];
    let occ = tail.find("<occurrences>").expect("occurrences follows") + idx;
    let seg_end = xml[occ..].find("</occurrences>").expect("closed") + occ;
    let seg = &xml[occ..seg_end];
    let new_seg = seg.replace("<upper>1</upper>", "<upper>5</upper>");
    assert_ne!(seg, new_seg, "expected an <upper>1</upper> in occurrences");
    let xml = format!("{}{}{}", &xml[..occ], new_seg, &xml[seg_end..]);
    expect_code(&parse(&xml), "Members_valid");
}

#[test]
fn varid_malformed_archetype_id() {
    let xml = minimal_xml().replace(
        "openEHR-EHR-COMPOSITION.minimal.v1",
        "openEHR-EHR-COMPOSITION.minimal",
    );
    expect_code(&parse(&xml), "VARID");
}

#[test]
fn varid_tolerates_multipart_version_and_tooling_names() {
    // The published-template tolerances: ADL2-era `v1.0.0` versions and
    // parenthesized tooling concept names must pass.
    let xml = minimal_xml().replace(
        "openEHR-EHR-COMPOSITION.minimal.v1",
        "openEHR-EHR-COMPOSITION.minimal.v1.0.0",
    );
    validate_opt_artefact(&parse(&xml)).expect("multi-part version accepted");
    let xml = minimal_xml().replace(
        "openEHR-EHR-COMPOSITION.minimal.v1",
        "openEHR-EHR-COMPOSITION.t_exam(1-17)_lanit.v1",
    );
    validate_opt_artefact(&parse(&xml)).expect("tooling concept name accepted");
}

#[test]
fn vardt_root_type_mismatch() {
    // Rename the root archetype id's type slot: COMPOSITION definition vs an
    // OBSERVATION-typed id.
    let xml = minimal_xml().replace(
        "openEHR-EHR-COMPOSITION.minimal.v1",
        "openEHR-EHR-OBSERVATION.minimal.v1",
    );
    expect_code(&parse(&xml), "VARDT");
}

#[test]
fn c_boolean_unsatisfiable() {
    use openehr_its::opt14::types::{CAttribute, CBoolean, CObject, CPrimitive, CPrimitiveObject};
    let mut opt = parse(&minimal_xml());
    let occ = opt.definition.occurrences.clone();
    let boolean_node = CObject::CPrimitiveObject(CPrimitiveObject {
        rm_type_name: "BOOLEAN".to_owned(),
        occurrences: occ.clone(),
        node_id: String::new(),
        item: Some(Box::new(CPrimitive::CBoolean(CBoolean {
            true_valid: false,
            false_valid: false,
            assumed_value: None,
        }))),
    });
    opt.definition.attributes.push(CAttribute::CSingleAttribute(
        opt14::types::CSingleAttribute {
            rm_attribute_name: "name".to_owned(),
            existence: Intervalofinteger {
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: Some(true),
                upper_included: Some(true),
                lower: Some(1),
                upper: Some(1),
            },
            children: vec![boolean_node],
        },
    ));
    expect_code(&opt, "C_BOOLEAN_validity");
}

#[test]
fn assumed_value_outside_closed_list() {
    use openehr_its::opt14::types::{CAttribute, CObject, CPrimitive, CPrimitiveObject, CString};
    let mut opt = parse(&minimal_xml());
    let occ = opt.definition.occurrences.clone();
    // A C_STRING with a closed value list and an assumed value outside it.
    let string_node = CObject::CPrimitiveObject(CPrimitiveObject {
        rm_type_name: "STRING".to_owned(),
        occurrences: occ,
        node_id: String::new(),
        item: Some(Box::new(CPrimitive::CString(CString {
            pattern: None,
            list: vec!["red".to_owned(), "green".to_owned()],
            list_open: None,
            assumed_value: Some("blue".to_owned()),
        }))),
    });
    opt.definition.attributes.push(CAttribute::CSingleAttribute(
        opt14::types::CSingleAttribute {
            rm_attribute_name: "name".to_owned(),
            existence: Intervalofinteger {
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: Some(true),
                upper_included: Some(true),
                lower: Some(1),
                upper: Some(1),
            },
            children: vec![string_node],
        },
    ));
    expect_code(&opt, "Assumed_value_valid");
}

#[test]
fn pattern_validity_rejects_nonmonotonic_temporal_pattern() {
    use openehr_its::opt14::types::{CAttribute, CDate, CObject, CPrimitive, CPrimitiveObject};
    let mut opt = parse(&minimal_xml());
    let occ = opt.definition.occurrences.clone();
    // `yyyy-XX-dd`: disallowed month followed by a mandatory day violates the
    // Month_validity_disallowed ordering.
    let date_node = CObject::CPrimitiveObject(CPrimitiveObject {
        rm_type_name: "DATE".to_owned(),
        occurrences: occ,
        node_id: String::new(),
        item: Some(Box::new(CPrimitive::CDate(CDate {
            pattern: Some("yyyy-XX-dd".to_owned()),
            timezone_validity: None,
            range: None,
            assumed_value: None,
        }))),
    });
    opt.definition.attributes.push(CAttribute::CSingleAttribute(
        opt14::types::CSingleAttribute {
            rm_attribute_name: "name".to_owned(),
            existence: Intervalofinteger {
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: Some(true),
                upper_included: Some(true),
                lower: Some(1),
                upper: Some(1),
            },
            children: vec![date_node],
        },
    ));
    expect_code(&opt, "Pattern_validity");
}

#[test]
fn stcdc_duplicate_code_in_code_list() {
    // Duplicate a code in the first C_CODE_PHRASE code_list.
    let xml = mutate_after(
        &minimal_xml(),
        "C_CODE_PHRASE",
        "</code_list>",
        "</code_list><code_list>433</code_list><code_list>433</code_list>",
    );
    expect_code(&parse(&xml), "STCDC");
}

#[test]
fn vtlc_language_code_set_mismatch() {
    let mut opt = parse(&minimal_xml());
    let term = |code: &str| ArchetypeTerm {
        code: code.to_owned(),
        items: indexmap::IndexMap::default(),
    };
    opt.ontology = Some(FlatArchetypeOntology {
        archetype_id: "openEHR-EHR-COMPOSITION.minimal.v1".to_owned(),
        term_definitions: vec![
            Codedefinitionset {
                language: "en".to_owned(),
                items: vec![term("at0000"), term("at0001")],
            },
            Codedefinitionset {
                language: "de".to_owned(),
                // at0001 missing in `de` → language inconsistency.
                items: vec![term("at0000")],
            },
        ],
        constraint_definitions: Vec::new(),
        term_bindings: Vec::new(),
        constraint_bindings: Vec::new(),
    });
    expect_code(&opt, "VTLC");
}
