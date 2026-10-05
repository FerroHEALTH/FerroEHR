// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 4 — RENDER. The text producers: given the analysed model and the
//! planned shapes, emit deterministic, byte-stable Rust source. This is the
//! only stage that writes code text. [`emit`] renders the BMM spec crates;
//! [`emit_json`] the canonical-JSON `ToJson` codecs; [`emit_xml`] the
//! canonical-XML codecs; [`emit_rest`] the ITS-REST contract;
//! [`emit_rm_model`] the static RM attribute/type model; [`emit_validate`] the
//! RM class-invariant cores; [`emit_opt`] the OPT 1.4 model; [`model_types`]
//! turns an analysed BMM type into the Rust type text every emitter writes (and
//! the import set that must agree with it); [`naming`] is the
//! shared identifier-casing helper. [`spdx`] is the SPDX licensing header every
//! emitted file carries, so licensing survives a generated file being copied
//! out of this repository. [`model_query`] renders the read-only
//! `model-query` report (BMM facts beside the current field-shape decisions) —
//! a text producer over the same stages, never generated code.

pub(crate) mod emit;
pub(crate) mod emit_json;
pub(crate) mod emit_opt;
pub(crate) mod emit_rest;
pub(crate) mod emit_rm_model;
pub(crate) mod emit_templates;
pub(crate) mod emit_validate;
pub(crate) mod emit_xml;
pub(crate) mod model_query;
pub(crate) mod model_types;
pub(crate) mod naming;
pub(crate) mod spdx;
