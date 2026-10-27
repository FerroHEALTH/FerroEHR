// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The position of a listing that can show patient data, kept out of the URL.
//!
//! FerroUI's mandate (its `leptos-ui.md` §0): a listing that can show patient
//! data keeps its page and size out of the address. Its pager posts them
//! (`table::pager::post_open_pager`) to [`PAGING_ROUTE`], which keeps them in
//! a cookie that lasts the browser's session, never a URL or the history, and
//! sends the browser back to the listing, which the server draws at that
//! position. The cookie names the listing and holds two numbers, never patient
//! data. No openEHR spec governs this: our own design.

#[cfg(not(target_arch = "wasm32"))]
use axum::extract::Form;
#[cfg(not(target_arch = "wasm32"))]
use axum::response::{IntoResponse, Redirect, Response};
use ferrohealth_ui::table::model::Paging;
#[cfg(not(target_arch = "wasm32"))]
use http::StatusCode;

/// The route a session listing's pager posts its page and size to.
pub const PAGING_ROUTE: &str = "/paging";

/// The prefix of the cookie a listing's position is kept in.
const PAGING_COOKIE: &str = "ferroehr-viewer-paging-";

/// The listings that keep their position in the browser's session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    /// The EHRs list.
    Ehrs,
    /// One EHR's compositions.
    Compositions,
    /// One EHR's contributions.
    Contributions,
    /// The audit log.
    Audit,
}

impl Listing {
    /// Every listing.
    pub const ALL: [Self; 4] = [
        Self::Ehrs,
        Self::Compositions,
        Self::Contributions,
        Self::Audit,
    ];

    /// The listing's key in its cookie's name and in the pager's post.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Ehrs => "ehrs",
            Self::Compositions => "compositions",
            Self::Contributions => "contributions",
            Self::Audit => "audit",
        }
    }

    /// The listing `key` names.
    #[must_use]
    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|listing| listing.key() == key)
    }
}

/// The position the request keeps for `listing`, the first page for none.
#[must_use]
pub fn current(listing: Listing) -> Paging {
    let cookies = crate::request::kept_cookies();
    let kept =
        ferrohealth_ui::theme::cookie(&cookies, &format!("{PAGING_COOKIE}{}", listing.key()))
            .and_then(|value| value.split_once('.'));
    Paging::read(kept.map(|(page, _)| page), kept.map(|(_, size)| size))
}

/// The fields a listing's pager posts beside its page and size.
#[must_use]
pub fn fields(listing: Listing) -> Vec<(String, String)> {
    vec![
        ("listing".to_owned(), listing.key().to_owned()),
        ("back".to_owned(), crate::request::path_and_query()),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
/// What a session listing's pager posts.
#[derive(Debug, serde::Deserialize)]
pub struct PagingChoice {
    /// The key of the listing paged.
    listing: String,
    /// The 1-based page asked for, read defensively.
    page: String,
    /// The page size asked for, read defensively and clamped.
    size: String,
    /// The page to return to.
    back: String,
}

#[cfg(not(target_arch = "wasm32"))]
/// Keeps the position a listing's pager posted in a cookie for the browser's
/// session and sends the browser back to the listing.
pub async fn choose_paging(Form(choice): Form<PagingChoice>) -> Response {
    let Some(listing) = Listing::parse(&choice.listing) else {
        return (
            StatusCode::BAD_REQUEST,
            "the listing is not one the viewer pages in its session",
        )
            .into_response();
    };
    let paging = Paging::read(Some(&choice.page), Some(&choice.size));
    let cookie = format!(
        "{PAGING_COOKIE}{}={}.{}; Path=/; SameSite=Lax; HttpOnly",
        listing.key(),
        paging.page,
        paging.size
    );
    let mut response = Redirect::to(crate::request::return_path(&choice.back)).into_response();
    match http::HeaderValue::from_str(&cookie) {
        Ok(value) => {
            response
                .headers_mut()
                .append(http::header::SET_COOKIE, value);
            response
        }
        Err(_invalid) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::Listing;

    #[test]
    fn every_listing_reads_back_from_its_key() {
        for listing in Listing::ALL {
            assert_eq!(Some(listing), Listing::parse(listing.key()));
        }
        assert_eq!(None, Listing::parse("templates"));
    }
}
