// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for `openehr-lang`: the shared lexical layer, the ODIN +
//! `P_BMM` readers and the Basic Expression Language parser — the per-language
//! lexer-surface battery, the vendored-fixture batteries (`tests/vendor/**`),
//! their 100%-coverage gate, the adjudicated `P_BMM` schema→`BMM_MODEL` outcome
//! table over the same corpus, the `rm_access` schema-repository facade over a
//! temp-directory copy of that corpus, and BEL parsing.
//!
//! One binary per crate, split into topic modules
//! (`.claude/rules/testing.md` §One integration-test binary per crate).

mod bel_parse;
mod bmm3_model;
mod bmm_enumeration_validity;
mod el_assertions;
mod escape_validation;
mod lexer_equivalence;
mod odin_spec_examples;
mod rm_access;
mod vendor_bmm_odin;
mod vendor_bmm_schema;
mod vendor_coverage;
mod vendor_odin;
