// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `COMPOSITION.category` — the openEHR `composition_category` terminology
//! group, as named constants.
//!
//! Spec: RM composition `master05-composition_package.adoc` §COMPOSITION and
//! `RM/docs/UML/classes/org.openehr.rm.composition.composition.adoc` — `category`
//! is a `DV_CODED_TEXT` "coded by openEHR terminology group `composition
//! category`", and `is_persistent()` is defined as that code being the
//! `persistent` member. The wire form of the category is its **numeric group
//! code** (`431`, not the rubric `persistent`); the rubric is resolved from the
//! `openehr-term` bundle at the render edge, never hardcoded.
//!
//! This module is the composition-side sibling of
//! [`crate::versioning::lifecycle::state`]: membership questions still go
//! through the bundle (which is the authority), and the constants exist so no
//! read or write path ever spells a category code as a bare string literal.
//! `tests::category_group_is_the_expected_four_members` is the completeness
//! guard the `audit_change_type` and `version_lifecycle_state` constant sets
//! also carry — it fails if the TERM bundle's group ever gains or loses a
//! member, which is when this module needs a new constant.

/// The `composition_category` group members this chapter names.
pub(in crate::service) mod code {
    /// `431|persistent|` — content that persists across the life of the EHR
    /// (RM composition `COMPOSITION.is_persistent`).
    pub(in crate::service) const PERSISTENT: &str = "431";
}

#[cfg(test)]
mod tests {
    use openehr_term::bundle::openehr;

    use super::code;

    /// The `composition_category` openEHR terminology group id.
    const COMPOSITION_CATEGORY: &str = "composition_category";

    /// The COMPLETE `composition_category` group (TERM 3.1.0
    /// `openehr_terminology.xml`): `431|persistent|`, `433|event|`,
    /// `451|episodic|`, `815|report|`. A member added to (or removed from) the
    /// bundle fails this test, which is the signal that the constants above
    /// need revisiting — the same guard `versioning::audit`'s
    /// `audit_change_type` and `versioning::lifecycle`'s
    /// `version_lifecycle_state` sets carry.
    #[test]
    fn category_group_is_the_expected_four_members() {
        let t = openehr();
        let mut group: Vec<String> = t
            .concepts_in_group(COMPOSITION_CATEGORY)
            .iter()
            .map(|c| c.id.clone())
            .collect();
        group.sort();
        assert_eq!(group, ["431", "433", "451", "815"]);
        for c in &group {
            assert!(t.is_valid_composition_category(c), "code {c}");
            // `code_string` must be numeric (the group's wire form).
            assert!(c.chars().all(char::is_numeric), "code {c}");
        }
        // The named constant is a real member of that group.
        assert!(t.is_valid_composition_category(code::PERSISTENT));
        assert!(group.iter().any(|c| c == code::PERSISTENT));
    }
}
