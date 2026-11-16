// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The container cardinalities the ADL2 front end captures on a node: the
//! serialized `cardinalities` of the Web Template metadata document, and the
//! validation-only `card_all` superset (AOM2 §`C_ATTRIBUTE`, `cardinality`).

use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_base::prelude::Cardinality;

use super::builder_v2_4::co_attributes;
use super::interval_v2_4::occ;
use super::model::WebTemplateCardinality;

pub(super) fn cardinalities(co: &CObject, node_path: &str) -> Vec<WebTemplateCardinality> {
    let mut out = Vec::new();
    for attr in co_attributes(co) {
        if !attr.is_multiple {
            continue;
        }
        let Some(card) = &attr.cardinality else {
            continue;
        };
        if requires_cardinality(card, attr.children.as_ref().map_or(0, Vec::len)) {
            let (min, max) = occ(&card.interval);
            out.push(WebTemplateCardinality {
                min,
                max,
                ids: None,
                path: format!("{node_path}/{}", attr.rm_attribute_name),
                unique: card.is_unique,
            });
        }
    }
    out
}

/// Whether a container cardinality genuinely constrains the container beyond
/// what the child count already implies (mirrors the OPT-1.4 front end).
fn requires_cardinality(card: &Cardinality, children_count: usize) -> bool {
    let (min, max) = occ(&card.interval);
    let Some(min) = min else {
        return max != -1;
    };
    if min == 0 {
        return max != -1 && i32::try_from(children_count).unwrap_or(i32::MAX) > max;
    }
    let count = i32::try_from(children_count).unwrap_or(i32::MAX);
    if min == 1 && max == 1 && children_count == 1 {
        false
    } else {
        min > 1 || (max != -1 && max < count)
    }
}

/// Capture EVERY constraining `C_ATTRIBUTE.cardinality` (AOM2 §C_ATTRIBUTE:
/// "Cardinality constraint of attribute, if a container attribute") for the
/// validation walk: any interval with a lower bound `>= 1` or a bounded upper
/// bound constrains the container, and so does `unique`. A superset of the serialized [`cardinalities`]
/// selection, so `0..1`/`1..1`/`1..*` container bounds are enforced from
/// [`WebTemplateNode::card_all`] too (mirrors the OPT-1.4 front end).
pub(super) fn all_cardinalities(co: &CObject, node_path: &str) -> Vec<WebTemplateCardinality> {
    let mut out = Vec::new();
    for attr in co_attributes(co) {
        let Some(card) = &attr.cardinality else {
            continue;
        };
        let (min, max) = occ(&card.interval);
        if min.unwrap_or(0) >= 1 || max != -1 || card.is_unique {
            out.push(WebTemplateCardinality {
                min,
                max,
                ids: None,
                path: format!("{node_path}/{}", attr.rm_attribute_name),
                unique: card.is_unique,
            });
        }
    }
    out
}
