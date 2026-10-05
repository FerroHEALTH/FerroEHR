// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The facts-card primitive: one label/value line.
//!
//! Every detail screen states its object's facts as a two-column grid of
//! `label: value` lines carrying a stable E2E hook. The line looks the same
//! everywhere; only the hook ATTRIBUTE differs per family
//! (`data-demographic-fact`, `data-versioned-fact`), so each screen keeps a
//! one-line wrapper that fixes its own attribute and passes everything else
//! through.

use leptos::prelude::*;

/// One label/value line of a facts card.
///
/// `hook_attr` is the family's hook attribute name and `hook` the value it
/// carries — together the stable E2E handle on the line. An empty `value`
/// shows an em dash, so an absent fact reads as absent rather than as blank
/// markup.
#[must_use]
pub fn fact_row(
    label: &'static str,
    hook_attr: &'static str,
    hook: &'static str,
    value: String,
) -> AnyView {
    let shown = if value.is_empty() {
        "—".to_owned()
    } else {
        value
    };
    // The hook attribute's NAME is data, so the whole attribute set is applied
    // with the custom-attribute API: attributes serialize in the order they are
    // applied, and a view-declared one would sort AFTER them.
    let shown = view! { <span>{shown}</span> }
        .attr("class", "font-mono break-all text-ink")
        .attr(hook_attr, hook);
    view! {
        <div>
            <span class="font-medium text-ink-muted mr-1">{label}":"</span>
            {shown}
        </div>
    }
    .into_any()
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::fact_row;
    use leptos::prelude::*;

    /// Render one row's server pass, the way the axum integration does.
    fn render(label: &'static str, hook: &'static str, value: &str) -> String {
        let owner = Owner::new();
        owner.with(|| fact_row(label, "data-versioned-fact", hook, value.to_owned()).to_html())
    }

    #[test]
    fn the_hook_attribute_reaches_the_served_markup_after_the_class() {
        // The E2E journeys select on this attribute, and its NAME is a
        // parameter here — so the server pass is what proves it is emitted,
        // in the order a fact line has always carried it.
        let html = render("version", "version", "8849182c::example.org::2");
        assert!(
            html.contains(
                "<span class=\"font-mono break-all text-ink\" data-versioned-fact=\"version\">"
            ),
            "the hook attribute is missing or misplaced: {html}"
        );
        assert!(html.contains("8849182c::example.org::2"), "{html}");
    }

    #[test]
    fn an_absent_value_reads_as_an_em_dash_not_as_blank_markup() {
        let html = render("lifecycle", "lifecycle", "");
        assert!(html.contains("data-versioned-fact=\"lifecycle\""), "{html}");
        assert!(html.contains('—'), "{html}");
    }
}
