// @generated-from-template templates/openehr-rm/data_structures/item_structure/item_table_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `ITEM_TABLE` invariants and functions
//! (`org.openehr.rm.data_structures.item_table.adoc`): `Valid_structure` and
//! the inherited `Archetype_node_id_valid`.
//!
//! NOTE: `Valid_number_of_rows` and `Row_regularity` are our own names for the
//! §Description rule "Each row Cluster must have an identical number of
//! Elements … identical names and value types".

use crate::v1_1::data_structures::item_structure::item_table::ItemTable;
use crate::v1_1::data_structures::representation::cluster::Cluster;
use crate::v1_1::data_structures::representation::element::Element;
use crate::v1_1::data_structures::representation::item::Item;
use crate::v1_1::data_types::basic::data_value::DataValue;
use crate::v1_1::data_types::text::dv_text::DvText;
use openehr_base::validate::{InvariantViolation, Validate};

impl ItemTable {
    /// Number of rows (`row_count`).
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.as_deref().unwrap_or_default().len()
    }

    /// Number of columns, read from the first row (`column_count`).
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.rows
            .as_deref()
            .unwrap_or_default()
            .first()
            .map_or(0, |row| row.items.len())
    }

    /// The row names (`row_names`): each row `CLUSTER` is named "the stringified
    /// number of the row" (`master04-item_structure_package.adoc` §ITEM_TABLE).
    #[must_use]
    pub fn row_names(&self) -> Vec<DvText> {
        self.rows
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|row| row.name.clone())
            .collect()
    }

    /// The column names (`column_names`), the row `ELEMENT` names.
    #[must_use]
    pub fn column_names(&self) -> Vec<DvText> {
        self.rows
            .as_deref()
            .unwrap_or_default()
            .first()
            .into_iter()
            .flat_map(|row| row.items.iter())
            .map(|item| match item {
                Item::Element(element) => element.name.clone(),
                Item::Cluster(cluster) => cluster.name.clone(),
            })
            .collect()
    }

    /// The `i`-th row, from 1, so it agrees with the row names (`ith_row`).
    #[must_use]
    pub fn ith_row(&self, i: usize) -> Option<&Cluster> {
        self.rows.as_deref()?.get(i.checked_sub(1)?)
    }

    /// Whether a row has name `a_key` (`has_row_with_name`, whose description
    /// says "COLUMN", a copy of the next cell).
    #[must_use]
    pub fn has_row_with_name(&self, a_key: &str) -> bool {
        self.named_row(a_key).is_some()
    }

    /// Whether a column has name `a_key` (`has_column_with_name`).
    #[must_use]
    pub fn has_column_with_name(&self, a_key: &str) -> bool {
        self.column_names()
            .iter()
            .any(|name| text_of(name) == a_key)
    }

    /// The row named `a_key`, or `None` (`named_row`).
    #[must_use]
    pub fn named_row(&self, a_key: &str) -> Option<&Cluster> {
        self.rows
            .as_deref()?
            .iter()
            .find(|row| text_of(&row.name) == a_key)
    }

    /// Whether a row has key `keys` (`has_row_with_key`).
    #[must_use]
    pub fn has_row_with_key(&self, keys: &[String]) -> bool {
        self.row_with_key(keys).is_some()
    }

    /// The row with key `keys`, or `None` (`row_with_key`).
    ///
    /// No openEHR spec designates key columns or renders a cell as text, our own
    /// design: the keys match the leading columns, text cells only.
    #[must_use]
    pub fn row_with_key(&self, keys: &[String]) -> Option<&Cluster> {
        if keys.is_empty() {
            return None;
        }
        self.rows.as_deref()?.iter().find(|row| {
            keys.len() <= row.items.len()
                && keys
                    .iter()
                    .zip(row.items.iter())
                    .all(|(key, item)| cell_text(item).is_some_and(|value| value == key.as_str()))
        })
    }

    /// The cell at row `i`, column `j`, both from 1 (`element_at_cell_ij`).
    #[must_use]
    pub fn element_at_cell_ij(&self, i: usize, j: usize) -> Option<&Element> {
        match self.ith_row(i)?.items.get(j.checked_sub(1)?)? {
            Item::Element(element) => Some(element),
            Item::Cluster(_) => None,
        }
    }

    /// This table as one `CLUSTER` per row named by its number (`master04` §ISO
    /// 13606 Encoding Rules), or `None` for an empty table.
    ///
    /// NOTE: the class table's `as_hierarchy` says column clusters, against
    /// §ISO 13606 Encoding Rules and §Description; the row encoding governs.
    #[must_use]
    pub fn as_hierarchy(&self) -> Option<Cluster> {
        let rows = self.rows.as_deref()?;
        let encoded: Vec<Item> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let mut renamed = row.clone();
                renamed.name = DvText::DvText(crate::v1_1::data_types::text::dv_text::DvTextData {
                    value: (index + 1).to_string(),
                    hyperlink: None,
                    formatting: None,
                    mappings: None,
                    language: None,
                    encoding: None,
                });
                Item::from(renamed)
            })
            .collect();
        row_cluster(self.name.clone(), &self.archetype_node_id, encoded)
    }
}

/// A `CLUSTER` over `items`, or `None` when there are none — `CLUSTER.items` is
/// `1..*`, so an empty one is not a `CLUSTER`.
fn row_cluster(name: DvText, archetype_node_id: &str, items: Vec<Item>) -> Option<Cluster> {
    Some(Cluster {
        name,
        archetype_node_id: archetype_node_id.to_owned(),
        uid: None,
        links: openehr_base::containers::present_nonempty(Vec::new()),
        archetype_details: None,
        feeder_audit: None,
        // NOTE: the only failure is an empty `items`, which is this function's
        // absent case rather than a defect — see the doc comment.
        items: openehr_base::containers::NonEmptyVec::new(items).ok()?,
    })
}

/// The text of a name, whichever `DV_TEXT` form carries it.
fn text_of(name: &DvText) -> &str {
    match name {
        DvText::DvText(text) => &text.value,
        DvText::DvCodedText(text) => &text.value,
    }
}

/// A cell's value as text, when the cell is text-valued.
fn cell_text(item: &Item) -> Option<&str> {
    let Item::Element(element) = item else {
        return None;
    };
    match element.value.as_deref()? {
        DataValue::DvText(text) => Some(text_of(text)),
        _ => None,
    }
}

impl Validate for ItemTable {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        // Valid_structure: rows contain only ELEMENTs.
        if self.rows.iter().flatten().any(|row| {
            row.items
                .iter()
                .any(|item| !matches!(item, Item::Element(_)))
        }) {
            out.push(InvariantViolation::here(
                "Invariant Valid_structure failed on type ITEM_TABLE",
            ));
        }
        // Valid_number_of_rows (item_table.adoc §Description): every row has the
        // same number of items.
        if let Some(first) = self.rows.iter().flatten().next()
            && self
                .rows
                .iter()
                .flatten()
                .any(|row| row.items.len() != first.items.len())
        {
            out.push(InvariantViolation::here(
                "Invariant Valid_number_of_rows failed on type ITEM_TABLE",
            ));
        }
        // Row_regularity (item_table.adoc §Description).
        if let Some(first) = self.rows.iter().flatten().next() {
            use crate::v1_1::data_types::text::dv_text::DvText;
            let name_of = |name: &DvText| match name {
                DvText::DvText(t) => t.value.clone(),
                DvText::DvCodedText(t) => t.value.clone(),
            };
            let signature = |row: &Cluster| {
                row.items
                    .iter()
                    .map(|item| match item {
                        Item::Element(e) => (
                            name_of(&e.name),
                            e.value.as_deref().map(std::mem::discriminant),
                        ),
                        Item::Cluster(c) => (name_of(&c.name), None),
                    })
                    .collect::<Vec<_>>()
            };
            let first_sig = signature(first);
            if self
                .rows
                .iter()
                .flatten()
                .skip(1)
                .any(|row| signature(row) != first_sig)
            {
                out.push(InvariantViolation::here(
                    "Invariant Row_regularity failed on type ITEM_TABLE \
                     (corresponding row ELEMENTs must share names and value types)",
                ));
            }
        }
        crate::v1_1::validate::generated::archetype_node_id_core(
            "ITEM_TABLE",
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_structures::representation::cluster::Cluster;
    use crate::v1_1::data_structures::representation::element::Element;
    use crate::v1_1::data_types::basic::data_value::DataValue;
    use crate::v1_1::data_types::basic::dv_boolean::DvBoolean;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};

    fn text(value: &str) -> DvText {
        DvText::DvText(DvTextData {
            value: value.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
        })
    }

    fn element(id: &str) -> Element {
        Element {
            name: text("e"),
            archetype_node_id: id.to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            null_flavour: None,
            value: Some(Box::new(DataValue::DvBoolean(DvBoolean { value: true }))),
            null_reason: None,
        }
    }

    fn row(items: Vec<Item>) -> Cluster {
        Cluster {
            name: text("row"),
            archetype_node_id: "at0002".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            items: openehr_base::containers::NonEmptyVec::new(items)
                .expect("a fixture container declared 1..* must have members"),
        }
    }

    fn table(rows: Vec<Cluster>) -> ItemTable {
        ItemTable {
            name: text("table"),
            archetype_node_id: "at0001".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            rows: openehr_base::containers::present(rows),
        }
    }

    #[test]
    fn valid_table() {
        let t = table(vec![
            row(vec![Item::from(element("id5")), Item::from(element("id6"))]),
            row(vec![Item::from(element("id7")), Item::from(element("id8"))]),
        ]);
        assert_eq!(t.invariants(), Vec::<InvariantViolation>::new());
        assert_eq!(table(vec![]).invariants(), Vec::<InvariantViolation>::new()); // empty table is valid
    }

    #[test]
    fn cluster_in_row_invalid() {
        let nested = row(vec![Item::from(element("id7"))]);
        let t = table(vec![row(vec![
            Item::from(element("id5")),
            Item::from(nested),
        ])]);
        let v = t.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Valid_structure failed on type ITEM_TABLE"),
            "got {v:?}"
        );
    }

    #[test]
    fn ragged_rows_invalid() {
        let t = table(vec![
            row(vec![Item::from(element("id5")), Item::from(element("id6"))]),
            row(vec![Item::from(element("id7"))]),
        ]);
        let v = t.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Valid_number_of_rows failed on type ITEM_TABLE"),
            "got {v:?}"
        );
    }

    /// Row regularity (`item_table.adoc` §description): corresponding row
    /// ELEMENTs must share names and value types across all rows.
    #[test]
    fn row_regularity_names_and_value_types() {
        use crate::v1_1::data_types::quantity::dv_count::DvCount;
        let named = |name: &str, value: DataValue| Element {
            name: text(name),
            archetype_node_id: "at0001".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            null_flavour: None,
            value: Some(Box::new(value)),
            null_reason: None,
        };
        let row = |elems: Vec<Element>| Cluster {
            name: text("row"),
            archetype_node_id: "at0002".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            items: openehr_base::containers::NonEmptyVec::new(
                elems.into_iter().map(Item::from).collect(),
            )
            .expect("the fixture row carries elements"),
        };
        let bool_val = || DataValue::DvBoolean(DvBoolean { value: true });
        let count_val = || {
            DataValue::DvCount(DvCount {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                magnitude_status: None,
                accuracy: None,
                accuracy_is_percent: None,
                magnitude: 1,
            })
        };
        let table = |rows: Vec<Cluster>| ItemTable {
            name: text("t"),
            archetype_node_id: "at0000".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            rows: openehr_base::containers::present(rows),
        };
        let regular = table(vec![
            row(vec![named("a", bool_val()), named("b", count_val())]),
            row(vec![named("a", bool_val()), named("b", count_val())]),
        ]);
        let mut out = Vec::new();
        regular.validate_invariants(&mut out);
        assert!(
            !out.iter().any(|m| m.message.contains("Row_regularity")),
            "regular rows must pass, got {out:?}"
        );
        // Same count, different value type in position 2 → Row_regularity.
        let irregular = table(vec![
            row(vec![named("a", bool_val()), named("b", count_val())]),
            row(vec![named("a", bool_val()), named("b", bool_val())]),
        ]);
        let mut out = Vec::new();
        irregular.validate_invariants(&mut out);
        assert!(
            out.iter().any(|m| m.message.contains("Row_regularity")),
            "type-irregular rows must fail, got {out:?}"
        );
        // Different name in position 1 → Row_regularity.
        let renamed = table(vec![
            row(vec![named("a", bool_val())]),
            row(vec![named("x", bool_val())]),
        ]);
        let mut out = Vec::new();
        renamed.validate_invariants(&mut out);
        assert!(
            out.iter().any(|m| m.message.contains("Row_regularity")),
            "name-irregular rows must fail, got {out:?}"
        );
    }

    /// A table of two rows and two named columns.
    fn cell(column: &str, value: &str) -> Element {
        use crate::v1_1::data_types::text::dv_text::DvTextData;
        Element {
            name: text(column),
            archetype_node_id: "at0010".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            null_flavour: None,
            value: Some(Box::new(DataValue::DvText(DvText::DvText(DvTextData {
                value: value.to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: openehr_base::containers::present_nonempty(Vec::new()),
                language: None,
                encoding: None,
            })))),
            null_reason: None,
        }
    }

    fn numbered_row(number: &str, cells: Vec<Element>) -> Cluster {
        Cluster {
            name: text(number),
            archetype_node_id: "at0002".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            items: openehr_base::containers::NonEmptyVec::new(
                cells.into_iter().map(Item::from).collect(),
            )
            .expect("the fixture row carries cells"),
        }
    }

    fn acuity() -> ItemTable {
        table(vec![
            numbered_row("1", vec![cell("site", "left eye"), cell("result", "6/6")]),
            numbered_row("2", vec![cell("site", "right eye"), cell("result", "6/9")]),
        ])
    }

    /// The counts and the two name lists.
    #[test]
    fn counts_and_names_come_from_the_encoding_rules() {
        let acuity = acuity();
        assert_eq!(acuity.row_count(), 2);
        assert_eq!(acuity.column_count(), 2);
        assert_eq!(
            acuity.row_names().iter().map(text_of).collect::<Vec<_>>(),
            ["1", "2"]
        );
        assert_eq!(
            acuity
                .column_names()
                .iter()
                .map(text_of)
                .collect::<Vec<_>>(),
            ["site", "result"]
        );

        let empty = table(vec![]);
        assert_eq!(empty.row_count(), 0);
        assert_eq!(empty.column_count(), 0);
        assert!(empty.row_names().is_empty() && empty.column_names().is_empty());
    }

    /// `ith_row` agrees with `named_row`.
    #[test]
    fn ith_row_agrees_with_the_row_numbering_that_names_the_rows() {
        let acuity = acuity();
        for number in 1..=acuity.row_count() {
            assert_eq!(
                acuity.ith_row(number),
                acuity.named_row(&number.to_string()),
                "row {number} must be the row named {number}"
            );
        }
        assert!(acuity.ith_row(0).is_none(), "there is no row zero");
        assert!(acuity.ith_row(3).is_none());
    }

    /// Lookup by name on both axes, the two functions differing.
    #[test]
    fn name_lookup_distinguishes_rows_from_columns() {
        let acuity = acuity();
        assert!(acuity.has_row_with_name("1") && acuity.has_row_with_name("2"));
        assert!(!acuity.has_row_with_name("site"), "that is a column");

        assert!(acuity.has_column_with_name("site") && acuity.has_column_with_name("result"));
        assert!(!acuity.has_column_with_name("1"), "that is a row");

        assert!(acuity.named_row("nope").is_none());
    }

    /// Key lookup reads the leading columns and matches text-valued cells.
    #[test]
    fn key_lookup_matches_the_leading_columns() {
        let acuity = acuity();
        let key = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();

        assert!(acuity.has_row_with_key(&key(&["left eye"])));
        assert_eq!(
            acuity.row_with_key(&key(&["right eye", "6/9"])),
            acuity.ith_row(2)
        );
        assert!(
            !acuity.has_row_with_key(&key(&["6/6"])),
            "that is the second column, not the leading key"
        );
        assert!(!acuity.has_row_with_key(&key(&["left eye", "6/9"])));
        assert!(
            !acuity.has_row_with_key(&[]),
            "no key selects no row, rather than the first one"
        );
        assert!(
            !acuity.has_row_with_key(&key(&["left eye", "6/6", "extra"])),
            "a key longer than the row cannot match"
        );
    }

    /// Cell access, one-based on both axes.
    #[test]
    fn cells_are_addressed_one_based_on_both_axes() {
        let acuity = acuity();
        assert_eq!(
            acuity.element_at_cell_ij(2, 1).map(|e| text_of(&e.name)),
            Some("site")
        );
        assert!(acuity.element_at_cell_ij(0, 1).is_none());
        assert!(acuity.element_at_cell_ij(1, 0).is_none());
        assert!(acuity.element_at_cell_ij(1, 3).is_none());
        assert!(acuity.element_at_cell_ij(3, 1).is_none());
    }

    /// The hierarchy is one `CLUSTER` per row, named by row number.
    #[test]
    fn as_hierarchy_encodes_rows_with_stringified_names() {
        let hierarchy = acuity().as_hierarchy().expect("a table with cells");
        assert_eq!(text_of(&hierarchy.name), "table");
        assert_eq!(hierarchy.items.len(), 2, "one CLUSTER per row");

        let Item::Cluster(first) = &hierarchy.items[0] else {
            panic!("a row is a CLUSTER");
        };
        assert_eq!(
            text_of(&first.name),
            "1",
            "stringified one-based row number"
        );
        assert_eq!(first.items.len(), 2, "the row's ELEMENTs, one per column");
        assert_eq!(
            first
                .items
                .iter()
                .map(|item| match item {
                    Item::Element(element) => text_of(&element.name),
                    Item::Cluster(cluster) => text_of(&cluster.name),
                })
                .collect::<Vec<_>>(),
            ["site", "result"],
            "ELEMENT names are the column names"
        );
        let Item::Cluster(second) = &hierarchy.items[1] else {
            panic!("a row is a CLUSTER");
        };
        assert_eq!(text_of(&second.name), "2");

        assert!(
            table(vec![]).as_hierarchy().is_none(),
            "CLUSTER.items is 1..*, so an empty table has no hierarchy"
        );
    }
}
