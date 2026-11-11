// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The canonical-JSON codec of the AOM 1.4 `C_QUANTITY_ITEM` at BMM schema
//! revision `1.4.0.3`, which adds the optional `units_system` and
//! `units_display_name` (SPECAM-109,
//! `AM/docs/UML/classes/org.openehr.am.aom14.c_quantity_item.adoc`).

use openehr_am::v1_4::aom14::openehr_archetype_profile::c_quantity_item::CQuantityItem;
use openehr_its::json::{from_canonical_json, to_canonical_json};

const MAGNITUDE: &str = r#"{"_type":"Proper_interval","lower":0.0,"upper":1000.0,"lower_included":true,"upper_included":true,"lower_unbounded":false,"upper_unbounded":false}"#;

#[test]
fn a_quantity_item_reads_and_writes_units_system_and_display_name() {
    let json = format!(
        r#"{{"_type":"C_QUANTITY_ITEM","magnitude":{MAGNITUDE},"units":"[lb_av]","units_system":"http://unitsofmeasure.org","units_display_name":"lb"}}"#
    );
    let item: CQuantityItem = from_canonical_json(&json).expect("the item reads");
    assert_eq!(item.units.as_deref(), Some("[lb_av]"));
    assert_eq!(
        item.units_system.as_deref(),
        Some("http://unitsofmeasure.org")
    );
    assert_eq!(item.units_display_name.as_deref(), Some("lb"));
    let written = to_canonical_json(&item);
    let again: CQuantityItem = from_canonical_json(&written).expect("the written item reads");
    assert_eq!(again, item);
}

#[test]
fn a_quantity_item_without_the_new_attributes_leaves_them_empty() {
    let json = format!(r#"{{"_type":"C_QUANTITY_ITEM","magnitude":{MAGNITUDE},"units":"kg"}}"#);
    let item: CQuantityItem = from_canonical_json(&json).expect("the item reads");
    assert_eq!(item.units_system, None);
    assert_eq!(item.units_display_name, None);
    assert!(!to_canonical_json(&item).contains("units_system"));
}
