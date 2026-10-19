// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The locale the viewer renders in, and the catalog of its own words.
//!
//! The viewer's words are messages of the Fluent catalogs under the crate's
//! `locales/` directory, one directory per language
//! (`locales/en/ferroehr-viewer.ftl`), embedded at build time by
//! `fluent_templates::static_loader!`, the mechanism FerroUI catalogs its own
//! words with. English is the one catalog that ships and the one every lookup
//! falls back to; a language joins as one catalog file and no code change.
//!
//! The server chooses the locale once per request: the best language of the
//! request's `Accept-Language` the catalog holds, else the configured
//! `[ui] locale`. The document writes it as `<html lang>` (WCAG 2.2 SC 3.1.1),
//! the browser reads it back from there, and [`crate::app::App`] provides it
//! to FerroUI with `ferrohealth_ui::i18n::provide_locale`, so the viewer's
//! words and the components' words switch together. No openEHR spec governs
//! the viewer's localisation: our own design.

use ferrohealth_ui::i18n::{Locale, locale};
use fluent_templates::Loader;

fluent_templates::static_loader! {
    static CATALOG = {
        locales: "./locales",
        fallback_language: "en",
        // NOTE: Fluent's guide, "Bidirectionality": the isolation marks around a
        // placeable are for mixed-direction text; the English catalog draws none.
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

/// The viewer's catalogs, every language it ships.
#[must_use]
pub fn catalog() -> &'static dyn Loader {
    &*CATALOG
}

/// Declares the viewer's messages: each variant names one message of the
/// catalog by its id.
macro_rules! messages {
    ($($(#[$meta:meta])+ $name:ident = $id:literal,)+) => {
        /// A string the viewer draws, by its message in the catalog.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Msg {
            $($(#[$meta])+ $name,)+
        }

        impl Msg {
            /// Every message, in declaration order.
            pub const ALL: &[Self] = &[$(Self::$name,)+];

            /// The message's id in the catalog.
            #[must_use]
            pub const fn id(self) -> &'static str {
                match self {
                    $(Self::$name => $id,)+
                }
            }
        }
    };
}

messages! {
    /// The title of the page for a path the viewer does not serve.
    Title404 = "not-found-title",
    /// That page's heading.
    Heading404 = "not-found-heading",
    /// That page's sentence.
    Body404 = "not-found-body",
}

/// The viewer's message `msg` in the page's locale.
#[must_use]
pub fn tr(msg: Msg) -> String {
    tr_in(catalog(), &locale(), msg)
}

/// The message `msg` of `catalog` in `locale`.
#[must_use]
pub fn tr_in(catalog: &dyn Loader, locale: &Locale, msg: Msg) -> String {
    locale.lookup(catalog, msg.id())
}

/// The locale of the page being rendered.
///
/// On the server it is the best language of the request's `Accept-Language`
/// the catalog holds, else the configured one; in the browser, the `lang` the
/// server wrote on `<html>`.
#[must_use]
pub fn current() -> Locale {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use leptos::prelude::use_context;
        let configured = use_context::<crate::state::AppState>()
            .and_then(|state| Locale::parse(&state.config.ui.locale).ok())
            .unwrap_or_default();
        let header = use_context::<http::request::Parts>()
            .and_then(|parts| {
                parts
                    .headers
                    .get(http::header::ACCEPT_LANGUAGE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        Locale::negotiate_with(&header, catalog(), configured)
    }
    // NOTE: no openEHR spec governs this: our own design; the browser reads the
    // tag the server wrote, as the theme reads `data-theme`.
    #[cfg(target_arch = "wasm32")]
    {
        leptos::prelude::document()
            .document_element()
            .and_then(|root| root.get_attribute("lang"))
            .and_then(|tag| Locale::parse(&tag).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ferrohealth_ui::i18n::Locale;

    use super::{Msg, catalog, tr_in};

    fluent_templates::static_loader! {
        static THROWAWAY = {
            locales: "./tests/locales",
            fallback_language: "qaa",
            customise: |bundle| bundle.set_use_isolating(false),
        };
    }

    #[test]
    fn every_catalog_directory_is_a_language_holding_every_message() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        let mut found: Vec<String> = std::fs::read_dir(&root)
            .expect("the locales directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort();
        let mut shipped: Vec<String> = catalog().locales().map(ToString::to_string).collect();
        shipped.sort();
        assert_eq!(found, shipped, "a catalog directory is a language");
        assert!(
            found.contains(&String::from("en")),
            "English is the reference"
        );
        for tag in found {
            let language = tag.parse().expect("a language");
            for msg in Msg::ALL {
                let words = catalog().try_lookup(&language, msg.id());
                assert!(
                    words.is_some_and(|words| !words.trim().is_empty()),
                    "{tag} holds {}",
                    msg.id()
                );
            }
        }
    }

    #[test]
    fn a_throwaway_catalog_changes_the_words_with_no_code_change() {
        let english = Locale::english();
        assert_eq!("Not found", tr_in(catalog(), &english, Msg::Title404));
        let throwaway = Locale::parse("qaa").expect("a private-use language");
        assert_eq!("Qot qound", tr_in(&*THROWAWAY, &throwaway, Msg::Title404));
        assert_eq!(
            "Page not found",
            tr_in(catalog(), &throwaway, Msg::Heading404),
            "the shipped catalogs fall back to English"
        );
    }
}
