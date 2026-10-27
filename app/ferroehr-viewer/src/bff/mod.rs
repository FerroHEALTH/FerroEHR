// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's server half over the CDR, area by area: the server functions
//! the pages call as they are drawn, and the pure readings of what the CDR
//! answers.
//!
//! Every server function guards with
//! [`require_session`](crate::session::require_session) first (it is a
//! public endpoint), and the CDR credential never leaves the server. The wire
//! is ITS-REST (`docs/specs/openehr/ITS-REST/`); the pages are our own design.

#[cfg(not(target_arch = "wasm32"))]
pub mod audit;
#[cfg(not(target_arch = "wasm32"))]
pub mod composition;
pub mod dashboard;
#[cfg(not(target_arch = "wasm32"))]
pub mod demographics;
#[cfg(not(target_arch = "wasm32"))]
pub mod ehr_detail;
#[cfg(not(target_arch = "wasm32"))]
pub mod ehr_tags;
#[cfg(not(target_arch = "wasm32"))]
pub mod ehrs;
#[cfg(not(target_arch = "wasm32"))]
pub mod item_tags;
#[cfg(not(target_arch = "wasm32"))]
pub mod system;
#[cfg(not(target_arch = "wasm32"))]
pub mod template_adl2;
#[cfg(not(target_arch = "wasm32"))]
pub mod template_detail;
#[cfg(not(target_arch = "wasm32"))]
pub mod templates;
#[cfg(not(target_arch = "wasm32"))]
pub mod wire;

/// The rows one page of a listing holds: FerroUI's default page size.
pub const PAGE_SIZE: u32 = ferrohealth_ui::table::model::DEFAULT_SIZE;

/// A body pretty-printed for reading in `format`: JSON indented, canonical
/// XML as the CDR sent it, and a body that is no JSON passed through whole.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
#[expect(
    clippy::disallowed_types,
    reason = "pretty-printing reads the CDR's JSON wire as an untyped tree"
)]
pub fn pretty_body(body: &str, format: crate::format::ReprFormat) -> String {
    match format {
        crate::format::ReprFormat::CanonicalXml => body.to_owned(),
        // NOTE: no specification governs this: our own design. A body that is
        // not JSON is legitimately not of this form and is shown as sent.
        _ => serde_json::from_str::<serde_json::Value>(body)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .unwrap_or_else(|_| body.to_owned()),
    }
}
