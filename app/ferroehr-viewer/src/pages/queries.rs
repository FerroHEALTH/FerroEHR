// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/queries` screen: the CDR's stored queries (left) and those same
//! queries grouped by the namespace of their qualified name (right).
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! The stored-query surface is the ITS-REST Definition/Query API
//! (`docs/specs/openehr/ITS-REST/specifications/docs/definition/`). The
//! grouping is **derived, never stored**: a query's group IS its namespace
//! (`crate::query_namespace`, which cites the name format), so the right pane
//! is a projection of the very listing the left pane shows — one round trip,
//! no viewer-local state, and the same grouping every API client sees. There
//! is therefore no group CRUD: a query joins a group by being saved under that
//! namespace.
//!
//! All data flows through the [`crate::queries_api`] server fns plus the admin
//! gate + stored-query delete in [`crate::admin`] — each guards the session
//! itself; this screen declares no server fn of its own.

use leptos::component;
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::admin::AdminAvailability;
use crate::components::data_table::{
    CELL, CELL_MONO, ROW, TablePaging, page_rows, page_window, paging_from_url, row_total,
    table_footer, table_shell, table_skeleton,
};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_DANGER, BTN_PRIMARY, BTN_SECONDARY};
use crate::components::format_view::DocumentPane;
use crate::components::page_header::PageHeader;
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::toast::{toast_error, toast_success};
use crate::error::ViewerError;
use crate::queries_api::{StoredQueryRow, fetch_stored_query, list_stored_queries};
use crate::query_namespace::{QueryNamespaceGroup, bare_name_of, group_by_namespace, group_label};

/// The stored-query delete action: the `(name, version)` it was dispatched
/// with, paired with the CDR's answer, so both toasts can name the exact
/// query. This deletes from the **CDR's** stored-query store; the namespace
/// panel has no destructive action at all (see [`namespaces_panel`]).
type CdrQueryDelete = Action<(String, String), ((String, String), Result<(), ViewerError>)>;

/// The `/queries` screen: a stored-queries table with an on-demand AQL detail,
/// alongside the read-only namespace grouping derived from the same listing.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn QueriesPage() -> impl IntoView {
    let toaster = thaw::ToasterInjection::expect_context();
    // The admin probe and the CDR stored-query delete it gates (discover-and-hide:
    // no advertised admin group, no delete affordance). Created in setup so the
    // gated view can re-render without re-creating them.
    let gate = crate::admin::admin_gate();
    let cdr_delete: CdrQueryDelete = Action::new(|key: &(String, String)| {
        let key = key.clone();
        async move {
            let outcome =
                crate::admin::admin_delete_stored_query(key.0.clone(), key.1.clone()).await;
            (key, outcome)
        }
    });
    // ONE listing feeds both panes — the table (left) and the derived namespace
    // grouping (right); a Resource is Copy so both read it. A CDR delete bumps
    // the action's version, which is the resource's source, so the grouping
    // re-derives with the table.
    let stored = Resource::new(
        move || cdr_delete.version().get(),
        |_| async move { list_stored_queries().await },
    );
    // The CDR stored-query delete reports as a toast — its copy names the
    // query, and a failure carries the actionable next action.
    Effect::new(move |_| match cdr_delete.value().get() {
        Some(((name, version), Ok(()))) => toast_success(
            toaster,
            "Stored query deleted",
            &format!("{name} v{version} was removed from the CDR."),
        ),
        Some(((name, version), Err(error))) => toast_error(
            toaster,
            "Delete failed",
            &crate::admin::delete_failure_copy(&format!("Stored query {name} v{version}"), &error),
        ),
        None => {}
    });

    let table = stored_queries_panel(stored, gate, cdr_delete);
    let namespaces = namespaces_panel(stored);

    view! {
        <Title text="Stored queries" />
        <div class="p-6">
            <PageHeader
                title="Queries"
                subtitle="The CDR's stored queries, grouped by the namespace in their qualified name."
            >
                <a href="/queries/builder" class=BTN_PRIMARY>
                    "New query"
                </a>
                <a href="/queries/aql" class=BTN_SECONDARY>
                    "Raw AQL"
                </a>
            </PageHeader>
            <div class="grid grid-cols-1 lg:grid-cols-2 gap-6 items-start">
                {table} {namespaces}
            </div>
        </div>
    }
}

// ── Stored queries (left) ────────────────────────────────────────────────

/// The stored-queries panel: the list table plus the single-select AQL detail
/// shown below it.
fn stored_queries_panel(
    stored: Resource<Result<Vec<StoredQueryRow>, ViewerError>>,
    gate: Resource<Result<AdminAvailability, ViewerError>>,
    cdr_delete: CdrQueryDelete,
) -> AnyView {
    // The table's page window, read from the URL here in SETUP — never inside
    // the `Suspend` that awaits the listing. Turning the page therefore
    // re-renders the row window without refetching the listing.
    let paging = paging_from_url();
    // The row (name, version) whose AQL detail is currently expanded.
    let selected = RwSignal::new(Option::<(String, String)>::None);
    // The selected query's AQL, fetched on demand; `None` source → no call.
    let detail = Resource::new(
        move || selected.get(),
        |key| async move {
            match key {
                Some((name, version)) => fetch_stored_query(name, version).await.map(Some),
                None => Ok(None),
            }
        },
    );
    // The `(name, version)` awaiting confirmation in the modal (`None` = no
    // dialog). ONE dialog serves every row — the signal is both "which row" and
    // "open".
    let pending_delete = RwSignal::new(Option::<(String, String)>::None);
    let table = stored_table(stored, selected, paging, gate, cdr_delete, pending_delete);
    let detail_view = stored_detail(detail);
    let confirm = cdr_delete_dialog(pending_delete, cdr_delete);
    view! {
        <section class="space-y-3">
            <h2 class=CARD_TITLE>"Stored queries"</h2>
            {table}
            {detail_view}
            {confirm}
        </section>
    }
    .into_any()
}

/// The panel's ONE CDR-delete confirmation modal, driven by `pending_delete`
/// (which stored-query version triggered it). Rendered once outside the table so
/// a list refetch never re-creates it, and inert while nothing is pending —
/// which is why it needs no admin gate of its own: only an admin-gated trigger
/// can set the signal. The copy is explicit that this removes the version from
/// the CDR for every client.
fn cdr_delete_dialog(
    pending_delete: RwSignal<Option<(String, String)>>,
    cdr_delete: CdrQueryDelete,
) -> AnyView {
    let message = Signal::derive(move || {
        pending_delete
            .get()
            .map_or_else(String::new, |(name, version)| {
                format!(
                    "Permanently delete stored query “{name}” version {version} from the CDR? \
                     Every client loses that version, and it cannot be undone."
                )
            })
    });
    view! {
        <crate::components::confirm_dialog::ConfirmDialog
            open=Signal::derive(move || pending_delete.get().is_some())
            title="Delete stored query from the CDR"
            message=message
            confirm_label="Delete from CDR"
            confirm_id="stored-query-delete-confirm"
            on_cancel=Callback::new(move |()| pending_delete.set(None))
            on_confirm=Callback::new(move |()| {
                if let Some(key) = pending_delete.get_untracked() {
                    cdr_delete.dispatch(key);
                }
                pending_delete.set(None);
            })
        />
    }
    .into_any()
}

/// The stored-queries table, read under `<Transition>` (a CDR delete refetches
/// the list — keep the current rows visible instead of flashing the skeleton).
/// The admin probe is awaited in the SAME `Suspend` as the list, so every row
/// agrees on whether the delete affordance exists.
fn stored_table(
    stored: Resource<Result<Vec<StoredQueryRow>, ViewerError>>,
    selected: RwSignal<Option<(String, String)>>,
    paging: TablePaging,
    gate: Resource<Result<AdminAvailability, ViewerError>>,
    cdr_delete: CdrQueryDelete,
    pending_delete: RwSignal<Option<(String, String)>>,
) -> AnyView {
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                let admin = crate::admin::renders_admin_ops(&gate.await);
                match stored.await {
                    Ok(rows) => {
                        stored_rows_view(rows, selected, paging, admin, cdr_delete, pending_delete)
                    }
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// Render the stored-query rows (or the empty state): one page of the listing
/// plus the shared pagination footer. The row list is a `<For>` keyed by
/// `name@version` (a stable, data-derived key), and its `each` closure is what
/// tracks the URL's page window, so paging re-renders the rows without
/// touching the listing resource. `data-stored-query` is the stable E2E hook
/// for a row.
fn stored_rows_view(
    rows: Vec<StoredQueryRow>,
    selected: RwSignal<Option<(String, String)>>,
    paging: TablePaging,
    admin: bool,
    cdr_delete: CdrQueryDelete,
    pending_delete: RwSignal<Option<(String, String)>>,
) -> AnyView {
    if rows.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuSearchCode
                message="No stored queries"
                hint="Build one in the query builder, or write raw AQL, then save it."
            />
        }
        .into_any();
    }
    // The CDR returned the whole listing, so the total is known and the window
    // is pure view state: total is fixed for this render, page/size reactive.
    let total = row_total(rows.len());
    let body = view! {
        <For
            each=move || {
                let window = page_window(total, paging.page.get(), paging.size.get());
                page_rows(&rows, window)
            }
            key=|row| format!("{}@{}", row.name, row.version)
            let:row
        >
            {stored_row(row, selected, admin, cdr_delete, pending_delete)}
        </For>
    }
    .into_any();
    let footer = table_footer(
        "/queries",
        "stored queries",
        paging,
        Signal::derive(move || total),
    );
    view! {
        {table_shell(&["Name", "Version", "Saved", ""], body)}
        {footer}
    }
    .into_any()
}

/// One stored-query row. Clicking it toggles the single-select detail below the
/// table (`on:click` is a Rust listener — zero authored JS).
fn stored_row(
    row: StoredQueryRow,
    selected: RwSignal<Option<(String, String)>>,
    admin: bool,
    cdr_delete: CdrQueryDelete,
    pending_delete: RwSignal<Option<(String, String)>>,
) -> impl IntoView {
    let key_for_class = (row.name.clone(), row.version.clone());
    let key_for_click = (row.name.clone(), row.version.clone());
    let is_selected = move || selected.with(|current| current.as_ref() == Some(&key_for_class));
    let on_click = move |_| {
        let key = key_for_click.clone();
        selected.update(|current| {
            if current.as_ref() == Some(&key) {
                *current = None;
            } else {
                *current = Some(key);
            }
        });
    };
    // The three per-row hand-offs, all carrying `?load=name@version` (percent-
    // encoded as ONE value): the raw editor seeds its textarea from it, the
    // builder LIFTS the AQL back into the point-and-click state, and the runner
    // executes it with parameters. Each click is stopped from bubbling to the
    // row's toggle handler (so it never also expands the detail); with no router
    // delegation reached, the browser does a plain navigation to the fresh page.
    let load_href = crate::pages::query_aql::load_href(&row.name, &row.version);
    let builder_href = crate::pages::query_builder::load_href(&row.name, &row.version);
    let run_href = crate::pages::query_stored::run_href(&row.name, &row.version);
    let delete_button = if admin {
        cdr_delete_button(&row.name, &row.version, cdr_delete, pending_delete)
    } else {
        ().into_any()
    };
    // The row's stable E2E key (`name@version`), reused by each hand-off link so
    // a journey can address the exact row's action.
    let row_hook = format!("{}@{}", row.name, row.version);
    let run_hook = row_hook.clone();
    let builder_hook = row_hook.clone();
    view! {
        <tr
            class=format!("{ROW} cursor-pointer")
            class=("bg-accent-subtle", is_selected)
            data-stored-query=row_hook
            on:click=on_click
        >
            <td class=CELL_MONO>{row.name}</td>
            <td class=CELL_MONO>{row.version}</td>
            <td class=format!("{CELL} text-xs text-ink-muted")>{row.saved}</td>
            <td class=format!("{CELL} text-right")>
                // `whitespace-nowrap`: the panel is half-width, so without it the
                // action labels wrap mid-phrase ("Delete from / CDR"). The row
                // wraps between whole actions instead.
                <div class="flex flex-wrap items-center justify-end gap-2 whitespace-nowrap">
                    <a
                        href=run_href
                        class=BTN_SECONDARY
                        data-run-stored=run_hook
                        on:click=|ev| ev.stop_propagation()
                    >
                        "Run"
                    </a>
                    <a href=load_href class=BTN_SECONDARY on:click=|ev| ev.stop_propagation()>
                        "Open in editor"
                    </a>
                    <a
                        href=builder_href
                        class=BTN_SECONDARY
                        data-open-in-builder=builder_hook
                        on:click=|ev| ev.stop_propagation()
                    >
                        "Open in builder"
                    </a>
                    {delete_button}
                </div>
            </td>
        </tr>
    }
}

/// The admin **Delete from CDR** button for one stored-query version: it opens
/// the panel's confirmation modal for THIS `(name, version)` (only the dialog's
/// confirm dispatches). Deliberately labelled: it removes the query VERSION
/// from the CDR's stored-query store for every client — the only destructive
/// action on this screen. The click never bubbles to the row's detail toggle.
/// `data-query-delete` (`name@version`) is the stable E2E hook.
fn cdr_delete_button(
    name: &str,
    version: &str,
    cdr_delete: CdrQueryDelete,
    pending_delete: RwSignal<Option<(String, String)>>,
) -> AnyView {
    let key = format!("{name}@{version}");
    let target = (name.to_owned(), version.to_owned());
    let on_click = move |ev: leptos::ev::MouseEvent| {
        ev.stop_propagation();
        pending_delete.set(Some(target.clone()));
    };
    view! {
        <button
            type="button"
            class=BTN_DANGER
            data-query-delete=key
            disabled=Signal::derive(move || cdr_delete.pending().get())
            on:click=on_click
        >
            "Delete from CDR"
        </button>
    }
    .into_any()
}

/// The AQL detail below the table: the selected query's AQL in a
/// [`DocumentPane`] plus a Run button. Read under `<Transition>` so the old
/// detail stays visible while a new selection loads.
fn stored_detail(detail: Resource<Result<Option<String>, ViewerError>>) -> AnyView {
    view! {
        <div class="mt-3">
            <Transition fallback=|| {
                view! { <p class="text-sm text-ink-muted">"Loading query…"</p> }
            }>
                {move || Suspend::new(async move {
                    match detail.await {
                        Ok(Some(text)) => detail_panel(text),
                        Ok(None) => ().into_any(),
                        Err(e) => crate::components::notice::inline_error(&e),
                    }
                })}
            </Transition>
        </div>
    }
    .into_any()
}

/// The expanded detail for one stored query: its AQL and a button that navigates
/// to the raw-AQL screen with the query pre-filled via `?aql=` — an AD-HOC run
/// of that text, deliberately distinct from the row's **Run**, which executes the
/// STORED definition (and can bind its parameters).
fn detail_panel(aql: String) -> AnyView {
    let navigate = use_navigate();
    let aql_for_run = aql.clone();
    let on_run = move |_| {
        navigate(
            &crate::pages::query_aql::aql_href(&aql_for_run),
            NavigateOptions::default(),
        );
    };
    let body = Signal::derive(move || aql.clone());
    view! {
        <div class=CARD_PAD>
            <div class="flex items-center justify-between mb-2">
                <span class="text-xs font-semibold uppercase tracking-wide text-ink-muted">
                    "AQL"
                </span>
                <button type="button" class=BTN_PRIMARY on:click=on_run>
                    "Run as ad-hoc AQL"
                </button>
            </div>
            <DocumentPane body=body />
        </div>
    }
    .into_any()
}

// ── Derived namespace grouping (right) ───────────────────────────────────

/// The namespace panel: the SAME stored-query listing the table shows, grouped
/// by the namespace of each qualified name. Read-only by construction — a query
/// joins a group by being saved under that namespace, so there is nothing to
/// create, edit, or remove here (and nothing stored viewer-side).
fn namespaces_panel(stored: Resource<Result<Vec<StoredQueryRow>, ViewerError>>) -> AnyView {
    // `<Transition>`: the listing refetches after a CDR delete — keep the
    // current grouping visible instead of flashing a fallback. The `Result`
    // resolves INSIDE the `Suspend`.
    let list = view! {
        <Transition fallback=|| {
            view! { <p class="text-sm text-ink-muted">"Loading namespaces…"</p> }
        }>
            {move || Suspend::new(async move {
                match stored.await {
                    Ok(rows) => namespace_cards(&rows),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();
    view! {
        <section class="space-y-3">
            <h2 class=CARD_TITLE>"Namespaces"</h2>
            <p class="text-sm text-ink-muted">
                "A query's group is the namespace of its qualified name ("
                <span class="font-mono">"namespace::name"</span>
                "), chosen when you save it. The viewer stores no grouping of its own, so every openEHR client sees the same one."
            </p>
            {list}
        </section>
    }
    .into_any()
}

/// Render one card per derived namespace group, or the empty state when the CDR
/// holds no stored queries at all.
fn namespace_cards(rows: &[StoredQueryRow]) -> AnyView {
    let groups = group_by_namespace(rows);
    if groups.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuInbox
                message="No namespaces yet"
                hint="Save a query as namespace::name — its namespace becomes a group here and a tile on the dashboard."
            />
        }
        .into_any();
    }
    let cards = groups.iter().map(namespace_card).collect::<Vec<_>>();
    view! { <div class="flex flex-col gap-2">{cards}</div> }.into_any()
}

/// One namespace card: the namespace heading (or the unqualified-bucket label),
/// how many member queries it covers, and a chip per member showing the bare
/// name and version. `data-query-namespace` is the stable E2E hook.
fn namespace_card(group: &QueryNamespaceGroup) -> AnyView {
    let label = group_label(group.namespace.as_deref()).to_owned();
    let label_hook = label.clone();
    let count = group.members.len();
    let summary = if count == 1 {
        "1 query".to_owned()
    } else {
        format!("{count} queries")
    };
    let chips = group
        .members
        .iter()
        .map(|row| {
            let name = bare_name_of(&row.name).to_owned();
            let version = row.version.clone();
            view! {
                <span class="inline-flex items-center rounded-control bg-accent-subtle px-2 py-0.5 text-xs text-accent-ink">
                    <span class="font-mono">{name}</span>
                    {(!version.is_empty())
                        .then(|| view! { <span class="opacity-60">" @"{version}</span> })}
                </span>
            }
        })
        .collect::<Vec<_>>();
    view! {
        <div class=CARD_PAD data-query-namespace=label_hook>
            <div class="flex items-center justify-between gap-2 mb-2">
                <span class="font-mono font-medium text-ink truncate">{label}</span>
                <span class="text-xs text-ink-muted whitespace-nowrap">{summary}</span>
            </div>
            <div class="flex flex-wrap gap-1">{chips}</div>
        </div>
    }
    .into_any()
}
