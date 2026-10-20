// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared read-only document viewer.
//!
//! A format selector row plus the document pane every screen shows a wire
//! document in (the composition viewer, the EHR status tab, a contribution,
//! the template detail's OPT and example tabs, a stored query's AQL).
//!
//! The pane is FerroUI's document pane (`ferrohealth_ui::widget::document`):
//! three views of one body — **Highlighted** (the default: the byte-exact
//! document through FerroUI's pure-Rust highlighter), **Raw** (the same text
//! unstyled), and **Rendered** (the template-free clinical reading from
//! [`crate::clinical`]) — plus a copy button. The selectors are FerroUI's
//! segmented control. Everything here is Rust, and both the tokenizer and the
//! renderer are pure functions of the body string, so the server pass and
//! client hydration emit identical markup.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::widget::document::document_pane_opening;
use ferrohealth_ui::widget::segmented::segmented;
use leptos::prelude::*;

use crate::clinical::{RenderedNode, RenderedRow, RenderedSection};
use crate::format::ReprFormat;
use crate::locale::{Msg, tr};

/// The scroll container of the rendered clinical view (proportional text, no
/// whitespace preservation).
const RENDERED_PANE: &str = "text-sm text-ink";

/// The four representations a document surface offers, each its key and its
/// words.
const FORMATS: &[(&str, &str)] = &[
    ("json", "JSON"),
    ("xml", "XML"),
    ("flat", "FLAT"),
    ("structured", "STRUCTURED"),
];

/// The key a format carries in [`FORMATS`].
fn format_key(format: ReprFormat) -> &'static str {
    match format {
        ReprFormat::CanonicalJson | ReprFormat::WebTemplate => "json",
        ReprFormat::CanonicalXml => "xml",
        ReprFormat::Flat => "flat",
        ReprFormat::Structured => "structured",
    }
}

/// The format a key of [`FORMATS`] names; anything else is canonical JSON.
fn format_of(key: &str) -> ReprFormat {
    match key {
        "xml" => ReprFormat::CanonicalXml,
        "flat" => ReprFormat::Flat,
        "structured" => ReprFormat::Structured,
        _ => ReprFormat::CanonicalJson,
    }
}

/// A FerroUI segmented control over `choices`, bound to `selected` through
/// each value's key: the one choice strip every selector of the viewer draws.
pub(crate) fn choice_strip<T>(
    label: String,
    choices: &[(&'static str, String)],
    selected: RwSignal<T>,
    key_of: fn(T) -> &'static str,
    value_of: fn(&str) -> T,
) -> AnyView
where
    T: Copy + Send + Sync + 'static,
{
    segmented(
        label,
        choices,
        Signal::derive(move || key_of(selected.get())),
        move |key| selected.set(value_of(key)),
    )
    .into_any()
}

/// The format-selector strip: the four representations, driving the shared
/// `selected` signal.
#[component]
pub fn FormatSelector(
    /// The currently selected format (shared with the surface's resource
    /// source so a change refetches).
    selected: RwSignal<ReprFormat>,
) -> impl IntoView {
    let formats: Vec<_> = FORMATS
        .iter()
        .map(|(key, words)| (*key, (*words).to_owned()))
        .collect();
    choice_strip(tr(Msg::Format), &formats, selected, format_key, format_of)
}

/// Which view of the document the pane is showing.
///
/// Deep-linkable: a screen that carries a `?view=` query parameter reads it
/// with [`PaneView::from_param`] in SETUP and hands the answer to
/// [`DocumentPane`] as its opening mode, which is how a composition row can
/// open straight into the rendered clinical reading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PaneView {
    /// The document text with syntax tokens (the default).
    #[default]
    Highlighted,
    /// The document text, unstyled.
    Raw,
    /// The template-free clinical rendering.
    Rendered,
}

impl PaneView {
    /// The `?view=` value that deep-links this mode.
    #[must_use]
    pub fn param(self) -> &'static str {
        match self {
            Self::Highlighted => "highlighted",
            Self::Raw => "raw",
            Self::Rendered => "rendered",
        }
    }

    /// The mode a `?view=` value names, or `None` for anything else — the
    /// parameter is user input, so an unknown value simply leaves the pane on
    /// its default view.
    #[must_use]
    pub fn from_param(value: &str) -> Option<Self> {
        [Self::Highlighted, Self::Raw, Self::Rendered]
            .into_iter()
            .find(|mode| mode.param() == value)
    }
}

/// The document pane: FerroUI's, with the view choice, the copy button and the
/// bounded body over `body`.
///
/// `body` is the raw representation text (already pretty-printed where
/// applicable — see [`pretty_body`]); the pane never rewrites it. The Rendered
/// view draws the template-free clinical reading when the body is a canonical
/// openEHR JSON document; a body with no rendered reading (XML, FLAT) offers
/// the highlighted and raw views alone.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn DocumentPane(
    /// The pane's DOM id, unique on its page.
    id: &'static str,
    /// The document text to display.
    #[prop(into)]
    body: Signal<String>,
    /// The view the pane opens in; omitted leaves it on
    /// [`PaneView::Highlighted`].
    #[prop(optional)]
    initial_view: PaneView,
) -> impl IntoView {
    let rendered = Memo::new(move |_| body.with(|text| crate::clinical::render(text.as_str())));
    let language = Signal::derive(move || body.with(|text| Language::detect(text.as_str())));
    let available = Signal::derive(move || rendered.with(Option::is_some));
    let rendered_view =
        view! { {move || rendered.with(|document| document.as_ref().map(rendered_pane))} }
            .into_any();
    document_pane_opening(
        ferrohealth_ui::widget::document::DocumentPane {
            id,
            label: tr(Msg::Document).into(),
            formats: Vec::new(),
            format: Signal::derive(|| ""),
            text: body,
            language,
            rendered: Some(rendered_view),
        },
        initial_view.param(),
        available,
        |_| {},
    )
}
/// The template-free clinical rendering of a canonical openEHR document.
fn rendered_pane(document: &RenderedSection) -> AnyView {
    let section = section_view(document.clone());
    view! { <div class=RENDERED_PANE>{section}</div> }.into_any()
}

/// One RM node as a titled section: heading, type chip, archetype chip, and its
/// children indented under a hairline rule.
///
/// `<For>` with the node's RM path as the key (stable, unique,
/// data-derived — never an index).
fn section_view(section: RenderedSection) -> AnyView {
    let RenderedSection {
        key,
        title,
        rm_type,
        archetype_node_id,
        children,
    } = section;
    let archetype = archetype_node_id.map(|id| {
        view! { <span class="font-mono text-[10px] text-ink-faint">{id}</span> }
    });
    // A childless node (an empty ITEM_TREE, a bare SECTION) renders its heading
    // alone rather than an empty indent rule.
    let body = (!children.is_empty()).then(move || {
        view! {
            <div class="mt-1 ml-1 space-y-1 border-l border-edge pl-3">
                <For
                    each=move || children.clone()
                    key=|node| node.key().to_owned()
                    children=node_view
                />
            </div>
        }
    });
    view! {
        <section class="mb-3 last:mb-0" data-doc-section=key>
            <div class="flex flex-wrap items-baseline gap-2">
                <h3 class="text-sm font-semibold text-ink">{title}</h3>
                <span class="rounded-full bg-accent-subtle px-2 py-0.5 text-[10px] font-medium tracking-wide text-accent-ink">
                    {rm_type}
                </span>
                {archetype}
            </div>
            {body}
        </section>
    }
    .into_any()
}

/// One rendered node: a nested section or a label/value row.
fn node_view(node: RenderedNode) -> AnyView {
    match node {
        RenderedNode::Section(section) => section_view(section),
        RenderedNode::Row(row) => row_view(row),
    }
}

/// One leaf as a label/value row, with the terminology code beside the value
/// where the value was coded.
fn row_view(row: RenderedRow) -> AnyView {
    let RenderedRow {
        key,
        label,
        value,
        code,
    } = row;
    let code_chip = code.map(|code| {
        view! { <span class="font-mono text-[10px] text-ink-faint">{code}</span> }
    });
    view! {
        <div
            class="grid grid-cols-1 gap-x-3 sm:grid-cols-[minmax(6rem,14rem)_1fr]"
            data-doc-row=key
        >
            <span class="text-xs text-ink-muted">{label}</span>
            <span class="flex flex-wrap items-baseline gap-2 break-words text-ink">
                <span class="font-medium">{value}</span>
                {code_chip}
            </span>
        </div>
    }
    .into_any()
}

/// The pane a detail screen shows its loaded document in, over the SAME
/// resource its facts section reads.
///
/// A `<Transition>` so a refetch keeps the current document visible, with the
/// `Result` resolved inside it. A failed or absent read renders nothing HERE,
/// because the facts section above it states that once — the screen as a whole
/// never renders an error as nothing. `body_of` picks the verbatim wire body
/// out of the loaded state; `id` is the pane's stable E2E hook.
#[must_use]
pub fn document_section<T>(
    resource: Resource<Result<Option<T>, crate::error::ViewerError>>,
    id: &'static str,
    body_of: fn(&T) -> &str,
) -> AnyView
where
    T: Clone + Send + Sync + 'static,
{
    view! {
        <Transition fallback=crate::components::data_table::table_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(Some(state)) => {
                        let pretty = pretty_body(body_of(&state), ReprFormat::CanonicalJson);
                        let doc = RwSignal::new(pretty);
                        view! {
                            <div id=id>
                                <DocumentPane id=pane_id(id) body=doc />
                            </div>
                        }
                            .into_any()
                    }
                    Ok(None) | Err(_) => ().into_any(),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The pane id of the document section `section`: FerroUI's pane takes a
/// static id, and the sections are a closed set.
fn pane_id(section: &'static str) -> &'static str {
    match section {
        "party-document" => "party-document-pane",
        "relationship-document" => "relationship-document-pane",
        _ => "document-pane",
    }
}

/// Pretty-print a JSON body for display; non-JSON (XML, FLAT with odd
/// content types) passes through unchanged.
#[must_use]
pub fn pretty_body(body: &str, format: ReprFormat) -> String {
    match format {
        ReprFormat::CanonicalXml => body.to_owned(),
        _ => serde_json::from_str::<serde_json::Value>(body)
            .and_then(|v| serde_json::to_string_pretty(&v))
            .unwrap_or_else(|_| body.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use crate::components::format_view::{FORMATS, PaneView, format_key, format_of, pretty_body};
    use crate::format::ReprFormat;

    #[test]
    fn json_pretty_prints_and_xml_passes_through() {
        assert_eq!(
            pretty_body("{\"a\":1}", ReprFormat::CanonicalJson),
            "{\n  \"a\": 1\n}"
        );
        let xml = "<x a=\"1\"/>";
        assert_eq!(pretty_body(xml, ReprFormat::CanonicalXml), xml);
        // Broken JSON falls back to the raw body rather than erroring.
        assert_eq!(pretty_body("not json", ReprFormat::Flat), "not json");
    }

    #[test]
    fn every_offered_format_round_trips_through_its_key_and_shows_its_label() {
        for (key, words) in FORMATS {
            let format = format_of(key);
            assert_eq!(format_key(format), *key);
            assert_eq!(format.label(), *words);
        }
    }

    #[test]
    fn every_mode_round_trips_through_its_deep_link_value() {
        for mode in [PaneView::Highlighted, PaneView::Raw, PaneView::Rendered] {
            assert_eq!(PaneView::from_param(mode.param()), Some(mode));
        }
        // The parameter is user input: anything else leaves the default.
        assert_eq!(PaneView::from_param("Rendered"), None);
        assert_eq!(PaneView::from_param(""), None);
        assert_eq!(PaneView::from_param("pretty"), None);
        assert_eq!(PaneView::default(), PaneView::Highlighted);
    }
}
