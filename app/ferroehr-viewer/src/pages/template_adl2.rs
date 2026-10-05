// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/templates/adl2/{template_id}` screen — ADL2 template detail.
//!
//! Four panes over one stored ADL2 operational template. Three of them are
//! what the ITS-REST Definition API serves for the resource: **Source** (the
//! stored artefact verbatim, `Accept: text/plain`), **AOM2 JSON** (the
//! `OperationalTemplateV2` canonical-JSON projection,
//! `Accept: application/json`), and **Example** (the CDR-generated example
//! composition, negotiated across the four `Accept_LOCATABLE` forms). A
//! version bar drives the versioned get
//! (`definition/template/adl2/{template_id}/{version}`) as `?version=` URL
//! state, so a pinned view is shareable.
//!
//! The fourth, **Path catalog**, is built viewer-side: the ADL2 resource
//! serves no Web Template representation, so the BFF reads the same
//! `application/json` body, parses the AOM2 operational template, builds its
//! Web Template and projects the [`crate::builder::catalog::CatalogNode`] tree
//! the ADL 1.4 detail screen renders — same tree component, same node
//! inspector, one code path for both families.
//!
//! No openEHR spec governs the viewer — our own design / product extension;
//! the wire it reads is the ITS-REST Definition API.
//!
//! Absence (`404`) is `Ok(None)`, a first-class state rather than an error;
//! tab and version state live in the URL.

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;
use leptos_router::hooks::{use_params_map, use_query_map};

use crate::adl2::{Adl2Tab, TemplateFamily};
use crate::builder::catalog::CatalogNode;
use crate::components::field::{BTN_SECONDARY, INPUT};
use crate::components::page_header::{Crumb, PageHeader};
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::error::ViewerError;
use crate::example_options::{ExampleDetail, ExampleType};
use crate::format::ReprFormat;
use crate::pages::template_detail::{
    CatalogTreeNode, catalog_error_view, node_inspector, tree_skeleton,
};
use crate::pages::templates::TemplateRow;

/// Fetch the stored ADL2 operational-template SOURCE.
///
/// GET `definition/template/adl2/{template_id}` (or
/// `…/{template_id}/{version}` when `version` is given) with
/// `Accept: text/plain` — the artefact interchange form the resource serves
/// verbatim. `version` is an exact SEMVER or a `{major}` / `{major}.{minor}`
/// prefix resolving to the highest match; an unknown template or version is a
/// `404`, returned as `Ok(None)` because absence is a state this screen
/// renders, not a failure.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_adl2_source(
    /// The ADL2 template to read (a full HRID, or a partial that resolves to
    /// the latest matching version).
    template_id: String,
    /// The release version to pin the read to, or `None` for the artefact
    /// `template_id` itself resolves to.
    version: Option<String>,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&crate::adl2::template_path(
        &template_id,
        version.as_deref(),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "text/plain")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(crate::cdr::CdrClient::expect_success(response)?.body))
}

/// Fetch the `OperationalTemplateV2` canonical JSON of a stored ADL2 template.
///
/// The same resource as [`fetch_adl2_source`] under
/// `Accept: application/json`, which the CDR answers with the AOM2
/// canonical-JSON projection of the operational template. `404` → `Ok(None)`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_adl2_json(
    /// The ADL2 template to read.
    template_id: String,
    /// The release version to pin the read to, or `None`.
    version: Option<String>,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&crate::adl2::template_path(
        &template_id,
        version.as_deref(),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(crate::cdr::CdrClient::expect_success(response)?.body))
}

/// Build the Web-Template path catalog of one `OperationalTemplateV2`
/// canonical-JSON body.
///
/// The body is read with [`openehr_its::json::from_canonical_json`] into the
/// AOM2
/// [`OperationalTemplate`](openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate)
/// — the exact type the CDR serialized — then
/// [`openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4`]
/// produces the Web Template and [`crate::builder::catalog::from_web_template`]
/// the slim tree the views render. Both stages are named in the error.
/// # Errors
/// [`ViewerError::Internal`] when the AOM2 JSON fails to parse or the Web
/// Template fails to build (the diagnostic named, never a panic).
#[cfg(feature = "ssr")]
pub fn adl2_catalog_from_json(json: &str) -> Result<CatalogNode, ViewerError> {
    let opt = openehr_its::json::from_canonical_json::<
        openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate,
    >(json)
    .map_err(|e| ViewerError::Internal(format!("OperationalTemplateV2 parse: {e}")))?;
    let web_template = openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4(&opt)
        .map_err(|e| ViewerError::Internal(format!("WebTemplate build: {e}")))?;
    Ok(crate::builder::catalog::from_web_template(&web_template))
}

/// Fetch the Web-Template path catalog of a stored ADL2 template.
///
/// The same resource and `Accept` as [`fetch_adl2_json`]: the ADL2 template
/// GET's `application/json` 200 body IS the `OperationalTemplateV2` canonical
/// form (`docs/specs/openehr/ITS-REST/specifications/responses/
/// 200_Template_adl2_retrieved.yaml`). The catalog is built viewer-side by
/// [`adl2_catalog_from_json`], because the resource serves no `wt+json`
/// representation. `404` → `Ok(None)`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR;
/// [`ViewerError::Internal`] when the served JSON fails to parse or its Web
/// Template fails to build.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_adl2_catalog(
    /// The ADL2 template to build the path catalog from.
    template_id: String,
    /// The release version to pin the read to, or `None`.
    version: Option<String>,
) -> Result<Option<CatalogNode>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&crate::adl2::template_path(
        &template_id,
        version.as_deref(),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    adl2_catalog_from_json(&body).map(Some)
}

/// Fetch the CDR-generated example composition for a stored ADL2 template, in
/// `format`.
///
/// GET `definition/template/adl2/{template_id}/example` with `Accept` set to
/// the selected representation's media type, and the operator's `detail_level`
/// and `type` on the query string
/// (`docs/specs/openehr/ITS-REST/specifications/operations/definition_template_adl2_example_get.yaml`).
/// The Definition API declares no VERSIONED example resource, so the example is
/// generated from the artefact `template_id` itself resolves to.
/// `404` → `Ok(None)`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR (a template that cannot be
/// compiled into a Web Template answers `422`).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_adl2_example(
    /// The ADL2 template to generate an example composition for.
    template_id: String,
    /// Which representation to negotiate for the example.
    format: ReprFormat,
    /// How much of the template the example fills in (`detail_level`).
    detail: ExampleDetail,
    /// Which form the example is shaped for (`type`).
    kind: ExampleType,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state
        .cdr
        .rest_v1(&crate::adl2::example_path(&template_id, detail, kind));
    let response = state
        .cdr
        .get(&session.credential, &url, format.media_type())
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(crate::cdr::CdrClient::expect_success(response)?.body))
}

/// What one pane holds.
///
/// The three states are distinct on purpose: a pane whose tab is not selected
/// has fetched NOTHING and renders nothing, while a pane whose fetch came back
/// `404` has a real answer to report. Collapsing both into `Option` would show
/// the absence bar behind every closed tab, and flash it while a newly opened
/// one loads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum PaneBody {
    /// This pane's tab is not selected; nothing has been requested.
    Idle,
    /// The CDR answered `404` — no such template at the selected version.
    Absent,
    /// The representation the CDR served.
    Loaded(String),
}

impl PaneBody {
    /// Read one fetched representation: `Some` is what the CDR served, `None`
    /// its `404`.
    fn of(fetched: Option<String>) -> Self {
        match fetched {
            Some(body) => Self::Loaded(body),
            None => Self::Absent,
        }
    }
}

/// What the path-catalog pane holds — [`PaneBody`]'s three states over the
/// catalog tree instead of a served representation, for the same reason: an
/// unopened tab has fetched nothing, while a `404` is a real answer to report.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum CatalogBody {
    /// This pane's tab is not selected; nothing has been requested.
    Idle,
    /// The CDR answered `404` — no such template at the selected version.
    Absent,
    /// The catalog built from the served `OperationalTemplateV2`.
    Loaded(CatalogNode),
}

impl CatalogBody {
    /// Read one built catalog: `Some` is the tree, `None` the CDR's `404`.
    fn of(built: Option<CatalogNode>) -> Self {
        match built {
            Some(node) => Self::Loaded(node),
            None => Self::Absent,
        }
    }
}

/// The ADL2 template detail screen: the version bar, the tab bar, and the
/// source / AOM2-JSON / path-catalog / example panes.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "one setup pass: the URL state, one resource per pane, the tab bar"
)]
pub fn Adl2TemplateDetailPage() -> impl IntoView {
    // NOTE: the route param arrives ALREADY percent-decoded on both targets
    // (`leptos_router`'s `ParamsMap::insert` runs every value through
    // `Url::unescape`), so decoding here would be a second, corrupting pass.
    let params = use_params_map();
    let template_id =
        Signal::derive(move || params.with(|map| map.get("template_id").unwrap_or_default()));

    // Both view selections are URL state, read in setup: the pane (`?tab=`)
    // and the release version the reads are pinned to (`?version=`).
    let query = use_query_map();
    let tab =
        Memo::new(move |_| Adl2Tab::from_query(&query.with(|q| q.get("tab").unwrap_or_default())));
    let version = Memo::new(move |_| {
        query
            .with(|q| q.get("version"))
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    });
    let example_format = RwSignal::new(ReprFormat::CanonicalJson);
    let example_detail = RwSignal::new(ExampleDetail::default());
    let example_kind = RwSignal::new(ExampleType::default());
    let selected_node = RwSignal::new(None::<CatalogNode>);

    // Each pane's resource is gated on its tab being active so only the
    // visible one fetches — the example fetch in particular runs the CDR's
    // example GENERATOR. The stable source keeps loaded state on re-show.
    let source: Resource<Result<PaneBody, ViewerError>> = Resource::new(
        move || (tab.get() == Adl2Tab::Source).then(|| (template_id.get(), version.get())),
        |active| async move {
            match active {
                Some((id, version)) => Ok(PaneBody::of(fetch_adl2_source(id, version).await?)),
                None => Ok(PaneBody::Idle),
            }
        },
    );
    let json: Resource<Result<PaneBody, ViewerError>> = Resource::new(
        move || (tab.get() == Adl2Tab::Json).then(|| (template_id.get(), version.get())),
        |active| async move {
            match active {
                Some((id, version)) => Ok(PaneBody::of(fetch_adl2_json(id, version).await?)),
                None => Ok(PaneBody::Idle),
            }
        },
    );
    let catalog: Resource<Result<CatalogBody, ViewerError>> = Resource::new(
        move || (tab.get() == Adl2Tab::Catalog).then(|| (template_id.get(), version.get())),
        |active| async move {
            match active {
                Some((id, version)) => Ok(CatalogBody::of(fetch_adl2_catalog(id, version).await?)),
                None => Ok(CatalogBody::Idle),
            }
        },
    );
    let example: Resource<Result<PaneBody, ViewerError>> = Resource::new(
        move || {
            (tab.get() == Adl2Tab::Example).then(|| {
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
                Some((id, format, detail, kind)) => Ok(PaneBody::of(
                    fetch_adl2_example(id, format, detail, kind).await?,
                )),
                None => Ok(PaneBody::Idle),
            }
        },
    );
    // The stored versions of this HRID family, read from the ONE listing
    // endpoint the Template Manager already reads (one reader per
    // claim), so the version bar offers what the CDR actually holds
    // instead of guessing at a range.
    let listing: Resource<Result<Vec<TemplateRow>, ViewerError>> = Resource::new(
        || (),
        |()| async move { crate::pages::templates::list_adl2_templates().await },
    );

    let versions = version_section(template_id, tab, version, listing);
    let source_pane = source_tab(source);
    let json_pane = json_tab(json);
    let catalog_pane = catalog_tab(catalog, selected_node);
    let example_pane = example_tab(example, example_format, example_detail, example_kind);

    let tab_link = move |value: Adl2Tab| {
        let class = move || {
            let base = "rounded-control px-3 py-1.5 text-sm font-medium transition-colors";
            if tab.get() == value {
                format!("{base} bg-accent-subtle text-accent-ink")
            } else {
                format!("{base} text-ink-muted hover:bg-sunken")
            }
        };
        let href =
            move || crate::adl2::view_href(&template_id.get(), value, version.get().as_deref());
        view! {
            <leptos_router::components::A
                href=href
                attr:class=class
                attr:data-adl2-tab=value.as_query()
            >
                {value.label()}
            </leptos_router::components::A>
        }
        .into_any()
    };

    view! {
        <Title text=move || format!("ADL2 template · {}", template_id.get()) />
        <div class="p-6">
            <PageHeader
                title=template_id
                crumbs=vec![Crumb::new("Templates", TemplateFamily::Adl2.href())]
                mono=true
            />
            {versions}
            <nav aria-label="ADL2 template views" class="flex gap-1 mb-4">
                {tab_link(Adl2Tab::Source)}
                {tab_link(Adl2Tab::Json)}
                {tab_link(Adl2Tab::Catalog)}
                {tab_link(Adl2Tab::Example)}
            </nav>
            <div>
                <div id="adl2-source-pane" class:hidden=move || tab.get() != Adl2Tab::Source>
                    {source_pane}
                </div>
                <div id="adl2-json-pane" class:hidden=move || tab.get() != Adl2Tab::Json>
                    {json_pane}
                </div>
                <div id="adl2-catalog-pane" class:hidden=move || tab.get() != Adl2Tab::Catalog>
                    {catalog_pane}
                </div>
                <div id="adl2-example-pane" class:hidden=move || tab.get() != Adl2Tab::Example>
                    {example_pane}
                </div>
            </div>
        </div>
    }
}

/// The version bar: one chip per stored version of this HRID family, plus a
/// free-text box for the wire's prefix resolution.
///
/// Both controls write the same `?version=` URL state, so a pinned view is
/// shareable and survives a reload. The chips come from the ADL2 listing —
/// the exact versions the CDR holds — while the box reaches the versioned
/// get's `{major}` / `{major}.{minor}` prefix forms, which resolve to the
/// highest match.
fn version_section(
    template_id: Signal<String>,
    tab: Memo<Adl2Tab>,
    version: Memo<Option<String>>,
    listing: Resource<Result<Vec<TemplateRow>, ViewerError>>,
) -> AnyView {
    let chips = move || {
        Suspend::new(async move {
            let rows = match listing.await {
                Ok(rows) => rows,
                Err(e) => return crate::components::notice::inline_error(&e),
            };
            let ids = rows
                .into_iter()
                .map(|row| row.template_id)
                .collect::<Vec<_>>();
            let id = template_id.get();
            let (stem, _) = crate::adl2::split_hrid(&id);
            let stored = crate::adl2::family_versions(&ids, stem);
            if stored.is_empty() {
                return view! {
                    <p class="text-sm text-ink-muted">
                        "The listing holds no other version of this template family."
                    </p>
                }
                .into_any();
            }
            let buttons = stored
                .into_iter()
                .map(|value| version_chip(template_id, tab, version, Some(value)))
                .collect::<Vec<_>>();
            view! {
                <div class="flex flex-wrap items-center gap-1">
                    {version_chip(template_id, tab, version, None)} {buttons}
                </div>
            }
            .into_any()
        })
    };
    view! {
        <section class=format!("{CARD_PAD} mb-4")>
            <h2 class=CARD_TITLE>"Version"</h2>
            <Transition fallback=|| {
                view! {
                    <thaw::Skeleton class="h-8">
                        <thaw::SkeletonItem />
                    </thaw::Skeleton>
                }
            }>{chips}</Transition>
            {version_form(template_id, tab, version)}
        </section>
    }
    .into_any()
}

/// One version chip: `None` reads the artefact the route names, `Some(v)` pins
/// the read to that release version through the versioned get.
fn version_chip(
    template_id: Signal<String>,
    tab: Memo<Adl2Tab>,
    version: Memo<Option<String>>,
    value: Option<String>,
) -> AnyView {
    let label = value.clone().unwrap_or_else(|| "As stored".to_owned());
    let marker = value.clone().unwrap_or_else(|| "stored".to_owned());
    let selected = {
        let value = value.clone();
        move || version.get() == value
    };
    let class = move || {
        let base = "rounded-full px-3 py-1 text-xs font-medium transition-colors";
        if selected() {
            format!("{base} bg-accent text-on-accent")
        } else {
            format!("{base} bg-accent-subtle text-accent-ink hover:bg-sunken")
        }
    };
    let href = move || crate::adl2::view_href(&template_id.get(), tab.get(), value.as_deref());
    view! {
        <leptos_router::components::A href=href attr:class=class attr:data-adl2-version=marker>
            {label}
        </leptos_router::components::A>
    }
    .into_any()
}

/// The free-text version box: a GET `<Form>` submitting `?version=` back to
/// this screen, so it works before the WASM bundle loads. The hidden `tab`
/// field keeps the open pane across the submit — a GET form replaces the
/// action's whole query string, so a `?tab=` in the action would be
/// discarded.
fn version_form(
    template_id: Signal<String>,
    tab: Memo<Adl2Tab>,
    version: Memo<Option<String>>,
) -> AnyView {
    let action = move || {
        format!(
            "/templates/adl2/{}",
            urlencoding::encode(&template_id.get())
        )
    };
    let initial = version.get_untracked().unwrap_or_default();
    view! {
        <leptos_router::components::Form method="GET" action=action attr:class="mt-3">
            <div class="flex flex-wrap items-end gap-2">
                // The hidden field carries live state on `prop:value` with the
                // attribute for the server pass (attributes set the initial
                // state, properties carry the live one).
                <input
                    type="hidden"
                    name="tab"
                    value=tab.get_untracked().as_query()
                    prop:value=move || tab.get().as_query()
                />
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "Release version (1.1.0, or a 1 / 1.0 prefix)"
                    <input
                        id="adl2-version-input"
                        type="text"
                        name="version"
                        class=format!("w-64 {INPUT}")
                        placeholder="as stored"
                        value=initial
                    />
                </label>
                <button id="adl2-version-apply" type="submit" class=BTN_SECONDARY>
                    "Show version"
                </button>
            </div>
        </leptos_router::components::Form>
    }
    .into_any()
}

/// The Path catalog pane: the recursive path-catalog tree (left) and the node
/// inspector (right), the SAME components the ADL 1.4 detail screen uses
/// ([`CatalogTreeNode`], [`node_inspector`]) over a tree the BFF built from the
/// served `OperationalTemplateV2`.
fn catalog_tab(
    catalog: Resource<Result<CatalogBody, ViewerError>>,
    selected: RwSignal<Option<CatalogNode>>,
) -> AnyView {
    view! {
        <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 items-start">
            <section class=CARD_PAD>
                <h2 class=CARD_TITLE>"Path catalog (WT tree)"</h2>
                <p class="mb-2 text-xs text-ink-muted">
                    "Built by the viewer from the served OperationalTemplateV2: the ADL2 resource \
                     serves no Web Template representation of its own."
                </p>
                <div class="overflow-auto max-h-[70vh]">
                    <Transition fallback=tree_skeleton>
                        {move || Suspend::new(async move {
                            match catalog.await {
                                Ok(CatalogBody::Idle) => ().into_any(),
                                Ok(CatalogBody::Loaded(node)) => {
                                    view! {
                                        <ul class="text-sm">
                                            <CatalogTreeNode node=node selected=selected depth=0 />
                                        </ul>
                                    }
                                        .into_any()
                                }
                                Ok(CatalogBody::Absent) => absent_view("path catalog"),
                                Err(e) => catalog_error_view(&e, "/templates?family=adl2"),
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

/// The absence state: the CDR answered `404` for the template (or the pinned
/// version), which is a fact about the store, not a failure of the screen.
fn absent_view(what: &str) -> AnyView {
    let message = format!(
        "The CDR holds no {what} for this template at the selected version. Pick another version, \
         or go back to the ADL2 listing."
    );
    view! {
        <thaw::MessageBar intent=thaw::MessageBarIntent::Warning>
            <thaw::MessageBarBody>
                {message} " "
                <leptos_router::components::A
                    href=TemplateFamily::Adl2.href()
                    attr:class="text-accent hover:underline"
                >
                    "ADL2 templates"
                </leptos_router::components::A>
            </thaw::MessageBarBody>
        </thaw::MessageBar>
    }
    .into_any()
}

/// The Source pane: the stored ADL2 artefact verbatim in the shared document
/// pane (monospace, scrollable, copyable).
fn source_tab(source: Resource<Result<PaneBody, ViewerError>>) -> AnyView {
    view! {
        <Transition fallback=tree_skeleton>
            {move || Suspend::new(async move {
                match source.await {
                    Ok(PaneBody::Idle) => ().into_any(),
                    Ok(PaneBody::Loaded(text)) => {
                        view! { <crate::components::format_view::DocumentPane body=text /> }
                            .into_any()
                    }
                    Ok(PaneBody::Absent) => absent_view("ADL2 source"),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The AOM2 JSON pane: the `OperationalTemplateV2` canonical JSON,
/// pretty-printed into the shared document pane.
fn json_tab(json: Resource<Result<PaneBody, ViewerError>>) -> AnyView {
    view! {
        <Transition fallback=tree_skeleton>
            {move || Suspend::new(async move {
                match json.await {
                    Ok(PaneBody::Idle) => ().into_any(),
                    Ok(PaneBody::Loaded(raw)) => {
                        let pretty = crate::components::format_view::pretty_body(
                            &raw,
                            ReprFormat::CanonicalJson,
                        );
                        view! { <crate::components::format_view::DocumentPane body=pretty /> }
                            .into_any()
                    }
                    Ok(PaneBody::Absent) => absent_view("OperationalTemplateV2 JSON"),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The Example pane: the shared example controls — representation, detail
/// level, and the form the example is shaped for — then the generated
/// composition.
fn example_tab(
    example: Resource<Result<PaneBody, ViewerError>>,
    format: RwSignal<ReprFormat>,
    detail: RwSignal<ExampleDetail>,
    kind: RwSignal<ExampleType>,
) -> AnyView {
    view! {
        <div class="space-y-3">
            <p class="text-sm text-ink-muted">
                "The example is generated from the artefact this route names — the Definition API \
                 declares no versioned example resource, so the version bar above does not change \
                 it."
            </p>
            <crate::components::example_controls::ExampleControls
                format=format
                detail=detail
                kind=kind
            />
            <Transition fallback=tree_skeleton>
                {move || Suspend::new(async move {
                    match example.await {
                        Ok(PaneBody::Idle) => ().into_any(),
                        Ok(PaneBody::Loaded(raw)) => {
                            let pretty = crate::components::format_view::pretty_body(
                                &raw,
                                format.get_untracked(),
                            );
                            view! { <crate::components::format_view::DocumentPane body=pretty /> }
                                .into_any()
                        }
                        Ok(PaneBody::Absent) => absent_view("example composition"),
                        Err(e) => crate::components::notice::inline_error(&e),
                    }
                })}
            </Transition>
        </div>
    }
    .into_any()
}
