// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for `openehr-query`: the AQL 1.1.0 front end exercised
//! against the official vendored worked-example corpus (`vendor/examples/`).
//!
//! One binary per crate, split into topic modules
//! (`.claude/rules/testing.md` §One integration-test binary per crate).

mod backtracking;
mod bind;
mod corpus;
mod depth_bounds;
#[cfg(feature = "federation")]
mod federation;
mod parse_errors;
mod printer_round_trip;
mod spans;
mod visit;
