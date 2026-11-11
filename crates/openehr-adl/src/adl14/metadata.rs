// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The description / meta-data / version half of the 1.4→2 conversion.
//!
//! [`crate::adl14::convert`] owns the code spaces and the definition rewrite;
//! this module owns everything that lands in `RESOURCE_DESCRIPTION` and the
//! archetype HRID's version fields.
//!
//! The meta-data half is spec-governed in two places, the rest of the `adl14`
//! module being our own design (see the [`crate::adl14`] flag):
//!
//! - the standardised `description/other_details` items, which
//!   `ADL1.4/masterAppB-extended_metadata.adoc` §Standardised Items says "are
//!   intended to be implemented by any ADL 1.4 => ADL 2 conversion tool";
//! - the translator, copyright and conversion-record rules of
//!   `AOM2/master01-preface.adoc` §Release 2.3 changes (SPECAM-107), applied by
//!   [`transform_translations`], [`transform_description`] and
//!   [`record_conversion`].

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::archetype::archetype_hrid_impl::version_status_of_modifier;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetypeData;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::v1_3::resource::translation_details::TranslationDetails;

/// The `other_details` key an ADL 1.4 translation lists its additional
/// translators under, one per line (`ADL1.4/master08-adl.adoc` §Language
/// Section and Language Translation).
const OTHER_CONTRIBUTORS: &str = "other_contributors";

/// The `other_details` key a description item's copyright sits under.
const COPYRIGHT: &str = "copyright";

/// Apply the 1.4→2 meta-data transform to the translations and the
/// description in place, then record the conversion.
///
/// `notes` are the conversion's non-mechanical decisions (collapse remaps,
/// VCOSU re-mints), kept as conversion provenance in
/// `RESOURCE_DESCRIPTION.conversion_details` under `code_remap_NNN`.
pub(super) fn convert_meta_data(
    data: &mut AuthoredArchetypeData,
    record: &ConversionRecord<'_>,
    notes: &[String],
) {
    if let Some(translations) = data.translations.as_mut() {
        transform_translations(translations);
    }
    let Some(desc) = data.description.as_mut() else {
        return;
    };
    transform_description(desc, &data.original_language.code_string);
    if !notes.is_empty() {
        let details = desc.conversion_details.get_or_insert_with(BTreeMap::new);
        for (index, note) in notes.iter().enumerate() {
            details.insert(format!("code_remap_{index:03}"), note.clone());
        }
    }
    record_conversion(desc, record);
}

/// Apply the whole 1.4→2 `description` transform in place.
///
/// `original_language` is the archetype's original language code, whose
/// description item carries the copyright the whole archetype takes.
fn transform_description(desc: &mut ResourceDescription, original_language: &str) {
    // NOTE: every 1.4 lifecycle state converts to `unmanaged`, matching the
    // vendored `upgrade_from_14` expected `.adls`; no openEHR spec governs
    // 1.4→2 conversion — our own design/extension.
    "unmanaged".clone_into(&mut desc.lifecycle_state);

    convert_copyright(desc, original_language);
    convert_standardised_meta_data(desc);

    // Drop the consumed `revision` from other_details.
    if let Some(o) = desc.other_details.as_mut() {
        o.remove("revision");
    }
}

/// Move the ADL 1.4 per-language copyright to the one ADL 2
/// `RESOURCE_DESCRIPTION.copyright`.
///
/// `AOM2/master01-preface.adoc` §Release 2.3 changes: the original language's
/// copyright goes to `RESOURCE_DESCRIPTION.copyright`; a translation's
/// copyright that differs from it stays in that translation's
/// `other_details["copyright"]`, and one identical to it is dropped; with no
/// original-language copyright the field stays empty. A description whose
/// `copyright` is already populated is left as it is, nothing consumed.
fn convert_copyright(desc: &mut ResourceDescription, original_language: &str) {
    if desc.copyright.is_some() {
        return;
    }
    let Some(details) = desc.details.as_mut() else {
        return;
    };
    let original = details
        .get_mut(original_language)
        .and_then(|item| take_detail(&mut item.other_details, COPYRIGHT));
    if let Some(original) = &original {
        for (language, item) in details.iter_mut() {
            let identical = item
                .other_details
                .as_ref()
                .and_then(|o| o.get(COPYRIGHT))
                .is_some_and(|copyright| copyright == original);
            if language != original_language && identical {
                take_detail(&mut item.other_details, COPYRIGHT);
            }
        }
    }
    desc.copyright = original;
}

/// Split each translation's ADL 1.4 `other_details["other_contributors"]` into
/// `TRANSLATION_DETAILS.other_contributors`, one entry per non-blank line.
///
/// `AOM2/master01-preface.adoc` §Release 2.3 changes: converters "should map
/// each line to one entry of `TRANSLATION_DETAILS._other_contributors_`". Each
/// line is trimmed and blank lines are dropped; the key is consumed. A
/// translation whose `other_contributors` is already populated, or whose value
/// holds no non-blank line, keeps the key as it is.
fn transform_translations(translations: &mut BTreeMap<String, TranslationDetails>) {
    for translation in translations.values_mut() {
        if translation
            .other_contributors
            .as_ref()
            .is_some_and(|list| !list.is_empty())
        {
            continue;
        }
        let Some(raw) = translation
            .other_details
            .as_ref()
            .and_then(|o| o.get(OTHER_CONTRIBUTORS))
        else {
            continue;
        };
        let lines: Vec<String> = raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        if lines.is_empty() {
            continue;
        }
        translation.other_contributors = Some(lines);
        take_detail(&mut translation.other_details, OTHER_CONTRIBUTORS);
    }
}

/// The record of one conversion: what was converted, by which tool, and when.
#[derive(Debug)]
pub(super) struct ConversionRecord<'a> {
    /// The identifier of the ADL 1.4 archetype or template converted.
    pub(super) source_model: &'a str,
    /// The converter's name and version.
    pub(super) tool: &'a str,
    /// The moment of the conversion.
    pub(super) time: jiff::Timestamp,
}

/// Record the conversion in `RESOURCE_DESCRIPTION.conversion_details` under the
/// keys `AOM2/master01-preface.adoc` §Release 2.3 changes names:
/// `"source_model"`, `"tool"` and `"time"` (ISO 8601).
fn record_conversion(desc: &mut ResourceDescription, record: &ConversionRecord<'_>) {
    let details = desc.conversion_details.get_or_insert_with(BTreeMap::new);
    details.insert("source_model".to_owned(), record.source_model.to_owned());
    details.insert("tool".to_owned(), record.tool.to_owned());
    details.insert("time".to_owned(), record.time.to_string());
}

/// Remove `key` from an `other_details` map, returning its value; a map left
/// empty becomes `None`, so no empty `other_details` is written.
fn take_detail(other: &mut Option<BTreeMap<String, String>>, key: &str) -> Option<String> {
    let map = other.as_mut()?;
    let value = map.remove(key);
    if map.is_empty() {
        *other = None;
    }
    value
}

/// Convert the ADL 1.4 standardised `description/other_details` meta-data items
/// to their AOM2 `RESOURCE_DESCRIPTION` homes, consuming each converted key.
///
/// `ADL1.4/masterAppB-extended_metadata.adoc` §Standardised Items is the one
/// part of 1.4→2 conversion the spec text governs directly: the items' "naming
/// and rules should be followed, and … are intended to be implemented by any
/// ADL 1.4 => ADL 2 conversion tool". The mapping applied here is the table's:
///
/// - `original_namespace`, `original_publisher`, `custodian_namespace`,
///   `custodian_organisation`, `licence` transfer verbatim to the same-named
///   `RESOURCE_DESCRIPTION` attributes. The table's `"name <URN>"` shapes are
///   DISPLAY conventions ("the use of the typical string for a person or
///   organisation of the form \"name \<URN\>\", which enables email addresses,
///   website URLs etc to be easily extracted", §Extended Meta-data Guide
///   preamble) — the AOM2 attributes are single strings, so the value is not
///   decomposed.
/// - `references` and `ip_acknowledgements` are "string with one LF (`\n`)
///   terminated line for each reference. Intervening LFs and leading and
///   trailing whitespace may be added for clarity, to be stripped on
///   conversion to ADL2" — so the value splits on LF, each line is trimmed,
///   and blank lines are dropped.
///
/// §Other Items (`MD5-CAM-1.0.1`, `current_contact`, `review_date`,
/// `responsible_organisation`) are reserved/display-only names with no
/// conversion mandated; they stay in `other_details` untouched, as does any
/// value that violates its item's stated syntax.
///
/// An AOM2 attribute already populated from elsewhere is never overwritten, and
/// its `other_details` key is then left in place (nothing was consumed).
fn convert_standardised_meta_data(desc: &mut ResourceDescription) {
    let Some(other) = desc.other_details.as_mut() else {
        return;
    };
    take_verbatim(other, "original_namespace", &mut desc.original_namespace);
    take_verbatim(other, "original_publisher", &mut desc.original_publisher);
    take_verbatim(other, "custodian_namespace", &mut desc.custodian_namespace);
    take_verbatim(
        other,
        "custodian_organisation",
        &mut desc.custodian_organisation,
    );
    take_verbatim(other, "licence", &mut desc.licence);
    take_keyed_lines(other, "references", &mut desc.references);
    take_keyed_lines(other, "ip_acknowledgements", &mut desc.ip_acknowledgements);
}

/// Move `other[key]` verbatim into `target`, consuming the key; a no-op when
/// `target` is already populated or the key is absent.
fn take_verbatim(other: &mut BTreeMap<String, String>, key: &str, target: &mut Option<String>) {
    if target.is_some() {
        return;
    }
    if let Some(value) = other.remove(key) {
        *target = Some(value);
    }
}

/// Move the LF-separated lines of `other[key]` into `target` as a keyed list,
/// consuming the key; a no-op when `target` is already populated, the key is
/// absent, or no non-blank line survives the strip (nothing to convert — the
/// value stays in `other_details` rather than being dropped).
fn take_keyed_lines(
    other: &mut BTreeMap<String, String>,
    key: &str,
    target: &mut Option<BTreeMap<String, String>>,
) {
    if target.is_some() {
        return;
    }
    let Some(raw) = other.get(key) else {
        return;
    };
    let lines: Vec<String> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    if lines.is_empty() {
        return;
    }
    // (Ordinals are unpadded, so ten or more entries sort lexicographically in
    // the `BTreeMap` — a display order, not a semantic one; the ordinal itself
    // still names the source line.)
    // NOTE: the appendix prescribes the LF-per-entry source syntax and the AOM2
    // target but no key scheme — no openEHR spec governs it, so this is our own
    // design: stable 1-based ordinals in source line order.
    *target = Some(
        lines
            .into_iter()
            .enumerate()
            .map(|(index, line)| ((index + 1).to_string(), line))
            .collect(),
    );
    other.remove(key);
}

/// Write a 1.4 `revision` string into the ADL2 HRID's version fields.
pub(super) fn set_release_version(
    hrid: &mut openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid,
    version: &str,
) {
    // `version` may be `1.1.0` or `0.0.1-alpha`; split a `-status.build` tail.
    let (numeric, status, build) = split_version(version);
    hrid.release_version = numeric;
    hrid.version_status = version_status_of_modifier(status)
        .unwrap_or_else(|| openehr_base::prelude::VersionStatus::from_wire(status));
    hrid.build_count = build;
}

/// Split a 1.4 revision string into `(numeric, version status, build count)`.
fn split_version(v: &str) -> (String, &'static str, String) {
    for (marker, status) in [("-rc", "rc"), ("-alpha", "alpha"), ("-beta", "beta")] {
        if let Some((numeric, tail)) = v.split_once(marker) {
            let numeric = normalise_numeric(numeric);
            let build = tail.strip_prefix('.').unwrap_or("").to_owned();
            return (numeric, status, build);
        }
    }
    (normalise_numeric(v), "", String::new())
}

/// Pad a partial `major[.minor[.patch]]` string to a full three-part version.
fn normalise_numeric(v: &str) -> String {
    let mut parts = v.split('.');
    let major = parts.next().unwrap_or("1");
    let minor = parts.next().unwrap_or("0");
    let patch = parts.next().unwrap_or("0");
    format!(
        "{}.{}.{}",
        if major.is_empty() { "1" } else { major },
        if minor.is_empty() { "0" } else { minor },
        if patch.is_empty() { "0" } else { patch }
    )
}
