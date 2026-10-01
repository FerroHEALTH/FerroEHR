// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for `openehr-query`: the AQL 1.1.0 front end exercised
//! against the official vendored worked-example corpus (`vendor/examples/`).
//!
//! One binary per crate, split into topic modules
//! (`.claude/rules/testing.md` §One integration-test binary per crate).

mod backtracking;
mod bind;
mod corpus;
#[cfg(feature = "federation")]
mod federation;
mod parse_errors;
mod printer_round_trip;
mod spans;
mod visit;
