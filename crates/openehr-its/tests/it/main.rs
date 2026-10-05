// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for `openehr-its`: the openEHR Implementation
//! Technology Specification surfaces — canonical JSON (fidelity, contract,
//! codec parity, ITS-JSON schema validation), canonical XML (round-trip,
//! namespaces, C14N/hash, locatable attributes), the generated ITS-REST
//! contract with its client half (wiremock contract tests), its server half
//! (the generated routers, driven without a socket) and the operation matcher,
//! and the wire-boundary RM validation dispatch.
//!
//! One binary per crate, split into topic modules
//! (`.claude/rules/testing.md` §One integration-test binary per crate);
//! `common` is the shared corpus plumbing, never a test binary of its own.

#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

mod common;

mod aom2_model_xml;
mod aom2_xml;
mod attested_version;
mod canonical_contract;
mod canonical_json_literals;
mod ckm_archetype_xml;
mod corpus;
mod dispatch_lockstep;
mod fidelity;
mod fixture_twins;
mod its_json_delta;
mod json_codec_parity;
mod nonempty_wire;
mod oas_update_version_sync;
mod opt14_corpus;
mod opt14_ordinal_symbol;
mod opt14_v1_4_divergence;
mod rest_client;
mod rest_client_runtime;
mod rest_contract;
mod rest_routes;
mod rest_server;
mod rm_validation;
mod xml_abstract_root;
mod xml_c14n;
mod xml_diagnostics;
mod xml_ehrbase;
mod xml_hash;
mod xml_hostile_input;
mod xml_locatable_attr;
mod xml_namespace;
mod xml_roundtrip;
mod xml_smoke;
mod xml_xsd_validity;
