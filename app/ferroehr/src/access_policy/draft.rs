// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one place the access policy names the generated draft access control
//! types, so a move of the draft module changes this file alone.

/// The generated draft `ACCESS_POLICY`.
pub type Policy = crate::access_control_draft::access_control::access_policy::AccessPolicy;

/// The generated draft `VERSIONED_ACCESS_POLICY`.
pub type VersionedPolicy =
    crate::access_control_draft::access_control::versioned_access_policy::VersionedAccessPolicy;
