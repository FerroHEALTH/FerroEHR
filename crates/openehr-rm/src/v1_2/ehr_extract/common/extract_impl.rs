// @generated-from-template templates/openehr-rm/ehr_extract/common/extract_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `EXTRACT.Sequence_nr_valid` (RM 1.2.0
//! `org.openehr.rm.ehr_extract.extract.adoc`), in the generated core.

use crate::v1_2::ehr_extract::common::extract::Extract;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Extract {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::extract_core(self.sequence_nr, out);
    }
}
