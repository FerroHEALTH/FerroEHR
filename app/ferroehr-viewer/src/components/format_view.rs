// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared read-only document viewer.
//!
//! A format selector row plus the document pane every screen shows a wire
//! document in (the composition viewer, the EHR status tab, a contribution,
//! the template detail's OPT and example tabs, a stored query's AQL).
//!
//! The pane is three views of one body — **Highlighted** (the default: the
//! byte-exact document with pure-Rust syntax tokens from [`crate::highlight`]),
//! **Raw** (the same text unstyled), and **Rendered** (the template-free
//! clinical reading from [`crate::clinical`], offered only when the body is a
//! canonical openEHR JSON document) — plus a copy affordance. Everything here
//! is Rust: no JS highlighter, no clipboard shim (`leptos_use::use_clipboard`
//! wraps the Clipboard API), and both the tokenizer and the renderer are pure
//! functions of the body string, so the server pass and client hydration emit
//! identical markup.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::prelude::*;
use leptos_use::{UseClipboardReturn, use_clipboard};

use crate::clinical::{RenderedNode, RenderedRow, RenderedSection};
use crate::format::ReprFormat;
use crate::highlight::{Language, Token, TokenKind};

/// The segmented-control wrapper shared by the format selector, the pane's
/// view-mode tabs and the example controls.
pub(crate) const SEGMENTED: &str = "inline-flex overflow-hidden rounded-control border border-edge-strong divide-x divide-edge-strong";

/// A selected segment.
const SEGMENT_ON: &str = "px-3 py-1.5 text-sm font-medium bg-accent text-on-accent";

/// An unselected segment.
const SEGMENT_OFF: &str = "px-3 py-1.5 text-sm font-medium text-ink-muted hover:bg-sunken";

/// One button of a segmented control: its label, whether it is the current
/// choice, and what picking it does.
///
/// The ONE segment implementation the viewer draws — the format selector, the
/// pane's view tabs and the example controls all call it, so a segmented
/// control looks and behaves the same wherever it appears. The label stays a
/// plain visible string so the E2E suite can click it by text.
pub(crate) fn segment_button(
    label: &'static str,
    active: Signal<bool>,
    on_select: impl Fn() + 'static,
) -> AnyView {
    let class = move || {
        if active.get() {
            SEGMENT_ON
        } else {
            SEGMENT_OFF
        }
    };
    view! {
        <button type="button" class=class on:click=move |_| on_select()>
            {label}
        </button>
    }
    .into_any()
}

/// The scroll container both text views share: monospaced, both-axis
/// scrollable, whitespace preserved.
const PANE: &str = "overflow-auto max-h-[70vh] whitespace-pre rounded-card border border-edge bg-sunken p-3 font-mono text-xs leading-relaxed text-ink";

/// The scroll container of the rendered clinical view (proportional text, no
/// whitespace preservation).
const RENDERED_PANE: &str =
    "overflow-auto max-h-[70vh] rounded-card border border-edge bg-sunken p-3 text-sm text-ink";

/// The format-selector strip: a segmented control, one [`segment_button`] per
/// offered format, driving the shared `selected` signal.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn FormatSelector(
    /// The formats this surface offers, in display order.
    offered: Vec<ReprFormat>,
    /// The currently selected format (shared with the surface's resource
    /// source so a change refetches).
    selected: RwSignal<ReprFormat>,
) -> impl IntoView {
    let buttons = offered
        .into_iter()
        .map(|format| {
            segment_button(
                format.label(),
                Signal::derive(move || selected.get() == format),
                move || selected.set(format),
            )
        })
        .collect::<Vec<_>>();
    view! { <div class=SEGMENTED>{buttons}</div> }
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
    /// The tab label (also the E2E click target).
    fn label(self) -> &'static str {
        match self {
            Self::Highlighted => "Highlighted",
            Self::Raw => "Raw",
            Self::Rendered => "Rendered",
        }
    }

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

/// The document pane: view tabs and a copy button above a scrollable rendering
/// of `body`.
///
/// `body` is the raw representation text (already pretty-printed where
/// applicable — see [`pretty_body`]); the pane never rewrites it. The tabs
/// appear only where they mean something: the Raw/Highlighted pair is offered
/// for a JSON or XML body (a plain-text body such as an AQL statement has
/// nothing to highlight), and Rendered only for a canonical openEHR JSON
/// document. Every one of those decisions is a pure function of the body, so
/// the server and the browser always render the same tabs.
///
/// `initial_view` is the mode the pane OPENS in — a plain value, taken once, so
/// the server pass and hydration agree; the reader switches tabs freely
/// afterwards. Callers derive it from the URL in their own setup, never from an
/// effect chasing the address bar.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn DocumentPane(
    /// The document text to display.
    #[prop(into)]
    body: Signal<String>,
    /// The view the pane opens in; omitted leaves it on
    /// [`PaneView::Highlighted`].
    #[prop(optional)]
    initial_view: PaneView,
) -> impl IntoView {
    let view_mode = RwSignal::new(initial_view);
    // Deterministic, memoized derivations of the body — never effects writing
    // signals.
    let tokens = Memo::new(move |_| body.with(|text| crate::highlight::tokenize(text.as_str())));
    let rendered = Memo::new(move |_| body.with(|text| crate::clinical::render(text.as_str())));
    let highlightable =
        Memo::new(move |_| body.with(|text| Language::detect(text.as_str()) != Language::Plain));

    let toolbar = pane_toolbar(body, view_mode, rendered, highlightable);
    let content = move || match view_mode.get() {
        PaneView::Raw => raw_pane(body),
        PaneView::Highlighted => highlighted_pane(tokens),
        // A format switch can take the rendered view away (JSON → XML) while
        // it is selected: fall back rather than render nothing.
        PaneView::Rendered => rendered.with(|document| {
            document
                .as_ref()
                .map_or_else(|| highlighted_pane(tokens), rendered_pane)
        }),
    };
    view! { <div class="flex flex-col gap-2">{toolbar} {content}</div> }
}

/// The pane toolbar: the view tabs (left) and the copy affordance with its
/// inline outcome (right).
fn pane_toolbar(
    body: Signal<String>,
    view_mode: RwSignal<PaneView>,
    rendered: Memo<Option<RenderedSection>>,
    highlightable: Memo<bool>,
) -> AnyView {
    let tabs = move || {
        let mut buttons = Vec::new();
        if highlightable.get() {
            buttons.push(view_tab(PaneView::Highlighted, view_mode));
            buttons.push(view_tab(PaneView::Raw, view_mode));
        }
        if rendered.with(Option::is_some) {
            buttons.push(view_tab(PaneView::Rendered, view_mode));
        }
        if buttons.is_empty() {
            return ().into_any();
        }
        view! { <div class=SEGMENTED>{buttons}</div> }.into_any()
    };
    view! {
        <div class="flex flex-wrap items-center justify-between gap-2">
            {tabs} {copy_button(body)}
        </div>
    }
    .into_any()
}

/// One view tab.
fn view_tab(mode: PaneView, view_mode: RwSignal<PaneView>) -> AnyView {
    segment_button(
        mode.label(),
        Signal::derive(move || view_mode.get() == mode),
        move || view_mode.set(mode),
    )
}

/// The copy affordance: writes the raw document text to the system clipboard
/// and reports the outcome inline (a read, not a CDR mutation — no toast).
///
/// The clipboard is reached through `leptos_use::use_clipboard`, which probes
/// support with the Permissions/Clipboard API on the client and is a documented
/// no-op on the server — so the server-rendered button is inert until
/// hydration, and the view structure is identical on both sides. The transient
/// "Copied" state is the primitive's own timeout; a browser that exposes no
/// clipboard (an insecure origin, an old engine) reports the failure instead of
/// silently doing nothing.
fn copy_button(body: Signal<String>) -> AnyView {
    let UseClipboardReturn {
        is_supported,
        copied,
        copy,
        ..
    } = use_clipboard();
    let unsupported = RwSignal::new(false);
    let on_click = move |_| {
        if is_supported.get_untracked() {
            unsupported.set(false);
            copy(&body.get_untracked());
        } else {
            unsupported.set(true);
        }
    };
    let status = move || {
        if copied.get() {
            "Copied"
        } else if unsupported.get() {
            "Clipboard unavailable in this browser"
        } else {
            ""
        }
    };
    view! {
        <div class="flex items-center gap-2">
            <span class="text-xs text-ink-muted" role="status" aria-live="polite">
                {status}
            </span>
            <button
                type="button"
                class="inline-flex items-center gap-1 rounded-control border border-edge-strong px-3 py-1.5 text-sm font-medium text-ink-muted hover:bg-sunken"
                title="Copy the document to the clipboard"
                on:click=on_click
            >
                <leptos_icons::Icon icon=icondata_lu::LuClipboardCopy width="14" height="14" />
                "Copy"
            </button>
        </div>
    }
    .into_any()
}

/// The document text, unstyled.
fn raw_pane(body: Signal<String>) -> AnyView {
    view! { <pre class=PANE>{move || body.get()}</pre> }.into_any()
}

/// The document text with one `<span>` per syntax token.
///
/// A plain collected `Vec` rather than `<For>`: the token stream is a derived
/// projection replaced wholesale whenever the body changes, and a token has no
/// data-derived identity to key on; an index key is forbidden, and a synthetic
/// key would be exactly that.
fn highlighted_pane(tokens: Memo<Vec<Token>>) -> AnyView {
    view! {
        <pre class=PANE>
            {move || {
                tokens
                    .with(|tokens| {
                        tokens
                            .iter()
                            .map(|token| {
                                view! {
                                    <span class=token_class(token.kind)>{token.text.clone()}</span>
                                }
                            })
                            .collect::<Vec<_>>()
                    })
            }}
        </pre>
    }
    .into_any()
}

/// The design-system class for one token class (the `syntax-*` tokens in
/// `style/tailwind.css`, defined for both themes like every other token).
fn token_class(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Plain => "",
        TokenKind::Key => "text-syntax-key",
        TokenKind::Str => "text-syntax-string",
        TokenKind::Number => "text-syntax-number",
        TokenKind::Keyword => "text-syntax-keyword",
        TokenKind::Punctuation => "text-syntax-punctuation",
        TokenKind::Tag => "text-syntax-tag",
        TokenKind::Attribute => "text-syntax-attribute",
        TokenKind::Comment => "text-syntax-comment italic",
    }
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
                                <DocumentPane body=doc />
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
    use crate::components::format_view::{PaneView, pretty_body, token_class};
    use crate::format::ReprFormat;
    use crate::highlight::TokenKind;

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
    fn every_token_class_maps_to_a_design_token() {
        // Plain text carries no class; every other kind draws from the
        // `syntax-*` token layer (style/tailwind.css), never a raw palette
        // utility.
        for kind in [
            TokenKind::Key,
            TokenKind::Str,
            TokenKind::Number,
            TokenKind::Keyword,
            TokenKind::Punctuation,
            TokenKind::Tag,
            TokenKind::Attribute,
            TokenKind::Comment,
        ] {
            let class = token_class(kind);
            assert!(
                class.starts_with("text-syntax-"),
                "{kind:?} → `{class}` is not a syntax design token"
            );
        }
        assert_eq!(token_class(TokenKind::Plain), "");
    }

    #[test]
    fn the_view_tabs_carry_their_e2e_labels() {
        assert_eq!(PaneView::Highlighted.label(), "Highlighted");
        assert_eq!(PaneView::Raw.label(), "Raw");
        assert_eq!(PaneView::Rendered.label(), "Rendered");
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
