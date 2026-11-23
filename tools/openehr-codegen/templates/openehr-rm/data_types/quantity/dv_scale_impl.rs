// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! `DV_SCALE` invariants (RM 1.2.0 `org.openehr.rm.data_types.dv_scale.adoc`):
//! the inherited `Normal_range_and_status_consistency`;
//! `Normal_status_validity` is terminology-bound.
//!
//! NOTE: `dv_scale.adoc` permits an uncoded scale point, while CODE_PHRASE
//! `Code_string_valid` forbids an empty code; the invariant is enforced.

use crate::v1_2::data_types::quantity::dv_ordered::DvOrdered;
use crate::v1_2::data_types::quantity::dv_ordered_impl::push_normal_range_consistency;
use crate::v1_2::data_types::quantity::dv_scale::DvScale;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for DvScale {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        push_normal_range_consistency(
            out,
            "DV_SCALE",
            self.normal_status.as_deref(),
            self.normal_range.as_deref(),
            &DvOrdered::from(self.clone()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use openehr_base::v1_3::prelude::TerminologyId;

    fn scale(value: f64) -> DvScale {
        DvScale {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            symbol: DvCodedText {
                value: "very very slight".to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: openehr_base::containers::present_nonempty(Vec::new()),
                language: None,
                encoding: None,
                defining_code: CodePhrase {
                    terminology_id: TerminologyId {
                        value: "local".to_owned(),
                    },
                    code_string: "at0002".to_owned(),
                    preferred_term: None,
                },
            },
            value,
        }
    }

    #[test]
    fn plain_scale_valid() {
        assert_eq!(scale(0.5).invariants(), Vec::<InvariantViolation>::new());
    }

    #[test]
    fn ordering_functions_allow_non_integer_values() {
        assert_eq!(scale(0.5).less_than(&scale(1.0)), Some(true));
        assert!(scale(0.5).is_strictly_comparable_to(&scale(3.0)));
    }
}
