// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for `openehr-adl`: the ADL 2.4 engine — the ADL2/cADL/ODIN
//! parser, AOM2 validation (basic integrity, parent + RM conformance, flat
//! form), the flattener, OPT2 generation, ADL 1.4 reading and 1.4→2 conversion
//! — driven by the vendored reference corpora under `tests/corpus/**` and their
//! coverage gate.
//!
//! One binary per crate, split into topic modules
//! (`.claude/rules/testing.md` §One integration-test binary per crate).

#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

mod adl14_assertions;
mod adl14_cadl_gates;
mod adl14_cnf_fixture_slot;
mod adl14_conversion;
mod adl14_custom_constraints;
mod adl14_dadl_breadth;
mod adl14_header_sections;
mod assembly;
mod ckm_archetype_packs;
mod ckm_conversion_breadth;
mod corpus_coverage;
mod corpus_definition_parse;
mod corpus_definition_structure;
mod corpus_lex;
mod corpus_outer_parse;
mod corpus_roundtrip;
mod corpus_rules_parse;
mod corpus_validity_integrity;
mod corpus_validity_parent_conformance;
mod corpus_validity_rm;
mod default_value_intervals;
mod flattener_spec;
mod legacy14_corpus;
mod nesting_bounds;
mod opt_spec;
mod rules_parse;
mod templates_corpus;
mod validate_source_flat_form;
mod validation_adl14;
mod validation_integrity_cases;
mod vetdf_terminology;
