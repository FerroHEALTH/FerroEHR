// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `ARCHETYPE_SLOT` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.archetype_slot.adoc` §Attributes +
//! §Functions.

use crate::v2_4::aom2::constraint_model::archetype_slot::ArchetypeSlot;
use crate::v2_4::beom::core::assertion::Assertion;

impl ArchetypeSlot {
    /// Returns true if this slot admits any archetype.
    ///
    /// `any_allowed` (`org.openehr.am.aom2.archetype_slot.adoc` §Functions):
    /// "True if no constraints stated, and slot is not closed" — the
    /// constraints being the same page's `includes` and `excludes` assertion
    /// lists, and `is_closed` its own mandatory flag.
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        !self.is_closed
            && self.includes.as_deref().is_none_or(<[Assertion]>::is_empty)
            && self.excludes.as_deref().is_none_or(<[Assertion]>::is_empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2_4::beom::core::expression::Expression;
    use openehr_lang::v1_1::beom::core::expr_literal::ExprLiteral;

    fn slot(
        includes: Option<Vec<Assertion>>,
        excludes: Option<Vec<Assertion>>,
        is_closed: bool,
    ) -> ArchetypeSlot {
        ArchetypeSlot {
            parent: None,
            soc_parent: None,
            rm_type_name: "CLUSTER".to_owned(),
            occurrences: None,
            node_id: "id2".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            includes,
            excludes,
            is_closed,
        }
    }

    fn an_assertion() -> Assertion {
        Assertion {
            tag: None,
            string_expression: None,
            expression: Box::new(Expression::ExprLiteral(ExprLiteral {
                item: serde_json::Value::Bool(true),
            })),
        }
    }

    #[test]
    fn an_open_slot_with_no_assertions_admits_anything() {
        assert!(slot(None, None, false).any_allowed());
        assert!(slot(Some(Vec::new()), Some(Vec::new()), false).any_allowed());
    }

    #[test]
    fn a_closed_slot_admits_nothing() {
        assert!(!slot(None, None, true).any_allowed());
    }

    #[test]
    fn one_assertion_on_either_list_is_already_a_constraint() {
        assert!(!slot(Some(vec![an_assertion()]), None, false).any_allowed());
        assert!(!slot(None, Some(vec![an_assertion()]), false).any_allowed());
    }
}
