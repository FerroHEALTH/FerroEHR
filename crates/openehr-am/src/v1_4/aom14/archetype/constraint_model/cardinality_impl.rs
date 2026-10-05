// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM 1.4 `CARDINALITY` spec functions.
//!
//! `CARDINALITY` states whether a container attribute behaves as a list, set or
//! bag, from the `is_ordered`/`is_unique` attribute pair.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom14.cardinality.adoc` §Functions.

use crate::v1_4::aom14::archetype::constraint_model::cardinality::Cardinality;

impl Cardinality {
    /// Returns true if this cardinality represents a bag.
    ///
    /// `is_bag` (`org.openehr.am.aom14.cardinality.adoc` §Functions): "unordered,
    /// non-unique membership".
    #[must_use]
    pub fn is_bag(&self) -> bool {
        !self.is_ordered && !self.is_unique
    }

    /// Returns true if this cardinality represents a list.
    ///
    /// `is_list` (`org.openehr.am.aom14.cardinality.adoc` §Functions): "ordered,
    /// non-unique membership".
    #[must_use]
    pub fn is_list(&self) -> bool {
        self.is_ordered && !self.is_unique
    }

    /// Returns true if this cardinality represents a set.
    ///
    /// `is_set`: unordered, unique membership. The AOM 1.4 class page repeats
    /// the `is_bag` wording in the `is_set` row
    /// (`org.openehr.am.aom14.cardinality.adoc` §Functions), which would make
    /// the two functions identical and leave "set" undefined; the governing
    /// definition is the BASE class this one mirrors
    /// (`BASE/docs/UML/classes/org.openehr.base.foundation_types.cardinality.adoc`
    /// §Functions, `is_set` = `not is_ordered and is_unique`).
    #[must_use]
    pub fn is_set(&self) -> bool {
        !self.is_ordered && self.is_unique
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::v1_3::foundation_types::interval::interval::Interval;
    use openehr_base::v1_3::foundation_types::interval::proper_interval::{
        ProperInterval, ProperIntervalData,
    };

    fn cardinality(is_ordered: bool, is_unique: bool) -> Cardinality {
        Cardinality {
            interval: Interval::ProperInterval(ProperInterval::ProperInterval(
                ProperIntervalData {
                    lower: Some(0),
                    upper: None,
                    lower_unbounded: false,
                    upper_unbounded: true,
                    lower_included: true,
                    upper_included: false,
                },
            )),
            is_ordered,
            is_unique,
        }
    }

    #[test]
    fn the_three_classifications_partition_the_ordered_unique_pairs() {
        assert!(cardinality(false, false).is_bag());
        assert!(cardinality(true, false).is_list());
        assert!(cardinality(false, true).is_set());
    }

    #[test]
    fn an_ordered_unique_container_is_none_of_the_three() {
        let both = cardinality(true, true);
        assert!(!both.is_bag());
        assert!(!both.is_list());
        assert!(!both.is_set());
    }

    #[test]
    fn the_classifications_are_mutually_exclusive() {
        for (ordered, unique) in [(false, false), (true, false), (false, true), (true, true)] {
            let c = cardinality(ordered, unique);
            let set = usize::from(c.is_bag()) + usize::from(c.is_list()) + usize::from(c.is_set());
            assert!(set <= 1, "ordered={ordered} unique={unique} matched {set}");
        }
    }
}
