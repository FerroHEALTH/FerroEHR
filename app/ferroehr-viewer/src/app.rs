// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HTML document every page is drawn in.
//!
//! Every page is one document the server draws whole (FerroUI
//! `docs/consoles.md`, "How a console builds"): FerroUI's
//! `view::shell::document` writes `<html lang>` with the reader's theme,
//! density, motion and hint preferences as `data-theme`, `data-density`,
//! `data-motion` and `data-hints`, the stylesheet, the FerroEHR theme and
//! FerroUI's one loader script, so the first paint is right with no script
//! and every behaviour is a module over what the server drew. Each page
//! names a bare title, and [`viewer_title`] adds the product name once. No
//! openEHR spec governs the viewer: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::serve::Assets;
use ferrohealth_ui::style::Brand;
use ferrohealth_ui::view::shell::{Document, document};

use crate::locale::{Msg, tr};
use crate::markup::{h, t};

/// The path the FerroEHR mark is served at as the tab icon.
pub const ICON_PATH: &str = "/favicon.svg";

/// The document title of a page whose bare title is `section`: the section,
/// then the product name, once.
#[must_use]
pub fn viewer_title(mut section: String) -> String {
    let product = tr(Msg::ProductName);
    if section.is_empty() || section == product {
        return product;
    }
    section.push_str(" · ");
    section.push_str(&product);
    section
}

/// The whole document of a page titled `title` whose body is `body`, in the
/// reader's preferences and the page's language.
#[must_use]
pub fn page_document(title: String, body: Html) -> Html {
    document(Document {
        locale: crate::locale::current(),
        preferences: crate::prefs::current(),
        title: viewer_title(title),
        assets: Assets::of(Brand::Ehr),
        nonce: None,
        head: h(format!(
            r#"<link rel="icon" type="image/svg+xml" href="{}">"#,
            t(ICON_PATH)
        )),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::viewer_title;

    #[test]
    fn a_section_name_gains_the_product_suffix_once() {
        let titled = viewer_title("Templates".to_owned());
        assert!(titled.starts_with("Templates · "), "{titled}");
        assert_eq!(1, titled.matches(" · ").count(), "{titled}");
        assert_eq!(
            viewer_title(String::new()),
            viewer_title(viewer_title(String::new()))
        );
    }
}
