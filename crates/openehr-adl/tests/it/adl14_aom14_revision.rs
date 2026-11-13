// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two AOM 1.4 model changes of schema revision `1.4.0.3`, as the ADL 1.4
//! reader meets them.
//!
//! - SPECAM-109: `C_QUANTITY_ITEM` gains the optional `units_system` and
//!   `units_display_name` (`AM/docs/AOM1.4/master00-amendment_record.adoc`
//!   1.4.7, `AM/docs/UML/classes/org.openehr.am.aom14.c_quantity_item.adoc`).
//!   A 1.4 `C_DV_QUANTITY` list row carrying them lowers to `DV_QUANTITY`
//!   tuple members of the same names, the RM attributes they mirror.
//! - SPECPR-515: `ARCHETYPE.uid` is the `UUID` inherited from BASE
//!   `AUTHORED_RESOURCE`, and "ISO OIDs, internet identifiers and extended
//!   (`root::extension`) forms of `HIER_OBJECT_ID` are not used"
//!   (`AM/docs/AOM1.4/master03-archetype_package.adoc` §Machine Identifier).
//!   A UUID fills `uid`; any other form never does, and is kept verbatim in
//!   `other_meta_data["uid"]` so the reader loses nothing it was given.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::adl14::convert::{ConvertConfig, parse_and_convert};
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::print::print;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};

/// A 1.4 archetype whose header `uid` is `UID` and whose one element holds a
/// `C_DV_QUANTITY` with the SPECAM-109 attributes on its list row.
fn source(uid: &str) -> String {
    format!(
        "\
archetype (adl_version=1.4; uid={uid})
\topenEHR-EHR-CLUSTER.aom14_revision.v1

concept
\t[at0000]\t-- AOM 1.4 revision 1.4.0.3

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"AuthorDraft\">

definition
\tCLUSTER[at0000] matches {{
\t\titems cardinality matches {{0..*; unordered}} matches {{
\t\t\tELEMENT[at0001] occurrences matches {{0..1}} matches {{
\t\t\t\tvalue matches {{
\t\t\t\t\tC_DV_QUANTITY <
\t\t\t\t\t\tproperty = <[openehr::125]>
\t\t\t\t\t\tlist = <
\t\t\t\t\t\t\t[\"1\"] = <
\t\t\t\t\t\t\t\tunits = <\"[lb_av]\">
\t\t\t\t\t\t\t\tunits_system = <\"http://unitsofmeasure.org\">
\t\t\t\t\t\t\t\tunits_display_name = <\"lb\">
\t\t\t\t\t\t\t>
\t\t\t\t\t\t>
\t\t\t\t\t>
\t\t\t\t}}
\t\t\t}}
\t\t}}
\t}}

ontology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"root\"> description=<\"root\">>
\t\t\t\t[\"at0001\"] = <text=<\"weight\"> description=<\"weight\">>
\t\t\t>
\t\t>
\t>
"
    )
}

/// The `AUTHORED_ARCHETYPE` data of an assembled archetype.
fn authored(archetype: &Archetype) -> &AuthoredArchetypeData {
    match archetype {
        Archetype::AuthoredArchetype(a) => match a.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => d,
            other
            @ (AuthoredArchetype::Template(_) | AuthoredArchetype::OperationalTemplate(_)) => {
                panic!("expected a plain AUTHORED_ARCHETYPE, got {other:?}")
            }
        },
        other @ Archetype::TemplateOverlay(_) => {
            panic!("expected an AUTHORED_ARCHETYPE, got {other:?}")
        }
    }
}

const UUID: &str = "15e82d77-7dbb-4f70-8d8e-eed6ff241b2d";

#[test]
fn a_uuid_uid_fills_the_machine_identifier() {
    let art = parse_artefact(&source(UUID), Dialect::Adl14).expect("the 1.4 source parses");
    let data = authored(&art);
    let uid = data.uid.as_ref().expect("a UUID uid fills `uid`");
    assert_eq!(uid.value().to_string(), UUID);
    assert!(!data.other_meta_data.contains_key("uid"));
}

#[test]
fn an_oid_uid_is_no_machine_identifier_and_is_kept_verbatim() {
    let oid = "2.16.840.1.113883.6.96";
    let art = parse_artefact(&source(oid), Dialect::Adl14).expect("the 1.4 source parses");
    let data = authored(&art);
    assert_eq!(data.uid, None, "an OID never fills the UUID `uid`");
    assert_eq!(
        data.other_meta_data.get("uid").map(String::as_str),
        Some(oid)
    );
}

#[test]
fn a_quantity_item_units_system_and_display_name_reach_the_converted_tuple() {
    let mut log = ConversionLog::default();
    let converted = parse_and_convert(&source(UUID), &ConvertConfig::default(), &mut log)
        .expect("the 1.4 archetype converts");
    let adl2 = print(&converted).expect("the converted archetype prints");
    let tuple = adl2
        .lines()
        .find(|line| line.contains("units_system"))
        .unwrap_or_else(|| panic!("no units_system tuple member in:\n{adl2}"));
    assert!(
        tuple.contains("units_display_name"),
        "the three list-row members co-vary in one tuple: {tuple}"
    );
    assert!(
        adl2.contains("\"http://unitsofmeasure.org\"") && adl2.contains("\"lb\""),
        "the row's values constrain the tuple:\n{adl2}"
    );
}
