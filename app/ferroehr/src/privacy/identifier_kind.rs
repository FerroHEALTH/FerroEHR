// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a `DV_IDENTIFIER` says it identifies, read from its own `type`.
//!
//! The RM gives the value a declared kind: `type` is the "Optional identifier
//! type, such as prescription, or Social Security Number", and the class's own
//! examples run from "social security number" to "prescription id, order id"
//! (RM `UML/classes/org.openehr.rm.data_types.dv_identifier.adoc`
//! §Description). The attribute is free text, as the RM's design section says
//! ("no definitive vocabularies are available", `data_types/
//! master04-basic_package.adoc` §Design), so the reading is word-based.
//!
//! **No openEHR spec governs the word lists: our own design.** A word naming a
//! person identifier always wins, so a type that names both kinds stays
//! scanned, and a type naming neither proves nothing.

/// The kind a `DV_IDENTIFIER.type` declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclaredKind {
    /// The type names a person identifier, or one of a person's records.
    Person,
    /// The type names an identifier of a thing: an order, a specimen, a lot.
    Item,
    /// No type, or one naming neither.
    Undeclared,
}

/// Words that name a person identifier or a person's record, lower case.
///
/// Each shipped rule's own name is here too, so a value typed as the very
/// identifier a rule recognises is always scanned.
const PERSON_WORDS: &[&str] = &[
    "ahv",
    "ahvn13",
    "beneficiary",
    "birth",
    "bsn",
    "burgerservicenummer",
    "citizen",
    "client",
    "cpr",
    "epd",
    "fodselsnummer",
    "guarantor",
    "health",
    "hetu",
    "insurance",
    "insured",
    "kvnr",
    "licence",
    "license",
    "medical",
    "member",
    "mrn",
    "national",
    "nhs",
    "passport",
    "patient",
    "person",
    "personal",
    "personnummer",
    "pid",
    "record",
    "resident",
    "social",
    "ssn",
    "subject",
];

/// Words that name an identifier of an item rather than of a person,
/// lower case.
const ITEM_WORDS: &[&str] = &[
    "accession",
    "barcode",
    "batch",
    "container",
    "filler",
    "lot",
    "order",
    "placer",
    "prescription",
    "requisition",
    "sample",
    "serial",
    "specimen",
];

/// Read the kind a `DV_IDENTIFIER.type` declares.
pub(crate) fn declared_kind(type_text: Option<&str>) -> DeclaredKind {
    let Some(text) = type_text else {
        return DeclaredKind::Undeclared;
    };
    let words = words(text);
    if words
        .iter()
        .any(|word| PERSON_WORDS.contains(&word.as_str()))
    {
        DeclaredKind::Person
    } else if words.iter().any(|word| ITEM_WORDS.contains(&word.as_str())) {
        DeclaredKind::Item
    } else {
        DeclaredKind::Undeclared
    }
}

/// Split free text into lower-case words at every non-alphanumeric character
/// and every lower-to-upper case step, so `filler_order_number`,
/// `Filler Order Number` and `fillerOrderNumber` read alike.
fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for ch in text.chars() {
        if !ch.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_lower = false;
            continue;
        }
        if ch.is_uppercase() && previous_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        previous_lower = ch.is_lowercase() || ch.is_ascii_digit();
        current.extend(ch.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::{DeclaredKind, declared_kind};

    #[test]
    fn an_order_or_specimen_type_declares_an_item() {
        for text in [
            "filler_order_number",
            "Filler Order Number",
            "fillerOrderNumber",
            "placer_order_number",
            "Accession ID",
            "specimen identifier",
        ] {
            assert_eq!(declared_kind(Some(text)), DeclaredKind::Item, "{text}");
        }
    }

    #[test]
    fn a_person_word_wins_over_an_item_word() {
        for text in [
            "NHS Number",
            "nhs_number",
            "BSN",
            "patient order number",
            "Social Security Number",
            "medical record number MRN",
            "insuredPersonId",
        ] {
            assert_eq!(declared_kind(Some(text)), DeclaredKind::Person, "{text}");
        }
    }

    #[test]
    fn no_type_or_an_unknown_one_declares_nothing() {
        assert_eq!(declared_kind(None), DeclaredKind::Undeclared);
        assert_eq!(declared_kind(Some("")), DeclaredKind::Undeclared);
        assert_eq!(declared_kind(Some("reference")), DeclaredKind::Undeclared);
        assert_eq!(
            declared_kind(Some("ordering")),
            DeclaredKind::Undeclared,
            "words match whole, never as a prefix"
        );
    }
}
