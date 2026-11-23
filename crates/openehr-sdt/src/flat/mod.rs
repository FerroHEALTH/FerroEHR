// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR Simplified Formats: FLAT and STRUCTURED data instances and the Web
//! Template model.
//!
//! ITS-REST `simplified_formats/` is the wire authority: `master04` field
//! identifiers and node ids, `master05` RM mappings, `master06` `ctx/`, and the
//! media types of `master02 §MIME Types`. Both wire variants are codecs over
//! [`sim::SimNode`].
#![allow(
    clippy::doc_markdown,
    reason = "the module docs name openEHR RM classes and Simplified-Formats key names throughout as prose; backticking every occurrence would drown the text"
)]

pub mod attribute_keys;
pub mod build;
// The WebTemplate cache is the crate's only `moka` user, and a browser-side
// consumer of the Simplified Formats keeps no such cache.
#[cfg(feature = "cache")]
pub mod cache;
pub mod convert;
pub(crate) mod ctx;
mod ctx_emit;
mod defaults;
pub mod error;
pub mod example;
pub mod flatten;
pub(crate) mod identity;
pub(crate) mod instance;
mod instant;
pub(crate) mod map;
pub mod navigate;
#[cfg(test)]
mod parity;
pub mod path;
pub(crate) mod rmpath;
pub mod sim;
mod stubbed;
pub(crate) mod typed;
mod uid;
pub mod validation;
pub mod webtemplate;
