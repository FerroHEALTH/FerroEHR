// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 custom constraint forms of
//! `docs/specs/openehr/AM/docs/ADL1.4/master09-customising_adl.adoc`
//! ("Customising ADL") — the two spellings openEHR profiled onto standard ADL,
//! and the boundary where neither is defined.
//!
//! 1. **The inline dADL section.** §Introduction admits any `C_DOMAIN_TYPE`
//!    descendant as a typed dADL block inside the `{}` where its standard-ADL
//!    equivalent would stand; §Custom Syntax works `C_CODE_PHRASE` through in
//!    full and states that the block and the compact `[local:: at0039, at0040]`
//!    form "express exactly the same constraint" — so the two must lower to the
//!    same constraint object, which is what these tests pin structurally.
//! 2. **The pipe-ordinal shorthand.** `ADL2/master04.4-cadl_second_order.adoc`
//!    §Tuple Constraints records `0|[local::at1], 1|[local::at2]` as the
//!    deprecated openEHR-profiled 1.4 syntax and names the generic
//!    `DV_ORDINAL` `[value, symbol]` tuple as its replacement ("This hides the
//!    `DV_ORDINAL` type altogether"). It is accepted in the 1.4 dialect and
//!    lowered to exactly that replacement; ADL 2, which removed it, still
//!    refuses it — both twins are pinned here.
//! 3. **The refusals.** A domain constrainer with no vendored shape
//!    (`C_DV_STATE`) and a malformed `C_CODE_PHRASE` block are typed refusals,
//!    never a silent guess.
//!
//! The grammar productions cited (`c_ordinal`, `ordinal_term`,
//! `assumed_ordinal_value`, `domain_specific_extension`, `c_terminology_code`)
//! are the vendored normative ANTLR set at `vendor/grammar/v1_4/cadl14.g4` +
//! `cadl14_primitives.g4`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::adl14::convert::{ConvertConfig, parse_and_convert};
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::error::{SyntaxError, SyntaxErrorCode};
use openehr_adl::parse::Dialect;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::validate_source_integrity;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;

/// A whole ADL 1.4 archetype whose single `ELEMENT[at0001]/value` carries
/// `constraint` verbatim, with `at0039`/`at0040` defined in the ontology.
fn archetype_with(constraint: &str) -> String {
    format!(
        "archetype (adl_version=1.4)
\topenEHR-EHR-CLUSTER.custom_constraint14.v1

concept
\t[at0000]\t-- Custom constraint
language
\toriginal_language = <[ISO_639-1::en]>
description
\tlifecycle_state = <\"AuthorDraft\">
definition
\tCLUSTER[at0000] matches {{
\t\titems matches {{
\t\t\tELEMENT[at0001] matches {{
\t\t\t\tvalue matches {{
{constraint}
\t\t\t\t}}
\t\t\t}}
\t\t}}
\t}}
ontology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"Custom constraint\"> description=<\"root\">>
\t\t\t\t[\"at0001\"] = <text=<\"Item\"> description=<\"an item\">>
\t\t\t\t[\"at0039\"] = <text=<\"Lying\"> description=<\"lying\">>
\t\t\t\t[\"at0040\"] = <text=<\"Sitting\"> description=<\"sitting\">>
\t\t\t>
\t\t>
\t>
"
    )
}

/// The single object constraining `ELEMENT[at0001]/value` in a parsed 1.4
/// archetype built by [`archetype_with`].
fn value_object(src: &str) -> CObject {
    let archetype =
        parse_artefact(src, Dialect::Adl14).unwrap_or_else(|e| panic!("must parse: {e:?}"));
    let Archetype::AuthoredArchetype(authored) = archetype else {
        panic!("not an authored archetype");
    };
    let AuthoredArchetype::AuthoredArchetype(data) = *authored else {
        panic!("not a plain authored archetype");
    };
    let items = openehr_adl::aom::access::complex_attributes(&data.definition)
        .iter()
        .find(|a| a.rm_attribute_name == "items")
        .expect("the items attribute");
    let element = items
        .children
        .iter()
        .flatten()
        .next()
        .expect("the ELEMENT child");
    let CObject::CComplexObject(element) = element else {
        panic!("the ELEMENT child is not a complex object");
    };
    openehr_adl::aom::access::complex_attributes(element)
        .iter()
        .find(|a| a.rm_attribute_name == "value")
        .expect("the value attribute")
        .children
        .iter()
        .flatten()
        .next()
        .expect("the value constraint")
        .clone()
}

/// The refusal codes a 1.4 source raises at parse.
fn refusal_codes(src: &str) -> Vec<SyntaxErrorCode> {
    let errors: Vec<SyntaxError> = parse_artefact(src, Dialect::Adl14)
        .err()
        .unwrap_or_else(|| panic!("must be refused at parse"));
    errors.iter().map(|e| e.code).collect()
}

/// The single `[value, symbol]` attribute tuple of a lowered `DV_ORDINAL`.
fn ordinal_tuple(obj: &CObject) -> &CAttributeTuple {
    let CObject::CComplexObject(CComplexObject::CComplexObject(data)) = obj else {
        panic!("the ordinal must lower to a plain C_COMPLEX_OBJECT, got {obj:?}");
    };
    assert_eq!(
        data.rm_type_name, "DV_ORDINAL",
        "the ordinal lowers onto the DV_ORDINAL reference-model type"
    );
    assert!(
        data.attributes.as_ref().is_none_or(Vec::is_empty),
        "the ordinal tuple is the whole constraint; no plain attribute is emitted"
    );
    assert_eq!(
        data.attribute_tuples.as_ref().map_or(0, Vec::len),
        1,
        "exactly one tuple"
    );
    data.attribute_tuples
        .iter()
        .flatten()
        .next()
        .expect("the one tuple")
}

/// The `(value, symbol-constraint)` pairs of a lowered ordinal tuple.
fn ordinal_rows(tuple: &CAttributeTuple) -> Vec<(f64, String)> {
    tuple
        .tuples
        .iter()
        .flatten()
        .map(|row| {
            let value = match row.members.iter().next().expect("the value member") {
                CPrimitiveObject::CInteger(c) => match c.constraint.iter().flatten().next() {
                    Some(openehr_base::prelude::Interval::PointInterval(p)) => {
                        f64::from(p.lower.expect("a point ordinal value"))
                    }
                    other => panic!("the ordinal value is not a point constraint: {other:?}"),
                },
                CPrimitiveObject::CReal(c) => match c.constraint.iter().flatten().next() {
                    Some(openehr_base::prelude::Interval::PointInterval(p)) => {
                        p.lower.expect("a point ordinal value")
                    }
                    other => panic!("the ordinal value is not a point constraint: {other:?}"),
                },
                other => panic!("the ordinal value member is not numeric: {other:?}"),
            };
            let Some(CPrimitiveObject::CTerminologyCode(symbol)) = row.members.get(1) else {
                panic!("the ordinal symbol member is not a terminology code");
            };
            (value, symbol.constraint.clone())
        })
        .collect()
}

// ── 1. the inline dADL C_CODE_PHRASE section ────────────────────────────────

/// The chapter's own worked example (§Custom Syntax) and the compact custom
/// syntax it is presented as an alternative spelling of ("While these two ADL
/// fragments express exactly the same constraint…") must produce the SAME
/// constraint object — structurally, not merely both-parse.
#[test]
fn code_phrase_dadl_block_lowers_like_the_custom_syntax() {
    let dadl = value_object(&archetype_with(
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <
\t\t\t\t\t\t\tvalue = <\"local\">
\t\t\t\t\t\t>
\t\t\t\t\t\tcode_list = <
\t\t\t\t\t\t\t[\"1\"] = <\"at0039\">
\t\t\t\t\t\t\t[\"2\"] = <\"at0040\">
\t\t\t\t\t\t>
\t\t\t\t\t>",
    ));
    let custom = value_object(&archetype_with(
        "\t\t\t\t\t[local::\n\t\t\t\t\tat0039,\n\t\t\t\t\tat0040]",
    ));
    assert_eq!(
        dadl, custom,
        "the dADL block and the custom syntax must lower identically"
    );
    let CObject::CTerminologyCode(code) = &dadl else {
        panic!("a CODE_PHRASE constraint lowers to a C_TERMINOLOGY_CODE, got {dadl:?}");
    };
    assert_eq!(code.constraint, "local::at0039,at0040");
    assert_eq!(code.rm_type_name, "Terminology_code");
}

/// The parenthesised spelling of the same block (`(C_CODE_PHRASE) <…>`, the
/// ODIN type cast of `LANG/docs/odin/master05-content.adoc` §Adding Type
/// Information) lowers identically to the bare one.
#[test]
fn parenthesised_code_phrase_block_lowers_the_same() {
    let bare = value_object(&archetype_with(
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\tcode_list = <[\"1\"] = <\"at0039\">>
\t\t\t\t\t>",
    ));
    let cast = value_object(&archetype_with(
        "\t\t\t\t\t(C_CODE_PHRASE) <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\tcode_list = <[\"1\"] = <\"at0039\">>
\t\t\t\t\t>",
    ));
    assert_eq!(bare, cast);
}

/// `AOM1.4/master04-constraint_model_package.adoc` §`Assumed_value` puts assumed
/// values on `C_DOMAIN_TYPE` descendants, and the profiled `C_CODE_PHRASE`
/// carries a whole `CODE_PHRASE` there. It lowers to the `;code` tail the
/// custom syntax spells with `; at0039]`.
#[test]
fn code_phrase_dadl_assumed_value_reaches_the_constraint() {
    let dadl = value_object(&archetype_with(
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\tcode_list = <
\t\t\t\t\t\t\t[\"1\"] = <\"at0039\">
\t\t\t\t\t\t\t[\"2\"] = <\"at0040\">
\t\t\t\t\t\t>
\t\t\t\t\t\tassumed_value = <
\t\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\t\tcode_string = <\"at0039\">
\t\t\t\t\t\t>
\t\t\t\t\t>",
    ));
    let custom = value_object(&archetype_with(
        "\t\t\t\t\t[local:: at0039, at0040; at0039]",
    ));
    assert_eq!(dadl, custom);
    let CObject::CTerminologyCode(code) = &dadl else {
        panic!("expected a C_TERMINOLOGY_CODE");
    };
    assert_eq!(code.constraint, "local::at0039,at0040;at0039");
}

/// The codes the block names are real archetype terms: an undefined one raises
/// VATDF exactly as it does through the custom syntax
/// (`ADL1.4/master08-adl.adoc` §Validity Rules VATDF).
#[test]
fn code_phrase_dadl_undefined_code_raises_vatdf() {
    let src = archetype_with(
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\tcode_list = <
\t\t\t\t\t\t\t[\"1\"] = <\"at0039\">
\t\t\t\t\t\t\t[\"2\"] = <\"at9999\">
\t\t\t\t\t\t>
\t\t\t\t\t>",
    );
    let codes: Vec<ValidationCode> = validate_source_integrity(&src, Dialect::Adl14, None)
        .expect("the block parses")
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect();
    assert!(
        codes.contains(&ValidationCode::Vatdf),
        "an undefined lowered code must raise VATDF, got {codes:?}"
    );
}

/// An external terminology in the block is not an archetype term, so the
/// definedness rules stay quiet (the same reading the custom syntax gets).
#[test]
fn code_phrase_dadl_external_terminology_is_not_an_archetype_term() {
    let src = archetype_with(
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"SNOMED-CT\">>
\t\t\t\t\t\tcode_list = <[\"1\"] = <\"163035008\">>
\t\t\t\t\t>",
    );
    let codes: Vec<ValidationCode> = validate_source_integrity(&src, Dialect::Adl14, None)
        .expect("the block parses")
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect();
    assert!(codes.is_empty(), "expected clean, got {codes:?}");
    let CObject::CTerminologyCode(code) = &value_object(&src) else {
        panic!("expected a C_TERMINOLOGY_CODE");
    };
    assert_eq!(code.constraint, "SNOMED-CT::163035008");
}

/// A block that is not a `C_CODE_PHRASE` instance is refused loudly, never
/// guessed at: the chapter requires the dADL instance to "obey the semantics of
/// the custom type of which it is an instance, i.e. include the correct
/// attribute names and relationships" (§Introduction).
#[test]
fn malformed_code_phrase_blocks_are_refused() {
    let cases = [
        // No terminology at all.
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tcode_list = <[\"1\"] = <\"at0039\">>
\t\t\t\t\t>",
        // No code list.
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t>",
        // An empty code list.
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\tcode_list = <>
\t\t\t\t\t>",
        // An attribute the class does not define.
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\tcode_list = <[\"1\"] = <\"at0039\">>
\t\t\t\t\t\tcode_string = <\"at0039\">
\t\t\t\t\t>",
        // An assumed value in another terminology than the constraint.
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\tcode_list = <[\"1\"] = <\"at0039\">>
\t\t\t\t\t\tassumed_value = <
\t\t\t\t\t\t\tterminology_id = <value = <\"SNOMED-CT\">>
\t\t\t\t\t\t\tcode_string = <\"at0039\">
\t\t\t\t\t\t>
\t\t\t\t\t>",
    ];
    for case in cases {
        let codes = refusal_codes(&archetype_with(case));
        assert!(
            codes.contains(&SyntaxErrorCode::Sdinv),
            "expected SDINV for {case}, got {codes:?}"
        );
    }
}

/// The list rules that govern the custom syntax govern the block too — the two
/// spellings are the same constraint, so they are judged the same way
/// (STCAC: `ADL2/master04.6-cadl_validity_rules.adoc` §Syntax Validity Rules).
#[test]
fn code_phrase_dadl_assumed_value_outside_the_list_raises_stcac() {
    let codes = refusal_codes(&archetype_with(
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\tcode_list = <[\"1\"] = <\"at0039\">>
\t\t\t\t\t\tassumed_value = <
\t\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\t\tcode_string = <\"at0040\">
\t\t\t\t\t\t>
\t\t\t\t\t>",
    ));
    assert!(
        codes.contains(&SyntaxErrorCode::Stcac),
        "expected STCAC, got {codes:?}"
    );
}

// ── 2. the pipe-ordinal shorthand ───────────────────────────────────────────

/// `ADL2/master04.4-cadl_second_order.adoc` §Tuple Constraints, deprecated
/// block: `0|[local::at1], 1|[local::at2], 2|[local::at3]`. It lowers to the
/// generic replacement the same section gives — a `DV_ORDINAL` with a
/// `[value, symbol]` tuple, one row per term.
#[test]
fn pipe_ordinal_builds_the_value_symbol_tuple() {
    let obj = value_object(&archetype_with(
        "\t\t\t\t\t0|[local::at0039],\n\t\t\t\t\t1|[local::at0040]",
    ));
    let tuple = ordinal_tuple(&obj);
    let members: Vec<&str> = tuple
        .members
        .iter()
        .flatten()
        .map(|m| m.rm_attribute_name.as_str())
        .collect();
    assert_eq!(members, vec!["value", "symbol"]);
    assert_eq!(
        ordinal_rows(tuple),
        vec![
            (0.0, "local::at0039".to_owned()),
            (1.0, "local::at0040".to_owned()),
        ]
    );
}

/// A three-term list with an external-terminology symbol: `c_terminology_code`
/// (`cadl14_primitives.g4`) admits any qualified code, and the definedness
/// rules leave a non-archetype terminology alone.
#[test]
fn pipe_ordinal_accepts_many_terms_and_external_symbols() {
    let src = archetype_with(
        "\t\t\t\t\t0|[local::at0039],\n\t\t\t\t\t1|[local::at0040],\n\t\t\t\t\t2|[SNOMED-CT::163035008]",
    );
    let obj = value_object(&src);
    assert_eq!(
        ordinal_rows(ordinal_tuple(&obj)),
        vec![
            (0.0, "local::at0039".to_owned()),
            (1.0, "local::at0040".to_owned()),
            (2.0, "SNOMED-CT::163035008".to_owned()),
        ]
    );
    let codes: Vec<ValidationCode> = validate_source_integrity(&src, Dialect::Adl14, None)
        .expect("parses")
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect();
    assert!(codes.is_empty(), "expected clean, got {codes:?}");
}

/// The lowered symbols ARE archetype terms: an undefined at-code raises VATDF
/// through the tuple, the same as any other 1.4 term constraint.
#[test]
fn pipe_ordinal_undefined_symbol_raises_vatdf() {
    let src = archetype_with("\t\t\t\t\t0|[local::at0039],\n\t\t\t\t\t1|[local::at9999]");
    let codes: Vec<ValidationCode> = validate_source_integrity(&src, Dialect::Adl14, None)
        .expect("parses")
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect();
    assert!(
        codes.contains(&ValidationCode::Vatdf),
        "expected VATDF, got {codes:?}"
    );
}

/// `cadl14.g4` `c_ordinal : ordinal_term (',' ordinal_term)* (';'
/// assumed_ordinal_value)?` — the tail names one of the listed ordinals, and
/// lands on that row's own value constraint.
#[test]
fn pipe_ordinal_assumed_value_lands_on_its_own_row() {
    let obj = value_object(&archetype_with(
        "\t\t\t\t\t0|[local::at0039],\n\t\t\t\t\t1|[local::at0040]; 1",
    ));
    let tuple = ordinal_tuple(&obj);
    let assumed: Vec<Option<f64>> = tuple
        .tuples
        .iter()
        .flatten()
        .map(
            |row| match row.members.iter().next().expect("the value member") {
                CPrimitiveObject::CInteger(c) => c.assumed_value,
                other => panic!("not an integer ordinal value: {other:?}"),
            },
        )
        .collect();
    assert_eq!(
        assumed,
        vec![None, Some(1.0)],
        "the assumed value belongs to exactly the ordinal it names"
    );
}

/// An assumed ordinal value naming no listed term is refused loudly rather than
/// bound to an arbitrary row (`ADL1.4/master05-cadl.adoc` §Assumed Values
/// L1012: the assumed value is "of the same type as that implied by the
/// preceding part of the constraint").
#[test]
fn pipe_ordinal_assumed_value_outside_the_list_is_refused() {
    let codes = refusal_codes(&archetype_with(
        "\t\t\t\t\t0|[local::at0039],\n\t\t\t\t\t1|[local::at0040]; 7",
    ));
    assert!(
        codes.contains(&SyntaxErrorCode::Sciav),
        "expected SCIAV, got {codes:?}"
    );
}

/// The pipe form is ADL 1.4-ONLY. ADL 2 removed it (§Tuple Constraints: the
/// custom syntax "is now replaced by the above generic form"), so the ADL2
/// entry point must keep refusing the identical text.
#[test]
fn pipe_ordinal_is_refused_by_the_adl2_dialect() {
    let src = archetype_with("\t\t\t\t\t0|[local::at0039],\n\t\t\t\t\t1|[local::at0040]");
    assert!(
        parse_artefact(&src, Dialect::Adl2).is_err(),
        "the deprecated 1.4 pipe-ordinal must not parse as ADL 2"
    );
}

// ── the 1.4→2 conversion of both lowered forms ──────────────────────────────

/// Both custom forms survive the whole 1.4 pipeline: they convert to ADL 2 and
/// print as the constructs ADL 2 defines for them — the qualified codes become
/// archetype-local at-codes with a terminology binding, and the ordinal keeps
/// the `[value, symbol]` tuple `ADL2/master04.4-cadl_second_order.adoc`
/// §Tuple Constraints prescribes. (No openEHR spec governs 1.4→2 conversion —
/// our own design; this pins the shapes, not a spec clause.)
#[test]
fn both_custom_forms_convert_to_adl2_and_print() {
    for constraint in [
        "\t\t\t\t\tC_CODE_PHRASE <
\t\t\t\t\t\tterminology_id = <value = <\"local\">>
\t\t\t\t\t\tcode_list = <[\"1\"] = <\"at0039\"> [\"2\"] = <\"at0040\">>
\t\t\t\t\t>",
        "\t\t\t\t\t0|[local::at0039],\n\t\t\t\t\t1|[local::at0040]",
    ] {
        let mut log = ConversionLog::new();
        let converted = parse_and_convert(
            &archetype_with(constraint),
            &ConvertConfig::default(),
            &mut log,
        )
        .unwrap_or_else(|e| panic!("{constraint} must convert: {e:?}"));
        let printed = openehr_adl::print::print(&converted).expect("print the converted artefact");
        assert!(
            parse_artefact(&printed, Dialect::Adl2).is_ok(),
            "the converted ADL 2 text must re-parse as ADL 2:\n{printed}"
        );
    }
}

// ── 3. the adjudicated refusal ──────────────────────────────────────────────

/// `C_DV_STATE` appears nowhere in the vendored openEHR spec text — the oAP
/// custom type is not vendored, and `AOM1.4/masterAppA-domain_extension.adoc`
/// works only `C_ORDINAL`/`C_QUANTITY`/`C_CODED_TEXT` — so it has no citable
/// shape and stays a typed refusal naming the type.
#[test]
fn c_dv_state_stays_refused_by_name() {
    let src = archetype_with(
        "\t\t\t\t\t(C_DV_STATE) <
\t\t\t\t\t\tvalue = <\"initial\">
\t\t\t\t\t>",
    );
    let errors = parse_artefact(&src, Dialect::Adl14)
        .err()
        .unwrap_or_else(|| panic!("C_DV_STATE must be refused"));
    assert!(
        errors.iter().any(|e| e.code == SyntaxErrorCode::Sdinv),
        "expected SDINV, got {:?}",
        errors.iter().map(|e| e.code).collect::<Vec<_>>()
    );
    assert!(
        errors.iter().any(|e| e.to_string().contains("C_DV_STATE")),
        "the refusal must name the unsupported type: {errors:?}"
    );
}
