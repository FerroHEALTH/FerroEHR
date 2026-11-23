// @generated-from-template templates/openehr-rm/data_types/encapsulated/dv_multimedia_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `DV_MULTIMEDIA` invariants (`org.openehr.rm.data_types.dv_multimedia.adoc`):
//! `Not_empty`, `Integrity_check_validity` and `Size_valid` here; the five
//! code-set invariants in `validate::terminology`.

use crate::v1_1::data_types::encapsulated::dv_multimedia::DvMultimedia;
use openehr_base::validate::{InvariantViolation, Validate};

impl DvMultimedia {
    /// Whether the data is inline: `data` is present (`is_inline`).
    #[must_use]
    pub const fn is_inline(&self) -> bool {
        self.data.is_some()
    }

    /// Whether the data is external: `uri` is present (`is_external`); inline and
    /// external are not exclusive.
    #[must_use]
    pub const fn is_external(&self) -> bool {
        self.uri.is_some()
    }

    /// Whether `compression_algorithm` is present (`is_compressed`).
    #[must_use]
    pub const fn is_compressed(&self) -> bool {
        self.compression_algorithm.is_some()
    }

    /// Whether `integrity_check_algorithm` is present (`has_integrity_check`).
    #[must_use]
    pub const fn has_integrity_check(&self) -> bool {
        self.integrity_check_algorithm.is_some()
    }
}

impl Validate for DvMultimedia {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if !self.is_inline() && !self.is_external() {
            out.push(InvariantViolation::here(
                "Invariant Not_empty failed on type DV_MULTIMEDIA",
            ));
        }
        if self.integrity_check.is_some() && self.integrity_check_algorithm.is_none() {
            out.push(InvariantViolation::here(
                "Invariant Integrity_check_validity failed on type DV_MULTIMEDIA",
            ));
        }
        if self.size < 0 {
            out.push(InvariantViolation::here(
                "Invariant Size_valid failed on type DV_MULTIMEDIA",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use openehr_base::v1_2::prelude::TerminologyId;

    fn media_type() -> CodePhrase {
        CodePhrase {
            terminology_id: TerminologyId {
                value: "IANA_media-types".to_owned(),
            },
            code_string: "image/png".to_owned(),
            preferred_term: None,
        }
    }

    fn valid() -> DvMultimedia {
        DvMultimedia {
            charset: None,
            language: None,
            alternate_text: None,
            uri: None,
            data: Some("AAA=".to_owned()),
            media_type: media_type(),
            compression_algorithm: None,
            integrity_check: None,
            integrity_check_algorithm: None,
            thumbnail: None,
            size: 1,
        }
    }

    #[test]
    fn valid_multimedia() {
        assert_eq!(valid().invariants(), Vec::<InvariantViolation>::new());
    }

    #[test]
    fn empty_invalid() {
        let mut m = valid();
        m.data = None;
        let v = m.invariants();
        assert!(
            v.iter()
                .any(|x| x.message == "Invariant Not_empty failed on type DV_MULTIMEDIA")
        );
    }

    #[test]
    fn integrity_check_without_algorithm() {
        let mut m = valid();
        m.integrity_check = Some("deadbeef".to_owned());
        let v = m.invariants();
        assert!(v.iter().any(
            |x| x.message == "Invariant Integrity_check_validity failed on type DV_MULTIMEDIA"
        ));
    }

    #[test]
    fn negative_size_invalid() {
        let mut m = valid();
        m.size = -10;
        let v = m.invariants();
        assert!(
            v.iter()
                .any(|x| x.message == "Invariant Size_valid failed on type DV_MULTIMEDIA")
        );
    }

    /// `dv_multimedia.adoc` §Invariants `Size_valid` is `size >= 0`, so a
    /// zero-length multimedia is valid.
    #[test]
    fn zero_size_valid_per_spec() {
        let mut m = valid();
        m.size = 0;
        assert_eq!(m.invariants(), Vec::<InvariantViolation>::new());
    }

    /// The four predicates, `is_inline` and `is_external` both true on one value.
    #[test]
    fn the_computed_predicates_follow_their_attributes() {
        let mut m = valid();
        assert!(m.is_inline(), "data is present");
        assert!(!m.is_external(), "no uri is present");
        assert!(!m.is_compressed());
        assert!(!m.has_integrity_check());

        m.uri = Some(crate::v1_1::data_types::uri::dv_uri::DvUri::DvUri(
            crate::v1_1::data_types::uri::dv_uri::DvUriData {
                value: "s3://bucket/key".to_owned(),
            },
        ));
        assert!(
            m.is_inline() && m.is_external(),
            "Not_empty is an OR: a value stored both ways answers true to both"
        );
        assert_eq!(m.invariants(), Vec::<InvariantViolation>::new());
    }
}
