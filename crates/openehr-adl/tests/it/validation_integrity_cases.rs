// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Hand-written phase-1 validation cases — one per catalogue code that has no
//! coverage in the vendored corpus (INVENTORY §3b), so every phase-1 code that
//! the phase-1 validator raises has at least one asserted test.
//!
//! Each case cites the spec rule text it encodes. Where the parser structurally
//! prevents the invalid state (e.g. a differential path in a top-level
//! archetype is the syntax error SDSF, never the semantic VDIFV), the case
//! builds the state by mutating a parsed model and calls the model-level
//! [`validate_integrity`]; otherwise it drives the invalid source through
//! [`validate_source_integrity`].
//!
//! Spec oracle: `docs/specs/openehr/AM/docs/AOM2/`.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions panic/unwrap/expect by design, including in fixture helpers outside #[test] fns"
)]

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::catalogue::ValidationCode;
use openehr_adl::validate::{ValidationIssue, validate_integrity, validate_source_integrity};
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

/// A minimal, phase-1-clean ADL2 archetype (non-specialised, id-coded).
const BASE: &str = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-ENTRY.integrity_base.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"draft">

definition
    ENTRY[id1] matches {
        element_attr matches {
            ELEMENT[id2]
        }
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = < text = <"root"> description = <"root"> >
            ["id2"] = < text = <"el"> description = <"el"> >
        >
    >
"#;

fn parse(src: &str) -> Archetype {
    parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("source must parse: {e:?}"))
}

/// The mnemonics of the raised issues.
fn mnemonics(issues: &[ValidationIssue]) -> Vec<&'static str> {
    issues.iter().map(|i| i.code.mnemonic()).collect()
}

fn assert_raises(issues: &[ValidationIssue], code: ValidationCode) {
    assert!(
        issues.iter().any(|i| i.code == code),
        "expected {code}; got {:?}",
        mnemonics(issues)
    );
}

/// Mutable access to the base archetype's `AUTHORED_ARCHETYPE` data.
fn authored_mut(a: &mut Archetype) -> &mut AuthoredArchetypeData {
    match a {
        Archetype::AuthoredArchetype(b) => match b.as_mut() {
            AuthoredArchetype::AuthoredArchetype(d) => d,
            _ => panic!("base is a plain AUTHORED_ARCHETYPE"),
        },
        Archetype::TemplateOverlay(_) => panic!("base is not an overlay"),
    }
}

fn root_data_mut(
    a: &mut Archetype,
) -> &mut openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObjectData {
    match &mut authored_mut(a).definition {
        CComplexObject::CComplexObject(d) => d,
        CComplexObject::CArchetypeRoot(_) => panic!("base root is a plain C_COMPLEX_OBJECT"),
    }
}

// ── source-driven cases ───────────────────────────────────────────────────

#[test]
fn varav_invalid_adl_version() {
    // master03 §Validity Rules — VARAV: `adl_version` must be a valid 3-part
    // version identifier.
    let src = BASE.replace("adl_version=2.0.5", "adl_version=2.0");
    let issues = validate_source_integrity(&src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Varav);
}

#[test]
fn varrv_invalid_rm_release() {
    // master03 §Validity Rules — VARRV: `rm_release` must be a valid 3-part
    // version identifier.
    let src = BASE.replace("rm_release=1.0.2", "rm_release=1.0");
    let issues = validate_source_integrity(&src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Varrv);
}

#[test]
fn vard_missing_description() {
    // master03 §Validity Rules — VARD: a `description` section must exist.
    let src = BASE.replace("description\n    lifecycle_state = <\"draft\">\n\n", "");
    let issues = validate_source_integrity(&src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Vard);
}

#[test]
fn vcatu_duplicate_sibling_attribute() {
    // master04.5 §C_COMPLEX_OBJECT — VCATU: sibling attributes must be uniquely
    // named.
    let src = BASE.replace(
        "        element_attr matches {\n            ELEMENT[id2]\n        }\n",
        "        element_attr matches { ELEMENT[id2] }\n        element_attr matches { ELEMENT[id3] }\n",
    );
    let issues = validate_source_integrity(&src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Vcatu);
}

/// A minimal specialisation of [`BASE`] whose two root-level differential paths
/// end in the same RM attribute name (`element_attr`) yet address different
/// parent nodes.
const CHILD_WITH_DIFFERENTIAL_PATHS: &str = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-ENTRY.integrity_base-child.v1.0.0

specialise
    openEHR-EHR-ENTRY.integrity_base.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"draft">

definition
    ENTRY[id1.1] matches {
        /element_attr matches {
            ELEMENT[id0.1]
        }
        /element_attr[id2]/element_attr matches {
            ELEMENT[id0.2]
        }
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1.1"] = < text = <"child"> description = <"child"> >
            ["id0.1"] = < text = <"new"> description = <"new"> >
            ["id0.2"] = < text = <"nested"> description = <"nested"> >
        >
    >
"#;

#[test]
fn vcatu_differential_paths_sharing_a_leading_segment_are_distinct() {
    // master04.5 §C_COMPLEX_OBJECT — VCATU judges sibling attributes; in a
    // differential archetype `/element_attr` and `/element_attr[id2]/element_attr`
    // are different attributes of the flat parent (ADL2 master09.02
    // §Differential Paths), not a duplicate.
    let issues =
        validate_source_integrity(CHILD_WITH_DIFFERENTIAL_PATHS, Dialect::Adl2, None).unwrap();
    assert!(
        !mnemonics(&issues).contains(&"VCATU"),
        "differential paths must not be read as duplicate siblings: {issues:?}"
    );
}

#[test]
fn vasid_stated_parent_absent_from_the_repository() {
    // master03 §Validity Rules — VASID: the stated parent must be the identifier
    // of the immediate parent archetype; an id the repository cannot resolve is
    // not that, and phase 2 (master08) cannot run without the flat parent.
    let empty = ArchetypeRepository::new();
    let issues =
        validate_source_integrity(CHILD_WITH_DIFFERENTIAL_PATHS, Dialect::Adl2, Some(&empty))
            .unwrap();
    assert_raises(&issues, ValidationCode::Vasid);

    let mut with_parent = ArchetypeRepository::new();
    with_parent.insert(parse(BASE));
    let issues = validate_source_integrity(
        CHILD_WITH_DIFFERENTIAL_PATHS,
        Dialect::Adl2,
        Some(&with_parent),
    )
    .unwrap();
    assert!(
        !mnemonics(&issues).contains(&"VASID"),
        "a resolvable parent satisfies VASID: {issues:?}"
    );
}

#[test]
fn vasid_absent_parent_is_not_raised_for_an_operational_template() {
    // master08 §Phase 3 — an operational template is the flat form itself, so
    // its `specialise` clause records lineage and needs no stored parent.
    let src = CHILD_WITH_DIFFERENTIAL_PATHS
        .replacen("archetype (adl_version", "operational_template (adl_version", 1)
        .replace(
            "        /element_attr matches {\n            ELEMENT[id0.1]\n        }\n        /element_attr[id2]/element_attr matches {\n            ELEMENT[id0.2]\n        }\n",
            "        element_attr matches {\n            ELEMENT[id0.1]\n            ELEMENT[id0.2]\n        }\n",
        );
    let empty = ArchetypeRepository::new();
    let issues = validate_source_integrity(&src, Dialect::Adl2, Some(&empty)).unwrap();
    assert!(
        !mnemonics(&issues).contains(&"VASID"),
        "an operational template validates without its parent: {issues:?}"
    );
}

#[test]
fn vcosu_duplicate_node_id() {
    // master04.5 §C_OBJECT — VCOSU: object node ids must be unique within the
    // archetype.
    let src = BASE.replace(
        "            ELEMENT[id2]\n",
        "            ELEMENT[id2]\n            ELEMENT[id2]\n",
    );
    let issues = validate_source_integrity(&src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Vcosu);
}

#[test]
fn vtvsid_value_set_id_not_defined() {
    // master07 §Validity Rules — VTVSID: a value-set id must be defined in the
    // terminology.
    let src = BASE.replace(
        "    term_definitions = <",
        "    value_sets = <\n        [\"ac9\"] = < id = <\"ac9\"> members = <\"at2\"> >\n    >\n    term_definitions = <",
    );
    let issues = validate_source_integrity(&src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Vtvsid);
}

#[test]
fn vtcbk_undefined_constraint_binding_key() {
    // master07 §Validity Rules — VTCBK: a constraint (ac) binding key must be a
    // defined ac-code.
    let src = BASE.replace(
        "    term_definitions = <",
        "    term_bindings = <\n        [\"snomed\"] = < [\"ac9\"] = <http://x/1> >\n    >\n    term_definitions = <",
    );
    let issues = validate_source_integrity(&src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Vtcbk);
}

#[test]
fn vatcv_malformed_defined_code() {
    // master08 §Code Validation — VATCV: a defined terminology code must be a
    // valid code form.
    let src = BASE.replace(
        "            [\"id2\"] = < text = <\"el\"> description = <\"el\"> >",
        "            [\"id2\"] = < text = <\"el\"> description = <\"el\"> >\n            [\"notacode\"] = < text = <\"x\"> description = <\"x\"> >",
    );
    let issues = validate_source_integrity(&src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Vatcv);
}

// ── model-mutation cases (the parser structurally prevents the state) ──────

#[test]
fn vdifv_differential_path_without_specialisation() {
    // master04.5 §C_ATTRIBUTE — VDIFV: a differential path is only valid in a
    // specialised archetype. (In source this would be the syntax error SDSF, so
    // the state is built by mutation.)
    let mut a = parse(BASE);
    root_data_mut(&mut a)
        .attributes
        .as_deref_mut()
        .unwrap_or_default()[0]
        .differential_path = Some("/element_attr".to_owned());
    let issues = validate_integrity(&a, None);
    assert_raises(&issues, ValidationCode::Vdifv);
}

#[test]
fn varid_malformed_identifier() {
    // master03 §Validity Rules — VARID: the archetype identifier must be
    // well-formed (here: a missing RM class).
    let mut a = parse(BASE);
    authored_mut(&mut a).archetype_id.rm_class = String::new();
    let issues = validate_integrity(&a, None);
    assert_raises(&issues, ValidationCode::Varid);
}

#[test]
fn vobav_assumed_value_outside_constraint() {
    // master04.5 §C_PRIMITIVE_OBJECT — VOBAV: an assumed value must fall within
    // the value space of its constraint.
    use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;
    let mut a = parse(BASE);
    let el = &mut root_data_mut(&mut a)
        .attributes
        .as_deref_mut()
        .unwrap_or_default()[0]
        .children
        .as_deref_mut()
        .unwrap_or_default()[0];
    if let CObject::CComplexObject(CComplexObject::CComplexObject(elem)) = el {
        // give the ELEMENT a `value` attribute constrained to a C_STRING whose
        // assumed value is not in the constraint list.
        use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
        elem.attributes.get_or_insert_default().push(CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name: "value".to_owned(),
            existence: None,
            children: Some(vec![CObject::CString(CString {
                parent: None,
                soc_parent: None,
                rm_type_name: "String".to_owned(),
                occurrences: None,
                node_id: String::new(),
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: Some("z".to_owned()),
                is_enumerated_type_constraint: None,
                constraint: Some(vec!["a".to_owned(), "b".to_owned()]),
            })]),
            differential_path: None,
            cardinality: None,
            is_multiple: false,
        });
    } else {
        panic!("expected a complex ELEMENT child");
    }
    let issues = validate_integrity(&a, None);
    assert_raises(&issues, ValidationCode::Vobav);
}

#[test]
fn vobav_ordered_assumed_value_outside_interval_constraint() {
    // master04.5 §C_PRIMITIVE_OBJECT / §C_ORDERED — VOBAV for an ordered
    // primitive: the assumed value must fall within some constraint interval.
    // A C_INTEGER constrained to [0..10] with assumed value 20 violates it.
    use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
    use openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
    use openehr_base::prelude::{Interval, ProperInterval, ProperIntervalData};
    let interval_0_10 =
        Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
            lower: Some(0),
            upper: Some(10),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }));
    let mut a = parse(BASE);
    let el = &mut root_data_mut(&mut a)
        .attributes
        .as_deref_mut()
        .unwrap_or_default()[0]
        .children
        .as_deref_mut()
        .unwrap_or_default()[0];
    if let CObject::CComplexObject(CComplexObject::CComplexObject(elem)) = el {
        elem.attributes.get_or_insert_default().push(CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name: "value".to_owned(),
            existence: None,
            children: Some(vec![CObject::CInteger(CInteger {
                parent: None,
                soc_parent: None,
                rm_type_name: "Integer".to_owned(),
                occurrences: None,
                node_id: String::new(),
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: Some(20.0),
                is_enumerated_type_constraint: None,
                constraint: Some(vec![interval_0_10]),
            })]),
            differential_path: None,
            cardinality: None,
            is_multiple: false,
        });
    } else {
        panic!("expected a complex ELEMENT child");
    }
    let issues = validate_integrity(&a, None);
    assert_raises(&issues, ValidationCode::Vobav);
}

#[test]
fn vrmvp_and_vrmvav_rm_overlay() {
    // master06 §Validity — VRMVP: an rm_visibility path referencing archetype
    // nodes must be valid; VRMVAV: an alias must be a defined at-code.
    use openehr_am::v2_4::aom2::rm_overlay::rm_attribute_visibility::RmAttributeVisibility;
    use openehr_am::v2_4::aom2::rm_overlay::rm_overlay::RmOverlay;
    use openehr_am::v2_4::aom2::rm_overlay::visibility_type::VisibilityType;
    use openehr_base::prelude::TerminologyCode;
    use std::collections::BTreeMap;

    let mut a = parse(BASE);
    let mut map = BTreeMap::new();
    map.insert(
        "/element_attr[id99]".to_owned(),
        RmAttributeVisibility {
            visibility: Some(VisibilityType::Hide),
            alias: Some(TerminologyCode {
                terminology_id: "local".to_owned(),
                terminology_version: None,
                code_string: "at999".to_owned(),
                uri: None,
            }),
        },
    );
    authored_mut(&mut a).rm_overlay = Some(RmOverlay {
        rm_visibility: Some(map),
    });
    let issues = validate_integrity(&a, None);
    assert_raises(&issues, ValidationCode::Vrmvp);
    assert_raises(&issues, ValidationCode::Vrmvav);
}

// ── source-driven cases needing tailored definitions ──────────────────────

#[test]
fn vdfai_invalid_slot_archetype_id() {
    // master04.5 §ARCHETYPE_SLOT — VDFAI: an archetype id in a slot assertion
    // must conform to the archetype-id form. `not_an_id` is a literal (no regex
    // meta-characters) that is not a valid archetype id.
    let src = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-SECTION.slot_vdfai.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"draft">

definition
    SECTION[id1] matches {
        items cardinality matches {0..*} matches {
            allow_archetype OBSERVATION[id2] matches {
                include
                    archetype_id/value matches {/not_an_id/}
            }
        }
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = < text = <"s"> description = <"s"> >
            ["id2"] = < text = <"o"> description = <"o"> >
        >
    >
"#;
    let issues = validate_source_integrity(src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Vdfai);
}

#[test]
fn varxav_invalid_external_reference() {
    // master08 §Phase 1 (C_ARCHETYPE_ROOT validity) — VARXAV: the external
    // reference of a C_ARCHETYPE_ROOT must be a valid archetype id. The parser
    // enforces a valid `use_archetype` id at parse time (SUAID/SUAIDI), so the
    // invalid-reference state is built by mutating a parsed C_ARCHETYPE_ROOT.
    use openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot;
    let src = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-SECTION.ext_varxav.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"draft">

definition
    SECTION[id1] matches {
        items matches {
            use_archetype CLUSTER[id2, openEHR-EHR-CLUSTER.thing.v1.0.0]
        }
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = < text = <"s"> description = <"s"> >
            ["id2"] = < text = <"c"> description = <"c"> >
        >
    >
"#;
    let mut a = parse(src);
    // reach the C_ARCHETYPE_ROOT child of `items` and corrupt its reference.
    let root = root_data_mut(&mut a);
    let child = &mut root.attributes.as_deref_mut().unwrap_or_default()[0]
        .children
        .as_deref_mut()
        .unwrap_or_default()[0];
    if let CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) = child {
        let r: &mut CArchetypeRoot = r;
        r.archetype_ref = "bogus_ref".to_owned();
    } else {
        panic!("expected a C_ARCHETYPE_ROOT child, got {child:?}");
    }
    let issues = validate_integrity(&a, None);
    assert_raises(&issues, ValidationCode::Varxav);
}

/// Source for a definition containing a `C_ARCHETYPE_ROOT` (a `use_archetype`
/// external reference).
const WITH_ARCHETYPE_ROOT: &str = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-SECTION.arch_root.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"draft">

definition
    SECTION[id1] matches {
        items matches {
            use_archetype CLUSTER[id2, openEHR-EHR-CLUSTER.thing.v1.0.0]
        }
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = < text = <"s"> description = <"s"> >
            ["id2"] = < text = <"c"> description = <"c"> >
        >
    >
"#;

fn archetype_root_child_mut(
    a: &mut Archetype,
) -> &mut openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot {
    let root = root_data_mut(a);
    let child = root
        .attributes
        .as_deref_mut()
        .unwrap_or_default()
        .first_mut()
        .and_then(|attr| attr.children.as_deref_mut().unwrap_or_default().first_mut())
        .expect("the fixture root has a first attribute with a first child");
    match child {
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => r,
        other => panic!("expected a C_ARCHETYPE_ROOT child, got {other:?}"),
    }
}

#[test]
fn varxnc_archetype_root_missing_node_id() {
    // master08 §Phase 1 (C_ARCHETYPE_ROOT validity) — VARXNC: a C_ARCHETYPE_ROOT
    // must carry a node id.
    let mut a = parse(WITH_ARCHETYPE_ROOT);
    archetype_root_child_mut(&mut a).node_id = String::new();
    let issues = validate_integrity(&a, None);
    assert_raises(&issues, ValidationCode::Varxnc);
}

#[test]
fn varxtv_archetype_root_missing_type() {
    // master08 §Phase 1 (C_ARCHETYPE_ROOT validity) — VARXTV: a C_ARCHETYPE_ROOT
    // must carry an RM type.
    let mut a = parse(WITH_ARCHETYPE_ROOT);
    archetype_root_child_mut(&mut a).rm_type_name = String::new();
    let issues = validate_integrity(&a, None);
    assert_raises(&issues, ValidationCode::Varxtv);
}

#[test]
fn vatcd_code_level_above_archetype_level() {
    // master03 §Validity Rules — VATCD: an at/ac code used in the definition
    // must have a specialisation level no greater than the archetype's level. A
    // non-specialised archetype (level 0) using the level-1 code `at0004.1`
    // violates it.
    let src = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-ENTRY.vatcd.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"draft">

definition
    ENTRY[id1] matches {
        value matches {[at0004.1]}
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = < text = <"r"> description = <"r"> >
            ["at0004.1"] = < text = <"v"> description = <"v"> >
        >
    >
"#;
    let issues = validate_source_integrity(src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Vatcd);
}

#[test]
fn vrrlp_rule_path_not_in_archetype() {
    // master03 §Validity Rules — VRRLP: a path mentioned in a rule must be found
    // within the archetype. `/element_attr[id99]` references a non-existent node.
    let src = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-ENTRY.vrrlp.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"draft">

definition
    ENTRY[id1] matches {
        element_attr matches {
            ELEMENT[id2]
        }
    }

rules
    /element_attr[id99]/value/magnitude = 1

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = < text = <"r"> description = <"r"> >
            ["id2"] = < text = <"e"> description = <"e"> >
        >
    >
"#;
    let issues = validate_source_integrity(src, Dialect::Adl2, None).unwrap();
    assert_raises(&issues, ValidationCode::Vrrlp);
}

// ── repository (parent) case ───────────────────────────────────────────────

#[test]
fn valc_language_not_in_parent() {
    // master03 §Validity Rules — VALC: a specialised archetype's languages must
    // be the same as, or a subset of, the flat parent's.
    let parent = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-ENTRY.valc_parent.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"draft">

definition
    ENTRY[id1]

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = < text = <"p"> description = <"p"> >
        >
    >
"#;
    // child declares a `de` translation the parent does not have.
    let child = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-ENTRY.valc_child.v1.0.1

specialize
    openEHR-EHR-ENTRY.valc_parent.v1

language
    original_language = <[ISO_639-1::en]>
    translations = <
        ["de"] = < language = <[ISO_639-1::de]> author = <["name"] = <"x">> >
    >

description
    lifecycle_state = <"draft">

definition
    ENTRY[id1.1]

terminology
    term_definitions = <
        ["en"] = <
            ["id1.1"] = < text = <"c"> description = <"c"> >
        >
        ["de"] = <
            ["id1.1"] = < text = <"c"> description = <"c"> >
        >
    >
"#;
    let mut repo = ArchetypeRepository::new();
    repo.insert(parse(parent));
    let issues = validate_source_integrity(child, Dialect::Adl2, Some(&repo)).unwrap();
    assert_raises(&issues, ValidationCode::Valc);
}
