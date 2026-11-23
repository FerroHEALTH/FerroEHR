// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ADL 1.4 → ADL 2 conversion.
//!
//! NOTE: no openEHR spec governs the conversion algorithm: our own design,
//! pinned by the `tests/corpus/adl2-reference/upgrade/upgrade_from_14/**`
//! fixtures. The released text requires the conversion
//! (`ADL2/master01-preface.adoc` §Backward Compatibility), at-code to id-code
//! node ids (`master07.04` §Node Identifier Codes) and `v1.0.0` versions
//! (`master07.05`); the meta-data half follows
//! `ADL1.4/masterAppB-extended_metadata.adoc` and `AOM2/master01-preface.adoc`.

pub(crate) mod authored_flat;
mod code_shift;
pub mod convert;
pub mod differ;
pub mod domain;
pub mod log;
pub mod lower;
mod metadata;
mod multiplicity;
pub mod opt;
pub(crate) mod quantity_property;
pub mod specialised_codes;
pub(crate) mod term_constraint;
mod walk;
