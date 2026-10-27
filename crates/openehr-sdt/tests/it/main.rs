// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for `openehr-sdt`: the Simplified Formats (Web Template /
//! FLAT / STRUCTURED) against the spec example vectors, the `master05`
//! mapping tables and the OPT corpus, and the RM-instance / template
//! validation passes.
//!
//! One binary per crate, split into topic modules.
#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

mod common;

mod archetype_id_grammar;
mod canonical_json_literals;
mod category_default;
mod ckm_full_pack;
mod cnf_vitals_template;
mod coded_names;
mod constraint_binding_capture;
mod content_constraint_capture;
mod content_existence_capture;
mod example_committable;
mod example_rm_validity;
mod example_roots;
mod example_stub;
mod flat;
mod format_parity;
mod master05_tables;
mod model_walkgen;
mod party_build;
mod raw_attributes;
mod shape_tolerance;
mod spec_vectors;
mod structured;
mod untagged_nodes;
mod validation;
mod validation_checklist;
mod validation_rules;
mod value_alternatives;
mod webtemplate;
mod webtemplate_v2_4;
