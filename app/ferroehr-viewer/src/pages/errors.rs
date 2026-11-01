// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The error pages: an address the viewer has no page for (404) and a page
//! the CDR refuses this reader (403), drawn with FerroUI's error page (R20).
//!
//! A signed-in reader sees them in the shell; the session guard sends anyone
//! else to the sign-in page first. No openEHR spec governs the viewer's
//! pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::error_page::{ErrorPage, Status, Way, error_page};

use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::t;
use crate::routes::Drawn;

/// The 404 page's body: the address that is not a page, and where EHRs live.
fn not_found_body() -> Html {
    let path = crate::request::path();
    crate::request::set_status(http::StatusCode::NOT_FOUND);
    error_page(
        &current(),
        ErrorPage {
            status: Status::NotFound,
            title: tr(Msg::Title404),
            body: t(tr_with(Msg::Body404, &[("path", path.into())])),
            hint: None,
            ways: vec![
                Way {
                    label: tr(Msg::NavEhrs),
                    href: "/ehrs".to_owned(),
                    glyph: Some(Glyph::DATABASE),
                },
                Way {
                    label: tr(Msg::NavDashboard),
                    href: "/".to_owned(),
                    glyph: None,
                },
            ],
            detail: None,
        },
    )
}

/// The 404 page, for an address that names no page.
#[must_use]
pub fn not_found_page() -> Drawn {
    Drawn::page(tr(Msg::Title404), not_found_body())
}
