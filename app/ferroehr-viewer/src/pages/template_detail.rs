// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/templates/{template_id}` screen — template detail: OPT/WT/example tabs +
//! path catalog.
//!
//! Three tabs over one operational template: **WT** (the Web Template path
//! catalog as an expandable tree + a node inspector), **OPT** (the raw
//! canonical-XML operational template), and **Example** (the CDR-generated
//! example composition, switchable by representation, detail level and example
//! form). No openEHR spec governs the viewer — our own design / product
//! extension; the `WebTemplate` shape it renders
//! is `openehr_sdt::flat`'s (built from the CDR's OPT), per the ITS-REST
//! Simplified Formats spec (`master04`).
//!
//! The WT catalog, the OPT source pane and the identity card are three views
//! of ONE document, so the screen reads the operational template exactly once
//! per render — a single page-level [`Resource`] over
//! [`fetch_template_detail`], shared by every pane. The Example tab keeps its
//! own tab-gated resource: it is a different CDR resource whose fetch runs the
//! CDR's example generator.

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;
use leptos_router::hooks::{use_params_map, use_query_map};

use crate::builder::catalog::CatalogNode;
use crate::components::field::BTN_DANGER;
use crate::components::page_header::{Crumb, PageHeader};
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::toast::{toast_error, toast_success};
use crate::error::ViewerError;
use crate::example_options::{ExampleDetail, ExampleType};
use crate::format::ReprFormat;

/// Template identity + language metadata for the detail header card,
/// combined from the typed OPT (uid, concept, original language) and its
/// Web Template (version, default + available languages).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TemplateMeta {
    /// The operational-template id.
    pub template_id: String,
    /// The template concept / display name.
    pub concept: String,
    /// The OPT `uid` (empty when the template carries none).
    pub uid: String,
    /// The OPT's original language code.
    pub language: String,
    /// The Web Template `version`.
    pub version: String,
    /// Every language the template carries terms for.
    pub languages: Vec<String>,
}

/// Everything the template-detail screen shows about one operational template,
/// distilled from a SINGLE fetch of its OPT source.
///
/// The three panes of the screen are three views of the same document — the raw
/// source, its identity metadata, and its Web Template path catalog — so they
/// travel together and the screen reads the CDR once.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TemplateDetail {
    /// The raw OPT 1.4 canonical XML the CDR served, verbatim.
    pub source: String,
    /// The identity/metadata card fields.
    pub meta: TemplateMeta,
    /// The Web Template path catalog (the same [`CatalogNode`] tree the Query
    /// Builder navigates).
    pub catalog: CatalogNode,
}

/// Distil the fetched OPT source into the screen's three panes: one parse, one
/// Web Template build, both views derived from them.
///
/// The XML is parsed with [`openehr_its::opt14::from_xml`] — the OPT 1.4
/// canonical-XML parse entry (root `<template>` = `OPERATIONAL_TEMPLATE`) —
/// then [`openehr_sdt::flat::webtemplate::builder::build_web_template`]
/// produces the Web Template, and
/// [`crate::builder::catalog::from_web_template`] the slim serializable tree.
///
/// # Errors
/// [`ViewerError::Internal`] when the OPT fails to parse or the Web Template
/// fails to build (the diagnostic named, never a panic).
#[cfg(feature = "ssr")]
pub fn template_detail_from_opt(source: String) -> Result<TemplateDetail, ViewerError> {
    let opt = openehr_its::opt14::from_xml(&source)
        .map_err(|e| ViewerError::Internal(format!("OPT 1.4 parse: {e}")))?;
    let web_template = openehr_sdt::flat::webtemplate::builder::build_web_template(&opt)
        .map_err(|e| ViewerError::Internal(format!("WebTemplate build: {e}")))?;
    Ok(TemplateDetail {
        meta: TemplateMeta {
            template_id: opt.template_id.value.clone(),
            concept: opt.concept.clone(),
            uid: opt
                .uid
                .as_ref()
                .map(|u| u.value().to_owned())
                .unwrap_or_default(),
            language: opt.language.code_string.clone(),
            version: web_template.version.clone(),
            languages: web_template.languages.clone(),
        },
        catalog: crate::builder::catalog::from_web_template(&web_template),
        source,
    })
}

/// Fetch the operational template ONCE and distil everything the detail screen
/// shows from that one document.
///
/// GET `definition/template/adl1.4/{template_id}` with
/// `Accept: application/xml`; the `template_id` path segment is percent-encoded
/// server-side. The OPT is parsed exactly once, by
/// [`template_detail_from_opt`].
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Cdr`] (e.g. `404` for an unknown template) /
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR;
/// [`ViewerError::Internal`] when the OPT fails to parse or the Web Template
/// fails to build.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_template_detail(
    /// The template whose OPT to read.
    template_id: String,
) -> Result<TemplateDetail, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "definition/template/adl1.4/{}",
        urlencoding::encode(&template_id)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/xml")
        .await?;
    template_detail_from_opt(crate::cdr::CdrClient::expect_success(response)?.body)
}

/// Fetch a template's Web Template path catalog alone — the Query Builder's
/// reader, which navigates paths and has no use for the OPT source or the
/// identity card.
///
/// It rides [`fetch_template_detail`]'s single fetch + parse pipeline and
/// returns only the catalog, so the builder's wire payload stays the tree
/// rather than the whole operational template.
///
/// # Errors
/// As [`fetch_template_detail`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_template_catalog(
    /// The template to build the path catalog from.
    template_id: String,
) -> Result<CatalogNode, ViewerError> {
    Ok(fetch_template_detail(template_id).await?.catalog)
}

/// The ITS-REST path of the ADL 1.4 example resource for `template_id`, at
/// `detail` and `kind`.
///
/// The two example options ride the query string
/// ([`crate::example_options::example_query`]); the id is percent-encoded with
/// the `urlencoding` crate, because an operational-template id is CDR-supplied
/// text.
#[must_use]
pub fn example_path(template_id: &str, detail: ExampleDetail, kind: ExampleType) -> String {
    format!(
        "definition/template/adl1.4/{}/example{}",
        urlencoding::encode(template_id),
        crate::example_options::example_query(detail, kind)
    )
}

/// Fetch the CDR-generated example composition for the template, in `format`.
///
/// GET `definition/template/adl1.4/{template_id}/example` with `Accept` set to
/// the selected representation's media type, and the operator's `detail_level`
/// and `type` on the query string
/// (`docs/specs/openehr/ITS-REST/specifications/operations/definition_template_adl1.4_example_get.yaml`).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_example(
    /// The template to generate an example composition for.
    template_id: String,
    /// Which representation to negotiate for the example.
    format: ReprFormat,
    /// How much of the template the example fills in (`detail_level`).
    detail: ExampleDetail,
    /// Which form the example is shaped for (`type`).
    kind: ExampleType,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&example_path(&template_id, detail, kind));
    let response = state
        .cdr
        .get(&session.credential, &url, format.media_type())
        .await?;
    Ok(crate::cdr::CdrClient::expect_success(response)?.body)
}

/// The self-link back to this screen with a different `?tab=` selected.
///
/// `template_id` arrives percent-DEcoded from the route param, so the path
/// segment is re-encoded with the `urlencoding` crate; without it a tab click
/// on a template id containing `/`, `#`, `?` or `%` would navigate off-route.
/// The `tab` value is one of the three fixed literals below and is encoded for
/// the same reason the segment is — no call site is trusted to be URL-safe.
/// NOTE: no openEHR spec governs the viewer's internal links — our own
/// design/extension.
fn tab_href(template_id: &str, tab: &str) -> String {
    format!(
        "/templates/{}?tab={}",
        urlencoding::encode(template_id),
        urlencoding::encode(tab)
    )
}

/// The template detail screen: a header with a back link + tab bar, then the
/// WT / OPT / Example panes (all mounted, toggled by visibility so switching a
/// tab preserves each pane's loaded state).
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn TemplateDetailPage() -> impl IntoView {
    // NOTE: the route param arrives ALREADY percent-decoded on both targets
    // (`leptos_router`'s `ParamsMap::insert` runs every value through
    // `Url::unescape`), so decoding here would be a second, corrupting pass.
    let params = use_params_map();
    let template_id =
        Signal::derive(move || params.with(|map| map.get("template_id").unwrap_or_default()));

    // Tab state lives in the URL (`?tab=`), so it is shareable and
    // refresh-safe; it defaults to the WT catalog when absent.
    let query = use_query_map();
    let selected_tab = Memo::new(move |_| {
        query
            .with(|q| q.get("tab"))
            .unwrap_or_else(|| "wt".to_owned())
    });
    let selected_node = RwSignal::new(None::<CatalogNode>);
    let example_format = RwSignal::new(ReprFormat::CanonicalJson);
    let example_detail = RwSignal::new(ExampleDetail::default());
    let example_kind = RwSignal::new(ExampleType::default());

    // ONE page-level read of the operational template, shared by every pane
    // that describes it — the metadata card (tab-independent by owner
    // directive 2026-07-18), the WT path catalog, and the OPT source view are
    // three views of one document, so the screen fetches and parses it once.
    let detail = Resource::new(
        move || template_id.get(),
        |id| async move { fetch_template_detail(id).await },
    );
    // The example stays its own tab-gated resource: it is a DIFFERENT CDR
    // resource (`…/example`) whose fetch triggers the CDR's example generator,
    // and its format is a live selection — never run it for an unopened tab.
    let example: Resource<Result<Option<String>, ViewerError>> = Resource::new(
        move || {
            (selected_tab.get() == "example").then(|| {
                (
                    template_id.get(),
                    example_format.get(),
                    example_detail.get(),
                    example_kind.get(),
                )
            })
        },
        |active| async move {
            match active {
                Some((id, format, detail, kind)) => {
                    fetch_example(id, format, detail, kind).await.map(Some)
                }
                None => Ok(None),
            }
        },
    );

    let wt_pane = wt_tab(detail, selected_node);
    let opt_pane = opt_tab(detail);
    let example_pane = example_tab(example, example_format, example_detail, example_kind);
    let meta_card = meta_section(detail);
    let delete_action = delete_section(template_id);

    // The tabs are URL-driven pill links: a static-Tailwind anchor per view,
    // the active one styled from the `selected_tab` Memo. No thaw TabList —
    // the selected view is a shareable query param, not private widget state.
    // All three bodies stay mounted (toggled by `class:hidden`) so each pane
    // keeps its loaded state across tab switches.
    let tab_link = move |value: &'static str, label: &'static str| {
        let class = move || {
            let base = "rounded-control px-3 py-1.5 text-sm font-medium transition-colors";
            if selected_tab.get() == value {
                format!("{base} bg-accent-subtle text-accent-ink")
            } else {
                format!("{base} text-ink-muted hover:bg-sunken")
            }
        };
        let href = move || tab_href(&template_id.get(), value);
        view! {
            <leptos_router::components::A href=href attr:class=class>
                {label}
            </leptos_router::components::A>
        }
        .into_any()
    };

    view! {
        <Title text=move || format!("Template · {}", template_id.get()) />
        <div class="p-6">
            <PageHeader
                title=template_id
                crumbs=vec![Crumb::new("Templates", "/templates")]
                mono=true
            >
                {delete_action}
            </PageHeader>
            {meta_card}
            <nav aria-label="Template views" class="flex gap-1 mb-4">
                {tab_link("wt", "WT")}
                {tab_link("opt", "OPT")}
                {tab_link("example", "Example")}
            </nav>
            <div>
                <div class:hidden=move || selected_tab.get() != "wt">{wt_pane}</div>
                <div class:hidden=move || selected_tab.get() != "opt">{opt_pane}</div>
                <div class:hidden=move || selected_tab.get() != "example">{example_pane}</div>
            </div>
        </div>
    }
}

/// The admin **Delete template** affordance for the page-header action slot.
///
/// Probe-gated: it renders only when the CDR advertises its Admin API as
/// mounted (`crate::admin::when_admin_usable` — no admin group, no button).
/// The click opens the shared confirmation modal
/// ([`ConfirmDialog`](crate::components::confirm_dialog::ConfirmDialog)); only
/// the dialog's confirm dispatches. On success it toasts and returns to the
/// list, because the screen it was invoked from now describes a template that
/// no longer exists; on failure the actionable copy names this template and the
/// next action (the CDR refuses a template still referenced by a committed
/// version with `409`).
fn delete_section(template_id: Signal<String>) -> AnyView {
    let toaster = thaw::ToasterInjection::expect_context();
    let gate = crate::admin::admin_gate();
    let delete: Action<String, (String, Result<(), ViewerError>)> = Action::new(|id: &String| {
        let id = id.clone();
        async move {
            let outcome = crate::admin::admin_delete_template(id.clone()).await;
            (id, outcome)
        }
    });
    // Whether the confirmation modal is open (this screen has exactly one
    // deletable object, so a bool IS the "which object" state).
    let confirming = RwSignal::new(false);

    // Toast + navigation are side-effects on the outside world (the thaw
    // toaster, the router), so an Effect is their correct home; it never runs
    // on the server pass.
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| match delete.value().get() {
        Some((id, Ok(()))) => {
            toast_success(
                toaster,
                "Template deleted",
                &format!("{id} was removed from the CDR."),
            );
            navigate("/templates", leptos_router::NavigateOptions::default());
        }
        Some((id, Err(error))) => toast_error(
            toaster,
            "Delete failed",
            &crate::admin::delete_failure_copy(&format!("Template {id}"), &error),
        ),
        None => {}
    });

    let message = Signal::derive(move || {
        format!(
            "Permanently delete the operational template “{}” from the CDR? This cannot be \
             undone. The CDR refuses the delete while a committed version still references the \
             template.",
            template_id.get()
        )
    });

    crate::admin::when_admin_usable(gate, move || {
        view! {
            <button
                id="template-delete"
                type="button"
                class=BTN_DANGER
                disabled=Signal::derive(move || delete.pending().get())
                on:click=move |_| confirming.set(true)
            >
                <leptos_icons::Icon icon=icondata_lu::LuTrash width="14" height="14" />
                "Delete template"
            </button>
            <crate::components::confirm_dialog::ConfirmDialog
                open=confirming
                title="Delete template"
                message=message
                confirm_label="Delete template"
                confirm_id="template-delete-confirm"
                on_cancel=Callback::new(move |()| confirming.set(false))
                on_confirm=Callback::new(move |()| {
                    delete.dispatch(template_id.get_untracked());
                    confirming.set(false);
                })
            />
        }
        .into_any()
    })
}

/// The identity/metadata card: template id, concept, version, UID, and
/// languages — always visible above the tabs. Resolved inside `Suspense`
/// per the house error pattern, from the shared detail handle.
fn meta_section(detail: Resource<Result<TemplateDetail, ViewerError>>) -> AnyView {
    let entry = |label: &'static str, value: String, mono: bool| {
        let value_class = if mono {
            "font-mono text-xs text-ink break-all"
        } else {
            "text-sm text-ink"
        };
        let shown = if value.is_empty() {
            "—".to_owned()
        } else {
            value
        };
        view! {
            <div>
                <dt class="text-xs font-semibold uppercase tracking-wide text-ink-muted">
                    {label}
                </dt>
                <dd class=value_class>{shown}</dd>
            </div>
        }
        .into_any()
    };
    view! {
        <section class=format!("{CARD_PAD} mb-4")>
            <Suspense fallback=|| {
                view! {
                    <thaw::Skeleton class="h-10">
                        <thaw::SkeletonItem />
                    </thaw::Skeleton>
                }
            }>
                {move || {
                    Suspend::new(async move {
                        match detail.await {
                            Ok(loaded) => {
                                let m = loaded.meta;
                                let language_list = if m.languages.is_empty() {
                                    m.language.clone()
                                } else {
                                    m.languages.join(", ")
                                };
                                view! {
                                    <dl class="grid grid-cols-2 gap-x-6 gap-y-3 sm:grid-cols-3 lg:grid-cols-5">
                                        {entry("Concept", m.concept, false)}
                                        {entry("Version", m.version, false)}
                                        {entry("Default language", m.language, false)}
                                        {entry("Languages", language_list, false)}
                                        {entry("UID", m.uid, true)}
                                    </dl>
                                }
                                    .into_any()
                            }
                            Err(e) => crate::components::notice::inline_error(&e),
                        }
                    })
                }}
            </Suspense>
        </section>
    }
    .into_any()
}

/// The WT tab: a two-pane layout — the recursive path-catalog tree (left) and
/// the node inspector (right), the latter driven by the shared selection
/// signal. The tree is the shared detail handle's catalog.
fn wt_tab(
    detail: Resource<Result<TemplateDetail, ViewerError>>,
    selected: RwSignal<Option<CatalogNode>>,
) -> AnyView {
    view! {
        <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 items-start">
            <section class=CARD_PAD>
                <h2 class=CARD_TITLE>"Path catalog (WT tree)"</h2>
                <div class="overflow-auto max-h-[70vh]">
                    <Transition fallback=tree_skeleton>
                        {move || Suspend::new(async move {
                            match detail.await {
                                Ok(loaded) => {
                                    // Resolve inside the Transition: an SSR'd ErrorBoundary
                                    // fallback mismatches at hydration in leptos 0.8.
                                    view! {
                                        <ul class="text-sm">
                                            <CatalogTreeNode
                                                node=loaded.catalog
                                                selected=selected
                                                depth=0
                                            />
                                        </ul>
                                    }
                                        .into_any()
                                }
                                Err(e) => catalog_error_view(&e, "/templates"),
                            }
                        })}
                    </Transition>
                </div>
            </section>
            <section class=CARD_PAD>
                <h2 class=CARD_TITLE>"Node inspector"</h2>
                <div>{node_inspector(selected)}</div>
            </section>
        </div>
    }
    .into_any()
}

/// The `<Transition>` fallback the template detail panes share (both ADL
/// families — the ADL2 screen imports it rather than re-declaring a copy).
pub(crate) fn tree_skeleton() -> impl IntoView {
    view! {
        <thaw::Skeleton>
            <thaw::SkeletonItem class="h-4 mb-2" />
            <thaw::SkeletonItem class="h-4 mb-2 ml-4" />
            <thaw::SkeletonItem class="h-4 ml-4" />
        </thaw::Skeleton>
    }
}

/// The catalog error state (e.g. a `404` unknown template, or a `WebTemplate`
/// build failure naming the offending node) with a back link to `back_href` —
/// the caller's own listing, so the ADL2 screens point at their family. Used
/// by the tabs that resolve their `Result` inside the `<Transition>` — an SSR'd
/// `ErrorBoundary` fallback mismatches at hydration in leptos 0.8 — so the error
/// (with its back link) renders directly from the resolved `Err` branch.
pub(crate) fn catalog_error_view(error: &ViewerError, back_href: &'static str) -> AnyView {
    let message = error.to_string();
    view! {
        <thaw::MessageBar intent=thaw::MessageBarIntent::Error>
            <thaw::MessageBarBody>
                {message} " — "
                <leptos_router::components::A
                    href=back_href
                    attr:class="text-accent hover:underline"
                >
                    "back to templates"
                </leptos_router::components::A>
            </thaw::MessageBarBody>
        </thaw::MessageBar>
    }
    .into_any()
}

/// One node of the path catalog: a disclosure toggle (for branches), a
/// selectable label + RM-type tag, and — for a branch — its children.
///
/// Returns [`AnyView`] at every level: the type erasure is what lets the
/// component recurse (an un-erased recursive view would be an infinite type)
/// and keeps rustc's layout-recursion depth bounded on plain `cargo` builds.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "a component prop is owned data; the node is cloned into the row's selection and its children"
)]
#[component]
pub(crate) fn CatalogTreeNode(
    /// The catalog node to render (static data; the component runs once).
    node: CatalogNode,
    /// The shared "inspected node" selection, set when a label is clicked.
    selected: RwSignal<Option<CatalogNode>>,
    /// Depth from the root, used to auto-expand the top two levels.
    depth: i32,
) -> AnyView {
    let has_children = !node.children.is_empty();
    let expanded = RwSignal::new(depth < 2);
    let label = node.label.clone();
    let rm_type = node.rm_type.clone();
    let this_path = node.aql_path.clone();
    let select_node = node.clone();

    // One reactive class closure (a `class:` binding cannot reuse a non-`Copy`
    // closure twice), highlighting the row when it is the inspected node.
    let label_class = move || {
        let selected_here =
            selected.with(|current| current.as_ref().is_some_and(|n| n.aql_path == this_path));
        let mut class =
            String::from("flex items-center gap-2 rounded-control px-1 text-left hover:bg-sunken");
        if selected_here {
            class.push_str(" bg-accent-subtle text-accent-ink");
        }
        class
    };

    let child_views = node
        .children
        .clone()
        .into_iter()
        .map(|child| {
            view! { <CatalogTreeNode node=child selected=selected depth=depth + 1 /> }
        })
        .collect::<Vec<_>>();

    let disclosure = if has_children {
        view! {
            <button
                type="button"
                class="w-4 shrink-0 text-ink-muted"
                on:click=move |_| expanded.update(|open| *open = !*open)
            >
                {move || {
                    if expanded.get() {
                        view! {
                            <leptos_icons::Icon
                                icon=icondata_lu::LuChevronDown
                                width="12"
                                height="12"
                            />
                        }
                            .into_any()
                    } else {
                        view! {
                            <leptos_icons::Icon
                                icon=icondata_lu::LuChevronRight
                                width="12"
                                height="12"
                            />
                        }
                            .into_any()
                    }
                }}
            </button>
        }
        .into_any()
    } else {
        view! { <span class="inline-block w-4 shrink-0"></span> }.into_any()
    };

    let children_list = if has_children {
        view! {
            <ul class="ml-4" class:hidden=move || !expanded.get()>
                {child_views}
            </ul>
        }
        .into_any()
    } else {
        ().into_any()
    };

    view! {
        <li class="py-0.5">
            <div class="flex items-center gap-1">
                {disclosure}
                <button
                    type="button"
                    class=label_class
                    on:click=move |_| selected.set(Some(select_node.clone()))
                >
                    <span>{label}</span>
                    <span class="font-mono text-xs text-ink-muted">{rm_type}</span>
                </button>
            </div>
            {children_list}
        </li>
    }
    .into_any()
}

/// The node inspector: nothing until a node is picked, then its aqlPath,
/// rmType, node id, selectability, and any unit / code options as chips.
pub(crate) fn node_inspector(selected: RwSignal<Option<CatalogNode>>) -> AnyView {
    view! {
        {move || match selected.get() {
            None => {
                view! {
                    <p class="text-sm text-ink-muted">
                        "Select a node to inspect its path and metadata."
                    </p>
                }
                    .into_any()
            }
            Some(node) => inspector_body(&node),
        }}
    }
    .into_any()
}

/// Render the inspected node's metadata. `inspector_body` runs once per
/// selection (the caller re-invokes it when the selection changes), so the
/// chip sections are built conditionally here rather than via `<Show>` — a
/// `Vec<AnyView>` is not `Clone`, and `<Show>` would re-call its body.
fn inspector_body(node: &CatalogNode) -> AnyView {
    let node_id = if node.node_id.is_empty() {
        "—".to_owned()
    } else {
        node.node_id.clone()
    };
    let selectable = if node.selectable { "yes" } else { "no" };
    let units_section = chip_section("units", &node.unit_options);
    let codes_section = code_chip_section(node);

    view! {
        <dl class="grid grid-cols-[6rem_1fr] gap-x-3 gap-y-1 text-sm text-ink">
            <dt class="font-medium text-ink-muted">"label"</dt>
            <dd>{node.label.clone()}</dd>
            <dt class="font-medium text-ink-muted">"rmType"</dt>
            <dd class="font-mono">{node.rm_type.clone()}</dd>
            <dt class="font-medium text-ink-muted">"node id"</dt>
            <dd class="font-mono">{node_id}</dd>
            <dt class="font-medium text-ink-muted">"selectable"</dt>
            <dd>{selectable}</dd>
            <dt class="font-medium text-ink-muted">"aqlPath"</dt>
            <dd class="font-mono break-all">{node.aql_path.clone()}</dd>
        </dl>
        {units_section}
        {codes_section}
    }
    .into_any()
}

/// A titled chip row for a list of plain string options; empty → nothing.
fn chip_section(title: &'static str, values: &[String]) -> AnyView {
    if values.is_empty() {
        return ().into_any();
    }
    let chips = values
        .iter()
        .map(|value| {
            let value = value.clone();
            view! {
                <span class="rounded-full bg-accent-subtle px-2 py-0.5 text-xs text-accent-ink">
                    {value}
                </span>
            }
        })
        .collect::<Vec<_>>();
    view! {
        <div class="mt-3">
            <div class="text-xs font-medium text-ink-muted mb-1">{title}</div>
            <div class="flex flex-wrap gap-1">{chips}</div>
        </div>
    }
    .into_any()
}

/// The coded/ordinal options as chips (`label (code)` when they differ);
/// empty → nothing.
fn code_chip_section(node: &CatalogNode) -> AnyView {
    if node.code_options.is_empty() {
        return ().into_any();
    }
    let chips = node
        .code_options
        .iter()
        .map(|option| {
            let text = if option.label == option.code {
                option.code.clone()
            } else {
                format!("{} ({})", option.label, option.code)
            };
            view! {
                <span class="rounded-full bg-accent-subtle px-2 py-0.5 text-xs text-accent-ink">
                    {text}
                </span>
            }
        })
        .collect::<Vec<_>>();
    view! {
        <div class="mt-3">
            <div class="text-xs font-medium text-ink-muted mb-1">"codes"</div>
            <div class="flex flex-wrap gap-1">{chips}</div>
        </div>
    }
    .into_any()
}

/// The OPT tab: the raw canonical-XML operational template — the shared detail
/// handle's own source, verbatim — in the shared document pane.
fn opt_tab(detail: Resource<Result<TemplateDetail, ViewerError>>) -> AnyView {
    view! {
        <Transition fallback=tree_skeleton>
            {move || Suspend::new(async move {
                match detail.await {
                    Ok(loaded) => {
                        // Resolve inside the Transition: an SSR'd ErrorBoundary fallback
                        // mismatches at hydration in leptos 0.8.
                        view! {
                            <crate::components::format_view::DocumentPane body=loaded.source />
                        }
                            .into_any()
                    }
                    Err(e) => catalog_error_view(&e, "/templates"),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The Example tab: the shared example controls over the selection signals —
/// a change to any of them refetches the resource — plus the pretty-printed
/// example composition in the document pane.
fn example_tab(
    example: Resource<Result<Option<String>, ViewerError>>,
    format: RwSignal<ReprFormat>,
    detail: RwSignal<ExampleDetail>,
    kind: RwSignal<ExampleType>,
) -> AnyView {
    view! {
        <div class="space-y-3">
            <crate::components::example_controls::ExampleControls
                format=format
                detail=detail
                kind=kind
            />
            <Transition fallback=tree_skeleton>
                {move || Suspend::new(async move {
                    match example.await {
                        Ok(None) => ().into_any(),
                        Ok(Some(raw)) => {
                            let pretty = crate::components::format_view::pretty_body(
                                &raw,
                                format.get_untracked(),
                            );
                            // Resolve inside the Transition: an SSR'd ErrorBoundary fallback
                            // mismatches at hydration in leptos 0.8.
                            view! { <crate::components::format_view::DocumentPane body=pretty /> }
                                .into_any()
                        }
                        Err(e) => crate::components::notice::inline_error(&e),
                    }
                })}
            </Transition>
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use crate::example_options::{ExampleDetail, ExampleType};
    use crate::pages::template_detail::{example_path, tab_href};

    /// The operational template the viewer's own e2e stack is seeded with
    /// (`scripts/ui-e2e.sh`), so the derivation is pinned against the same
    /// document the browser journeys inspect.
    #[cfg(feature = "ssr")]
    const OPT_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/minimal_evaluation.opt"
    ));

    #[cfg(feature = "ssr")]
    #[test]
    fn one_parse_yields_the_source_the_meta_and_the_catalog() {
        // The screen's three panes come out of ONE parse: the source travels
        // verbatim, the identity card reads the typed OPT, and the path
        // catalog the Web Template built from it.
        let detail =
            crate::pages::template_detail::template_detail_from_opt(OPT_FIXTURE.to_owned())
                .expect("the seeded OPT parses");
        assert_eq!(detail.source, OPT_FIXTURE, "the source pane is verbatim");
        assert_eq!(detail.meta.template_id, "minimal_evaluation.en.v1");
        assert_eq!(detail.meta.concept, "Minimal evaluation");
        assert_eq!(detail.meta.uid, "711d7d49-b3c6-4a6a-a6b4-a4bd02fc353d");
        assert_eq!(detail.meta.language, "en");
        assert!(
            detail.meta.languages.contains(&"en".to_owned()),
            "the Web Template's language list carries the original language, got {:?}",
            detail.meta.languages
        );
        assert!(
            !detail.catalog.children.is_empty(),
            "the path catalog carries the template's own nodes"
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn a_defective_opt_names_the_failing_stage() {
        // A refusal is a named diagnostic, never a panic — the screen renders
        // it inline where the panes would be.
        let error = crate::pages::template_detail::template_detail_from_opt(
            "<template>not an OPT</template>".to_owned(),
        )
        .expect_err("a defective OPT is refused");
        assert!(
            error.to_string().contains("OPT 1.4 parse"),
            "the diagnostic names the stage that failed, got {error}"
        );
    }

    #[test]
    fn tab_href_leaves_a_url_safe_template_id_alone() {
        assert_eq!(
            tab_href("Vital signs-v2.0_TEST~x", "wt"),
            "/templates/Vital%20signs-v2.0_TEST~x?tab=wt"
        );
    }

    #[test]
    fn tab_href_re_encodes_the_decoded_route_param() {
        // The param arrived decoded, so the reserved characters are literal
        // again and must be re-escaped to land back on the same route.
        assert_eq!(tab_href("a/b", "opt"), "/templates/a%2Fb?tab=opt");
        assert_eq!(tab_href("a#b", "opt"), "/templates/a%23b?tab=opt");
        assert_eq!(tab_href("a%2Fb", "opt"), "/templates/a%252Fb?tab=opt");
        assert_eq!(
            tab_href("temperatur-°C", "example"),
            "/templates/temperatur-%C2%B0C?tab=example"
        );
    }

    #[test]
    fn tab_href_round_trips_through_the_router_unescape() {
        // `leptos_router::location::Url::unescape` under `ssr` is
        // `percent_encoding::percent_decode_str(..).decode_utf8_lossy()`;
        // `ParamsMap::insert` applies it to every param, so decoding the
        // segment this builder emits must return the original id.
        for id in [
            "a/b",
            "a#b",
            "a%2Fb",
            "temperatur-°C",
            "a b/c?d#e%f&g=h+i",
            "openEHR-EHR-COMPOSITION.encounter.v1",
        ] {
            let href = tab_href(id, "wt");
            let segment = href
                .strip_prefix("/templates/")
                .and_then(|rest| rest.split('?').next())
                .expect("the builder always emits /templates/<segment>?tab=…");
            assert_eq!(
                urlencoding::decode(segment).expect("valid UTF-8 percent-encoding"),
                id
            );
        }
    }

    #[test]
    fn the_example_path_carries_the_selected_options() {
        // Both parameters ride every request, defaults included, so the CDR
        // generates exactly what the pane's controls show.
        assert_eq!(
            example_path(
                "minimal_evaluation.en.v1",
                ExampleDetail::default(),
                ExampleType::default()
            ),
            "definition/template/adl1.4/minimal_evaluation.en.v1/example?detail_level=required&type=input"
        );
        assert_eq!(
            example_path(
                "minimal_evaluation.en.v1",
                ExampleDetail::Complete,
                ExampleType::Output
            ),
            "definition/template/adl1.4/minimal_evaluation.en.v1/example?detail_level=complete&type=output"
        );
    }

    #[test]
    fn the_example_path_re_encodes_the_decoded_route_param() {
        // The id arrived percent-decoded from the route, so a reserved
        // character must be escaped again or the request lands elsewhere.
        assert_eq!(
            example_path("a/b", ExampleDetail::Medium, ExampleType::Input),
            "definition/template/adl1.4/a%2Fb/example?detail_level=medium&type=input"
        );
        assert_eq!(
            example_path(
                "temperatur-°C",
                ExampleDetail::default(),
                ExampleType::default()
            ),
            "definition/template/adl1.4/temperatur-%C2%B0C/example?detail_level=required&type=input"
        );
    }
}
