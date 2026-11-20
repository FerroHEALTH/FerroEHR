// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The published definition of the access log details tree (#6143).
//!
//! It is the archetype `ACCESS_LOG_ENTRY.other_details` is kept under,
//! shipped with the default definitions so another system reads a stored or
//! exported entry by it. No openEHR spec governs these fields: our own design.

/// The archetype id the details tree is kept under.
pub const ARCHETYPE: &str = "openEHR-EHR-ITEM_TREE.access_log_details.v1";

/// The shipped definition of [`ARCHETYPE`], its file name and its ADL 2 text,
/// which each boot stores when no definition of its id is stored.
pub const DEFINITIONS: [(&str, &str); 1] = [(
    "openEHR-EHR-ITEM_TREE.access_log_details.v1.0.0.adls",
    include_str!(
        "../../assets/definitions/access_log/openEHR-EHR-ITEM_TREE.access_log_details.v1.0.0.adls"
    ),
)];
