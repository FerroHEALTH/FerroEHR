// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `description` section (`master07.08`) → the BASE resource types.

use std::collections::BTreeMap;

use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::v1_3::resource::resource_description_item::ResourceDescriptionItem;
use openehr_lang::v1_1::odin::OdinValue;

use crate::odin::{
    as_keyed, as_object, key_str, string_list, string_map, string_map_of, string_of, term_code,
    term_code_of,
};
use crate::parse::Dialect;

/// The `description` ODIN block → [`ResourceDescription`] (`master07.08`;
/// resource model `docs/specs/openehr/BASE/docs/resource/`). The `regression`
/// tag the corpus uses lands in `other_details`.
pub(super) fn assemble_description(desc: &OdinValue, dialect: Dialect) -> ResourceDescription {
    let empty = indexmap::IndexMap::new();
    let map = as_object(desc).unwrap_or(&empty);
    ResourceDescription {
        title: string_of(map.get("title")),
        original_author: string_map_of(map, "original_author"),
        original_namespace: string_of(map.get("original_namespace")),
        original_publisher: string_of(map.get("original_publisher")),
        other_contributors: openehr_base::containers::present(
            map.get("other_contributors")
                .map(string_list)
                .unwrap_or_default(),
        ),
        lifecycle_state: string_of(map.get("lifecycle_state")).unwrap_or_default(),
        custodian_namespace: string_of(map.get("custodian_namespace")),
        custodian_organisation: string_of(map.get("custodian_organisation")),
        copyright: string_of(map.get("copyright")),
        licence: string_of(map.get("licence")),
        ip_acknowledgements: map.get("ip_acknowledgements").map(string_map),
        references: map.get("references").map(string_map),
        resource_package_uri: string_of(map.get("resource_package_uri")),
        conversion_details: map.get("conversion_details").map(string_map),
        details: map
            .get("details")
            .and_then(|details| assemble_details(details, dialect)),
        other_details: map.get("other_details").map(string_map),
    }
}

/// `details = <["en"] = < purpose=<…> keywords=<…> … >>` →
/// `lang → RESOURCE_DESCRIPTION_ITEM`.
///
/// An ADL 1.4 item's own `copyright` has no attribute in the BASE
/// `RESOURCE_DESCRIPTION_ITEM` ADL 2 uses, so it is kept in the item's
/// `other_details["copyright"]`, where the 1.4→2 converter reads it
/// (`AOM2/master01-preface.adoc` §Release 2.3 changes).
fn assemble_details(
    details: &OdinValue,
    dialect: Dialect,
) -> Option<BTreeMap<String, ResourceDescriptionItem>> {
    let entries = as_keyed(details)?;
    let mut out = BTreeMap::new();
    for (key, value) in entries {
        let lang = key_str(key);
        let obj = as_object(value);
        let language = obj
            .and_then(|o| o.get("language"))
            .map_or_else(|| term_code("ISO_639-1", &lang), term_code_of);
        out.insert(
            lang,
            ResourceDescriptionItem {
                language,
                purpose: obj
                    .and_then(|o| string_of(o.get("purpose")))
                    .unwrap_or_default(),
                keywords: openehr_base::containers::present(
                    obj.and_then(|o| o.get("keywords"))
                        .map(string_list)
                        .unwrap_or_default(),
                ),
                use_: obj.and_then(|o| string_of(o.get("use"))),
                misuse: obj.and_then(|o| string_of(o.get("misuse"))),
                original_resource_uri: obj
                    .and_then(|o| o.get("original_resource_uri"))
                    .map(string_map),
                other_details: item_other_details(obj, dialect),
            },
        );
    }
    Some(out)
}

/// A description item's `other_details`, with an ADL 1.4 item `copyright`
/// added under that key (see [`assemble_details`]).
fn item_other_details(
    obj: Option<&indexmap::IndexMap<String, OdinValue>>,
    dialect: Dialect,
) -> Option<BTreeMap<String, String>> {
    let mut other = obj.and_then(|o| o.get("other_details")).map(string_map);
    if dialect == Dialect::Adl14
        && let Some(copyright) = obj.and_then(|o| string_of(o.get("copyright")))
    {
        other
            .get_or_insert_with(BTreeMap::new)
            .insert("copyright".to_owned(), copyright);
    }
    other
}
