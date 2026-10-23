// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The error pages: an address the viewer has no page for (404) and a page
//! the CDR refuses this reader (403), drawn with FerroUI's error page (R20).
//!
//! A signed-in reader sees them in the shell; the router's own fallback,
//! reached by nobody signed in, is the centred layout. No openEHR spec
//! governs the viewer's pages: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::error_page::{ErrorPage, Status, Way, error_page};
use ferrohealth_ui::view::shell::centred;
use leptos::prelude::*;
use leptos_meta::Title;

use crate::locale::{Msg, tr, tr_with};

/// The 404 page's body: the address that is not a page, and where EHRs live.
fn not_found_body() -> AnyView {
    let path = crate::request::path();
    crate::request::set_status(http::StatusCode::NOT_FOUND);
    error_page(ErrorPage {
        status: Status::NotFound,
        title: tr(Msg::Title404),
        body: tr_with(Msg::Body404, &[("path", path.into())]).into_any(),
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
    })
}

/// The router's fallback, outside the shell: the 404 in the centred layout.
#[must_use]
pub fn not_found_page() -> AnyView {
    let body = not_found_body();
    view! {
        <Title text=tr(Msg::Title404) />
        {centred(body, crate::shell::footer())}
    }
    .into_any()
}

/// The 404 page inside the shell, for an address under it that names no page.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn NotFoundPage() -> impl IntoView {
    view! {
        <Title text=tr(Msg::Title404) />
        {not_found_body()}
    }
}

/// The 403 page a page draws when the CDR refuses this reader what it shows:
/// `title` names the page, `needs` says what the CDR asks for.
#[must_use]
pub fn forbidden(title: String, needs: String) -> AnyView {
    crate::request::set_status(http::StatusCode::FORBIDDEN);
    let who = crate::request::session().map_or_else(String::new, |session| session.identity);
    error_page(ErrorPage {
        status: Status::Forbidden,
        title,
        body: tr_with(
            Msg::ForbiddenBody,
            &[("who", who.into()), ("needs", needs.into())],
        )
        .into_any(),
        hint: None,
        ways: vec![
            Way {
                label: tr(Msg::NavDashboard),
                href: "/".to_owned(),
                glyph: Some(Glyph::ARROW_LEFT),
            },
            Way {
                label: tr(Msg::YourAccess),
                href: "/access".to_owned(),
                glyph: None,
            },
        ],
        detail: None,
    })
}
