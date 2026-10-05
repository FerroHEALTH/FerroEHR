// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariants for `POINT_EVENT`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.point_event.adoc`.
//! `POINT_EVENT` declares no own invariants; the inherited LOCATABLE
//! `Archetype_node_id_valid` applies.
//!
//! NOTE: the inherited `EVENT.Offset_validity1`
//! (`offset = time.diff(parent.origin)`) constrains the *computed* `offset()`
//! function against the (unstored) parent origin — with `offset` realised as
//! [`Event::offset_from`](crate::v1_2::data_structures::history::event::Event)
//! computed from `time`, it holds by construction and is not a runnable check
//! on this type.

use crate::v1_2::data_structures::history::point_event::PointEvent;
use openehr_base::validate::{InvariantViolation, Validate};

impl<T> Validate for PointEvent<T> {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::archetype_node_id_core(
            "POINT_EVENT",
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};

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

    fn point_event(node_id: &str) -> PointEvent<i32> {
        PointEvent {
            name: text("event"),
            archetype_node_id: node_id.to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            time: DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                magnitude_status: None,
                accuracy: None,
                value: "2021-01-01T00:00:00".to_owned(),
            },
            state: None,
            data: 1,
        }
    }

    #[test]
    fn valid_event() {
        assert!(point_event("at0002").invariants().is_empty());
    }

    #[test]
    fn empty_archetype_node_id_invalid() {
        let v = point_event("").invariants();
        assert!(
            v.iter().any(
                |m| m.message == "Invariant Archetype_node_id_valid failed on type POINT_EVENT"
            ),
            "got {v:?}"
        );
    }
}
