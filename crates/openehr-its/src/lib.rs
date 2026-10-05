// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! openEHR **ITS** — Implementation Technology Specifications.
//!
//! This crate mirrors the four `specifications-ITS-*` sub-repos (aggregated by
//! `specifications-ITS`): how openEHR RM instances are serialized and exposed.
//!
//! - [`json`] — **ITS-JSON**: canonical JSON. The named entry points
//!   (`to_canonical_json`/`from_canonical_json`/`from_canonical_value`, over the
//!   native [`json_codec`]) + the vendored ITS-JSON schema + the interop
//!   fidelity gate (round-trip a vendored real-world canonical-JSON corpus,
//!   `tests/`).
//! - [`json_codec`] — **ITS-JSON**, native codec: emitted `ToJson`/`FromJson`
//!   impls over a hand-written writer/reader runtime — the canonical-JSON
//!   (de)serialization for every spec type (there is no serde derive on the spec
//!   types; the codec owns the `_type` / number-typing / omission contract).
//! - [`wire_validate`] — the wire-boundary RM class-invariant dispatch layer:
//!   reads a canonical-JSON node, deserializes via the codec, and runs the RM
//!   invariant cores of `openehr_rm::v1_2::validate` (the `Validate` impls and every
//!   value-level decision stay in `openehr-rm`/`openehr-base`).
//! - [`xml`] — **ITS-XML**: canonical XML via `quick-xml`, validated against the
//!   vendored XSDs (`schemas/xml/`).
//! - [`rest`] — **ITS-REST**: the openEHR REST API contract, generated from the
//!   vendored OpenAPI (`vendor/rest-oas/`): the DTOs and route tables under
//!   `rest`, the per-group server traits under `rest-server`, and the per-group
//!   clients with their runtime `rest::client` under `rest-client`.
//! - **ITS-BMM** has no module here: the vendored BMM meta-model that drives
//!   code generation lives in `openehr-codegen/vendor/bmm`, and the runtime
//!   BMM object model is `openehr-lang`.
//!
//! Serialization here is native codec machinery: the canonical-JSON `ToJson`/
//! `FromJson` codec and the canonical-XML codec are generated over hand-written
//! runtimes, so the spec crates carry no serde derive.
//!
//! # Features
//!
//! The default `full` feature is every surface above. The graph underneath it
//! is layered so a consumer takes only what it reads: `json` is the canonical
//! JSON base, `xml` adds the XML codecs and `opt14` the operational-template
//! reader. A `wasm32-unknown-unknown` consumer takes
//! `default-features = false, features = ["opt14"]`, which pulls that whole
//! chain; `schema-validation` (`jsonschema` plus the compiled-in ITS-JSON RM
//! schema) and the three ITS-REST features stay outside it. `rest` is the
//! contract both halves share (DTOs, param structs, route tables, `ApiError`)
//! over serde and `http`; `rest-server` adds the server traits and `axum`;
//! `rest-client` adds the generated clients and the client runtime over
//! `reqwest`.
//!
//! The Simplified Formats (FLAT / STRUCTURED / Web Template), the RM-instance
//! validation passes and the SMART scope grammar are hand-written engines over
//! this wire layer and live in the `openehr-sdt` crate.

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
#![allow(
    clippy::doc_markdown,
    reason = "module docs are prose with many proper nouns"
)]
#[cfg(feature = "xml")]
pub mod aom2;
#[cfg(feature = "xml")]
pub mod aom2_model;
#[cfg(feature = "json")]
pub mod json;
#[cfg(feature = "json")]
pub mod json_codec;
#[cfg(feature = "opt14")]
pub mod opt14;
pub mod rest;
#[cfg(feature = "json")]
pub mod wire_validate;
#[cfg(feature = "xml")]
pub mod xml;

/// The openEHR specification version this crate implements.
///
/// The pin is deliberately independent of the crates.io package version,
/// which is the crate's own `SemVer` line and moves only with this
/// implementation's code.
pub const SPEC_VERSION: &str = "1.1.0";
