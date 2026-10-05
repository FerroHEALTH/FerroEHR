// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The description / meta-data / version half of the 1.4→2 conversion.
//!
//! [`crate::adl14::convert`] owns the code spaces and the definition rewrite;
//! this module owns everything that lands in `RESOURCE_DESCRIPTION` and the
//! archetype HRID's version fields.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion — the whole `adl14` module is
//! our own design (see the [`crate::adl14`] flag). The one exception lives here:
//! the standardised `description/other_details` meta-data mapping, which
//! `ADL1.4/masterAppB-extended_metadata.adoc` §Standardised Items governs
//! directly — those items are "intended to be implemented by any ADL 1.4 =>
//! ADL 2 conversion tool".

use std::collections::BTreeMap;

/// Apply the whole 1.4→2 `description` transform in place.
pub(super) fn transform_description(
    desc: &mut openehr_am::v2_4::resource::resource_description::ResourceDescription,
) {
    // NOTE: every 1.4 lifecycle state converts to `unmanaged`, matching the
    // vendored `upgrade_from_14` expected `.adls`; no openEHR spec governs
    // 1.4→2 conversion — our own design/extension.
    "unmanaged".clone_into(&mut desc.lifecycle_state);

    // Hoist `details[lang].copyright` (if any) up to `description.copyright`.
    if desc.copyright.is_none()
        && let Some(details) = desc.details.as_ref()
        && let Some(cr) = details.values().find_map(|item| {
            item.other_details
                .as_ref()
                .and_then(|o| o.get("copyright"))
                .cloned()
        })
    {
        desc.copyright = Some(cr);
    }

    convert_standardised_meta_data(desc);

    // Drop the consumed `revision` from other_details.
    if let Some(o) = desc.other_details.as_mut() {
        o.remove("revision");
    }
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
fn convert_standardised_meta_data(
    desc: &mut openehr_am::v2_4::resource::resource_description::ResourceDescription,
) {
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
    hrid.version_status = openehr_base::prelude::VersionStatus::from_wire(status);
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
