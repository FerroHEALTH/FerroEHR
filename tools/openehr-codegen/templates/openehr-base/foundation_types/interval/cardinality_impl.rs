// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written BASE `Cardinality` spec functions.
//!
//! `Cardinality` expresses constraints on the cardinality of container objects
//! (the values of multiply-valued attributes), including uniqueness and
//! ordering, so a container can be stated to behave as a list, set or bag —
//! the classification the AM container-attribute validator interrogates.
//!
//! Spec source (vendored):
//! `BASE/docs/UML/classes/org.openehr.base.foundation_types.cardinality.adoc`
//! (`is_bag`/`is_list`/`is_set`, defined over the `is_ordered`/`is_unique`
//! attribute pair).

use super::cardinality::Cardinality;

impl Cardinality {
    /// `is_bag` (`cardinality.adoc`): true if the semantics represent a bag,
    /// i.e. unordered, non-unique membership (`not is_ordered and not
    /// is_unique`).
    #[must_use]
    pub fn is_bag(&self) -> bool {
        !self.is_ordered && !self.is_unique
    }

    /// `is_list` (`cardinality.adoc`): true if the semantics represent a list,
    /// i.e. ordered, non-unique membership (`is_ordered and not is_unique`).
    #[must_use]
    pub fn is_list(&self) -> bool {
        self.is_ordered && !self.is_unique
    }

    /// `is_set` (`cardinality.adoc`): true if the semantics represent a set,
    /// i.e. unordered, unique membership (`not is_ordered and is_unique`).
    #[must_use]
    pub fn is_set(&self) -> bool {
        !self.is_ordered && self.is_unique
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_3::foundation_types::interval::multiplicity_interval::MultiplicityInterval;

    fn cardinality(is_ordered: bool, is_unique: bool) -> Cardinality {
        Cardinality {
            // `0..*` — the interval value is irrelevant to the kind predicates.
            interval: MultiplicityInterval {
                lower: Some(0),
                upper: None,
                lower_unbounded: false,
                upper_unbounded: true,
                lower_included: true,
                upper_included: false,
            },
            is_ordered,
            is_unique,
        }
    }

    #[test]
    fn is_bag_unordered_non_unique() {
        let c = cardinality(false, false);
        assert!(c.is_bag());
        assert!(!c.is_list() && !c.is_set());
    }

    #[test]
    fn is_list_ordered_non_unique() {
        let c = cardinality(true, false);
        assert!(c.is_list());
        assert!(!c.is_bag() && !c.is_set());
    }

    #[test]
    fn is_set_unordered_unique() {
        let c = cardinality(false, true);
        assert!(c.is_set());
        assert!(!c.is_bag() && !c.is_list());
    }

    #[test]
    fn ordered_unique_is_none_of_the_three_named_kinds() {
        // (ordered, unique) is an ordered set — the spec names no predicate for
        // it, so all three classification functions are false.
        let c = cardinality(true, true);
        assert!(!c.is_bag() && !c.is_list() && !c.is_set());
    }
}
