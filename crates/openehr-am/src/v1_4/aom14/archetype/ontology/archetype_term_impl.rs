// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM 1.4 `ARCHETYPE_TERM` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom14.archetype_term.adoc` §Functions.

use crate::v1_4::aom14::archetype::ontology::archetype_term::ArchetypeTerm;

impl ArchetypeTerm {
    /// Returns the keys used in this term, or `None` when it carries no items.
    ///
    /// `keys` (`org.openehr.am.aom14.archetype_term.adoc` §Functions): "List of
    /// all keys used in this term." The declared result multiplicity is `0..1`
    /// and `items` is itself `0..1`, so an absent item table yields no list
    /// rather than an empty one. The keys come back in the sorted order the
    /// backing map holds them in, which the spec leaves unconstrained.
    #[must_use]
    pub fn keys(&self) -> Option<Vec<&str>> {
        self.items
            .as_ref()
            .map(|items| items.keys().map(String::as_str).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_of_a_populated_term_are_its_item_names() {
        let term = ArchetypeTerm {
            code: "at0001".to_owned(),
            items: Some(
                [
                    ("text".to_owned(), "blood group".to_owned()),
                    ("description".to_owned(), "the ABO group".to_owned()),
                ]
                .into_iter()
                .collect(),
            ),
        };
        assert_eq!(term.keys(), Some(vec!["description", "text"]));
    }

    #[test]
    fn an_absent_item_table_yields_no_key_list() {
        let term = ArchetypeTerm {
            code: "at0001".to_owned(),
            items: None,
        };
        assert_eq!(term.keys(), None);
    }

    #[test]
    fn an_empty_item_table_yields_an_empty_key_list() {
        let term = ArchetypeTerm {
            code: "at0001".to_owned(),
            items: Some(std::collections::BTreeMap::new()),
        };
        assert_eq!(term.keys(), Some(Vec::new()));
    }
}
