// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariants for `EXTRACT`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr_extract.extract.adoc`
//! §Invariants — `Sequence_nr_valid` (`sequence_nr >= 1`), evaluated by the
//! generated core.

use crate::v1_2::ehr_extract::common::extract::Extract;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Extract {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::extract_core(self.sequence_nr, out);
    }
}
