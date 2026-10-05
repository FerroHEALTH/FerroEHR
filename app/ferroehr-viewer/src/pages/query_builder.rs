// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/queries/builder` screen: the point-and-click Query Builder.
//!
//! A template-first, cascading builder over the viewer's AQL engine. The whole
//! editable state is one [`BuilderQuery`]; the live AQL is produced by
//! [`to_aql`] on every change, and the result runs against `POST query/aql`
//! through [`run_aql`]. No openEHR spec governs the viewer — our own design;
//! the wire it drives is spec-bound (ITS-REST Query API).
//!
//! A separate `struct_ver` signal gates the tree/output re-renders, so typing
//! into a field updates only the live preview and inputs keep focus.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use std::collections::BTreeMap;
use std::collections::HashMap;

use leptos::component;
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::components::A;

use crate::builder::catalog::CatalogNode;
use crate::builder::lift::LiftError;
use crate::builder::lower::{BuilderError, to_aql};
use crate::builder::model::{
    BoolOp, BuilderQuery, Criterion, CriterionKind, CriterionNode, OrderRule, QueryShape,
    SelectedColumn,
};
use crate::components::data_table::{CELL, PAGE_SIZE, ROW, table_shell, table_skeleton};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, LABEL, SELECT};
use crate::components::page_header::PageHeader;
use crate::components::surface::{CARD_PAD, CARD_TITLE, WELL};
use crate::components::toast::toast_success;
use crate::error::ViewerError;
use crate::pages::ehrs::{ResultPage, cell_text};
use crate::pages::query_aql::LoadedQuery;
use crate::pages::template_detail::fetch_template_catalog;
use crate::pages::templates::list_templates;
use crate::queries_api::{run_aql, store_query};
use crate::query_namespace::{is_full_semver, next_minor, qualify, split_qualified};
use crate::terminology::{TermRow, fetch_term, fetch_value_set, list_terminologies};

/// The shared, all-`Copy` signal bundle the builder's recursive views thread
/// through instead of a long argument list. `struct_ver` is bumped only on
/// structural edits (add/remove/regroup/toggle/shape) so text-field edits
/// re-render only the live preview, not the tree.
#[derive(Clone, Copy)]
struct BuilderCtx {
    /// The single source of truth for the whole builder state.
    query: RwSignal<BuilderQuery>,
    /// Bumped on every structural mutation to force a tree/output re-render.
    struct_ver: RwSignal<u32>,
    /// Path → the catalog node that spawned a criterion/column, so the typed
    /// editor and the readable sentence can recover the RM type, label, and
    /// the constrained code/unit option lists (keyed by the leaf `aql_path`).
    leaf_meta: RwSignal<HashMap<String, CatalogNode>>,
    /// The group a new criterion is added into (defaults to the root group).
    active_path: RwSignal<Vec<usize>>,
    /// The terminology ids the CDR serves, backing the coded editor's
    /// datalist. Created once in page setup: the criteria tree re-renders on
    /// every structural change, so a resource created inside it would be
    /// re-created, and re-fetched, on each one.
    terminologies: Resource<Result<Option<Vec<String>>, ViewerError>>,
}

impl BuilderCtx {
    /// Signal a structural change: re-render the tree/output sections.
    fn bump(self) {
        self.struct_ver.update(|v| *v = v.saturating_add(1));
    }
}

/// The Query Builder screen: template picker, path catalog, criterion tree,
/// output shape, a live AQL preview, and the run/save surface.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "one setup pass: signals, resources, the ?load lift wiring"
)]
pub fn QueryBuilderPage() -> impl IntoView {
    let ctx = BuilderCtx {
        query: RwSignal::new(BuilderQuery::new(String::new())),
        struct_ver: RwSignal::new(0),
        leaf_meta: RwSignal::new(HashMap::new()),
        active_path: RwSignal::new(Vec::new()),
        terminologies: Resource::new(|| (), |()| async move { list_terminologies().await }),
    };
    let offset = RwSignal::new(0_u32);
    let ran = RwSignal::new(None::<String>);
    // The two halves of the stored query's qualified name (`namespace::name`,
    // the namespace optional — see `crate::query_namespace`).
    let save_namespace = RwSignal::new(String::new());
    let save_name = RwSignal::new(String::new());
    // The optional store version: empty means "let the server assign it"
    // (the unversioned store), a `major.minor.patch` triple means "store this
    // immutable version" — see `store_query`.
    let save_version = RwSignal::new(String::new());
    let save_fields = SaveFields {
        namespace: save_namespace,
        name: save_name,
        version: save_version,
    };

    // `?load=name@version` fetches the stored query and lifts it back into the
    // builder state. `load` is URL-derived, so it is identical on the server
    // pass and on client hydration.
    let query_map = leptos_router::hooks::use_query_map();
    let has_load = query_map.with_untracked(|m| m.get("load").is_some_and(|s| !s.is_empty()));
    let load_resource = crate::pages::query_aql::loaded_query_resource(query_map);
    // Why a lift was refused, when it was; the notice then offers the raw
    // editor, which can hold any query.
    let lift_refusal = RwSignal::new(Option::<LiftError>::None);
    seed_builder_from_stored_query(load_resource, ctx, save_fields, lift_refusal);

    let templates: Resource<Result<Vec<crate::pages::templates::TemplateRow>, ViewerError>> =
        Resource::new(|| (), |()| async move { list_templates().await });
    let selected_template = Signal::derive(move || ctx.query.with(|q| q.template_id.clone()));
    let catalog: Resource<Result<Option<CatalogNode>, ViewerError>> = Resource::new(
        move || selected_template.get(),
        |id| async move {
            if id.is_empty() {
                Ok(None)
            } else {
                fetch_template_catalog(id).await.map(Some)
            }
        },
    );
    let results: Resource<Result<Option<ResultPage>, ViewerError>> = Resource::new(
        move || (ran.get(), offset.get()),
        |(ran, off)| async move {
            match ran {
                Some(aql) => run_aql(aql, "{}".to_owned(), off).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let save_action: SaveAction = Action::new(|input: &SaveInput| {
        let (name, version, aql) = input.clone();
        async move { store_query(name, version, aql).await }
    });
    // Both outcomes toast; the CDR's diagnostic also stays inline
    // (`save_feedback`), beside the AQL it rejected.
    let toaster = thaw::ToasterInjection::expect_context();
    Effect::new(move |_| match save_action.value().get() {
        Some(Ok(())) => toast_success(toaster, "Query saved", ""),
        Some(Err(error)) => {
            crate::feedback::toast_write_failure(
                toaster,
                "Save failed",
                "the stored query",
                &error,
            );
        }
        None => {}
    });

    // The live AQL / validation, recomputed from the whole state on any change.
    let preview = Memo::new(move |_| ctx.query.with(to_aql));

    let load_notice = if has_load {
        load_notice_section(load_resource, lift_refusal)
    } else {
        ().into_any()
    };
    // A lifted query arrives with its criteria already built, so their catalog
    // metadata has to come from the template's catalog rather than from the
    // click that would normally have added them.
    if has_load {
        seed_leaf_meta_from_catalog(catalog, ctx);
    }

    let template_step = template_step_section(ctx, ran, templates);
    let picker = picker_section(ctx, catalog);
    let criteria = criteria_section(ctx);
    let output = output_section(ctx);
    let preview_run = preview_run_section(preview, ran, offset, save_fields, save_action);
    // Export tracks the live AQL preview (empty while it is a `BuilderError`);
    // the builder binds no parameters, so its parameter payload is `{}`.
    let export_aql = Signal::derive(move || preview.with(|r| r.clone().unwrap_or_default()));
    let export_params = Signal::derive(|| "{}".to_owned());
    let results_pane = results_section(ctx, results, offset, export_aql, export_params);

    view! {
        <Title text="Query builder" />
        <div class="p-6 space-y-4">
            <PageHeader
                title="Query builder"
                subtitle="Pick a template, walk its paths, and turn data-value leaves into criteria and columns."
            />
            {load_notice}
            {template_step}
            <div class="grid grid-cols-1 lg:grid-cols-3 gap-4 items-start">
                <section class="rounded-card border border-edge bg-raised shadow-card p-4 overflow-auto max-h-[70vh]">
                    <h2 class=CARD_TITLE>"Path catalog"</h2>
                    {picker}
                </section>
                <div class="lg:col-span-2 space-y-4">
                    <section class=CARD_PAD>
                        <h2 class=CARD_TITLE>"Criteria"</h2>
                        {criteria}
                    </section>
                    <section class=CARD_PAD>{output}</section>
                </div>
            </div>
            {preview_run}
            {results_pane}
            {terminology_options(ctx)}
        </div>
    }
}

/// The terminology ids the CDR serves, as ONE document-level `<datalist>` every
/// coded criterion editor references by id (`list="qb-terminology-options"`).
///
/// One element rather than one per criterion: two `<datalist>`s sharing an id
/// would be invalid HTML, and hydration walks the DOM. A CDR that does not
/// serve the terminology surface answers `404`, which reads as no options at
/// all, leaving the field the plain text input an AQL author needs to name a
/// terminology this server does not host.
fn terminology_options(ctx: BuilderCtx) -> AnyView {
    view! {
        <Suspense fallback=|| ()>
            {move || Suspend::new(async move {
                let ids = ctx.terminologies.await.ok().flatten().unwrap_or_default();
                let options = ids
                    .into_iter()
                    .map(|id| view! { <option value=id></option> })
                    .collect::<Vec<_>>();
                view! { <datalist id="qb-terminology-options">{options}</datalist> }.into_any()
            })}
        </Suspense>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// Stored-query hand-off (?load= → reverse lift)
// ---------------------------------------------------------------------------

/// Build the link INTO this screen that lifts the stored query `name@version`
/// back into the builder ([`crate::builder::lift::from_aql`]), sharing the
/// `?load=name@version` encoding of [`crate::query_namespace::load_href`].
#[must_use]
pub(crate) fn load_href(name: &str, version: &str) -> String {
    crate::query_namespace::load_href("/queries/builder", name, version)
}

/// Lift a loaded stored query into the builder state, exactly once and
/// client-side.
///
/// The one seed on this screen that stays an `Effect`: what it writes,
/// `ctx.query.template_id`, is the SOURCE of the `catalog` resource the
/// always-mounted picker `<Transition>` renders, so seeding from inside a
/// `Suspend` would have one suspended view mutate another's resource source
/// mid-render. An `Effect` never runs on the server, which keeps the server
/// pass deterministic, and a one-shot `StoredValue` guard keeps it from
/// re-firing.
///
/// The save fields are seeded with the qualified name split back into namespace
/// and bare name, and the NEXT version proposed, because the loaded
/// `(name, version)` pair is immutable.
///
/// A query the builder cannot represent is not partially loaded: the refusal is
/// recorded for the notice and the builder stays empty.
fn seed_builder_from_stored_query(
    load_resource: Resource<Result<Option<LoadedQuery>, ViewerError>>,
    ctx: BuilderCtx,
    fields: SaveFields,
    refusal: RwSignal<Option<LiftError>>,
) {
    let seeded = StoredValue::new(false);
    Effect::new(move |_| {
        if seeded.get_value() {
            return;
        }
        let Some(Ok(Some((qualified, version, aql)))) = load_resource.get() else {
            return;
        };
        seeded.set_value(true);
        let (namespace, name) = split_qualified(&qualified);
        fields.namespace.set(namespace);
        fields.name.set(name);
        fields.version.set(next_minor(&version).unwrap_or_default());
        match crate::builder::lift::from_aql(&aql) {
            Ok(lifted) => {
                ctx.query.set(lifted);
                ctx.leaf_meta.set(HashMap::new());
                ctx.active_path.set(Vec::new());
                ctx.bump();
            }
            Err(error) => refusal.set(Some(error)),
        }
    });
}

/// Record every selectable node of the loaded template's catalog as leaf
/// metadata, once, so a LIFTED criterion shows its real label and its
/// constrained code/ordinal/unit options instead of a bare path segment.
///
/// A resource-reading `Effect`, kept deliberately: it writes `ctx.leaf_meta`
/// and bumps the criteria version that the always-mounted criteria section
/// renders from, outside the picker's `<Transition>` — a seed inside that
/// `Suspend` would write them during the server pass and again during hydration
/// replay. One-shot; existing entries win, so a node the user added by hand is
/// never overwritten.
fn seed_leaf_meta_from_catalog(
    catalog: Resource<Result<Option<CatalogNode>, ViewerError>>,
    ctx: BuilderCtx,
) {
    let seeded = StoredValue::new(false);
    Effect::new(move |_| {
        if seeded.get_value() {
            return;
        }
        let Some(Ok(Some(root))) = catalog.get() else {
            return;
        };
        seeded.set_value(true);
        let mut found = BTreeMap::new();
        collect_selectable(&root, &mut found);
        if found.is_empty() {
            return;
        }
        ctx.leaf_meta.update(|known| {
            for (path, node) in found {
                known.entry(path).or_insert(node);
            }
        });
        ctx.bump();
    });
}

/// Every selectable node of a catalog subtree, keyed by its `aql_path`.
///
/// Ordered by key so the seeding pass below walks the collected nodes
/// deterministically (a `HashMap` would visit them in an arbitrary order).
fn collect_selectable(node: &CatalogNode, out: &mut BTreeMap<String, CatalogNode>) {
    if node.selectable && !node.aql_path.is_empty() {
        out.insert(node.aql_path.clone(), node.clone());
    }
    for child in &node.children {
        collect_selectable(child, out);
    }
}

/// The `?load=` hand-off status: which stored query was loaded, or — when the
/// builder could not represent it — why, plus the raw editor as the way to work
/// on it anyway.
fn load_notice_section(
    load_resource: Resource<Result<Option<LoadedQuery>, ViewerError>>,
    refusal: RwSignal<Option<LiftError>>,
) -> AnyView {
    view! {
        <Transition fallback=move || {
            view! { <p class="text-sm text-ink-muted">"Loading stored query…"</p> }
        }>
            {move || Suspend::new(async move {
                match load_resource.await {
                    Ok(Some((qualified, version, _))) => {
                        loaded_notice(&qualified, &version, refusal)
                    }
                    Ok(None) => {
                        view! {
                            <p class="text-sm text-ink-muted">
                                "That link does not name a stored-query version, so the builder started empty."
                            </p>
                        }
                            .into_any()
                    }
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The notice for one loaded stored query: the confirmation line, and — while a
/// refusal is recorded — the reason beside a link into the raw editor.
/// `data-lift-refused` is the stable E2E hook for the refusal path.
fn loaded_notice(qualified: &str, version: &str, refusal: RwSignal<Option<LiftError>>) -> AnyView {
    let editor_href = crate::pages::query_aql::load_href(qualified, version);
    let name = qualified.to_owned();
    let version = version.to_owned();
    view! {
        <section class=CARD_PAD>
            <p class="text-sm text-ink-muted">
                "Loaded stored query " <span class="font-mono text-ink">{name}</span> " at version "
                <span class="font-mono text-ink">{version}</span>
                ". Saving stores the version in the field below — that version is immutable, so the next one is proposed."
            </p>
            {move || {
                refusal
                    .get()
                    .map(|error| {
                        let href = editor_href.clone();
                        view! {
                            <div
                                role="status"
                                data-lift-refused=""
                                class="mt-2 rounded-control border border-warn/40 bg-warn-subtle px-3 py-2 text-sm text-warn"
                            >
                                <p>{error.to_string()}</p>
                                <p class="mt-1">
                                    <a href=href class="underline">
                                        "Open it in the raw AQL editor instead"
                                    </a>
                                    " — the builder was left empty rather than loading a query it would change."
                                </p>
                            </div>
                        }
                    })
            }}
        </section>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// Template step
// ---------------------------------------------------------------------------

/// The template picker: a `<select>` over the CDR's templates. Choosing a
/// template re-seeds the whole builder state ([`BuilderQuery::new`]) and clears
/// the catalog-derived metadata, the active target, and any prior run.
fn template_step_section(
    ctx: BuilderCtx,
    ran: RwSignal<Option<String>>,
    templates: Resource<Result<Vec<crate::pages::templates::TemplateRow>, ViewerError>>,
) -> AnyView {
    view! {
        <section class=CARD_PAD>
            <div class="flex flex-col gap-1 max-w-md">
                <label class=LABEL r#for="qb-template">
                    "Template"
                </label>
                <Suspense fallback=move || {
                    view! { <span class="text-sm text-ink-muted">"Loading templates…"</span> }
                }>
                    {move || Suspend::new(async move {
                        match templates.await {
                            Ok(rows) => template_select(ctx, ran, rows),
                            Err(e) => crate::components::notice::inline_error(&e),
                        }
                    })}
                </Suspense>
            </div>
        </section>
    }
    .into_any()
}

/// The template `<select>` itself, once the list has loaded.
fn template_select(
    ctx: BuilderCtx,
    ran: RwSignal<Option<String>>,
    rows: Vec<crate::pages::templates::TemplateRow>,
) -> AnyView {
    let options = rows
        .into_iter()
        .map(|row| {
            let id = row.template_id.clone();
            view! { <option value=id.clone()>{row.template_id}</option> }
        })
        .collect::<Vec<_>>();
    view! {
        <select
            id="qb-template"
            class=SELECT
            prop:value=move || ctx.query.with(|q| q.template_id.clone())
            on:change:target=move |ev| {
                let id = ev.target().value();
                ctx.query.set(BuilderQuery::new(id));
                ctx.leaf_meta.set(HashMap::new());
                ctx.active_path.set(Vec::new());
                ran.set(None);
                ctx.bump();
            }
        >
            <option value="">"— any template —"</option>
            {options}
        </select>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// Path picker
// ---------------------------------------------------------------------------

/// The path catalog pane: the template's [`CatalogNode`] tree under a
/// `<Transition>`, with click-to-add affordances on selectable data-value
/// leaves.
fn picker_section(
    ctx: BuilderCtx,
    catalog: Resource<Result<Option<CatalogNode>, ViewerError>>,
) -> AnyView {
    let shape_is_dv = Signal::derive(move || ctx.query.with(|q| q.shape == QueryShape::DataValues));
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match catalog.await {
                    Ok(None) => {
                        // Resolve inside the Transition: an SSR'd ErrorBoundary fallback
                        // mismatches at hydration in leptos 0.8.
                        view! {
                            <p class="text-sm text-ink-muted">
                                "Pick a template to browse its paths."
                            </p>
                        }
                            .into_any()
                    }
                    Ok(Some(node)) => {
                        view! { <ul class="text-sm">{picker_node(&node, ctx, shape_is_dv, 0)}</ul> }
                            .into_any()
                    }
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The path-picker row's disclosure chevron. Icon-only control: the chevron is
/// decoration, so the button carries the node it opens as its name and its
/// open/closed state as `aria-expanded` (WAI-ARIA Authoring Practices,
/// "Disclosure" pattern). A leaf renders the aligning spacer instead.
fn disclosure_toggle(node_label: &str, expanded: RwSignal<bool>, has_children: bool) -> AnyView {
    if !has_children {
        return view! { <span class="inline-block w-4 shrink-0"></span> }.into_any();
    }
    let toggle_label = format!("Toggle {node_label}");
    view! {
        <button
            type="button"
            class="w-4 shrink-0 text-ink-muted"
            aria-label=toggle_label
            aria-expanded=move || if expanded.get() { "true" } else { "false" }
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
}

fn picker_node(
    node: &CatalogNode,
    ctx: BuilderCtx,
    shape_is_dv: Signal<bool>,
    depth: i32,
) -> AnyView {
    let has_children = !node.children.is_empty();
    let expanded = RwSignal::new(depth < 2);
    let child_views = node
        .children
        .iter()
        .map(|child| picker_node(child, ctx, shape_is_dv, depth + 1))
        .collect::<Vec<_>>();

    let disclosure = disclosure_toggle(&node.label, expanded, has_children);

    let label = node.label.clone();
    let rm_type = node.rm_type.clone();
    let row = if node.selectable {
        let add_node = node.clone();
        let col_node = node.clone();
        view! {
            <div class="flex items-center gap-2 flex-wrap">
                <span>{label}</span>
                <span class="font-mono text-xs text-ink-muted">{rm_type}</span>
                <button
                    type="button"
                    class="text-xs rounded border border-accent text-accent px-1.5 hover:bg-accent-subtle"
                    on:click=move |_| add_criterion(ctx, &add_node)
                >
                    "+ condition"
                </button>
                <button
                    type="button"
                    class="text-xs rounded border border-ok text-ok px-1.5 hover:bg-ok-subtle"
                    class:hidden=move || !shape_is_dv.get()
                    on:click=move |_| add_column(ctx, &col_node)
                >
                    "+ column"
                </button>
            </div>
        }
        .into_any()
    } else {
        view! {
            <button
                type="button"
                class="flex items-center gap-2 text-left rounded px-1 hover:bg-sunken"
                on:click=move |_| expanded.update(|open| *open = !*open)
            >
                <span>{label}</span>
                <span class="font-mono text-xs text-ink-muted">{rm_type}</span>
            </button>
        }
        .into_any()
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
            <div class="flex items-center gap-1">{disclosure}{row}</div>
            {children_list}
        </li>
    }
    .into_any()
}

/// Turn a selectable catalog leaf into a criterion in the active group, keying
/// its default typed constraint off the RM type and recording the node in the
/// metadata map (so the editor and sentence can recover its options/label).
fn add_criterion(ctx: BuilderCtx, node: &CatalogNode) {
    let criterion = Criterion {
        aql_path: node.aql_path.clone(),
        negated: false,
        kind: default_kind_for(&node.rm_type),
    };
    let target = ctx.active_path.get_untracked();
    ctx.query.update(|q| add_leaf_at(q, &target, criterion));
    let node = node.clone();
    ctx.leaf_meta.update(move |m| {
        m.insert(node.aql_path.clone(), node);
    });
    ctx.bump();
}

/// Add a selectable catalog leaf as a projection column (data-value shape).
/// Ignores a duplicate path so a double-click is idempotent.
fn add_column(ctx: BuilderCtx, node: &CatalogNode) {
    let path = node.aql_path.clone();
    ctx.query.update(|q| {
        if !q.columns.iter().any(|col| col.aql_path == path) {
            q.columns.push(SelectedColumn {
                aql_path: path,
                alias: String::new(),
            });
        }
    });
    ctx.bump();
}

// ---------------------------------------------------------------------------
// Criteria tree
// ---------------------------------------------------------------------------

/// The criterion-tree pane: re-rendered on every structural change
/// (`struct_ver`), reading the query untracked so that typing into a leaf field
/// does not tear down the editor. The empty state invites adding a condition.
fn criteria_section(ctx: BuilderCtx) -> AnyView {
    view! {
        <div>
            {move || {
                ctx.struct_ver.get();
                let snapshot = ctx.query.get_untracked();
                match snapshot.criteria {
                    None => {
                        view! {
                            <EmptyState
                                icon=icondata_lu::LuListChecks
                                message="No conditions yet"
                                hint="Add one with \"+ condition\" from the path catalog on the left."
                            />
                        }
                            .into_any()
                    }
                    Some(root) => criterion_view(root, Vec::new(), ctx),
                }
            }}
        </div>
    }
    .into_any()
}

/// Render one node of the criterion tree as a card. A group shows its
/// connective (AND/OR), a whole-group NOT, an "add group" and a remove/clear
/// control, an "add here" target toggle, and its children (n-ary). A leaf shows
/// a live readable sentence, its typed editor, a per-leaf NOT, and remove.
/// Returns [`AnyView`] at every level so the recursion has a finite type.
fn criterion_view(node: CriterionNode, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    match node {
        CriterionNode::Leaf(criterion) => leaf_card(&criterion, path, ctx),
        CriterionNode::Group {
            op,
            negated,
            children,
        } => group_card(op, negated, children, path, ctx),
    }
}

/// The card for a leaf condition.
fn leaf_card(criterion: &Criterion, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    let kind = criterion.kind.clone();
    let meta = ctx
        .leaf_meta
        .with_untracked(|m| m.get(&criterion.aql_path).cloned());
    let sentence_path = path.clone();
    let sentence = move || {
        ctx.query
            .with(|q| sentence_of(q, &sentence_path, ctx.leaf_meta))
    };
    let editor = leaf_editor(kind, meta, path.clone(), ctx);
    let not_path = path.clone();
    let remove_path = path;
    view! {
        <div class="rounded-card border border-edge p-2 bg-raised">
            <div class="flex items-start justify-between gap-2">
                <div class="text-sm text-ink">{sentence}</div>
                <div class="flex gap-1 shrink-0">
                    <button
                        type="button"
                        class=toggle_class(criterion.negated)
                        on:click=move |_| {
                            ctx.query.update(|q| toggle_negated(q, &not_path));
                            ctx.bump();
                        }
                    >
                        "NOT"
                    </button>
                    <button
                        type="button"
                        class="text-xs rounded border border-danger/40 text-danger px-1.5 hover:bg-danger-subtle"
                        aria-label="Remove this condition"
                        on:click=move |_| {
                            ctx.query.update(|q| remove_at(&mut q.criteria, &remove_path));
                            ctx.active_path.set(Vec::new());
                            ctx.bump();
                        }
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuX width="12" height="12" />
                    </button>
                </div>
            </div>
            <div class="mt-2">{editor}</div>
        </div>
    }
    .into_any()
}

/// The card for an n-ary boolean group.
fn group_card(
    op: BoolOp,
    negated: bool,
    children: Vec<CriterionNode>,
    path: Vec<usize>,
    ctx: BuilderCtx,
) -> AnyView {
    let is_root = path.is_empty();
    let op_word = if op == BoolOp::And { "AND" } else { "OR" };
    let child_views = children
        .into_iter()
        .enumerate()
        .map(|(i, child)| {
            let mut child_path = path.clone();
            child_path.push(i);
            criterion_view(child, child_path, ctx)
        })
        .collect::<Vec<_>>();
    let toolbar = group_toolbar(op_word, negated, is_root, path, ctx);

    view! {
        <div class="rounded-card border border-edge p-2 bg-sunken/50">
            {toolbar} <div class="pl-3 border-l border-edge space-y-2">{child_views}</div>
        </div>
    }
    .into_any()
}

/// The header toolbar of a group card: connective toggle, whole-group NOT,
/// "add group", the "add here" target toggle, and remove/clear.
fn group_toolbar(
    op_word: &'static str,
    negated: bool,
    is_root: bool,
    path: Vec<usize>,
    ctx: BuilderCtx,
) -> AnyView {
    let op_path = path.clone();
    let neg_path = path.clone();
    let add_path = path.clone();
    let target_path = path.clone();
    let remove_path = path.clone();
    let active_here = move || ctx.active_path.with(|p| *p == target_path);
    let target_set_path = path;
    // The root group's remove reads "clear" in words; a nested group's is the
    // icon alone, so it needs the name spelled out.
    let remove_label = if is_root {
        "Clear all conditions"
    } else {
        "Remove this group"
    };
    view! {
        <div class="flex items-center gap-1 flex-wrap mb-2">
            <button
                type="button"
                class="text-xs font-semibold rounded-control bg-accent-subtle text-accent-ink px-1.5"
                on:click=move |_| {
                    ctx.query.update(|q| toggle_op(q, &op_path));
                    ctx.bump();
                }
            >
                {op_word}
            </button>
            <button
                type="button"
                class=toggle_class(negated)
                on:click=move |_| {
                    ctx.query.update(|q| toggle_negated(q, &neg_path));
                    ctx.bump();
                }
            >
                "NOT"
            </button>
            <button
                type="button"
                class="text-xs rounded border border-edge-strong px-1.5 hover:bg-sunken"
                on:click=move |_| {
                    ctx.query.update(|q| add_group_at(q, &add_path));
                    ctx.bump();
                }
            >
                "+ group"
            </button>
            <button
                type="button"
                class=move || {
                    if active_here() {
                        "text-xs rounded border border-accent bg-accent-subtle px-1.5".to_owned()
                    } else {
                        "text-xs rounded border border-edge-strong px-1.5 hover:bg-sunken"
                            .to_owned()
                    }
                }
                on:click=move |_| ctx.active_path.set(target_set_path.clone())
            >
                "add here"
            </button>
            <button
                type="button"
                class="text-xs rounded border border-danger/40 text-danger px-1.5 hover:bg-danger-subtle"
                aria-label=remove_label
                on:click=move |_| {
                    ctx.query.update(|q| remove_at(&mut q.criteria, &remove_path));
                    ctx.active_path.set(Vec::new());
                    ctx.bump();
                }
            >
                {if is_root {
                    view! { "clear" }.into_any()
                } else {
                    view! { <leptos_icons::Icon icon=icondata_lu::LuX width="12" height="12" /> }
                        .into_any()
                }}
            </button>
        </div>
    }
    .into_any()
}

/// The active/negated pill style shared by the NOT toggles.
fn toggle_class(active: bool) -> &'static str {
    if active {
        "text-xs font-semibold rounded border border-warn bg-warn-subtle text-warn px-1.5"
    } else {
        "text-xs rounded border border-edge-strong px-1.5 hover:bg-sunken"
    }
}

// ---------------------------------------------------------------------------
// Per-datatype leaf editors
// ---------------------------------------------------------------------------

/// The typed editor for a leaf, dispatched by its [`CriterionKind`]. Each
/// editor seeds local field signals from the snapshot and writes the rebuilt
/// kind back into the query on every input — untracked by the tree, so inputs
/// keep focus while the live preview updates.
fn leaf_editor(
    kind: CriterionKind,
    meta: Option<CatalogNode>,
    path: Vec<usize>,
    ctx: BuilderCtx,
) -> AnyView {
    match kind {
        CriterionKind::QuantityRange { min, max, units } => {
            quantity_editor(min, max, units, meta, path, ctx)
        }
        CriterionKind::CountRange { min, max } => count_editor(min, max, path, ctx),
        CriterionKind::ProportionNumeratorRange { min, max } => {
            proportion_editor(min, max, path, ctx)
        }
        CriterionKind::CodedIn { codes, terminology } => {
            coded_editor(codes, terminology, meta, path, ctx)
        }
        CriterionKind::OrdinalIn { values } => ordinal_editor(values, meta, path, ctx),
        CriterionKind::TextEquals { text } => text_editor(false, text, path, ctx),
        CriterionKind::TextLike { pattern } => {
            text_editor(true, strip_stars(&pattern).to_owned(), path, ctx)
        }
        CriterionKind::DateTimeRange { from, to } => datetime_editor(from, to, path, ctx),
        CriterionKind::BooleanIs { value } => boolean_editor(value, path, ctx),
        CriterionKind::Exists => view! {
            <p class="text-xs text-ink-muted italic">
                "Presence check only for this type — matches when the node exists."
            </p>
        }
        .into_any(),
    }
}

/// A controlled labelled numeric input (`type=number`) bound to `signal`,
/// rerunning `apply` on every input so the rebuilt criterion kind reaches the
/// query. Blank text parses to `None` at the editor level.
fn number_input(
    id: String,
    label: &'static str,
    signal: RwSignal<String>,
    apply: impl Fn() + 'static,
) -> AnyView {
    view! {
        <label class="flex flex-col gap-0.5 text-xs">
            <span class="text-ink-muted">{label}</span>
            <input
                id=id
                type="number"
                step="any"
                class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm w-28"
                prop:value=move || signal.get()
                on:input:target=move |ev| {
                    signal.set(ev.target().value());
                    apply();
                }
            />
        </label>
    }
    .into_any()
}

/// `DV_QUANTITY`: magnitude min/max plus a units selector (a `<select>` from
/// the catalog's constrained units, or free text when the node constrains none).
fn quantity_editor(
    min: Option<f64>,
    max: Option<f64>,
    units: String,
    meta: Option<CatalogNode>,
    path: Vec<usize>,
    ctx: BuilderCtx,
) -> AnyView {
    let min_s = RwSignal::new(fmt_opt_f64(min));
    let max_s = RwSignal::new(fmt_opt_f64(max));
    let units_s = RwSignal::new(units);
    let key = path_key(&path);
    let apply = move || {
        let kind = CriterionKind::QuantityRange {
            min: parse_opt_f64(&min_s.get_untracked()),
            max: parse_opt_f64(&max_s.get_untracked()),
            units: units_s.get_untracked(),
        };
        ctx.query.update(|q| set_leaf_kind(q, &path, kind));
    };
    let unit_options = meta.map(|m| m.unit_options).unwrap_or_default();
    let min_field = number_input(format!("{key}-min"), "min magnitude", min_s, apply.clone());
    let max_field = number_input(format!("{key}-max"), "max magnitude", max_s, apply.clone());
    let units_control = units_input(unit_options, units_s, apply);

    view! { <div class="flex flex-wrap items-end gap-2">{min_field}{max_field}{units_control}</div> }
    .into_any()
}

/// The units control: a constrained `<select>` when the catalog supplies unit
/// options, else a free-text input.
fn units_input(
    options: Vec<String>,
    units_s: RwSignal<String>,
    apply: impl Fn() + Clone + 'static,
) -> AnyView {
    if options.is_empty() {
        let apply = apply.clone();
        return view! {
            <label class="flex flex-col gap-0.5 text-xs">
                <span class="text-ink-muted">"units"</span>
                <input
                    type="text"
                    class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm w-28"
                    prop:value=move || units_s.get()
                    on:input:target=move |ev| {
                        units_s.set(ev.target().value());
                        apply();
                    }
                />
            </label>
        }
        .into_any();
    }
    let opts = options
        .into_iter()
        .map(|u| {
            let label = u.clone();
            view! { <option value=u>{label}</option> }
        })
        .collect::<Vec<_>>();
    view! {
        <label class="flex flex-col gap-0.5 text-xs">
            <span class="text-ink-muted">"units"</span>
            <select
                class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm"
                prop:value=move || units_s.get()
                on:change:target=move |ev| {
                    units_s.set(ev.target().value());
                    apply();
                }
            >
                <option value="">"(any units)"</option>
                {opts}
            </select>
        </label>
    }
    .into_any()
}

/// `DV_COUNT`: integer magnitude min/max.
fn count_editor(min: Option<i64>, max: Option<i64>, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    let min_s = RwSignal::new(fmt_opt_i64(min));
    let max_s = RwSignal::new(fmt_opt_i64(max));
    let key = path_key(&path);
    let apply = move || {
        let kind = CriterionKind::CountRange {
            min: parse_opt_i64(&min_s.get_untracked()),
            max: parse_opt_i64(&max_s.get_untracked()),
        };
        ctx.query.update(|q| set_leaf_kind(q, &path, kind));
    };
    let min_field = number_input(format!("{key}-min"), "min count", min_s, apply.clone());
    let max_field = number_input(format!("{key}-max"), "max count", max_s, apply);
    view! { <div class="flex flex-wrap items-end gap-2">{min_field}{max_field}</div> }.into_any()
}

/// `DV_PROPORTION`: numerator min/max (v1 numerator-only, per the model).
fn proportion_editor(
    min: Option<f64>,
    max: Option<f64>,
    path: Vec<usize>,
    ctx: BuilderCtx,
) -> AnyView {
    let min_s = RwSignal::new(fmt_opt_f64(min));
    let max_s = RwSignal::new(fmt_opt_f64(max));
    let key = path_key(&path);
    let apply = move || {
        let kind = CriterionKind::ProportionNumeratorRange {
            min: parse_opt_f64(&min_s.get_untracked()),
            max: parse_opt_f64(&max_s.get_untracked()),
        };
        ctx.query.update(|q| set_leaf_kind(q, &path, kind));
    };
    let min_field = number_input(format!("{key}-min"), "min numerator", min_s, apply.clone());
    let max_field = number_input(format!("{key}-max"), "max numerator", max_s, apply);
    view! { <div class="flex flex-wrap items-end gap-2">{min_field}{max_field}</div> }.into_any()
}

/// How a hand-entered code on a coded criterion was answered by the CDR's
/// terminology surface.
///
/// Display only: the model's `CriterionKind::CodedIn.codes` stays a plain
/// `Vec<String>` of bare codes whichever variant a chip carries, so lowering
/// and lifting never see a rubric.
#[derive(Clone, PartialEq, Eq)]
enum CodeChip {
    /// The CDR defined the code; the string is its `code — text` rubric.
    Defined(String),
    /// The CDR answered `404` for it — added anyway, and marked.
    Unvalidated,
    /// Added without asking the CDR (the free-text path).
    Plain,
}

/// The coded editor's shared, all-`Copy` signal bundle, threaded through its
/// sub-sections instead of a long argument list.
#[derive(Clone, Copy)]
struct CodedCtx {
    /// The criterion's codes — the model's own list, mirrored for editing.
    selected: RwSignal<Vec<String>>,
    /// The terminology id field.
    terminology: RwSignal<String>,
    /// Hand-entered codes and how the CDR answered for each; the template's
    /// own constrained options stay in their checkbox list instead.
    chips: RwSignal<Vec<(String, CodeChip)>>,
    /// The member terms of the last expanded value set.
    members: RwSignal<Vec<TermRow>>,
}

/// Write the coded criterion's current codes + terminology back into the query.
fn apply_coded(ctx: BuilderCtx, coded: CodedCtx, path: &[usize]) {
    let kind = CriterionKind::CodedIn {
        codes: coded.selected.get_untracked(),
        terminology: coded.terminology.get_untracked(),
    };
    ctx.query.update(|q| set_leaf_kind(q, path, kind));
}

/// Add `code` to the coded criterion at `path` and record how the CDR answered.
///
/// The model receives the bare code and nothing else; `chip` only decides how
/// the code READS on screen. Re-adding a known code refreshes its chip rather
/// than duplicating it.
fn add_coded_code(ctx: BuilderCtx, coded: CodedCtx, path: &[usize], code: &str, chip: CodeChip) {
    coded.selected.update(|codes| {
        if !codes.iter().any(|known| known == code) {
            codes.push(code.to_owned());
        }
    });
    coded.chips.update(
        |chips| match chips.iter_mut().find(|(known, _)| known == code) {
            Some(entry) => entry.1 = chip,
            None => chips.push((code.to_owned(), chip)),
        },
    );
    apply_coded(ctx, coded, path);
}

/// Drop a hand-entered code from the coded criterion at `path`.
fn remove_coded_code(ctx: BuilderCtx, coded: CodedCtx, path: &[usize], code: &str) {
    coded
        .selected
        .update(|codes| codes.retain(|known| known != code));
    coded
        .chips
        .update(|chips| chips.retain(|(known, _)| known != code));
    apply_coded(ctx, coded, path);
}

/// `DV_CODED_TEXT`: the template's constrained codes as checkboxes, plus a
/// terminology-backed picker — a datalist over the terminologies the CDR
/// serves, a code entry that can look its rubric up before adding, and a value
/// set whose members add with one click each.
///
/// Only the editor knows about terminology: a code added through a lookup and
/// one typed by hand are the same `Vec<String>` entry, so the model, the
/// lowering and the lifting never see the difference.
fn coded_editor(
    codes: Vec<String>,
    terminology: String,
    meta: Option<CatalogNode>,
    path: Vec<usize>,
    ctx: BuilderCtx,
) -> AnyView {
    let options = meta.map(|m| m.code_options).unwrap_or_default();
    // A code already on the criterion that the template does not offer is a
    // hand-entered one: it gets a chip so it stays visible and removable (the
    // `?load=` lift arrives this way), with no claim about the CDR having
    // defined it.
    let chips = codes
        .iter()
        .filter(|code| !options.iter().any(|opt| &&opt.code == code))
        .map(|code| (code.clone(), CodeChip::Plain))
        .collect::<Vec<_>>();
    let picker = CodedCtx {
        selected: RwSignal::new(codes),
        terminology: RwSignal::new(terminology),
        chips: RwSignal::new(chips),
        members: RwSignal::new(Vec::new()),
    };
    let key = path_key(&path);

    let boxes = coded_option_boxes(options, picker, &path, ctx);
    let terminology_field = coded_terminology_field(&key, picker, path.clone(), ctx);
    let entry = coded_code_entry(&key, picker, path.clone(), ctx);
    let value_set = coded_value_set(&key, picker, path, ctx);

    view! { <div class="space-y-2">{boxes} {terminology_field} {entry} {value_set}</div> }
        .into_any()
}

/// The template's constrained codes as a checkbox multi-pick.
fn coded_option_boxes(
    options: Vec<crate::builder::catalog::CodeOption>,
    coded: CodedCtx,
    path: &[usize],
    ctx: BuilderCtx,
) -> AnyView {
    // An inline hint, not an `EmptyState`: this is one field inside a
    // leaf-condition card, and the kit's dashed box is sized for a data region.
    if options.is_empty() {
        return view! {
            <p class="text-xs text-ink-muted italic">
                "No coded options in the template for this node."
            </p>
        }
        .into_any();
    }
    let selected = coded.selected;
    let items = options
        .into_iter()
        .map(|opt| {
            let code = opt.code.clone();
            let checked_code = opt.code.clone();
            let toggle_code = opt.code.clone();
            let path = path.to_vec();
            let text = if opt.label == opt.code {
                opt.code.clone()
            } else {
                format!("{} ({})", opt.label, opt.code)
            };
            view! {
                <label class="flex items-center gap-1 text-sm">
                    <input
                        type="checkbox"
                        class="accent-accent"
                        prop:checked=move || selected.with(|s| s.contains(&checked_code))
                        on:change:target=move |ev| {
                            let on = ev.target().checked();
                            selected
                                .update(|s| {
                                    if on {
                                        if !s.contains(&toggle_code) {
                                            s.push(toggle_code.clone());
                                        }
                                    } else {
                                        s.retain(|c| c != &toggle_code);
                                    }
                                });
                            apply_coded(ctx, coded, &path);
                        }
                    />
                    <span>{text}</span>
                    <span class="font-mono text-xs text-ink-muted">{code}</span>
                </label>
            }
        })
        .collect::<Vec<_>>();
    view! { <div class="flex flex-col gap-1">{items}</div> }.into_any()
}

/// The terminology field: a datalist-backed text input.
///
/// The options come from [`terminology_options`]; the value is free text and is
/// preserved verbatim, because an AQL author must be able to name a terminology
/// this server does not host.
fn coded_terminology_field(
    key: &str,
    coded: CodedCtx,
    path: Vec<usize>,
    ctx: BuilderCtx,
) -> AnyView {
    let id = format!("qb-coded-terminology-{key}");
    let terminology = coded.terminology;
    view! {
        <label class="flex flex-col gap-0.5 text-xs">
            <span class="text-ink-muted">"terminology"</span>
            <input
                id=id
                type="text"
                list="qb-terminology-options"
                class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm w-40"
                prop:value=move || terminology.get()
                on:input:target=move |ev| {
                    terminology.set(ev.target().value());
                    apply_coded(ctx, coded, &path);
                }
            />
        </label>
    }
    .into_any()
}

/// The code entry: type a code, then either look its rubric up in the named
/// terminology or add it as-is.
///
/// The lookup's answer is written in the action's own async continuation; a
/// `404` still adds the code, marked, because the builder must never refuse a
/// code the CDR happens not to define.
fn coded_code_entry(key: &str, coded: CodedCtx, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    let code_id = format!("qb-coded-code-{key}");
    let lookup_id = format!("qb-coded-lookup-{key}");
    let add_id = format!("qb-coded-add-{key}");
    let draft = RwSignal::new(String::new());
    let lookup = {
        let path = path.clone();
        Action::new(move |code: &String| {
            let code = code.clone();
            let terminology = coded.terminology.get_untracked();
            let path = path.clone();
            async move {
                let found = fetch_term(terminology, code.clone(), String::new()).await?;
                let chip = found
                    .as_ref()
                    .and_then(|extract| extract.terms.iter().find(|term| term.code == code))
                    .map_or(CodeChip::Unvalidated, |term| {
                        CodeChip::Defined(term.rubric())
                    });
                add_coded_code(ctx, coded, &path, &code, chip);
                Ok::<(), ViewerError>(())
            }
        })
    };
    let add_path = path.clone();
    let on_add = move |_| {
        let code = draft.get_untracked().trim().to_owned();
        if !code.is_empty() {
            add_coded_code(ctx, coded, &add_path, &code, CodeChip::Plain);
            draft.set(String::new());
        }
    };
    let on_lookup = move |_| {
        let code = draft.get_untracked().trim().to_owned();
        if !code.is_empty() {
            lookup.dispatch(code);
            draft.set(String::new());
        }
    };
    let chip_list = coded_chip_list(coded, path, ctx);
    view! {
        <div class="space-y-1">
            <div class="flex flex-wrap items-end gap-2">
                <label class="flex flex-col gap-0.5 text-xs">
                    <span class="text-ink-muted">"code"</span>
                    <input
                        id=code_id
                        type="text"
                        class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm w-40"
                        prop:value=move || draft.get()
                        on:input:target=move |ev| draft.set(ev.target().value())
                    />
                </label>
                <button
                    id=lookup_id
                    type="button"
                    class="text-xs rounded border border-accent text-accent px-1.5 py-1 hover:bg-accent-subtle"
                    disabled=Signal::derive(move || lookup.pending().get())
                    on:click=on_lookup
                >
                    "look up"
                </button>
                <button
                    id=add_id
                    type="button"
                    class="text-xs rounded border border-edge-strong px-1.5 py-1 hover:bg-sunken"
                    on:click=on_add
                >
                    "add"
                </button>
            </div>
            {move || match lookup.value().get() {
                Some(Err(error)) => crate::components::notice::inline_error(&error),
                Some(Ok(())) | None => ().into_any(),
            }}
            {chip_list}
        </div>
    }
    .into_any()
}

/// The hand-entered codes as removable chips, each reading the way the CDR
/// answered for it.
fn coded_chip_list(coded: CodedCtx, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    view! {
        <div class="flex flex-wrap gap-1">
            <For each=move || coded.chips.get() key=|(code, _)| code.clone() let:entry>
                {
                    let (code, chip) = entry;
                    let remove_code = code.clone();
                    let hook = code.clone();
                    let path = path.clone();
                    let (label, marker) = match chip {
                        CodeChip::Defined(rubric) => (rubric, ""),
                        CodeChip::Unvalidated => (code, "unvalidated"),
                        CodeChip::Plain => (code, ""),
                    };
                    view! {
                        <span
                            data-coded-chip=hook
                            class="inline-flex items-center gap-1 rounded-full border border-edge-strong bg-sunken px-2 py-0.5 text-xs text-ink"
                        >
                            <span class="font-mono">{label}</span>
                            <span class="text-ink-faint italic">{marker}</span>
                            <button
                                type="button"
                                class="text-danger"
                                aria-label=format!("Remove code {remove_code}")
                                on:click=move |_| remove_coded_code(ctx, coded, &path, &remove_code)
                            >
                                <leptos_icons::Icon icon=icondata_lu::LuX width="10" height="10" />
                            </button>
                        </span>
                    }
                }
            </For>
        </div>
    }
    .into_any()
}

/// The value-set picker: expand a value set in the named terminology and add
/// any of its members with one click.
fn coded_value_set(key: &str, coded: CodedCtx, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    let value_set_id = format!("qb-coded-value-set-{key}");
    let expand_id = format!("qb-coded-expand-{key}");
    let draft = RwSignal::new(String::new());
    let expand = Action::new(move |value_set: &String| {
        let value_set = value_set.clone();
        let terminology = coded.terminology.get_untracked();
        async move {
            let found = fetch_value_set(terminology, value_set).await?;
            let known = found.is_some();
            coded
                .members
                .set(found.map(|extract| extract.terms).unwrap_or_default());
            Ok::<bool, ViewerError>(known)
        }
    });
    let on_expand = move |_| {
        let value_set = draft.get_untracked().trim().to_owned();
        if !value_set.is_empty() {
            expand.dispatch(value_set);
        }
    };
    let outcome = move || match expand.value().get() {
        None => ().into_any(),
        Some(Err(error)) => crate::components::notice::inline_error(&error),
        Some(Ok(false)) => {
            let value_set = expand.input().get().unwrap_or_default();
            view! {
                <p class="text-xs text-ink-muted italic">
                    "No value set " <span class="font-mono">{value_set}</span>
                    " in this terminology."
                </p>
            }
            .into_any()
        }
        Some(Ok(true)) => coded_member_chips(coded, path.clone(), ctx),
    };
    view! {
        <div class="space-y-1">
            <div class="flex flex-wrap items-end gap-2">
                <label class="flex flex-col gap-0.5 text-xs">
                    <span class="text-ink-muted">"value set (optional)"</span>
                    <input
                        id=value_set_id
                        type="text"
                        class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm w-48"
                        prop:value=move || draft.get()
                        on:input:target=move |ev| draft.set(ev.target().value())
                    />
                </label>
                <button
                    id=expand_id
                    type="button"
                    class="text-xs rounded border border-edge-strong px-1.5 py-1 hover:bg-sunken"
                    disabled=Signal::derive(move || expand.pending().get())
                    on:click=on_expand
                >
                    "expand"
                </button>
            </div>
            {outcome}
        </div>
    }
    .into_any()
}

/// The expanded value set's members as clickable chips; a click adds that
/// member to the criterion as a defined code.
fn coded_member_chips(coded: CodedCtx, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    view! {
        <div class="flex flex-wrap gap-1">
            <For each=move || coded.members.get() key=|term| term.code.clone() let:term>
                {
                    let hook = term.code.clone();
                    let code = term.code.clone();
                    let label = term.rubric();
                    let path = path.clone();
                    view! {
                        <button
                            type="button"
                            data-value-set-code=hook
                            class="rounded-full border border-accent px-2 py-0.5 text-xs text-accent hover:bg-accent-subtle"
                            on:click=move |_| add_coded_code(
                                ctx,
                                coded,
                                &path,
                                &code,
                                CodeChip::Defined(label.clone()),
                            )
                        >
                            {term.rubric()}
                        </button>
                    }
                }
            </For>
        </div>
    }
    .into_any()
}

/// `DV_ORDINAL`: a checkbox multi-pick of the catalog's ordinal steps, keyed by
/// their ordinal integer.
fn ordinal_editor(
    values: Vec<i64>,
    meta: Option<CatalogNode>,
    path: Vec<usize>,
    ctx: BuilderCtx,
) -> AnyView {
    let selected = RwSignal::new(values);
    let apply = move || {
        let kind = CriterionKind::OrdinalIn {
            values: selected.get_untracked(),
        };
        ctx.query.update(|q| set_leaf_kind(q, &path, kind));
    };
    let options = meta
        .map(|m| m.code_options)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|o| o.ordinal.map(|ord| (i64::from(ord), o.label, o.code)))
        .collect::<Vec<_>>();
    if options.is_empty() {
        // An inline hint for the same reason as the coded editor's: a single
        // field inside a leaf card, not a data region.
        return view! {
            <p class="text-xs text-ink-muted italic">
                "No ordinal steps in the template for this node."
            </p>
        }
        .into_any();
    }
    let items = options
        .into_iter()
        .map(|(ord, label, code)| {
            let apply = apply.clone();
            let text = if label == code || label.is_empty() {
                format!("{ord}")
            } else {
                format!("{ord} · {label}")
            };
            view! {
                <label class="flex items-center gap-1 text-sm">
                    <input
                        type="checkbox"
                        class="accent-accent"
                        prop:checked=move || selected.with(|s| s.contains(&ord))
                        on:change:target=move |ev| {
                            let on = ev.target().checked();
                            selected
                                .update(|s| {
                                    if on {
                                        if !s.contains(&ord) {
                                            s.push(ord);
                                        }
                                    } else {
                                        s.retain(|v| *v != ord);
                                    }
                                });
                            apply();
                        }
                    />
                    <span>{text}</span>
                </label>
            }
        })
        .collect::<Vec<_>>();
    view! { <div class="flex flex-col gap-1">{items}</div> }.into_any()
}

/// `DV_TEXT`: an equals/contains mode radio plus the text. `contains` lowers to
/// a `*text*` LIKE pattern; `equals` to an exact match.
fn text_editor(is_contains: bool, text: String, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    let contains = RwSignal::new(is_contains);
    let text_s = RwSignal::new(text);
    let key = path_key(&path);
    let name = format!("text-mode-{key}");
    let apply = move || {
        let value = text_s.get_untracked();
        let kind = if contains.get_untracked() {
            CriterionKind::TextLike {
                pattern: format!("*{value}*"),
            }
        } else {
            CriterionKind::TextEquals { text: value }
        };
        ctx.query.update(|q| set_leaf_kind(q, &path, kind));
    };
    let apply_eq = apply.clone();
    let apply_ct = apply.clone();
    view! {
        <div class="flex flex-wrap items-center gap-3">
            <label class="flex items-center gap-1 text-sm">
                <input
                    type="radio"
                    class="accent-accent"
                    name=name.clone()
                    prop:checked=move || !contains.get()
                    on:change:target=move |_| {
                        contains.set(false);
                        apply_eq();
                    }
                />
                "equals"
            </label>
            <label class="flex items-center gap-1 text-sm">
                <input
                    type="radio"
                    class="accent-accent"
                    name=name
                    prop:checked=move || contains.get()
                    on:change:target=move |_| {
                        contains.set(true);
                        apply_ct();
                    }
                />
                "contains"
            </label>
            <input
                type="text"
                class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm flex-1 min-w-40"
                prop:value=move || text_s.get()
                on:input:target=move |ev| {
                    text_s.set(ev.target().value());
                    apply();
                }
            />
        </div>
    }
    .into_any()
}

/// `DV_DATE_TIME` / `DV_DATE` / `DV_TIME`: ISO-8601 from/to text bounds.
fn datetime_editor(from: String, to: String, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    let from_s = RwSignal::new(from);
    let to_s = RwSignal::new(to);
    let apply = move || {
        let kind = CriterionKind::DateTimeRange {
            from: from_s.get_untracked(),
            to: to_s.get_untracked(),
        };
        ctx.query.update(|q| set_leaf_kind(q, &path, kind));
    };
    let apply_from = apply.clone();
    let apply_to = apply;
    view! {
        <div class="flex flex-wrap items-end gap-2">
            <label class="flex flex-col gap-0.5 text-xs">
                <span class="text-ink-muted">"from"</span>
                <input
                    type="text"
                    placeholder="2026-01-01T00:00:00Z"
                    class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm w-56"
                    prop:value=move || from_s.get()
                    on:input:target=move |ev| {
                        from_s.set(ev.target().value());
                        apply_from();
                    }
                />
            </label>
            <label class="flex flex-col gap-0.5 text-xs">
                <span class="text-ink-muted">"to"</span>
                <input
                    type="text"
                    placeholder="2026-12-31T23:59:59Z"
                    class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm w-56"
                    prop:value=move || to_s.get()
                    on:input:target=move |ev| {
                        to_s.set(ev.target().value());
                        apply_to();
                    }
                />
            </label>
        </div>
    }
    .into_any()
}

/// `DV_BOOLEAN`: a true/false radio.
fn boolean_editor(value: bool, path: Vec<usize>, ctx: BuilderCtx) -> AnyView {
    let val = RwSignal::new(value);
    let key = path_key(&path);
    let name = format!("bool-{key}");
    let path_true = path.clone();
    let path_false = path;
    view! {
        <div class="flex items-center gap-3 text-sm">
            <label class="flex items-center gap-1">
                <input
                    type="radio"
                    class="accent-accent"
                    name=name.clone()
                    prop:checked=move || val.get()
                    on:change:target=move |_| {
                        val.set(true);
                        ctx.query
                            .update(|q| set_leaf_kind(
                                q,
                                &path_true,
                                CriterionKind::BooleanIs {
                                    value: true,
                                },
                            ));
                    }
                />
                "true"
            </label>
            <label class="flex items-center gap-1">
                <input
                    type="radio"
                    class="accent-accent"
                    name=name
                    prop:checked=move || !val.get()
                    on:change:target=move |_| {
                        val.set(false);
                        ctx.query
                            .update(|q| set_leaf_kind(
                                q,
                                &path_false,
                                CriterionKind::BooleanIs {
                                    value: false,
                                },
                            ));
                    }
                />
                "false"
            </label>
        </div>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// Output: shape, columns, order, limit
// ---------------------------------------------------------------------------

/// The output section: the result shape radio and, per shape, the projection
/// columns; then the order-by rules and the limit. Re-renders on `struct_ver`.
fn output_section(ctx: BuilderCtx) -> AnyView {
    view! {
        <div>
            <h2 class=CARD_TITLE>"Output"</h2>
            {move || {
                ctx.struct_ver.get();
                let snapshot = ctx.query.get_untracked();
                let shape = shape_radio(ctx, snapshot.shape);
                let columns = if snapshot.shape == QueryShape::DataValues {
                    columns_editor(ctx, &snapshot.columns)
                } else {
                    ().into_any()
                };
                let order = order_editor(ctx, &snapshot.order_by);
                let limit = limit_editor(ctx, snapshot.limit);
                view! { <div class="space-y-3">{shape}{columns}{order}{limit}</div> }.into_any()
            }}
        </div>
    }
    .into_any()
}

/// The result-shape radio (compositions / data values / count).
fn shape_radio(ctx: BuilderCtx, current: QueryShape) -> AnyView {
    let option = |shape: QueryShape, label: &'static str| {
        let checked = current == shape;
        view! {
            <label class="flex items-center gap-1 text-sm">
                <input
                    type="radio"
                    class="accent-accent"
                    name="qb-shape"
                    prop:checked=checked
                    on:change:target=move |_| {
                        ctx.query.update(|q| q.shape = shape);
                        ctx.bump();
                    }
                />
                {label}
            </label>
        }
    };
    view! {
        <div class="flex flex-wrap items-center gap-4">
            {option(QueryShape::Compositions, "Compositions")}
            {option(QueryShape::DataValues, "Data values")} {option(QueryShape::Count, "Count")}
            {option(QueryShape::Ehrs, "EHRs (cohort)")}
        </div>
    }
    .into_any()
}

/// The projection-columns editor (data-value shape): one row per column with an
/// alias input and a remove button; empty invites adding from the catalog.
fn columns_editor(ctx: BuilderCtx, columns: &[SelectedColumn]) -> AnyView {
    if columns.is_empty() {
        // An inline hint, not an `EmptyState`: the columns editor is one row of
        // the options strip beside "Order by" and "Limit".
        return view! {
            <p class="text-xs text-ink-muted">
                "Add projection columns with \"+ column\" in the path catalog."
            </p>
        }
        .into_any();
    }
    let rows = columns
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let alias = RwSignal::new(col.alias.clone());
            let path_text = col.aql_path.clone();
            // The row's controls have no visible label (the path text beside
            // them is the heading), so each names itself by its position.
            let position = i.saturating_add(1);
            let alias_label = format!("Alias for column {position}");
            let remove_label = format!("Remove column {position}");
            view! {
                <div class="flex items-center gap-2">
                    <span
                        class="font-mono text-xs text-ink-muted truncate max-w-xs"
                        title=col.aql_path.clone()
                    >
                        {path_text}
                    </span>
                    <input
                        id=format!("qb-col-alias-{i}")
                        type="text"
                        placeholder="alias"
                        aria-label=alias_label
                        class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm w-40"
                        prop:value=move || alias.get()
                        on:input:target=move |ev| {
                            let value = ev.target().value();
                            alias.set(value.clone());
                            ctx.query
                                .update(|q| {
                                    if let Some(c) = q.columns.get_mut(i) {
                                        value.clone_into(&mut c.alias);
                                    }
                                });
                        }
                    />
                    <button
                        type="button"
                        class="text-xs rounded border border-danger/40 text-danger px-1.5 hover:bg-danger-subtle"
                        aria-label=remove_label
                        on:click=move |_| {
                            ctx.query
                                .update(|q| {
                                    if i < q.columns.len() {
                                        q.columns.remove(i);
                                    }
                                });
                            ctx.bump();
                        }
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuX width="12" height="12" />
                    </button>
                </div>
            }
        })
        .collect::<Vec<_>>();
    view! {
        <div class="space-y-1">
            <div class="text-xs font-medium text-ink-muted">"Columns"</div>
            {rows}
        </div>
    }
    .into_any()
}

/// One order-by row: a path text input, a direction select, and a remove
/// button — all addressed by the rule's position `i`.
fn order_row(ctx: BuilderCtx, i: usize, rule: &OrderRule) -> AnyView {
    let path_s = RwSignal::new(rule.aql_path.clone());
    let desc = rule.descending;
    // "Order by" labels the whole editor, not the individual rows, so every
    // control in a row names itself by the rule's position.
    let position = i.saturating_add(1);
    let path_label = format!("Sort path for rule {position}");
    let direction_label = format!("Sort direction for rule {position}");
    let remove_label = format!("Remove sort rule {position}");
    view! {
        <div class="flex items-center gap-2">
            <input
                id=format!("qb-order-path-{i}")
                type="text"
                placeholder="context/start_time/value"
                aria-label=path_label
                class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm flex-1 min-w-48 font-mono"
                prop:value=move || path_s.get()
                on:input:target=move |ev| {
                    let value = ev.target().value();
                    path_s.set(value.clone());
                    ctx.query
                        .update(|q| {
                            if let Some(r) = q.order_by.get_mut(i) {
                                value.clone_into(&mut r.aql_path);
                            }
                        });
                }
            />
            <select
                class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm"
                aria-label=direction_label
                prop:value=move || if desc { "desc" } else { "asc" }
                on:change:target=move |ev| {
                    let descending = ev.target().value() == "desc";
                    ctx.query
                        .update(|q| {
                            if let Some(r) = q.order_by.get_mut(i) {
                                r.descending = descending;
                            }
                        });
                    ctx.bump();
                }
            >
                <option value="asc">"asc"</option>
                <option value="desc">"desc"</option>
            </select>
            <button
                type="button"
                class="text-xs rounded border border-danger/40 text-danger px-1.5 hover:bg-danger-subtle"
                aria-label=remove_label
                on:click=move |_| {
                    ctx.query
                        .update(|q| {
                            if i < q.order_by.len() {
                                q.order_by.remove(i);
                            }
                        });
                    ctx.bump();
                }
            >
                <leptos_icons::Icon icon=icondata_lu::LuX width="12" height="12" />
            </button>
        </div>
    }
    .into_any()
}

/// The order-by editor: an add button plus one row per rule (path text, a
/// direction select, remove).
fn order_editor(ctx: BuilderCtx, rules: &[OrderRule]) -> AnyView {
    let rows = rules
        .iter()
        .enumerate()
        .map(|(i, rule)| order_row(ctx, i, rule))
        .collect::<Vec<_>>();
    view! {
        <div class="space-y-1">
            <div class="flex items-center gap-2">
                <span class="text-xs font-medium text-ink-muted">"Order by"</span>
                <button
                    type="button"
                    class="text-xs rounded border border-edge-strong px-1.5 hover:bg-sunken"
                    on:click=move |_| {
                        ctx.query
                            .update(|q| {
                                q.order_by
                                    .push(OrderRule {
                                        aql_path: String::new(),
                                        descending: false,
                                    });
                            });
                        ctx.bump();
                    }
                >
                    "+ add"
                </button>
            </div>
            {rows}
        </div>
    }
    .into_any()
}

/// The limit editor: a numeric fetch size; empty clears the limit.
fn limit_editor(ctx: BuilderCtx, limit: Option<u32>) -> AnyView {
    let limit_s = RwSignal::new(limit.map(|n| n.to_string()).unwrap_or_default());
    view! {
        <label class="flex items-center gap-2 text-xs">
            <span class="text-ink-muted">"Limit"</span>
            <input
                id="qb-limit"
                type="number"
                min="1"
                class="rounded border border-edge-strong bg-raised px-2 py-1 text-sm w-28"
                prop:value=move || limit_s.get()
                on:input:target=move |ev| {
                    let raw = ev.target().value();
                    limit_s.set(raw.clone());
                    let parsed = raw.trim().parse::<u32>().ok();
                    ctx.query.update(|q| q.limit = parsed);
                }
            />
        </label>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// Preview + run + save
// ---------------------------------------------------------------------------

/// One store request as both save screens assemble it: the qualified name, the
/// optional explicit version (`None` = let the server assign it), and the AQL.
/// Mirrors [`store_query`]'s parameters so the action is a thin pass-through.
pub(crate) type SaveInput = (String, Option<String>, String);

/// The save action both query screens drive.
pub(crate) type SaveAction = Action<SaveInput, Result<(), ViewerError>>;

/// The three bound save fields, passed as one bundle so the version cannot be
/// forgotten at a call site.
#[derive(Clone, Copy)]
pub(crate) struct SaveFields {
    /// The optional namespace half of the qualified name.
    pub namespace: RwSignal<String>,
    /// The bare query name.
    pub name: RwSignal<String>,
    /// The optional store version, empty for the server-assigned one.
    pub version: RwSignal<String>,
}

impl SaveFields {
    /// The store version to send: `None` for the unversioned store, `Some` for
    /// an explicit immutable version. Reads untracked — it is called from a
    /// click handler, never from a render.
    pub(crate) fn version_arg(self) -> Option<String> {
        let version = self.version.get_untracked().trim().to_owned();
        (!version.is_empty()).then_some(version)
    }

    /// Is the version field filled in with something that is NOT a storable
    /// `major.minor.patch` triple? Blocks the save before a request is made, so
    /// a prefix pattern never reaches the CDR as a version to file under.
    pub(crate) fn version_is_unstorable(self) -> bool {
        self.version.with(|version| {
            let version = version.trim();
            !version.is_empty() && !is_full_semver(version)
        })
    }
}

/// The live AQL preview and the run/save surface. The preview reads the whole
/// query through [`to_aql`]; on `Ok` it shows the AQL and enables Run/Save, on
/// `Err` it shows the [`BuilderError`] inline and disables them.
fn preview_run_section(
    preview: Memo<Result<String, BuilderError>>,
    ran: RwSignal<Option<String>>,
    offset: RwSignal<u32>,
    fields: SaveFields,
    save_action: SaveAction,
) -> AnyView {
    let disabled = Signal::derive(move || preview.with(Result::is_err));
    let save_disabled = Signal::derive(move || {
        preview.with(Result::is_err)
            || fields.name.with(String::is_empty)
            || fields.version_is_unstorable()
    });
    let run_click = move |_| {
        if let Ok(aql) = preview.get_untracked() {
            ran.set(Some(aql));
            offset.set(0);
        }
    };
    let save_click = move |_| {
        if let Ok(aql) = preview.get_untracked() {
            save_action.dispatch((
                qualify(
                    &fields.namespace.get_untracked(),
                    &fields.name.get_untracked(),
                ),
                fields.version_arg(),
                aql,
            ));
        }
    };
    let save_fields = save_as_fields("qb", fields);

    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"AQL preview"</h2>
            <div class="space-y-3">
                {move || match preview.get() {
                    Ok(aql) => {
                        let href = crate::pages::query_aql::aql_href(&aql);
                        view! {
                            <div class="space-y-2">
                                <pre class=format!(
                                    "{WELL} overflow-auto font-mono text-xs whitespace-pre-wrap text-ink",
                                )>{aql}</pre>
                                <A href=href attr:class="text-sm text-accent hover:underline">
                                    "Open in raw editor "
                                    <leptos_icons::Icon
                                        icon=icondata_lu::LuArrowRight
                                        width="12"
                                        height="12"
                                    />
                                </A>
                            </div>
                        }
                            .into_any()
                    }
                    Err(error) => {
                        view! {
                            <div
                                role="status"
                                class="rounded-control border border-warn/40 bg-warn-subtle px-3 py-2 text-sm text-warn"
                            >
                                {error.to_string()}
                            </div>
                        }
                            .into_any()
                    }
                }} <div class="flex flex-wrap items-end gap-3">
                    <button type="button" class=BTN_PRIMARY disabled=disabled on:click=run_click>
                        "Run"
                    </button>
                    {save_fields}
                    <button
                        type="button"
                        class=BTN_PRIMARY
                        disabled=save_disabled
                        on:click=save_click
                    >
                        "Save"
                    </button>
                </div> {save_feedback(save_action)}
            </div>
        </section>
    }
    .into_any()
}

/// The save action's inline feedback: a pending hint and the CDR error verbatim.
/// Success is reported as a toast (dispatched from the page component), so it
/// renders nothing here.
fn save_feedback(save_action: SaveAction) -> AnyView {
    view! {
        <div class="text-sm">
            <Show when=move || save_action.pending().get()>
                <span class="text-ink-muted">"Saving…"</span>
            </Show>
            {move || match save_action.value().get() {
                Some(Err(error)) => crate::components::notice::inline_error(&error),
                Some(Ok(())) | None => ().into_any(),
            }}
        </div>
    }
    .into_any()
}

/// The shared **Save as** fields both query screens use: the optional
/// **namespace** beside the query **name**, plus the effective qualified name
/// the save will write.
///
/// A stored query's identifier is `[{namespace}::]{query-name}`, and the
/// namespace exists to separate stored queries "by teams, companies, etc."
/// (ITS-REST `specifications/docs/query/Qualified_query_name.md` §Qualified
/// query name). A `namespace::` prefix typed into the NAME field still works —
/// [`qualify`] lets it win — and the "Saves as" line always shows the exact
/// name that will be written.
///
/// The **version** field decides which store operation runs: filled in, the
/// save targets `PUT definition/query/{name}/{version}`, an immutable
/// `(name, version)` pair (`409` if it exists); left empty, it targets
/// `PUT definition/query/{name}`, where the server assigns the version and
/// replaces what is stored at it (ITS-REST
/// `operations/definition_query_version_store.yaml` /
/// `operations/definition_query_store.yaml`).
///
/// `id_prefix` scopes the three field ids (`{id_prefix}-save-namespace` /
/// `-save-name` / `-save-version`).
pub(crate) fn save_as_fields(id_prefix: &str, fields: SaveFields) -> AnyView {
    let SaveFields {
        namespace,
        name,
        version,
    } = fields;
    let namespace_id = format!("{id_prefix}-save-namespace");
    let name_id = format!("{id_prefix}-save-name");
    let version_id = format!("{id_prefix}-save-version");
    // The name that will actually be stored, shown only once there is one.
    let qualified = Signal::derive(move || {
        let composed = qualify(&namespace.get(), &name.get());
        (!composed.is_empty()).then_some(composed)
    });
    // What the version field currently means for the save: an explicit
    // immutable version, the server-assigned one, or a pattern that cannot be
    // stored at (blocked before any request — `SaveFields::version_is_unstorable`).
    let version_note = Signal::derive(move || {
        version.with(|version| {
            let version = version.trim();
            if version.is_empty() {
                Ok("the server assigns the version, replacing the query stored at it".to_owned())
            } else if is_full_semver(version) {
                Ok(format!("a new immutable version {version}"))
            } else {
                Err(format!(
                    "{version} is not a version to store at — use major.minor.patch, \
                     for example 1.0.0"
                ))
            }
        })
    });
    let field = "rounded-control border border-edge-strong bg-raised px-2 py-1 text-sm text-ink placeholder:text-ink-faint focus:outline-none focus:ring-2 focus:ring-accent";
    view! {
        <div class="flex flex-col gap-1">
            <div class="flex items-end gap-2">
                <label class="flex flex-col gap-0.5 text-xs">
                    <span class="text-ink-muted">"Namespace (optional)"</span>
                    <input
                        id=namespace_id
                        type="text"
                        placeholder="org.example"
                        class=format!("{field} w-40")
                        prop:value=move || namespace.get()
                        on:input:target=move |ev| namespace.set(ev.target().value())
                    />
                </label>
                <label class="flex flex-col gap-0.5 text-xs">
                    <span class="text-ink-muted">"Query name"</span>
                    <input
                        id=name_id
                        type="text"
                        placeholder="my_query"
                        class=format!("{field} w-56")
                        prop:value=move || name.get()
                        on:input:target=move |ev| name.set(ev.target().value())
                    />
                </label>
                <label class="flex flex-col gap-0.5 text-xs">
                    <span class="text-ink-muted">"Version (optional)"</span>
                    <input
                        id=version_id
                        type="text"
                        placeholder="1.0.0"
                        class=format!("{field} w-28")
                        prop:value=move || version.get()
                        on:input:target=move |ev| version.set(ev.target().value())
                    />
                </label>
            </div>
            {move || {
                qualified
                    .get()
                    .map(|composed| {
                        let note = version_note.get();
                        let (note_class, note_text) = match note {
                            Ok(text) => ("text-ink-muted", text),
                            Err(text) => ("text-danger", text),
                        };
                        view! {
                            <span class="text-xs text-ink-muted">
                                "Saves as " <span class="font-mono text-ink">{composed}</span>
                                " — " <span class=note_class>{note_text}</span>
                            </span>
                        }
                    })
            }}
        </div>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// The run result: a paged table (or a single big number for a count query)
/// under a `<Transition>` so paging keeps the prior page visible.
fn results_section(
    ctx: BuilderCtx,
    results: Resource<Result<Option<ResultPage>, ViewerError>>,
    offset: RwSignal<u32>,
    current_aql: Signal<String>,
    params: Signal<String>,
) -> AnyView {
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match results.await {
                    Ok(None) => ().into_any(),
                    Ok(Some(page)) => {
                        let is_count = ctx.query.with_untracked(|q| q.shape == QueryShape::Count);
                        let controls = paging_buttons(offset, page.rows.len());
                        let body = results_view(&page, is_count);
                        let export = export_forms(current_aql, params);
                        // Resolve inside the Transition: an SSR'd ErrorBoundary fallback
                        // mismatches at hydration in leptos 0.8.
                        view! {
                            <section class=CARD_PAD>
                                <div class="flex items-center justify-between gap-2 flex-wrap mb-3">
                                    <h2 class="text-sm font-semibold text-ink">"Results"</h2>
                                    {export}
                                </div>
                                {body}
                                {controls}
                            </section>
                        }
                            .into_any()
                    }
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// Result-export forms: two plain form-POSTs to the BFF `/export/aql` route,
/// native submissions the client router does not intercept. The pane hosting
/// them renders only after a hydrated Run click, and the hidden inputs track
/// the screen's query/parameters signals via `prop:value`, so nothing lands in
/// SSR markup. The route exports the query's own `LIMIT` window, or the CDR's
/// default fetch limit. Shared with the raw editor.
pub(crate) fn export_forms(current_aql: Signal<String>, params: Signal<String>) -> AnyView {
    view! {
        <div class="flex flex-wrap items-center gap-2">
            <form method="post" action="/export/aql" class="inline">
                <input type="hidden" name="q" prop:value=move || current_aql.get() />
                <input type="hidden" name="parameters_json" prop:value=move || params.get() />
                <input type="hidden" name="format" value="csv" />
                <button type="submit" class=BTN_SECONDARY>
                    "Export CSV"
                </button>
            </form>
            <form method="post" action="/export/aql" class="inline">
                <input type="hidden" name="q" prop:value=move || current_aql.get() />
                <input type="hidden" name="parameters_json" prop:value=move || params.get() />
                <input type="hidden" name="format" value="json" />
                <button type="submit" class=BTN_SECONDARY>
                    "Export JSON"
                </button>
            </form>
            <span class="text-xs text-ink-muted">
                "Exports the query's own LIMIT window, or the server default."
            </span>
        </div>
    }
    .into_any()
}

/// Render one page of an AQL `RESULT_SET`: a big single stat for a count query,
/// the empty state, or the table | chart pair. Shared with the raw AQL editor
/// screen.
pub(crate) fn results_view(page: &ResultPage, is_count: bool) -> AnyView {
    if is_count {
        let n = page
            .rows
            .first()
            .and_then(|r| r.first())
            .map(cell_text)
            .unwrap_or_default();
        return view! {
            <div class="py-6 text-center">
                <div class="text-4xl font-semibold tabular-nums text-ink">{n}</div>
                <div class="text-xs text-ink-muted mt-1">"matching rows"</div>
            </div>
        }
        .into_any();
    }
    if page.rows.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuSearchX
                message="No rows"
                hint="The query ran and matched nothing — widen a condition or clear a filter."
            />
        }
        .into_any();
    }
    // The result-set column aliases/paths are the table headers (never `#n`).
    let header_refs: Vec<&str> = page.columns.iter().map(String::as_str).collect();
    let rows = page
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let key = format!(
                "{i}\u{1}{}",
                row.iter().map(cell_text).collect::<Vec<_>>().join("|")
            );
            (key, row.clone())
        })
        .collect::<Vec<_>>();
    let body = view! {
        <For each=move || rows.clone() key=|(k, _)| k.clone() let:entry>
            {result_row(&entry.1)}
        </For>
    }
    .into_any();
    let table = table_shell(&header_refs, body);

    // Every non-empty page offers the table | chart pair: the chart derives one
    // series per numeric column, and a page with nothing chartable says so.
    let show_chart = RwSignal::new(false);
    let chart = crate::components::results_chart::results_chart(&page.columns, &page.rows);
    view! {
        <div>
            <div class="mb-2 inline-flex overflow-hidden rounded-control border border-edge-strong">
                <button
                    type="button"
                    class=move || {
                        if show_chart.get() {
                            "px-3 py-1 text-xs font-medium text-ink-muted hover:bg-sunken"
                        } else {
                            "px-3 py-1 text-xs font-medium bg-accent text-on-accent"
                        }
                    }
                    on:click=move |_| show_chart.set(false)
                >
                    "Table"
                </button>
                <button
                    type="button"
                    class=move || {
                        if show_chart.get() {
                            "px-3 py-1 text-xs font-medium bg-accent text-on-accent"
                        } else {
                            "px-3 py-1 text-xs font-medium text-ink-muted hover:bg-sunken"
                        }
                    }
                    on:click=move |_| show_chart.set(true)
                >
                    "Chart"
                </button>
            </div>
            <div class:hidden=move || show_chart.get()>{table}</div>
            <div class:hidden=move || !show_chart.get()>{chart}</div>
        </div>
    }
    .into_any()
}

/// One generic result row (all cells rendered as plain text).
fn result_row(row: &[serde_json::Value]) -> AnyView {
    let cells = row
        .iter()
        .map(|value| {
            let text = cell_text(value);
            view! { <td class=CELL>{text}</td> }
        })
        .collect::<Vec<_>>();
    view! { <tr class=ROW>{cells}</tr> }.into_any()
}

/// Prev/next paging buttons wired to a local `offset` signal (page window is
/// [`PAGE_SIZE`]). Prev is disabled at the first page; next when the page is
/// not full. Offsets use saturating arithmetic.
pub(crate) fn paging_buttons(offset: RwSignal<u32>, row_count: usize) -> AnyView {
    let full = u32::try_from(row_count).unwrap_or(u32::MAX) >= PAGE_SIZE;
    let prev_disabled = Signal::derive(move || offset.get() == 0);
    let next_disabled = Signal::derive(move || !full);
    view! {
        <div class="mt-3 flex gap-2">
            <button
                type="button"
                class=BTN_SECONDARY
                disabled=prev_disabled
                on:click=move |_| offset.update(|o| *o = o.saturating_sub(PAGE_SIZE))
            >
                <leptos_icons::Icon icon=icondata_lu::LuArrowLeft width="12" height="12" />
                " Previous"
            </button>
            <button
                type="button"
                class=BTN_SECONDARY
                disabled=next_disabled
                on:click=move |_| offset.update(|o| *o = o.saturating_add(PAGE_SIZE))
            >
                "Next "
                <leptos_icons::Icon icon=icondata_lu::LuArrowRight width="12" height="12" />
            </button>
        </div>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// Pick the initial typed constraint for a freshly-added criterion from its RM
/// type. Unknown types get a presence check (`Exists`) rather than a silent
/// drop.
fn default_kind_for(rm_type: &str) -> CriterionKind {
    match rm_type {
        "DV_QUANTITY" => CriterionKind::QuantityRange {
            min: None,
            max: None,
            units: String::new(),
        },
        "DV_COUNT" => CriterionKind::CountRange {
            min: None,
            max: None,
        },
        "DV_PROPORTION" => CriterionKind::ProportionNumeratorRange {
            min: None,
            max: None,
        },
        "DV_CODED_TEXT" => CriterionKind::CodedIn {
            codes: Vec::new(),
            terminology: "local".to_owned(),
        },
        "DV_ORDINAL" => CriterionKind::OrdinalIn { values: Vec::new() },
        "DV_TEXT" => CriterionKind::TextEquals {
            text: String::new(),
        },
        "DV_DATE_TIME" | "DV_DATE" | "DV_TIME" => CriterionKind::DateTimeRange {
            from: String::new(),
            to: String::new(),
        },
        "DV_BOOLEAN" => CriterionKind::BooleanIs { value: true },
        _ => CriterionKind::Exists,
    }
}

/// Human-readable one-line summary of a leaf condition, used on the card.
fn criterion_sentence(criterion: &Criterion, meta: Option<&CatalogNode>) -> String {
    let label = meta
        .map(|m| m.label.clone())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| last_segment(&criterion.aql_path));
    let body = match &criterion.kind {
        CriterionKind::QuantityRange { min, max, units } => {
            let mut s = format!(
                "{label} {}",
                range_phrase("magnitude", num_opt(*min), num_opt(*max))
            );
            if !units.is_empty() {
                s.push(' ');
                s.push_str(units);
            }
            s
        }
        CriterionKind::CountRange { min, max } => {
            format!(
                "{label} {}",
                range_phrase("count", int_opt(*min), int_opt(*max))
            )
        }
        CriterionKind::ProportionNumeratorRange { min, max } => format!(
            "{label} {}",
            range_phrase("numerator", num_opt(*min), num_opt(*max))
        ),
        CriterionKind::CodedIn { codes, .. } => {
            let names = code_labels(codes, meta);
            if names.is_empty() {
                format!("{label} is a coded value")
            } else {
                format!("{label} is one of {names}")
            }
        }
        CriterionKind::OrdinalIn { values } => {
            let names = ordinal_labels(values, meta);
            if names.is_empty() {
                format!("{label} is a graded value")
            } else {
                format!("{label} is one of {names}")
            }
        }
        CriterionKind::TextEquals { text } => format!("{label} equals \"{text}\""),
        CriterionKind::TextLike { pattern } => {
            format!("{label} contains \"{}\"", strip_stars(pattern))
        }
        CriterionKind::DateTimeRange { from, to } => match (from.is_empty(), to.is_empty()) {
            (false, false) => format!("{label} from {from} to {to}"),
            (false, true) => format!("{label} on or after {from}"),
            (true, false) => format!("{label} on or before {to}"),
            (true, true) => format!("{label} (any date)"),
        },
        CriterionKind::BooleanIs { value } => format!("{label} is {value}"),
        CriterionKind::Exists => format!("{label} is present"),
    };
    if criterion.negated {
        format!("NOT ({body})")
    } else {
        body
    }
}

/// Look up the leaf at `path` in the live query and render its sentence.
fn sentence_of(
    query: &BuilderQuery,
    path: &[usize],
    leaf_meta: RwSignal<HashMap<String, CatalogNode>>,
) -> String {
    let node = query.criteria.as_ref().and_then(|root| node_at(root, path));
    match node {
        Some(CriterionNode::Leaf(criterion)) => {
            let meta = leaf_meta.with(|m| m.get(&criterion.aql_path).cloned());
            criterion_sentence(criterion, meta.as_ref())
        }
        _ => String::new(),
    }
}

/// `"magnitude between 36 and 38.5"` / `"…at least 36"` / `"…at most 38.5"`.
fn range_phrase(word: &str, min: Option<String>, max: Option<String>) -> String {
    match (min, max) {
        (Some(a), Some(b)) => format!("{word} between {a} and {b}"),
        (Some(a), None) => format!("{word} at least {a}"),
        (None, Some(b)) => format!("{word} at most {b}"),
        (None, None) => format!("{word} (any)"),
    }
}

/// Resolve code strings to their catalog labels (falling back to the code).
fn code_labels(codes: &[String], meta: Option<&CatalogNode>) -> String {
    codes
        .iter()
        .map(
            |code| match meta.and_then(|m| m.code_options.iter().find(|o| &o.code == code)) {
                Some(opt) if opt.label != opt.code && !opt.label.is_empty() => {
                    format!("{} ({})", opt.label, opt.code)
                }
                _ => code.clone(),
            },
        )
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve ordinal ints to their catalog labels.
fn ordinal_labels(values: &[i64], meta: Option<&CatalogNode>) -> String {
    values
        .iter()
        .map(|value| {
            match meta.and_then(|m| {
                m.code_options
                    .iter()
                    .find(|o| o.ordinal.map(i64::from) == Some(*value))
            }) {
                Some(opt) if !opt.label.is_empty() && opt.label != opt.code => {
                    format!("{value} · {}", opt.label)
                }
                _ => value.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format an optional float as a display string.
fn fmt_opt_f64(value: Option<f64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

/// Format an optional integer as a display string.
fn fmt_opt_i64(value: Option<i64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

/// Parse a numeric field: blank → `None`, valid → `Some`, junk → `None`.
fn parse_opt_f64(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        trimmed.parse::<f64>().ok()
    }
}

/// Parse an integer field: blank → `None`, valid → `Some`, junk → `None`.
fn parse_opt_i64(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        trimmed.parse::<i64>().ok()
    }
}

/// `Option<f64>` → the pre-formatted `Option<String>` the sentence builder uses.
fn num_opt(value: Option<f64>) -> Option<String> {
    value.map(|v| v.to_string())
}

/// `Option<i64>` → pre-formatted `Option<String>`.
fn int_opt(value: Option<i64>) -> Option<String> {
    value.map(|v| v.to_string())
}

/// Strip a single leading and trailing `*` (the contains-mode LIKE wrapping).
fn strip_stars(pattern: &str) -> &str {
    let head = pattern.strip_prefix('*').unwrap_or(pattern);
    head.strip_suffix('*').unwrap_or(head)
}

/// The last `/`-separated segment of a path, for a fallback label.
fn last_segment(path: &str) -> String {
    path.trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned()
}

/// A deterministic, hydration-safe key for a tree path (radio `name`s, input
/// `id`s). Uses the child indices, not any random value.
fn path_key(path: &[usize]) -> String {
    if path.is_empty() {
        "root".to_owned()
    } else {
        path.iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("-")
    }
}

// ---------------------------------------------------------------------------
// Pure criterion-tree mutations (unit-tested)
// ---------------------------------------------------------------------------

/// Follow `path` (child indices) from a tree node to an immutable descendant.
fn node_at<'a>(root: &'a CriterionNode, path: &[usize]) -> Option<&'a CriterionNode> {
    match path.split_first() {
        None => Some(root),
        Some((idx, rest)) => match root {
            CriterionNode::Group { children, .. } => {
                children.get(*idx).and_then(|child| node_at(child, rest))
            }
            CriterionNode::Leaf(_) => None,
        },
    }
}

/// Follow `path` to a mutable descendant.
fn node_at_mut<'a>(root: &'a mut CriterionNode, path: &[usize]) -> Option<&'a mut CriterionNode> {
    match path.split_first() {
        None => Some(root),
        Some((idx, rest)) => match root {
            CriterionNode::Group { children, .. } => children
                .get_mut(*idx)
                .and_then(|child| node_at_mut(child, rest)),
            CriterionNode::Leaf(_) => None,
        },
    }
}

/// Add a leaf into the group at `path` (falling back to the root group, which
/// is created if absent).
fn add_leaf_at(query: &mut BuilderQuery, path: &[usize], criterion: Criterion) {
    if query.criteria.is_none() {
        query.criteria = Some(CriterionNode::Group {
            op: BoolOp::And,
            negated: false,
            children: Vec::new(),
        });
    }
    let pushed = query
        .criteria
        .as_mut()
        .and_then(|root| node_at_mut(root, path))
        .and_then(|target| match target {
            CriterionNode::Group { children, .. } => {
                children.push(CriterionNode::Leaf(criterion.clone()));
                Some(())
            }
            CriterionNode::Leaf(_) => None,
        });
    if pushed.is_none()
        && let Some(CriterionNode::Group { children, .. }) = query.criteria.as_mut()
    {
        children.push(CriterionNode::Leaf(criterion));
    }
}

/// Add an empty AND group into the group at `path`.
fn add_group_at(query: &mut BuilderQuery, path: &[usize]) {
    if let Some(root) = query.criteria.as_mut()
        && let Some(CriterionNode::Group { children, .. }) = node_at_mut(root, path)
    {
        children.push(CriterionNode::Group {
            op: BoolOp::And,
            negated: false,
            children: Vec::new(),
        });
    }
}

/// Remove the node at `path`. Removing the root (empty path), or emptying the
/// root group, clears the whole criteria tree.
fn remove_at(criteria: &mut Option<CriterionNode>, path: &[usize]) {
    match path.split_last() {
        None => *criteria = None,
        Some((last, parent)) => {
            if let Some(root) = criteria.as_mut()
                && let Some(CriterionNode::Group { children, .. }) = node_at_mut(root, parent)
                && *last < children.len()
            {
                children.remove(*last);
            }
            if let Some(CriterionNode::Group { children, .. }) = criteria.as_ref()
                && children.is_empty()
            {
                *criteria = None;
            }
        }
    }
}

/// Flip the NOT flag of the node (leaf or group) at `path`.
fn toggle_negated(query: &mut BuilderQuery, path: &[usize]) {
    if let Some(root) = query.criteria.as_mut() {
        match node_at_mut(root, path) {
            Some(CriterionNode::Leaf(criterion)) => criterion.negated = !criterion.negated,
            Some(CriterionNode::Group { negated, .. }) => *negated = !*negated,
            None => {}
        }
    }
}

/// Flip a group's connective (AND ↔ OR) at `path`.
fn toggle_op(query: &mut BuilderQuery, path: &[usize]) {
    if let Some(root) = query.criteria.as_mut()
        && let Some(CriterionNode::Group { op, .. }) = node_at_mut(root, path)
    {
        *op = if *op == BoolOp::And {
            BoolOp::Or
        } else {
            BoolOp::And
        };
    }
}

/// Set the typed constraint of the leaf at `path`.
fn set_leaf_kind(query: &mut BuilderQuery, path: &[usize], kind: CriterionKind) {
    if let Some(root) = query.criteria.as_mut()
        && let Some(CriterionNode::Leaf(criterion)) = node_at_mut(root, path)
    {
        criterion.kind = kind;
    }
}

#[cfg(test)]
mod tests {
    // The chart derivation this screen used to own (its column-picking rule and
    // its rejection cases) now lives in `crate::chart_model`, re-specified for
    // multi-series derivation and unit-tested beside it.
    use super::{
        add_group_at, add_leaf_at, criterion_sentence, default_kind_for, node_at, remove_at,
        set_leaf_kind, strip_stars, toggle_negated, toggle_op,
    };
    use crate::builder::catalog::{CatalogNode, CodeOption};
    use crate::builder::model::{BoolOp, BuilderQuery, Criterion, CriterionKind, CriterionNode};

    fn quantity_leaf() -> Criterion {
        Criterion {
            aql_path: "content[...]/items[at0004]/value".to_owned(),
            negated: false,
            kind: CriterionKind::QuantityRange {
                min: Some(36.0),
                max: Some(38.5),
                units: "°C".to_owned(),
            },
        }
    }

    fn temp_meta() -> CatalogNode {
        CatalogNode {
            label: "Temperature".to_owned(),
            rm_type: "DV_QUANTITY".to_owned(),
            aql_path: "content[...]/items[at0004]/value".to_owned(),
            node_id: "at0004".to_owned(),
            selectable: true,
            code_options: Vec::new(),
            unit_options: vec!["°C".to_owned()],
            children: Vec::new(),
        }
    }

    #[test]
    fn open_in_raw_editor_link_percent_encodes_the_generated_aql() {
        // The exact text this screen's preview produces (`to_aql` over a
        // quantity-range criterion), carrying every URL-hostile character
        // generated AQL actually contains: spaces, `/` on every path segment,
        // `'` around literals, `=`, and a non-ASCII unit.
        let mut query = BuilderQuery::new("vitals.v1".to_owned());
        query.criteria = Some(CriterionNode::Leaf(Criterion {
            aql_path: "content[openEHR-EHR-OBSERVATION.body_temperature.v2]/data[at0002]\
                       /events[at0003]/data[at0001]/items[at0004]/value"
                .to_owned(),
            negated: false,
            kind: CriterionKind::QuantityRange {
                min: Some(36.0),
                max: Some(38.5),
                units: "°C".to_owned(),
            },
        }));
        let aql = crate::builder::lower::to_aql(&query).expect("the fixture query lowers");
        assert!(aql.contains(' ') && aql.contains('/') && aql.contains('\'') && aql.contains("°C"));

        let href = crate::pages::query_aql::aql_href(&aql);
        let value = href
            .strip_prefix("/queries/aql?aql=")
            .expect("the builder always emits /queries/aql?aql=<value>");
        // Nothing left that could end the value early or forge a parameter.
        assert!(!value.contains(['?', '&', '=', '/', ' ', '#', '\'']));
        assert!(
            value.is_ascii(),
            "the non-ASCII unit must be escaped: {value}"
        );
        // And the raw editor gets the query back byte-for-byte (the router
        // percent-decodes `?aql=` before reading it).
        assert_eq!(
            urlencoding::decode(value).expect("valid UTF-8 percent-encoding"),
            aql
        );
    }

    #[test]
    fn quantity_sentence_reads_naturally() {
        let leaf = quantity_leaf();
        let meta = temp_meta();
        assert_eq!(
            criterion_sentence(&leaf, Some(&meta)),
            "Temperature magnitude between 36 and 38.5 °C"
        );
    }

    #[test]
    fn negated_sentence_is_wrapped() {
        let mut leaf = quantity_leaf();
        leaf.negated = true;
        let meta = temp_meta();
        assert_eq!(
            criterion_sentence(&leaf, Some(&meta)),
            "NOT (Temperature magnitude between 36 and 38.5 °C)"
        );
    }

    #[test]
    fn coded_sentence_uses_option_labels() {
        let criterion = Criterion {
            aql_path: "p/value".to_owned(),
            negated: false,
            kind: CriterionKind::CodedIn {
                codes: vec!["at0037".to_owned()],
                terminology: "local".to_owned(),
            },
        };
        let meta = CatalogNode {
            label: "Position".to_owned(),
            rm_type: "DV_CODED_TEXT".to_owned(),
            aql_path: "p/value".to_owned(),
            node_id: String::new(),
            selectable: true,
            code_options: vec![CodeOption {
                code: "at0037".to_owned(),
                label: "Sitting".to_owned(),
                ordinal: None,
            }],
            unit_options: Vec::new(),
            children: Vec::new(),
        };
        assert_eq!(
            criterion_sentence(&criterion, Some(&meta)),
            "Position is one of Sitting (at0037)"
        );
    }

    #[test]
    fn boolean_and_exists_sentences() {
        let boolean = Criterion {
            aql_path: "p/value".to_owned(),
            negated: false,
            kind: CriterionKind::BooleanIs { value: true },
        };
        assert_eq!(criterion_sentence(&boolean, None), "value is true");
        let exists = Criterion {
            aql_path: "a/b/present".to_owned(),
            negated: false,
            kind: CriterionKind::Exists,
        };
        assert_eq!(criterion_sentence(&exists, None), "present is present");
    }

    #[test]
    fn default_kind_maps_rm_types() {
        assert!(matches!(
            default_kind_for("DV_QUANTITY"),
            CriterionKind::QuantityRange { .. }
        ));
        assert!(matches!(
            default_kind_for("DV_CODED_TEXT"),
            CriterionKind::CodedIn { .. }
        ));
        assert!(matches!(
            default_kind_for("DV_DATE"),
            CriterionKind::DateTimeRange { .. }
        ));
        assert!(matches!(
            default_kind_for("SOMETHING_ELSE"),
            CriterionKind::Exists
        ));
    }

    #[test]
    fn strip_stars_unwraps_contains_pattern() {
        assert_eq!(strip_stars("*fever*"), "fever");
        assert_eq!(strip_stars("plain"), "plain");
        assert_eq!(strip_stars("*lead"), "lead");
    }

    #[test]
    fn add_leaf_creates_root_group_then_appends() {
        let mut q = BuilderQuery::new(String::new());
        add_leaf_at(&mut q, &[], quantity_leaf());
        add_leaf_at(&mut q, &[], quantity_leaf());
        match &q.criteria {
            Some(CriterionNode::Group { op, children, .. }) => {
                assert_eq!(*op, BoolOp::And);
                assert_eq!(children.len(), 2);
            }
            other => panic!("expected a two-child And group, got {other:?}"),
        }
    }

    #[test]
    fn add_leaf_into_nested_group() {
        let mut q = BuilderQuery::new(String::new());
        add_leaf_at(&mut q, &[], quantity_leaf()); // root child 0
        add_group_at(&mut q, &[]); // root child 1 = empty group
        add_leaf_at(&mut q, &[1], quantity_leaf()); // into the nested group
        let nested = node_at(q.criteria.as_ref().unwrap(), &[1]).unwrap();
        match nested {
            CriterionNode::Group { children, .. } => assert_eq!(children.len(), 1),
            CriterionNode::Leaf(_) => panic!("expected nested group"),
        }
    }

    #[test]
    fn remove_last_child_clears_tree() {
        let mut q = BuilderQuery::new(String::new());
        add_leaf_at(&mut q, &[], quantity_leaf());
        remove_at(&mut q.criteria, &[0]);
        assert!(q.criteria.is_none());
    }

    #[test]
    fn toggle_op_and_negation_flip() {
        let mut q = BuilderQuery::new(String::new());
        add_leaf_at(&mut q, &[], quantity_leaf());
        toggle_op(&mut q, &[]);
        toggle_negated(&mut q, &[0]);
        match &q.criteria {
            Some(CriterionNode::Group { op, children, .. }) => {
                assert_eq!(*op, BoolOp::Or);
                match &children[0] {
                    CriterionNode::Leaf(c) => assert!(c.negated),
                    CriterionNode::Group { .. } => panic!("expected leaf"),
                }
            }
            _ => panic!("expected group"),
        }
    }

    #[test]
    fn set_leaf_kind_replaces_constraint() {
        let mut q = BuilderQuery::new(String::new());
        add_leaf_at(&mut q, &[], quantity_leaf());
        set_leaf_kind(&mut q, &[0], CriterionKind::Exists);
        match node_at(q.criteria.as_ref().unwrap(), &[0]).unwrap() {
            CriterionNode::Leaf(c) => assert!(matches!(c.kind, CriterionKind::Exists)),
            CriterionNode::Group { .. } => panic!("expected leaf"),
        }
    }

    /// A catalog node for the metadata-collection test.
    fn catalog_node(
        aql_path: &str,
        rm_type: &str,
        selectable: bool,
        children: Vec<CatalogNode>,
    ) -> CatalogNode {
        CatalogNode {
            label: format!("label for {aql_path}"),
            rm_type: rm_type.to_owned(),
            aql_path: aql_path.to_owned(),
            node_id: String::new(),
            selectable,
            code_options: Vec::new(),
            unit_options: Vec::new(),
            children,
        }
    }

    #[test]
    fn collect_selectable_keys_every_data_value_leaf_by_its_path() {
        // The root COMPOSITION carries an empty `aqlPath` and is not selectable;
        // only the DV leaves become criterion metadata.
        let tree = catalog_node(
            "",
            "COMPOSITION",
            false,
            vec![catalog_node(
                "/content[openEHR-EHR-OBSERVATION.body_temperature.v2]",
                "OBSERVATION",
                false,
                vec![
                    catalog_node(
                        "/content[obs]/items[at0004]/value",
                        "DV_QUANTITY",
                        true,
                        vec![],
                    ),
                    catalog_node(
                        "/content[obs]/items[at0005]/value",
                        "DV_CODED_TEXT",
                        true,
                        vec![],
                    ),
                ],
            )],
        );
        let mut found = std::collections::BTreeMap::new();
        super::collect_selectable(&tree, &mut found);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found
                .get("/content[obs]/items[at0004]/value")
                .map(|n| n.rm_type.as_str()),
            Some("DV_QUANTITY")
        );
        assert!(!found.contains_key(""));
    }
}
