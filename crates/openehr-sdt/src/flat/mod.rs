// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR **Simplified Formats**: FLAT and STRUCTURED data instances and
//! the Web Template model.
//!
//! The wire authority is the ITS-REST Simplified Formats specification
//! (`docs/specs/openehr/ITS-REST/docs/simplified_formats/`, STABLE):
//! field-identifier syntax and node-id generation (`master04`), the
//! per-RM-type mapping tables (`master05`), the `ctx/` context vocabulary
//! (`master06`), and the FLAT ↔ STRUCTURED conversion algorithms
//! (`master04 §Conversion Between Formats`). Media types
//! (`master02 §MIME Types`): `application/openehr.wt.flat+json`,
//! `application/openehr.wt.structured+json`, and (for the template
//! resource) `application/openehr.wt+json`.
//!
//! Both wire variants are codecs over one internal tree ([`sim::SimNode`]);
//! the template-driven RM conversion is written once against it.
//!
//! Doc prose here is dense with openEHR spec class names (`COMPOSITION`,
//! `DV_QUANTITY`, `PARTY_IDENTIFIED`, …); backticking every occurrence is
//! noise, so `clippy::doc_markdown` is allowed module-wide.
#![allow(
    clippy::doc_markdown,
    reason = "the module docs name openEHR RM classes and Simplified-Formats key names throughout as prose; backticking every occurrence would drown the text"
)]

pub mod build;
// The WebTemplate cache is the crate's only `moka` user, and a browser-side
// consumer of the Simplified Formats keeps no such cache.
#[cfg(feature = "cache")]
pub mod cache;
pub mod convert;
pub(crate) mod ctx;
pub mod error;
pub mod example;
pub mod flatten;
pub(crate) mod map;
pub mod path;
pub(crate) mod rmpath;
pub mod sim;
pub mod tdd;
pub mod validation;
pub mod webtemplate;
