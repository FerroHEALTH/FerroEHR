// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Your preferences: FerroUI's three tabs, Appearance, Reading and Keyboard
//! (`view::preferences`), each preference its own cookie (family rule 9).
//!
//! The viewer's own preference, how long a revealed identifier stays shown,
//! sits inside Reading after the library's rows; the server caps it at two
//! minutes whatever the cookie says ([`crate::prefs::REVEAL_MAX`]). Every
//! preference is a plain form post to [`crate::prefs::PREFERENCE_ROUTE`],
//! which keeps the cookie and returns here saying which one it kept. No
//! openEHR spec governs the viewer: our own design.

use ferrohealth_ui::calendar::Weekday;
use ferrohealth_ui::i18n::Locale;
use ferrohealth_ui::view::preferences::{
    Post, PreferenceRow, PreferencesPage as Page, Section, SectionRoutes, SelectPreference,
    preference_row, preferences_page, select_preference,
};
use leptos::prelude::*;
use leptos_meta::Title;

use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{drawn, page_model};

/// The tab the address names: Appearance for none.
fn section() -> Section {
    crate::routes::params()
        .with(|params| params.get("section"))
        .and_then(|key| match key.as_str() {
            "reading" => Some(Section::Language),
            "keyboard" => Some(Section::Keyboard),
            _ => None,
        })
        .unwrap_or(Section::Appearance)
}

/// The route of each tab.
fn routes() -> SectionRoutes {
    SectionRoutes {
        appearance: "/preferences".to_owned(),
        language: "/preferences/reading".to_owned(),
        keyboard: "/preferences/keyboard".to_owned(),
    }
}

/// The viewer's own row of Reading: how long a revealed identifier stays
/// shown.
fn reveal_row(back: &str, saved: bool) -> AnyView {
    let current = crate::prefs::reveal_seconds();
    let mut choices = crate::prefs::REVEAL_CHOICES.to_vec();
    choices.sort_unstable();
    let choices = choices
        .into_iter()
        .map(|seconds| {
            let words = if seconds >= 60 && seconds % 60 == 0 {
                tr_with(
                    Msg::RevealMinutes,
                    &[("count", u64::from(seconds.div_euclid(60)).into())],
                )
            } else {
                tr_with(Msg::RevealSeconds, &[("count", u64::from(seconds).into())])
            };
            (seconds.to_string(), words.into())
        })
        .collect();
    preference_row(PreferenceRow {
        label: tr(Msg::RevealLabel).into(),
        description: tr(Msg::RevealDescription).into(),
        control_id: Some(crate::prefs::REVEAL.to_owned()),
        control: select_preference(SelectPreference {
            name: crate::prefs::REVEAL.to_owned(),
            choices,
            current: current.to_string(),
            post: Post {
                action: crate::prefs::PREFERENCE_ROUTE.to_owned(),
                back: back.to_owned(),
            },
            saved,
        }),
        saved,
    })
}

/// Loads the languages the browser asks for, its `Accept-Language` header.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load(
    /// The path and query of the page.
    href: String,
) -> Result<String, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_page().await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load`] reads, on the server.
#[expect(
    clippy::unused_async,
    reason = "a page model loads through a future, read once on the server"
)]
async fn read_page() -> String {
    use_context::<http::request::Parts>()
        .and_then(|parts| {
            parts
                .headers
                .get(http::header::ACCEPT_LANGUAGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// Your preferences.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn PreferencesPage() -> impl IntoView {
    let model = page_model!(load());
    let page = drawn(model, |accepted: String| {
        let section = section();
        let routes = routes();
        let saved = crate::request::query("saved");
        let own = if section == Section::Language {
            let saved_reveal = saved.as_deref() == Some(crate::prefs::REVEAL);
            vec![reveal_row(routes.of(section), saved_reveal)]
        } else {
            Vec::new()
        };
        preferences_page(Page {
            section,
            routes,
            reading: true,
            preferences: crate::prefs::current(),
            action: crate::prefs::PREFERENCE_ROUTE.to_owned(),
            saved,
            accent: tr(Msg::BrandAccent),
            languages: crate::locale::shipped(),
            browser: Locale::negotiate_with(&accepted, crate::locale::catalog(), Locale::english()),
            week_from_language: Weekday::Monday,
            shortcuts: None,
            reset: crate::prefs::RESET_ROUTE.to_owned(),
            own,
        })
    });
    view! {
        <Title text=tr(Msg::YourPreferences) />
        {page}
    }
}
