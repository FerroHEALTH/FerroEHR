// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_base::prelude::Interval;

use crate::parse::Dialect;
use crate::validate::catalogue::Severity;
use crate::validate::validate_integrity;

use super::primitive::map_primitive_object;
use super::slot::{binary_symbol, map_code_reference, unary_symbol};
use super::tuple::{map_constraint_ref, quantity_tuple, unbounded_occurrences};
use super::*;

/// Paths under `corpus/fixtures/service`, the vendored OPT corpus of real
/// Ocean/EHRbase-generated operational templates. One minimal
/// COMPOSITION+OBSERVATION, one minimal EVALUATION
/// (carries a `C_DV_QUANTITY`), and a large multi-archetype template
/// (`Vital Signs`: 12 embedded roots, ordinals/quantities/integers/reals/
/// strings/booleans/code phrases).
const MINIMAL_OBSERVATION: &str = "knowledge/opt/minimal_observation.opt";
const MINIMAL_EVALUATION: &str = "knowledge/opt/minimal_evaluation.opt";
const VITAL_SIGNS: &str = "knowledge/opt/Vital Signs Encounter (Composition).opt";

fn parse_opt(rel: &str) -> opt14::types::OperationalTemplate {
    let path = format!(
        "{}/../../corpus/fixtures/service/{rel}",
        env!("CARGO_MANIFEST_DIR")
    );
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse OPT {rel}: {e:?}"))
}

/// Every converted root converts, is a plain authored archetype, and passes
/// the AOM2 phase-1 catalogue clean — the converter's own oracle property
/// (`crates/openehr-adl/tests/adl14_conversion.rs` `assert_structural_match`
/// runs the same `validate_integrity(&got, None)` gate).
fn assert_converts_clean(rel: &str, min_roots: usize) {
    let opt = parse_opt(rel);
    let (archetypes, structure) =
        convert_opt_to_archetypes(&opt).unwrap_or_else(|e| panic!("convert {rel}: {e}"));
    assert!(
        archetypes.len() >= min_roots,
        "{rel}: expected >= {min_roots} converted roots, got {}",
        archetypes.len()
    );
    for ConvertedArchetype {
        archetype_id: id,
        archetype: art,
        ..
    } in &archetypes
    {
        assert!(
            matches!(art, Archetype::AuthoredArchetype(_)),
            "{rel}/{id}: not a plain authored archetype"
        );
        let errors: Vec<&str> = validate_integrity(art, None)
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.code.mnemonic())
            .collect();
        assert!(
            errors.is_empty(),
            "{rel}/{id}: converted output failed phase-1 validation: {errors:?}"
        );
    }
    // Every recorded fill edge names a real parent/child archetype id.
    let ids: std::collections::BTreeSet<&str> = archetypes
        .iter()
        .map(|root| root.archetype_id.as_str())
        .collect();
    for edge in &structure {
        assert!(
            ids.contains(edge.parent_archetype_id.as_str()),
            "{rel}: fill edge parent {:?} is not a converted root",
            edge.parent_archetype_id
        );
        assert!(
            ids.contains(edge.child_archetype_id.as_str()),
            "{rel}: fill edge child {:?} is not a converted root",
            edge.child_archetype_id
        );
    }
}

#[test]
fn minimal_observation_converts_clean() {
    // COMPOSITION root + one embedded OBSERVATION root = 2 sources, 1 edge.
    assert_converts_clean(MINIMAL_OBSERVATION, 2);
}

#[test]
fn minimal_evaluation_converts_clean() {
    assert_converts_clean(MINIMAL_EVALUATION, 2);
}

#[test]
fn vital_signs_converts_clean() {
    // A large multi-archetype template: 12 embedded C_ARCHETYPE_ROOTs, so
    // the COMPOSITION root plus its components.
    assert_converts_clean(VITAL_SIGNS, 2);
}

/// Two full conversions of the same OPT yield identical archetypes (each
/// unit converts under a fresh, deterministic log) — the converter's
/// idempotency oracle, at the OPT-front-end level.
#[test]
fn conversion_is_deterministic() {
    let opt = parse_opt(MINIMAL_OBSERVATION);
    let (first, first_edges) = convert_opt_to_archetypes(&opt).expect("first convert");
    let (second, second_edges) = convert_opt_to_archetypes(&opt).expect("second convert");
    assert_eq!(first, second, "OPT conversion is not deterministic");
    assert_eq!(
        first_edges, second_edges,
        "fill structure is not deterministic"
    );
}

/// EVERY vendored OPT converts phase-1-clean and its printed ADL2
/// re-parses — the whole-corpus fidelity gate (exercises default values,
/// slot assertions, tuples, bindings across the real template set).
#[test]
fn whole_opt_corpus_converts_and_reparses() {
    let dir = format!(
        "{}/../../corpus/fixtures/service/knowledge/opt",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut count = 0usize;
    for entry in std::fs::read_dir(&dir).expect("opt corpus dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("opt") {
            continue;
        }
        count += 1;
        let name = path.display().to_string();
        let xml = std::fs::read_to_string(&path).expect("read opt");
        let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse {name}: {e:?}"));
        let (archetypes, _) =
            convert_opt_to_archetypes(&opt).unwrap_or_else(|e| panic!("convert {name}: {e}"));
        for ConvertedArchetype {
            archetype_id: id,
            archetype: art,
            ..
        } in &archetypes
        {
            let errors: Vec<&str> = validate_integrity(art, None)
                .iter()
                .filter(|i| i.severity == Severity::Error)
                .map(|i| i.code.mnemonic())
                .collect();
            // No adjudicated exceptions: every decomposed source is
            // phase-1 clean — specialised embedded roots collapse to
            // depth-0 emission and reused 1.4 node codes re-mint
            // archetype-wide-unique ids in the converter core.
            assert!(errors.is_empty(), "{name}/{id}: phase-1 errors: {errors:?}");
            let printed = crate::print::print(art)
                .unwrap_or_else(|e| panic!("{name}/{id}: printing refused: {e}"));
            crate::assemble::parse_artefact(&printed, Dialect::Adl2).unwrap_or_else(|e| {
                panic!("{name}/{id}: printed ADL2 does not re-parse: {e:?}\n{printed}")
            });
        }
    }
    assert!(count >= 3, "the OPT corpus went missing ({count} files)");
}

/// `C_DV_QUANTITY` with per-unit magnitudes → the `[units, magnitude]`
/// tuple (`master04.4` §Tuple Constraints); with none → a plain `units`
/// attribute (the reference-corpus form for units-only constraints).
#[test]
fn quantity_tuple_emitted_when_magnitudes_present() {
    let item = |units: &str, magnitude: Option<(f64, f64)>| opt14::types::CQuantityItem {
        magnitude: magnitude.map(|(lo, hi)| opt14::types::Intervalofreal {
            lower_included: Some(true),
            upper_included: Some(true),
            lower_unbounded: false,
            upper_unbounded: false,
            lower: Some(lo),
            upper: Some(hi),
        }),
        precision: None,
        units: units.to_owned(),
    };
    let quantity = |list: Vec<opt14::types::CQuantityItem>| opt14::types::CDvQuantity {
        rm_type_name: "DV_QUANTITY".to_owned(),
        occurrences: unbounded_occurrences(),
        node_id: "at0004".to_owned(),
        assumed_value: None,
        property: None,
        list,
    };

    let mut cx = test_cx();
    let constrained = quantity_tuple(
        &quantity(vec![
            item("deg C", Some((0.0, 100.0))),
            item("deg F", Some((32.0, 212.0))),
        ]),
        &mut cx,
    );
    let CObject::CComplexObject(CComplexObject::CComplexObject(d)) = &constrained else {
        panic!("expected a complex object");
    };
    assert_eq!(
        d.attribute_tuples.as_ref().map_or(0, Vec::len),
        1,
        "one [units, magnitude] tuple"
    );
    assert_eq!(
        d.attribute_tuples
            .iter()
            .flatten()
            .next()
            .map(|t| t.tuples.as_ref().map_or(0, Vec::len)),
        Some(2),
        "one row per unit"
    );

    let widened = quantity_tuple(&quantity(vec![item("kg", None), item("mg", None)]), &mut cx);
    let CObject::CComplexObject(CComplexObject::CComplexObject(d)) = &widened else {
        panic!("expected a complex object");
    };
    assert!(
        d.attribute_tuples.as_ref().is_none_or(Vec::is_empty),
        "no tuple without magnitudes"
    );
    assert!(
        d.attributes
            .iter()
            .flatten()
            .any(|a| a.rm_attribute_name == "units"),
        "a plain units attribute instead"
    );
}

/// `map_code_reference` with a bare `referenceSetUri` mints the ac-code +
/// definition + binding (`AOM2/master07-terminology_package.adoc`
/// §Overview: an ac binding designates a ref-set / value set).
#[test]
fn reference_set_uri_becomes_ac_binding() {
    let mut cx = test_cx();
    let node = opt14::types::CCodeReference {
        rm_type_name: "CODE_PHRASE".to_owned(),
        occurrences: unbounded_occurrences(),
        node_id: "at0005".to_owned(),
        assumed_value: None,
        terminology_id: None,
        code_list: Vec::new(),
        referenceSetUri: "http://example.org/fhir/ValueSet/units".to_owned(),
    };
    let obj = map_code_reference(&node, &mut cx);
    let CObject::CTerminologyCode(tc) = obj else {
        panic!("expected a C_TERMINOLOGY_CODE, got {obj:?}");
    };
    assert_eq!(tc.constraint, "ac0001", "minted ac constraint");
    assert_eq!(cx.extra_terms.len(), 1, "one minted ac definition");
    assert_eq!(
        cx.extra_terms.first().map(|t| t.code.as_str()),
        Some("ac0001")
    );
    assert_eq!(
        cx.extra_bindings.first(),
        Some(&(
            "external".to_owned(),
            "ac0001".to_owned(),
            "http://example.org/fhir/ValueSet/units".to_owned()
        ))
    );
}

/// `map_constraint_ref`: a defined ac-code becomes the ac constraint; an
/// undefined one stays unconstrained with a `conversion_details` report
/// (never a dangling reference that would fail VACDF).
#[test]
fn constraint_ref_resolves_or_reports() {
    let mut cx = test_cx();
    cx.defined_acs.insert("ac0002".to_owned());
    let defined = opt14::types::ConstraintRef {
        rm_type_name: "CODE_PHRASE".to_owned(),
        occurrences: unbounded_occurrences(),
        node_id: "at0007".to_owned(),
        reference: "ac0002".to_owned(),
    };
    let CObject::CTerminologyCode(tc) = map_constraint_ref(&defined, &mut cx) else {
        panic!("expected a C_TERMINOLOGY_CODE");
    };
    assert_eq!(tc.constraint, "ac0002");
    assert!(cx.notes.is_empty(), "a resolved ref reports nothing");

    let dangling = opt14::types::ConstraintRef {
        reference: "ac0099".to_owned(),
        ..defined
    };
    let CObject::CTerminologyCode(tc) = map_constraint_ref(&dangling, &mut cx) else {
        panic!("expected a C_TERMINOLOGY_CODE");
    };
    assert!(
        tc.constraint.is_empty(),
        "a dangling ref stays unconstrained"
    );
    assert!(
        cx.notes.keys().any(|k| k.starts_with("constraint_ref.")),
        "the dangling ref must be reported: {:?}",
        cx.notes
    );
}

/// Temporal primitives carry BOTH the ISO8601 pattern and the range
/// (`org.openehr.am.aom2.c_duration.adoc`: the combined `"PWD/|P0W..P50W|"`
/// form), plus the assumed value.
#[test]
fn temporal_range_and_pattern_both_carried() {
    let node = opt14::types::CPrimitiveObject {
        rm_type_name: "DURATION".to_owned(),
        occurrences: unbounded_occurrences(),
        node_id: String::new(),
        item: Some(Box::new(opt14::types::CPrimitive::CDuration(
            opt14::types::CDuration {
                pattern: Some("PTH".to_owned()),
                range: Some(opt14::types::Intervalofduration {
                    lower_included: Some(true),
                    upper_included: Some(true),
                    lower_unbounded: false,
                    upper_unbounded: false,
                    lower: Some("PT0H".to_owned()),
                    upper: Some("PT12H".to_owned()),
                }),
                assumed_value: Some("PT1H".to_owned()),
            },
        ))),
    };
    let mut cx = test_cx();
    let CObject::CDuration(d) = map_primitive_object(&node, &mut cx) else {
        panic!("expected a C_DURATION");
    };
    assert_eq!(d.pattern_constraint.as_deref(), Some("PTH"));
    assert_eq!(
        d.constraint.as_ref().map_or(0, Vec::len),
        1,
        "the range must be carried"
    );
    assert_eq!(d.assumed_value.map(|a| a.value), Some("PT1H".to_owned()));
    assert!(cx.notes.is_empty(), "durations carry both without a report");
}

/// A date carrying BOTH a pattern and a range keeps the range and drops
/// the pattern with a report — the mixed `pattern/interval` ADL2 form is
/// duration-only (`master04.5` §Mixed Pattern and Interval), so emitting
/// it for a date would not re-parse.
#[test]
fn date_pattern_plus_range_keeps_range_and_reports() {
    let node = opt14::types::CPrimitiveObject {
        rm_type_name: "DATE".to_owned(),
        occurrences: unbounded_occurrences(),
        node_id: String::new(),
        item: Some(Box::new(opt14::types::CPrimitive::CDate(
            opt14::types::CDate {
                pattern: Some("yyyy-??-??".to_owned()),
                timezone_validity: None,
                range: Some(opt14::types::Intervalofdate {
                    lower_included: Some(true),
                    upper_included: Some(true),
                    lower_unbounded: false,
                    upper_unbounded: false,
                    lower: Some("2004-01-01".to_owned()),
                    upper: Some("2004-12-31".to_owned()),
                }),
                assumed_value: None,
            },
        ))),
    };
    let mut cx = test_cx();
    let CObject::CDate(d) = map_primitive_object(&node, &mut cx) else {
        panic!("expected a C_DATE");
    };
    assert!(d.pattern_constraint.is_none(), "the pattern must drop");
    assert_eq!(
        d.constraint.as_ref().map_or(0, Vec::len),
        1,
        "the range must be kept"
    );
    assert!(
        cx.notes.keys().any(|k| k.starts_with("temporal_pattern.")),
        "the dropped pattern must be reported: {:?}",
        cx.notes
    );
    // A pattern-only date keeps its pattern unreported.
    let pattern_only = opt14::types::CPrimitiveObject {
        item: Some(Box::new(opt14::types::CPrimitive::CDate(
            opt14::types::CDate {
                pattern: Some("yyyy-??-??".to_owned()),
                timezone_validity: None,
                range: None,
                assumed_value: None,
            },
        ))),
        ..node
    };
    let mut cx = test_cx();
    let CObject::CDate(d) = map_primitive_object(&pattern_only, &mut cx) else {
        panic!("expected a C_DATE");
    };
    assert_eq!(d.pattern_constraint.as_deref(), Some("yyyy-??-??"));
    assert!(cx.notes.is_empty());
}

/// An enumerated AOM 1.4 integer/real `list` survives as AOM2 point
/// intervals beside the `range` (AM `org.openehr.am.aom2.c_integer.adoc`
/// `constraint: List<Interval<Integer>>`).
#[test]
fn integer_and_real_lists_become_point_intervals() {
    let point = |iv: &Interval<i32>| match iv {
        Interval::PointInterval(p) => p.lower,
        Interval::ProperInterval(_) => None,
    };
    let node = opt14::types::CPrimitiveObject {
        rm_type_name: "INTEGER".to_owned(),
        occurrences: unbounded_occurrences(),
        node_id: String::new(),
        item: Some(Box::new(opt14::types::CPrimitive::CInteger(
            opt14::types::CInteger {
                list: vec![1, 2, 3],
                range: None,
                assumed_value: None,
            },
        ))),
    };
    let mut cx = test_cx();
    let CObject::CInteger(i) = map_primitive_object(&node, &mut cx) else {
        panic!("expected a C_INTEGER");
    };
    let points: Vec<Option<i32>> = i.constraint.iter().flatten().map(point).collect();
    assert_eq!(points, vec![Some(1), Some(2), Some(3)]);

    let real = opt14::types::CPrimitiveObject {
        rm_type_name: "REAL".to_owned(),
        item: Some(Box::new(opt14::types::CPrimitive::CReal(
            opt14::types::CReal {
                list: vec![0.5, 1.5],
                range: None,
                assumed_value: None,
            },
        ))),
        ..node
    };
    let CObject::CReal(r) = map_primitive_object(&real, &mut cx) else {
        panic!("expected a C_REAL");
    };
    let points: Vec<Option<f64>> = r
        .constraint
        .iter()
        .flatten()
        .map(|iv| match iv {
            Interval::PointInterval(p) => p.lower,
            Interval::ProperInterval(_) => None,
        })
        .collect();
    assert_eq!(points, vec![Some(0.5), Some(1.5)]);
}

/// An item-less `C_PRIMITIVE_OBJECT` converts to the unconstrained
/// constraint of the primitive type its `rm_type_name` names, never to a
/// `C_STRING` for a non-string type.
#[test]
fn itemless_primitive_keeps_its_type() {
    let node = |rm: &str| opt14::types::CPrimitiveObject {
        rm_type_name: rm.to_owned(),
        occurrences: unbounded_occurrences(),
        node_id: String::new(),
        item: None,
    };
    let mut cx = test_cx();
    let CObject::CInteger(i) = map_primitive_object(&node("INTEGER"), &mut cx) else {
        panic!("an item-less INTEGER must convert to a C_INTEGER");
    };
    assert_eq!(i.rm_type_name, "INTEGER");
    assert!(i.constraint.as_ref().is_none_or(Vec::is_empty));
    assert!(i.assumed_value.is_none());

    let CObject::CDate(d) = map_primitive_object(&node("DATE"), &mut cx) else {
        panic!("an item-less DATE must convert to a C_DATE");
    };
    assert_eq!(d.rm_type_name, "DATE");
    assert!(d.constraint.as_ref().is_none_or(Vec::is_empty));
    assert!(d.pattern_constraint.is_none());

    let CObject::CBoolean(b) = map_primitive_object(&node("BOOLEAN"), &mut cx) else {
        panic!("an item-less BOOLEAN must convert to a C_BOOLEAN");
    };
    assert_eq!(b.constraint.as_deref(), Some([true, false].as_slice()));

    assert!(matches!(
        map_primitive_object(&node("STRING"), &mut cx),
        CObject::CString(_)
    ));
    assert!(
        cx.notes.is_empty(),
        "a known primitive type is not reported"
    );

    // A name that is no AOM 1.4 primitive type: an unconstrained object of
    // that type, reported in conversion_details.
    let CObject::CComplexObject(_) = map_primitive_object(&node("DV_TEXT"), &mut cx) else {
        panic!("an unknown primitive type must convert to an unconstrained object");
    };
    assert!(
        cx.notes
            .keys()
            .any(|k| k.starts_with("primitive_without_item.DV_TEXT")),
        "{:?}",
        cx.notes
    );
}

/// A deployed `ROLE` template converts clean: its `capabilities` and
/// `credentials` single attributes keep one value each (ADL 1.4
/// `master05-cadl.adoc` §Occurrences), and its `ac0001` reference, which
/// the template's ontology does not define, is reported in
/// `conversion_details` rather than refused.
#[test]
fn role_template_converts_with_its_defects_reported() {
    let path = format!(
        "{}/../../corpus/fixtures/opt/valid/role.v2.opt",
        env!("CARGO_MANIFEST_DIR")
    );
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse Role.v2: {e:?}"));
    let (archetypes, _) =
        convert_opt_to_archetypes(&opt).unwrap_or_else(|e| panic!("convert Role.v2: {e}"));
    let role = &archetypes
        .iter()
        .find(|root| root.archetype_id == "openEHR-DEMOGRAPHIC-ROLE.Role.v1")
        .expect("the ROLE root converts")
        .archetype;
    let errors: Vec<&str> = validate_integrity(role, None)
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code.mnemonic())
        .collect();
    assert!(
        errors.is_empty(),
        "Role.v2 failed phase-1 validation: {errors:?}"
    );
    let Archetype::AuthoredArchetype(boxed) = role else {
        panic!("the ROLE root is an authored archetype");
    };
    let AuthoredArchetype::AuthoredArchetype(data) = boxed.as_ref() else {
        panic!("the ROLE root is a plain authored archetype");
    };
    let details = data
        .description
        .as_ref()
        .and_then(|d| d.conversion_details.as_ref())
        .expect("Role.v2 reports its defects");
    assert!(
        details.values().any(|v| v.contains("ac0001")),
        "the dangling ac0001 is reported: {details:?}"
    );
    for attr in ["capabilities", "credentials"] {
        assert!(
            details
                .keys()
                .any(|k| k.starts_with("single_attribute_occurrences.") && k.ends_with(attr)),
            "the {attr} cap is reported: {details:?}"
        );
    }
}

/// Notes recorded during a walk land in the converted archetype's
/// `RESOURCE_DESCRIPTION.conversion_details`.
#[test]
fn notes_land_in_conversion_details() {
    let mut notes = BTreeMap::new();
    notes.insert("k".to_owned(), "v".to_owned());
    let desc = minimal_description(notes);
    assert_eq!(
        desc.conversion_details
            .as_ref()
            .and_then(|m| m.get("k"))
            .map(String::as_str),
        Some("v")
    );
    assert!(
        minimal_description(BTreeMap::new())
            .conversion_details
            .is_none(),
        "an empty report stays absent"
    );
}

fn test_cx() -> RootCx {
    RootCx {
        slot_num: 1,
        next_ac: 1,
        defined_acs: std::collections::BTreeSet::new(),
        extra_terms: Vec::new(),
        extra_bindings: Vec::new(),
        notes: BTreeMap::new(),
    }
}

/// The embedded OBSERVATION root is decomposed into its own source and the
/// COMPOSITION → OBSERVATION fill is recorded in the structure log.
#[test]
fn embedded_root_recorded_in_structure() {
    let opt = parse_opt(MINIMAL_OBSERVATION);
    let (archetypes, structure) = convert_opt_to_archetypes(&opt).expect("convert");
    assert!(
        archetypes
            .iter()
            .any(|root| root.archetype_id == "openEHR-EHR-OBSERVATION.minimal.v1"),
        "the embedded OBSERVATION was not decomposed into its own source"
    );
    assert!(
        structure.iter().any(|e| {
            e.parent_archetype_id == "openEHR-EHR-COMPOSITION.minimal.v1"
                && e.child_archetype_id == "openEHR-EHR-OBSERVATION.minimal.v1"
        }),
        "the COMPOSITION → OBSERVATION fill edge was not recorded: {structure:?}"
    );
}

/// A rendered slot assertion is ADL surface syntax, i.e. it round-trips
/// through the slot-assertion parser the converted archetype is read back
/// with.
///
/// The `OPERATOR_KIND` value the OPT carries is an XSD facet id (`2007` for
/// `matches`), which is not an operator token in any grammar; only the
/// textual rendering of `LANG/docs/BEL/master03-language.adoc` §Operators
/// parses.
#[test]
fn rendered_operators_are_adl_surface_syntax() {
    let matches = binary_symbol(opt14::types::OperatorKind::Matches)
        .expect("`matches` has a surface rendering");
    let text = format!("archetype_id/value {matches} {{/openEHR-EHR-OBSERVATION\\.minimal\\.v1/}}");
    let parsed = crate::rules::parse_slot_assertions(&text)
        .unwrap_or_else(|e| panic!("rendered assertion {text:?} did not parse: {e:?}"));
    assert_eq!(parsed.len(), 1, "expected one assertion from {text:?}");

    // The facet id itself is not a token, so the pre-mapping rendering
    // could never have parsed.
    let raw = format!(
        "archetype_id/value {} {{/openEHR-EHR-OBSERVATION\\.minimal\\.v1/}}",
        opt14::types::OperatorKind::Matches.as_wire()
    );
    assert!(
        crate::rules::parse_slot_assertions(&raw).is_err(),
        "the raw XSD facet id must not be accepted as an operator"
    );
}

/// Every relational, arithmetic and logical `OPERATOR_KIND` renders to the
/// textual form of `LANG/docs/BEL/master03-language.adoc` §Operators, and
/// the two quantifiers — which have no infix syntax there — render to
/// nothing rather than to unparseable text.
#[test]
fn every_infix_operator_kind_maps_to_its_textual_rendering() {
    for (kind, symbol) in [
        (opt14::types::OperatorKind::Equal, "="),
        (opt14::types::OperatorKind::NotEqual, "!="),
        (opt14::types::OperatorKind::LessThanOrEqual, "<="),
        (opt14::types::OperatorKind::LessThan, "<"),
        (opt14::types::OperatorKind::GreaterThanOrEqual, ">="),
        (opt14::types::OperatorKind::GreaterThan, ">"),
        (opt14::types::OperatorKind::Matches, "matches"),
        (opt14::types::OperatorKind::And, "and"),
        (opt14::types::OperatorKind::Or, "or"),
        (opt14::types::OperatorKind::Xor, "xor"),
        (opt14::types::OperatorKind::Implies, "implies"),
        (opt14::types::OperatorKind::Plus, "+"),
        (opt14::types::OperatorKind::Minus, "-"),
        (opt14::types::OperatorKind::Multiply, "*"),
        (opt14::types::OperatorKind::Divide, "/"),
        (opt14::types::OperatorKind::Exponent, "^"),
    ] {
        assert_eq!(binary_symbol(kind), Some(symbol), "{kind:?}");
    }
    for kind in [
        opt14::types::OperatorKind::ForAll,
        opt14::types::OperatorKind::Exists,
        opt14::types::OperatorKind::Not,
    ] {
        assert_eq!(binary_symbol(kind), None, "{kind:?}");
    }
    assert_eq!(
        unary_symbol(opt14::types::OperatorKind::Not),
        Some("not"),
        "`not` is the prefix negation of BEL master03 §Logical Negation"
    );
    assert_eq!(unary_symbol(opt14::types::OperatorKind::Matches), None);
}
