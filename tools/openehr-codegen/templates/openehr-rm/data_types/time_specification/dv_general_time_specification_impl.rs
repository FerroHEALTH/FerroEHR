// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `DV_GENERAL_TIME_SPECIFICATION` extraction functions are not realized:
//! the GTS grammar (`master08-time_specification_package.adoc` §General Time
//! Specification Syntax) leaves `symbol` undefined, and a union carries
//! several alignments where the functions return one. The per-factor
//! functions are on `DV_PERIODIC_TIME_SPECIFICATION`.
