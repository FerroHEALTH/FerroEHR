// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::constraint_status::ConstraintStatus;
use openehr_base::prelude::{Interval, ProperInterval};

use crate::parse::{Dialect, parse_definition_body};

fn parse(body: &str) -> CComplexObject {
    parse_definition_body(body, Dialect::Adl2).unwrap_or_else(|e| panic!("parse failed: {e:?}"))
}

fn data(cco: &CComplexObject) -> &CComplexObjectData {
    match cco {
        CComplexObject::CComplexObject(d) => d,
        CComplexObject::CArchetypeRoot(_) => panic!("expected plain complex object"),
    }
}

/// The `C_INTEGER` constraint list of the `index`-th attribute's first
/// child.
fn integer_constraints(d: &CComplexObjectData, index: usize) -> &[Interval<i32>] {
    match &d.attributes.as_deref().unwrap_or_default()[index]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CInteger(ci) => ci.constraint.as_deref().unwrap_or_default(),
        _ => panic!("expected CInteger"),
    }
}

/// The first constraint of the `index`-th attribute, as a proper interval.
fn integer_proper(
    d: &CComplexObjectData,
    index: usize,
) -> &openehr_base::v1_3::foundation_types::interval::proper_interval::ProperIntervalData<i32> {
    match &integer_constraints(d, index)[0] {
        Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => pi,
        _ => panic!("expected proper interval"),
    }
}

#[test]
fn integer_forms() {
    let cco = parse(
        "WHOLE[id1] matches {\n\
         a matches {55}\n\
         b matches {10, 20, 30}\n\
         c matches {|0..100|}\n\
         d matches {|>0..<100|}\n\
         e matches {|>=10|}\n\
         f matches {|-10..-5|}\n\
         }",
    );
    let d = data(&cco);
    // a: point 55
    match &integer_constraints(d, 0)[0] {
        Interval::PointInterval(p) => assert_eq!(p.lower, Some(55)),
        Interval::ProperInterval(_) => panic!("expected point"),
    }
    // b: three points
    assert_eq!(integer_constraints(d, 1).len(), 3);
    // c: |0..100|
    let c = integer_proper(d, 2);
    assert_eq!(c.lower, Some(0));
    assert_eq!(c.upper, Some(100));
    assert!(c.lower_included && c.upper_included);
    // d: |>0..<100| exclusive both
    let dd = integer_proper(d, 3);
    assert!(!dd.lower_included && !dd.upper_included);
    // e: |>=10| lower bounded, upper unbounded
    let e = integer_proper(d, 4);
    assert_eq!(e.lower, Some(10));
    assert!(e.lower_included);
    assert!(e.upper_unbounded);
    // f: |-10..-5| negative endpoints
    let f = integer_proper(d, 5);
    assert_eq!(f.lower, Some(-10));
    assert_eq!(f.upper, Some(-5));
}

#[test]
fn string_boolean_and_regex() {
    let cco = parse(
        "WHOLE[id1] matches {\n\
         s1 matches {\"something\"}\n\
         s2 matches {/cardio.*/}\n\
         s3 matches {\"a\", \"b\"}\n\
         b1 matches {True}\n\
         b2 matches {True, False}\n\
         }",
    );
    let d = data(&cco);
    match &d.attributes.as_deref().unwrap_or_default()[0]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => assert_eq!(cs.constraint, Some(vec!["something".to_owned()])),
        _ => panic!("expected CString"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[1]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => assert_eq!(cs.constraint, Some(vec!["/cardio.*/".to_owned()])),
        _ => panic!("expected CString regex"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[2]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => assert_eq!(cs.constraint.as_ref().map_or(0, Vec::len), 2),
        _ => panic!("expected CString list"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[3]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CBoolean(cb) => assert_eq!(cb.constraint, Some(vec![true])),
        _ => panic!("expected CBoolean"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[4]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CBoolean(cb) => assert_eq!(cb.constraint, Some(vec![true, false])),
        _ => panic!("expected CBoolean list"),
    }
}

#[test]
fn date_time_patterns_and_durations() {
    let cco = parse(
        "WHOLE[id1] matches {\n\
         d1 matches {yyyy-mm-??}\n\
         d2 matches {|2000-01-01..2000-02-01|}\n\
         t1 matches {hh:mm:ss}\n\
         dur1 matches {PWD}\n\
         dur2 matches {PWD/|P38W..P39W4D|}\n\
         dur3 matches {|<=PT1H|}\n\
         }",
    );
    let d = data(&cco);
    match &d.attributes.as_deref().unwrap_or_default()[0]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CDate(c) => assert_eq!(c.pattern_constraint.as_deref(), Some("yyyy-mm-??")),
        _ => panic!("expected CDate pattern"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[1]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CDate(c) => assert_eq!(c.constraint.as_ref().map_or(0, Vec::len), 1),
        _ => panic!("expected CDate interval"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[2]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CTime(c) => assert_eq!(c.pattern_constraint.as_deref(), Some("hh:mm:ss")),
        _ => panic!("expected CTime pattern"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[3]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CDuration(c) => {
            assert_eq!(c.pattern_constraint.as_deref(), Some("PWD"));
            assert!(c.constraint.as_ref().is_none_or(Vec::is_empty));
        }
        _ => panic!("expected CDuration pattern"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[4]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CDuration(c) => {
            assert_eq!(c.pattern_constraint.as_deref(), Some("PWD"));
            assert_eq!(c.constraint.as_ref().map_or(0, Vec::len), 1);
        }
        _ => panic!("expected CDuration pattern+interval"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[5]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CDuration(c) => assert_eq!(c.constraint.as_ref().map_or(0, Vec::len), 1),
        _ => panic!("expected CDuration interval"),
    }
}

#[test]
fn terminology_codes() {
    let cco = parse(
        "WHOLE[id1] matches {\n\
         a matches {[ac1]}\n\
         b matches {[at0004]}\n\
         c matches {[ac2; at0022]}\n\
         d matches {preferred [at0004]}\n\
         e matches {[ac1@snomed_ct]}\n\
         }",
    );
    let d = data(&cco);
    match &d.attributes.as_deref().unwrap_or_default()[0]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CTerminologyCode(t) => assert_eq!(t.constraint, "ac1"),
        _ => panic!("expected CTerminologyCode"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[2]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CTerminologyCode(t) => {
            assert_eq!(t.constraint, "ac2");
            assert_eq!(
                t.assumed_value.as_ref().map(|a| a.code_string.as_str()),
                Some("at0022")
            );
        }
        _ => panic!("expected CTerminologyCode with assumed"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[3]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CTerminologyCode(t) => {
            assert_eq!(t.constraint_status, Some(ConstraintStatus::Preferred));
        }
        _ => panic!("expected CTerminologyCode with strength"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[4]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CTerminologyCode(t) => assert_eq!(t.constraint, "ac1@snomed_ct"),
        _ => panic!("expected CTerminologyCode with binding"),
    }
}

#[test]
fn regular_primitive_type_object() {
    let cco = parse("WHOLE[id1] matches {\n a matches {\n String [id2]\n }\n}");
    let d = data(&cco);
    match &d.attributes.as_deref().unwrap_or_default()[0]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => {
            assert_eq!(cs.node_id, "id2");
            assert_eq!(cs.rm_type_name, "String");
            assert!(cs.constraint.as_ref().is_none_or(Vec::is_empty));
        }
        _ => panic!("expected regular primitive CString"),
    }
}

#[test]
fn assumed_values() {
    let cco = parse("WHOLE[id1] matches {\n a matches {|0..10|; 5}\n s matches {\"x\"; \"y\"}\n}");
    let d = data(&cco);
    match &d.attributes.as_deref().unwrap_or_default()[0]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CInteger(ci) => assert_eq!(ci.assumed_value, Some(5.0)),
        _ => panic!("expected CInteger with assumed"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[1]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => assert_eq!(cs.assumed_value.as_deref(), Some("y")),
        _ => panic!("expected CString with assumed"),
    }
}

/// `ADL1.4/master05-cadl.adoc` §Regular Expression L696-702: the `^…^` and
/// the backslash-escaped `/…/` spellings "are equivalent", so the caret form
/// normalises onto the AOM's `/`-delimited carrier WITH the inner delimiters
/// escaped — and the result re-parses to itself (parse → print → parse is
/// lossless; the printer emits `C_STRING.constraint` verbatim).
#[test]
fn caret_regex_normalises_losslessly() {
    let cco = parse("WHOLE[id1] matches {\n u matches {^km/h|mi/h^}\n}");
    let printed = match &data(&cco).attributes.as_deref().unwrap_or_default()[0]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => cs.constraint.as_deref().unwrap_or_default()[0].clone(),
        other => panic!("expected CString regex, got {other:?}"),
    };
    assert_eq!(printed, r"/km\/h|mi\/h/");
    // The chapter's own equivalent slash spelling yields the same carrier…
    let slash = parse(
        r"WHOLE[id1] matches {\n u matches {/km\/h|mi\/h/}\n}"
            .replace("\\n", "\n")
            .as_str(),
    );
    match &data(&slash).attributes.as_deref().unwrap_or_default()[0]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => {
            assert_eq!(cs.constraint.as_deref().unwrap_or_default()[0], printed);
        }
        other => panic!("expected CString regex, got {other:?}"),
    }
    // …and re-parsing the printed form reproduces it unchanged.
    let again = parse(&format!(
        "WHOLE[id1] matches {{\n u matches {{{printed}}}\n}}"
    ));
    match &data(&again).attributes.as_deref().unwrap_or_default()[0]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => {
            assert_eq!(cs.constraint.as_deref().unwrap_or_default()[0], printed);
        }
        other => panic!("expected CString regex, got {other:?}"),
    }
}

/// `ADL1.4/master05-cadl.adoc` §Constraints on Character L813-825 —
/// `color_name matches {'r', 'g', 'b'}`. No AOM generation defines a
/// `C_CHARACTER`, so the carrier is `C_STRING` with the constrained RM type
/// name kept (see [`Parser::parse_c_character`]).
#[test]
fn character_lists_land_on_c_string() {
    let cco = parse(
        "WHOLE[id1] matches {\n\
         a matches {'r'}\n\
         b matches {'r', 'g', 'b'}\n\
         c matches {'r', 'g'; 'r'}\n\
         }",
    );
    let d = data(&cco);
    match &d.attributes.as_deref().unwrap_or_default()[0]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => {
            assert_eq!(cs.rm_type_name, "Character");
            assert_eq!(cs.constraint, Some(vec!["r".to_owned()]));
        }
        other => panic!("expected CString, got {other:?}"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[1]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => assert_eq!(cs.constraint.as_ref().map_or(0, Vec::len), 3),
        other => panic!("expected CString, got {other:?}"),
    }
    match &d.attributes.as_deref().unwrap_or_default()[2]
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CString(cs) => assert_eq!(cs.assumed_value.as_deref(), Some("r")),
        other => panic!("expected CString, got {other:?}"),
    }
}
