// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 → ADL 2 meta-data rules of `AOM2/master01-preface.adoc`
//! §Release 2.3 changes (SPECAM-107):
//!
//! 1. A translation's `other_details["other_contributors"]`, one translator per
//!    line, maps to one `TRANSLATION_DETAILS.other_contributors` entry per line
//!    (also `ADL1.4/master08-adl.adoc` §Language Section and Language
//!    Translation).
//! 2. The original language's copyright becomes `RESOURCE_DESCRIPTION.copyright`;
//!    a differing translation copyright stays in that translation's
//!    `other_details["copyright"]`, an identical one is dropped, and with no
//!    original-language copyright the field stays empty.
//! 3. The conversion is recorded in `RESOURCE_DESCRIPTION.conversion_details`
//!    under `"source_model"`, `"tool"` and `"time"`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::BTreeMap;
use std::fmt::Write;

use openehr_adl::adl14::convert::{ConvertConfig, parse_and_convert};
use openehr_adl::adl14::log::ConversionLog;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::resource::resource_description::ResourceDescription;

/// The extra rows of one ADL 1.4 description item: a `copyright` entry is the
/// item's own `copyright` attribute (AOM 1.4 `RESOURCE_DESCRIPTION_ITEM`),
/// every other entry an `other_details` row.
fn item_rows(entries: &[(&str, &str)]) -> String {
    let mut attribute = String::new();
    let mut rows = String::new();
    for (key, value) in entries {
        if *key == "copyright" {
            writeln!(attribute, "\t\t\tcopyright = <\"{value}\">").expect("writing to a String");
        } else {
            writeln!(rows, "\t\t\t\t[\"{key}\"] = <\"{value}\">").expect("writing to a String");
        }
    }
    if rows.is_empty() {
        attribute
    } else {
        format!("{attribute}\t\t\tother_details = <\n{rows}\t\t\t>\n")
    }
}

/// A 1.4 archetype in English with a German translation. `contributors` is the
/// raw `other_contributors` value of the translation; `en` and `de` are the
/// [`item_rows`] of each language's description item.
fn source(contributors: Option<&str>, en: &[(&str, &str)], de: &[(&str, &str)]) -> String {
    let translation_details = contributors.map_or_else(String::new, |raw| {
        format!(
            "\t\t\tother_details = <\n\t\t\t\t[\"other_contributors\"] = <\"{raw}\">\n\t\t\t>\n"
        )
    });
    let en_details = item_rows(en);
    let de_details = item_rows(de);
    format!(
        "\
archetype (adl_version=1.4)
\topenEHR-EHR-CLUSTER.conversion_metadata.v1

concept
\t[at0000]\t-- conversion meta-data

language
\toriginal_language = <[ISO_639-1::en]>
\ttranslations = <
\t\t[\"de\"] = <
\t\t\tlanguage = <[ISO_639-1::de]>
\t\t\tauthor = <
\t\t\t\t[\"name\"] = <\"Translator 1\">
\t\t\t>
{translation_details}\t\t>
\t>

description
\toriginal_author = <
\t\t[\"name\"] = <\"Author\">
\t>
\tlifecycle_state = <\"AuthorDraft\">
\tdetails = <
\t\t[\"en\"] = <
\t\t\tlanguage = <[ISO_639-1::en]>
\t\t\tpurpose = <\"purpose\">
{en_details}\t\t>
\t\t[\"de\"] = <
\t\t\tlanguage = <[ISO_639-1::de]>
\t\t\tpurpose = <\"Zweck\">
{de_details}\t\t>
\t>

definition
\tCLUSTER[at0000] matches {{
\t\titems cardinality matches {{0..*; unordered}} matches {{
\t\t\tELEMENT[at0001] occurrences matches {{0..1}} matches {{*}}
\t\t}}
\t}}

ontology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"root\"> description=<\"root\">>
\t\t\t\t[\"at0001\"] = <text=<\"element\"> description=<\"element\">>
\t\t\t>
\t\t>
\t\t[\"de\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"Wurzel\"> description=<\"Wurzel\">>
\t\t\t\t[\"at0001\"] = <text=<\"Element\"> description=<\"Element\">>
\t\t\t>
\t\t>
\t>
"
    )
}

/// A conversion configuration with a fixed record time.
fn config() -> ConvertConfig {
    ConvertConfig {
        time: "2026-11-11T09:30:00Z".parse().expect("a valid timestamp"),
        ..ConvertConfig::default()
    }
}

/// The converted `AUTHORED_ARCHETYPE` data of `src`.
fn convert(src: &str) -> AuthoredArchetypeData {
    let converted = parse_and_convert(src, &config(), &mut ConversionLog::default())
        .expect("the 1.4 archetype converts");
    match converted {
        Archetype::AuthoredArchetype(a) => match *a {
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

fn description(data: &AuthoredArchetypeData) -> &ResourceDescription {
    data.description.as_deref().expect("a description")
}

/// The `other_details` of the description item in `language`.
fn item_details(data: &AuthoredArchetypeData, language: &str) -> BTreeMap<String, String> {
    description(data)
        .details
        .as_ref()
        .and_then(|d| d.get(language))
        .and_then(|item| item.other_details.clone())
        .unwrap_or_default()
}

// ── rule 1: other_contributors ───────────────────────────────────────────────

#[test]
fn each_non_blank_contributor_line_becomes_one_entry_and_the_key_is_consumed() {
    let data = convert(&source(
        Some("Translator 2, Organisation 2, Norway\n\n   Translator 3, Organisation 3, Norway  \n"),
        &[],
        &[],
    ));
    let de = data
        .translations
        .as_ref()
        .and_then(|t| t.get("de"))
        .expect("the German translation");
    assert_eq!(
        de.other_contributors.as_deref(),
        Some(
            [
                "Translator 2, Organisation 2, Norway".to_owned(),
                "Translator 3, Organisation 3, Norway".to_owned(),
            ]
            .as_slice()
        )
    );
    assert!(
        de.other_details
            .as_ref()
            .is_none_or(|o| !o.contains_key("other_contributors")),
        "the converted key is consumed: {:?}",
        de.other_details
    );
}

// ── rule 2: copyright ────────────────────────────────────────────────────────

#[test]
fn the_original_language_copyright_is_hoisted() {
    let data = convert(&source(None, &[("copyright", "© Original")], &[]));
    assert_eq!(description(&data).copyright.as_deref(), Some("© Original"));
    assert!(!item_details(&data, "en").contains_key("copyright"));
}

#[test]
fn the_original_language_copyright_wins_over_an_alphabetically_earlier_translation() {
    let data = convert(&source(
        None,
        &[("copyright", "© Original")],
        &[("copyright", "© Übersetzung")],
    ));
    assert_eq!(description(&data).copyright.as_deref(), Some("© Original"));
}

#[test]
fn a_differing_translation_copyright_stays_with_its_translation() {
    let data = convert(&source(
        None,
        &[("copyright", "© Original")],
        &[("copyright", "© Übersetzung")],
    ));
    assert_eq!(
        item_details(&data, "de")
            .get("copyright")
            .map(String::as_str),
        Some("© Übersetzung")
    );
}

#[test]
fn an_identical_translation_copyright_is_dropped() {
    let data = convert(&source(
        None,
        &[("copyright", "© Original")],
        &[("copyright", "© Original"), ("note", "kept")],
    ));
    let de = item_details(&data, "de");
    assert!(!de.contains_key("copyright"), "{de:?}");
    assert_eq!(de.get("note").map(String::as_str), Some("kept"));
}

#[test]
fn no_original_language_copyright_leaves_the_field_empty() {
    let data = convert(&source(None, &[], &[("copyright", "© Übersetzung")]));
    assert_eq!(description(&data).copyright, None);
    assert_eq!(
        item_details(&data, "de")
            .get("copyright")
            .map(String::as_str),
        Some("© Übersetzung")
    );
}

// ── rule 3: the conversion record ────────────────────────────────────────────

#[test]
fn the_conversion_is_recorded_in_conversion_details() {
    let data = convert(&source(None, &[], &[]));
    let details = description(&data)
        .conversion_details
        .clone()
        .expect("conversion_details");
    assert_eq!(
        details.get("source_model").map(String::as_str),
        Some("openEHR-EHR-CLUSTER.conversion_metadata.v1")
    );
    assert_eq!(
        details.get("tool").map(String::as_str),
        Some(concat!("openehr-adl ", env!("CARGO_PKG_VERSION")))
    );
    assert_eq!(
        details.get("time").map(String::as_str),
        Some("2026-11-11T09:30:00Z")
    );
}

// ── the vendored upgrade_from_14 pairs ───────────────────────────────────────

/// Every `upgrade_from_14` source with its expected `.adls`.
const UPGRADE_PAIRS: &[(&str, &str)] = &[
    (
        "openEHR-EHR-ACTION.id_codes_as_at_codes.v1.adl",
        "openEHR-EHR-ACTION.id_codes_as_at_codes.v1.0.0.adls",
    ),
    (
        "openEHR-EHR-EVALUATION.exclusion-adverse_reaction.v1.adl",
        "openEHR-EHR-EVALUATION.exclusion-adverse_reaction.v1.0.0.adls",
    ),
    (
        "openEHR-EHR-EVALUATION.exclusion.v1.adl",
        "openEHR-EHR-EVALUATION.exclusion.v1.0.0.adls",
    ),
    (
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_node.v1.adl",
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_node.v1.0.0.adls",
    ),
    (
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_parent.v1.adl",
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_parent.v1.0.0.adls",
    ),
    (
        "openEHR-EHR-OBSERVATION.adl14_meta_data.adl",
        "openEHR-EHR-OBSERVATION.adl14_meta_data.v0.0.1-alpha.adls",
    ),
    (
        "openEHR-EHR-OBSERVATION.test_regex.v1.adl",
        "openEHR-EHR-OBSERVATION.test_regex.v1.1.0.adls",
    ),
    (
        "openEHR-EHR-OBSERVATION.upgrade_add_use_nodes.v1.adl",
        "openEHR-EHR-OBSERVATION.upgrade_add_use_nodes.v1.0.0.adls",
    ),
    (
        "openEHR-EHR-OBSERVATION.upgrade_parent.v1.adl",
        "openEHR-EHR-OBSERVATION.upgrade_parent.v1.0.0.adls",
    ),
];

fn upgrade_source(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/adl2-reference/upgrade/upgrade_from_14")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The expected outputs predate the SPECAM-107 rules, so they carry no
/// conversion record, but each states the copyright rule 2 gives: the original
/// language's, with no copy left in any description item.
#[test]
fn the_upgrade_pairs_agree_on_the_copyright_rule() {
    for (adl, adls) in UPGRADE_PAIRS {
        let got = convert(&upgrade_source(adl));
        let expected = openehr_adl::assemble::parse_artefact(
            &upgrade_source(adls),
            openehr_adl::parse::Dialect::Adl2,
        )
        .unwrap_or_else(|e| panic!("{adls}: {e:?}"));
        let expected = match expected {
            Archetype::AuthoredArchetype(a) => match *a {
                AuthoredArchetype::AuthoredArchetype(d) => d,
                other => panic!("{adls}: {other:?}"),
            },
            other @ Archetype::TemplateOverlay(_) => panic!("{adls}: {other:?}"),
        };
        assert_eq!(
            description(&got).copyright,
            description(&expected).copyright,
            "{adl}: the copyright differs from {adls}"
        );
        for language in description(&got).details.iter().flat_map(|d| d.keys()) {
            assert!(
                !item_details(&got, language).contains_key("copyright"),
                "{adl}: the {language} item keeps a copyright"
            );
        }
    }
}
