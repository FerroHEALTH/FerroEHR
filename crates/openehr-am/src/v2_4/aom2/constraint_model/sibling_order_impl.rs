// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `SIBLING_ORDER` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.sibling_order.adoc` §Attributes +
//! §Functions.

use crate::v2_4::aom2::constraint_model::sibling_order::SiblingOrder;

impl SiblingOrder {
    /// Returns true if the ordered node comes after its named sibling.
    ///
    /// `is_after` (`org.openehr.am.aom2.sibling_order.adoc` §Functions): "True
    /// if the order relationship is `after`, computed as the negation of
    /// `is_before`".
    #[must_use]
    pub fn is_after(&self) -> bool {
        !self.is_before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(is_before: bool) -> SiblingOrder {
        SiblingOrder {
            is_before,
            sibling_node_id: "id4".to_owned(),
        }
    }

    #[test]
    fn after_is_the_complement_of_before() {
        assert!(!order(true).is_after());
        assert!(order(false).is_after());
    }
}
