// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `VERSIONED_OBJECT` functions
//! (`org.openehr.rm.common.versioned_object.adoc` §Functions) all need the
//! stored version set, so the repository realizes them, not this value.
//!
//! NOTE: `UpdateVersion.yaml` declares no merge-commit shape, so
//! `commit_original_merged_version` has no wire realization.
