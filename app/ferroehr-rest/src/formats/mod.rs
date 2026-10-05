// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST **Simplified Formats** wire adapter (STABLE).
//!
//! Serves FLAT + STRUCTURED data-instance JSON and their media types
//! (`application/openehr.wt.flat+json`,
//! `application/openehr.wt.structured+json`, `master02 §MIME Types`) per
//! `docs/specs/openehr/ITS-REST/docs/simplified_formats/`.
//!
//! The conversion engine lives in `openehr_sdt::flat`; this module is the
//! wire seam ([`dispatch`]: COMPOSITION FLAT/STRUCTURED I/O, the CONTRIBUTION
//! envelope rule, and the uniform reject for spec-silent resources), composed
//! with the negotiation core in [`crate::overview::negotiate`].

pub mod dispatch;
