// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR **Simplified Data Template** formats and the engines written over
#![allow(
    clippy::doc_markdown,
    reason = "module docs are prose with many proper nouns"
)]
//!
//! the ITS wire layer of `openehr_its`. Everything here is hand-written: the
//! specifications it implements are prose sub-specifications of ITS-REST with
//! no machine-readable model to generate from.
//!
//! - [`flat`] — **ITS-REST Formats**: openEHR Simplified Formats (FLAT /
//!   STRUCTURED data instances, the Web Template model, TDD import). The
//!   *Formats* specification is a STABLE ITS-REST 1.1.0 sub-specification
//!   (`docs/specs/openehr/ITS-REST/docs/simplified_formats/`) over the
//!   Simplified Data Template (`simplified_data_template/`).
//! - [`rm_instance`] — the template-independent validation of a whole RM
//!   instance tree: the RM-invariant and terminology passes, the
//!   [`rm_instance::ValidationMessage`] report shape, and the composed
//!   [`rm_instance::validate_composition`] entry point. It composes the
//!   template pass of `flat::validation` over the RM pass, and
//!   `flat::validation` reads its report shape back, so the two are one layer.
//! - [`smart_scopes`] — the SMART on openEHR resource-scope grammar
//!   (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`),
//!   std-only, always compiled.
//!
//! # Features
//!
//! `flat` is the content layer: it pulls `openehr_its` with its `opt14`
//! feature (canonical JSON, canonical XML and the OPT 1.4 reader) and the five
//! generated spec crates. `cache` adds the `moka` WebTemplate cache. The default
//! `full` is both. A `wasm32-unknown-unknown` consumer takes
//! `default-features = false, features = ["flat"]`.
//!
//! Taken with `default-features = false` and nothing else the crate compiles to
//! [`smart_scopes`] alone — no dependency of any kind — so a REST client that
//! must parse scope strings in the browser (the viewer's scope previewer)
//! shares the very grammar the CDR enforces instead of carrying a second parser.

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
#[cfg(feature = "flat")]
pub mod flat;
#[cfg(feature = "flat")]
pub mod rm_instance;
pub mod smart_scopes;

/// The openEHR specification version this crate implements.
///
/// The ITS-REST release the Simplified Formats and SMART App Launch
/// sub-specifications belong to. The pin is deliberately independent of the
/// crates.io package version, which is the crate's own `SemVer` line and moves
/// only with this implementation's code.
pub const SPEC_VERSION: &str = "1.1.0";
