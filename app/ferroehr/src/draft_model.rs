// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reference model archetypes are validated against: the openEHR RM
//! 1.2.0 extended by the classes of FerroEHR's draft packages.
//!
//! An archetype over a draft class, such as the default
//! `openEHR-EHR-ACCESS_POLICY` archetype, names a class the published RM does
//! not define, so it is validated against the RM adapted by the draft's own
//! class table (`docs/specs/openehr/AM/docs/AOM2/master11-rm_adaptation.adoc`).
//! The draft packages are FerroEHR's change proposals and no openEHR release:
//! our own design (#5512).

use openehr_adl::validate::rm_extended::ExtendedRmModel;
use openehr_rm::v1_2::model::RmClass;

/// The openEHR RM 1.2.0 with the classes of the access control and access
/// log drafts.
pub static RM_WITH_DRAFTS: ExtendedRmModel = ExtendedRmModel::new(
    "openEHR RM 1.2.0 with the FerroEHR access control and access log drafts",
    &DRAFT_CLASSES,
);

/// The class tables of the draft packages.
static DRAFT_CLASSES: [&[RmClass]; 2] = [
    crate::access_control_draft::model::CLASSES,
    crate::access_log_draft::model::CLASSES,
];
