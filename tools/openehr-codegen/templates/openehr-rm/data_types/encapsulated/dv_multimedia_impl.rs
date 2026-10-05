// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariants for `DV_MULTIMEDIA`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_multimedia.adoc`
//! §Invariants. Enforced here — the three that need no code set:
//! - `Not_empty` (`is_inline or is_external`): `is_inline` is "computed from the
//!   value of the data attribute" and `is_external` from `uri` (§Functions), so
//!   at least one of `data` / `uri` must be present.
//! - `Integrity_check_validity` (`integrity_check /= Void implies
//!   integrity_check_algorithm /= Void`).
//! - `Size_valid` (`size >= 0`), which admits `size = 0`.
//!
//! The five code-set-bound invariants — `Media_type_valid`,
//! `Compression_algorithm_validity`, `Integrity_check_algorithm_validity`, and
//! DV_ENCAPSULATED's `Charset_valid` / `Language_valid` — cannot be decided in
//! this crate, which has no terminology dependency; they are enforced in the
//! terminology-aware path (`validate::terminology`, the `DV_MULTIMEDIA` slots)
//! against the `openehr-term` bundle.

use crate::v1_2::data_types::encapsulated::dv_multimedia::DvMultimedia;
use openehr_base::validate::{InvariantViolation, Validate};

impl DvMultimedia {
    /// Returns `true` when the data is stored in expanded form, within the EHR
    /// itself.
    ///
    /// Spec: `dv_multimedia.adoc` §Functions — "Computed from the value of the
    /// data attribute."
    #[must_use]
    pub const fn is_inline(&self) -> bool {
        self.data.is_some()
    }

    /// Returns `true` when the data is stored externally to the record.
    ///
    /// Spec: `dv_multimedia.adoc` §Functions — "Computed from the value of the
    /// `uri` attribute". Inline and external are not exclusive: the invariant
    /// is `is_inline or is_external`, and a copy may be held both ways.
    #[must_use]
    pub const fn is_external(&self) -> bool {
        self.uri.is_some()
    }

    /// Returns `true` when the data is stored in compressed form.
    ///
    /// Spec: `dv_multimedia.adoc` §Functions — "Computed from the value of the
    /// `compression_algorithm` attribute."
    #[must_use]
    pub const fn is_compressed(&self) -> bool {
        self.compression_algorithm.is_some()
    }

    /// Returns `true` when an integrity check has been computed.
    ///
    /// Spec: `dv_multimedia.adoc` §Functions — "Computed from the value of the
    /// `integrity_check_algorithm` attribute."
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
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use openehr_base::v1_3::prelude::TerminologyId;

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
        assert!(valid().invariants().is_empty());
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
        assert!(m.invariants().is_empty());
    }

    /// The four §Functions predicates, each "computed from the value of" one
    /// attribute.
    ///
    /// `is_inline` and `is_external` are asserted TOGETHER on one value that
    /// carries both: the invariant is `is_inline or is_external`, an OR, so a
    /// value held inline AND externally is valid and both must answer true.
    /// Testing them apart would pass on an implementation that treated them as
    /// exclusive.
    #[test]
    fn the_computed_predicates_follow_their_attributes() {
        let mut m = valid();
        assert!(m.is_inline(), "data is present");
        assert!(!m.is_external(), "no uri is present");
        assert!(!m.is_compressed());
        assert!(!m.has_integrity_check());

        m.uri = Some(crate::v1_2::data_types::uri::dv_uri::DvUri::DvUri(
            crate::v1_2::data_types::uri::dv_uri::DvUriData {
                value: "s3://bucket/key".to_owned(),
            },
        ));
        assert!(
            m.is_inline() && m.is_external(),
            "Not_empty is an OR: a value stored both ways answers true to both"
        );
        assert!(m.invariants().is_empty());
    }
}
