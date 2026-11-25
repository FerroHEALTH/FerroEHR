// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The description, meta-data and version half of the 1.4→2 conversion.
//!
//! Spec-governed: the standardised `description/other_details` items
//! (`ADL1.4/masterAppB-extended_metadata.adoc` §Standardised Items) and the
//! translator, copyright and conversion-record rules of
//! `AOM2/master01-preface.adoc` §Release 2.3 changes (SPECAM-107).

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetypeData;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::v1_3::resource::translation_details::TranslationDetails;

/// The `other_details` key an ADL 1.4 translation lists its additional
/// translators under, one per line (`ADL1.4/master08-adl.adoc` §Language
/// Section and Language Translation).
const OTHER_CONTRIBUTORS: &str = "other_contributors";

/// The `other_details` key a description item's copyright sits under.
const COPYRIGHT: &str = "copyright";

/// Apply the 1.4→2 meta-data transform to translations and description, and
/// record the conversion with `notes` under `code_remap_NNN`.
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

/// Move the 1.4 per-language copyright to `RESOURCE_DESCRIPTION.copyright`
/// (`AOM2/master01-preface.adoc` §Release 2.3 changes): a differing translation
/// copyright stays in its `other_details`. A populated field is left alone.
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

/// Split each translation's `other_details["other_contributors"]` into
/// `TRANSLATION_DETAILS.other_contributors`, one entry per non-blank line
/// (`AOM2/master01-preface.adoc` §Release 2.3 changes).
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
        let Some(lines) = nonblank_lines(raw) else {
            continue;
        };
        translation.other_contributors = Some(lines);
        take_detail(&mut translation.other_details, OTHER_CONTRIBUTORS);
    }
}

/// The trimmed non-blank lines of an LF-separated value, `None` when none survive.
fn nonblank_lines(raw: &str) -> Option<Vec<String>> {
    let lines: Vec<String> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    (!lines.is_empty()).then_some(lines)
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

/// Convert the ADL 1.4 standardised `description/other_details` items to their
/// AOM2 `RESOURCE_DESCRIPTION` homes, consuming each converted key
/// (`ADL1.4/masterAppB-extended_metadata.adoc` §Standardised Items).
///
/// Namespace, publisher, custodian and licence transfer verbatim; `references`
/// and `ip_acknowledgements` split on LF, trimmed. §Other Items, malformed
/// values and keys whose target is already populated stay in `other_details`.
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
    let Some(lines) = nonblank_lines(raw) else {
        return;
    };
    // NOTE: the appendix prescribes no key scheme: our own design uses 1-based
    // ordinals in source line order.
    *target = Some(
        lines
            .into_iter()
            .enumerate()
            .map(|(index, line)| ((index + 1).to_string(), line))
            .collect(),
    );
    other.remove(key);
}
