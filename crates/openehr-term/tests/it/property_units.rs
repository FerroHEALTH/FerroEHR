// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The vendored Support Terminology carries the TERM 3.1.0 amendment 3.1.4
//! (SPECPR-450) measurement properties.
//!
//! `docs/specs/openehr/TERM/docs/SupportTerminology/master00-amendment_record.adoc`
//! records the amendment, and `codesets/openehr_terminology-vocabularies.adoc`
//! lists the `property` group rows 761 to 765. The bundle reads both facts out
//! of the embedded computable assets, never out of a hand-written list.

use openehr_term::bundle::openehr;

/// Concepts 761 to 765 are members of the openEHR `property` group.
#[test]
fn specpr_450_properties_are_in_the_property_group() {
    let bundle = openehr();
    for (code, rubric) in [
        ("761", "Flow rate (volume) per area"),
        ("762", "Volume per area"),
        ("763", "Mitotic count"),
        ("764", "Length per area"),
        ("765", "Concentration per volume"),
    ] {
        assert!(
            bundle.is_valid_code("property", code),
            "{code} must be a property"
        );
        assert_eq!(
            bundle.rubric("property", code, "en"),
            Some(rubric),
            "{code}"
        );
    }
    assert!(!bundle.is_valid_code("property", "766"));
}

/// `PropertyUnitData.xml` gives property 765 its two concentration-per-volume
/// units and property 764 its length-per-area unit, each with its UCUM code.
#[test]
fn specpr_450_units_are_in_the_property_unit_table() {
    let bundle = openehr();
    let ucum = |code: &str| -> Vec<Option<String>> {
        bundle
            .units_for_property(code)
            .iter()
            .map(|u| u.ucum.clone())
            .collect()
    };
    let concentration = ucum("765");
    for expected in ["ng/mL/cm3", "ug/L/cm3"] {
        assert!(
            concentration.contains(&Some(expected.to_owned())),
            "765 must carry {expected}: {concentration:?}"
        );
    }
    assert!(
        ucum("764").contains(&Some("mm/m2".to_owned())),
        "764 must carry mm/m2"
    );
    let texts: Vec<&str> = bundle
        .units_for_property("765")
        .iter()
        .map(|u| u.text.as_str())
        .collect();
    assert!(texts.contains(&"ng/mL/cm³"), "{texts:?}");
    assert!(texts.contains(&"μg/L/cm³"), "{texts:?}");
}
