// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shorthands the pages write their own markup with, beside FerroUI's
//! parts: markup made from escaped parts, and text escaped for it.
//!
//! A page writes its own markup only around the library's parts (a form, a
//! list, a heading's text); every component is FerroUI's
//! (`view::*`, FerroUI `docs/consoles.md`). No openEHR spec governs this:
//! our own design.

use ferrohealth_ui::html::Html;

/// The markup `markup`, made by the page from escaped parts.
#[must_use]
pub const fn h(markup: String) -> Html {
    Html::raw(markup)
}

/// `text` escaped as a text node or an attribute value.
#[must_use]
pub fn t(text: impl AsRef<str>) -> Html {
    Html::text(text)
}

/// The attribute `name="value"` with `value` escaped, or nothing for `None`.
#[must_use]
pub fn attr(name: &str, value: Option<impl AsRef<str>>) -> Html {
    value.map_or_else(Html::empty, |value| {
        h(format!(" {name}=\"{}\"", t(value)))
    })
}

/// The boolean attribute ` name` when `on`, else nothing.
#[must_use]
pub fn flag(name: &str, on: bool) -> Html {
    if on { h(format!(" {name}")) } else { Html::empty() }
}

/// The part `part` makes, or nothing for `None`.
#[must_use]
pub fn opt(part: Option<Html>) -> Html {
    part.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{attr, flag, h, opt, t};

    #[test]
    fn text_is_escaped_and_markup_is_kept() {
        assert_eq!("a &#60;b&#62;", t("a <b>").as_str());
        assert_eq!("<b>x</b>", h("<b>x</b>".to_owned()).as_str());
        assert_eq!(" title=\"&#34;x&#34;\"", attr("title", Some("\"x\"")).as_str());
        assert_eq!("", attr("title", None::<&str>).as_str());
        assert_eq!(" open", flag("open", true).as_str());
        assert_eq!("", flag("open", false).as_str());
        assert_eq!("", opt(None).as_str());
    }
}
