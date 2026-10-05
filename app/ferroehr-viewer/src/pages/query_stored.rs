// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/queries/stored` screen: run a STORED query server-side, with its
//! parameters prompted and its version-resolution form chosen at run time.
//!
//! Arrives as `?load=name@version` from the stored-query list (or the raw
//! editor). The screen shows the stored definition, offers the three openEHR
//! version-resolution forms — latest / SEMVER prefix / exact — and one input per
//! `$placeholder` the AQL declares, then executes it through
//! [`run_stored_query`] (`POST query/{name}[/{version}]` carrying
//! `query_parameters`) and renders the `RESULT_SET` in the shared results pane.
//! No openEHR spec governs the viewer — our own design / product extension;
//! the wire it drives IS spec-bound (ITS-REST Query API,
//! `docs/specs/openehr/ITS-REST/specifications/docs/query/`).
//!
//! Running a stored query is a pure READ, so failures render inline and never
//! toast.

use std::collections::BTreeMap;

use leptos::component;
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_query_map;

use crate::aql_text::{carries_own_window, parameters_json, placeholders};
use crate::components::data_table::table_skeleton;
use crate::components::field::{BTN_PRIMARY, INPUT, LABEL, SELECT};
use crate::components::format_view::DocumentPane;
use crate::components::page_header::{Crumb, PageHeader};
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::error::ViewerError;
use crate::pages::ehrs::ResultPage;
use crate::pages::query_aql::{LoadedQuery, LoadedQueryResource};
use crate::pages::query_builder::{paging_buttons, results_view};
use crate::queries_api::run_stored_query;
use crate::query_namespace::{VersionResolution, resolve_version, split_query_ref};

/// One dispatched run: the qualified name, the resolved `version` path segment
/// (empty = the latest form, which sends none), the `query_parameters` JSON,
/// and whether the request may carry its own `fetch`/`offset` window (false
/// when the stored AQL windows itself with `LIMIT`/`TOP` — the two cannot be
/// combined, see `run_stored_query`). Decided at dispatch, in the click
/// handler, so no render-time code reads the definition resource. Named
/// because the resource source, its fetcher, and the run button all speak it.
type StoredRun = (String, String, String, bool);

/// Build the link INTO this screen that runs the stored query `name@version` —
/// the same `?load=name@version` encoding the other two query screens use
/// ([`crate::query_namespace::load_href`]).
#[must_use]
pub(crate) fn run_href(name: &str, version: &str) -> String {
    crate::query_namespace::load_href("/queries/stored", name, version)
}

/// The stored-query runner screen.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn QueryStoredPage() -> impl IntoView {
    let query_map = use_query_map();
    // The reference the hand-off carries. URL-derived, so the server pass and
    // the client hydration read the same values (hydration-safe) and the Run
    // button needs no async lookup to know what it is running.
    let (name, loaded_version) = query_map.with_untracked(|m| {
        m.get("load")
            .as_deref()
            .and_then(split_query_ref)
            .unwrap_or_default()
    });
    // The resolution form starts as the one that FITS the version the link
    // carried: an exact triple stays exact, a prefix stays a prefix, no version
    // means latest. Deterministic from the URL, so hydration-safe.
    let mode = RwSignal::new(VersionResolution::of(&loaded_version));
    let version = RwSignal::new(loaded_version);
    // The parameter bindings, keyed by placeholder name. Created in SETUP so the
    // values survive every re-render of the section that shows them.
    let bindings = RwSignal::new(BTreeMap::<String, String>::new());
    let ran = RwSignal::new(Option::<StoredRun>::None);
    let offset = RwSignal::new(0_u32);

    let definition = crate::pages::query_aql::loaded_query_resource(query_map);
    let results: Resource<Result<Option<ResultPage>, ViewerError>> = Resource::new(
        move || (ran.get(), offset.get()),
        |(ran, off)| async move {
            match ran {
                Some((name, version, parameters, paged)) => {
                    run_stored_query(name, version, parameters, off, paged)
                        .await
                        .map(Some)
                }
                None => Ok(None),
            }
        },
    );

    let definition_pane = definition_section(definition, &name);
    let resolution = resolution_section(&name, mode, version);
    let parameters = parameters_section(definition, bindings);
    let run = run_section(&name, mode, version, bindings, definition, ran, offset);
    let results_pane = results_section(results, offset, ran);

    view! {
        <Title text="Run stored query" />
        <div class="p-6 space-y-4">
            <PageHeader
                title="Run stored query"
                subtitle="Execute a stored query on the CDR: choose how its version resolves, bind its parameters, page the results."
                crumbs=vec![Crumb::new("Queries", "/queries")]
            />
            {definition_pane}
            {resolution}
            {parameters}
            {run}
            {results_pane}
        </div>
    }
}

// ── the stored definition ────────────────────────────────────────────────────

/// The stored definition the link named: its AQL, read once and shown verbatim
/// so the parameter and resolution choices below are made against what the CDR
/// actually holds. A pure read — the failure renders inline.
fn definition_section(
    definition: Resource<Result<Option<LoadedQuery>, ViewerError>>,
    name: &str,
) -> AnyView {
    if name.is_empty() {
        return view! {
            <section class=CARD_PAD>
                <p class="text-sm text-ink-muted">
                    "No stored query to run. Pick one from "
                    <a href="/queries" class="text-accent hover:underline">
                        "Stored queries"
                    </a> " and choose Run."
                </p>
            </section>
        }
        .into_any();
    }
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Definition"</h2>
            <Transition fallback=move || {
                view! { <p class="text-sm text-ink-muted">"Loading stored query…"</p> }
            }>
                {move || Suspend::new(async move {
                    match definition.await {
                        Ok(Some((qualified, stored_version, aql))) => {
                            definition_pane(qualified, stored_version, aql)
                        }
                        Ok(None) => {
                            view! {
                                <p class="text-sm text-ink-muted">
                                    "That link does not name a stored-query version."
                                </p>
                            }
                                .into_any()
                        }
                        Err(e) => crate::components::notice::inline_error(&e),
                    }
                })}
            </Transition>
        </section>
    }
    .into_any()
}

/// The loaded definition: which version was read, its AQL, and the cross-links
/// to the two editing surfaces.
fn definition_pane(qualified: String, stored_version: String, aql: String) -> AnyView {
    let editor_href = crate::pages::query_aql::load_href(&qualified, &stored_version);
    let builder_href = crate::pages::query_builder::load_href(&qualified, &stored_version);
    let body = Signal::derive(move || aql.clone());
    view! {
        <div class="space-y-2">
            <p class="text-sm text-ink-muted">
                <span class="font-mono text-ink">{qualified}</span>
                " as stored at version "
                <span class="font-mono text-ink">{stored_version}</span>
                "."
            </p>
            <DocumentPane body=body />
            <div class="flex flex-wrap items-center gap-3 text-sm">
                <a href=editor_href class="text-accent hover:underline">
                    "Open in editor"
                </a>
                <a href=builder_href class="text-accent hover:underline" data-open-in-builder="">
                    "Open in builder"
                </a>
            </div>
        </div>
    }
    .into_any()
}

// ── version resolution ──────────────────────────────────────────────────────

/// The three openEHR version-resolution forms, selectable at run time, with the
/// exact request the current choice will send.
fn resolution_section(
    name: &str,
    mode: RwSignal<VersionResolution>,
    version: RwSignal<String>,
) -> AnyView {
    let name = name.to_owned();
    let options = VersionResolution::ALL
        .into_iter()
        .map(|option| {
            view! { <option value=option.as_str()>{option.label()}</option> }
        })
        .collect::<Vec<_>>();
    let needs_version = Signal::derive(move || mode.get() != VersionResolution::Latest);
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Version"</h2>
            <div class="flex flex-wrap items-end gap-3">
                <label class="flex flex-col gap-1">
                    <span class=LABEL>"Resolution"</span>
                    <select
                        id="stored-run-mode"
                        class=SELECT
                        prop:value=move || mode.get().as_str()
                        on:change:target=move |ev| {
                            mode.set(VersionResolution::from_str_or_exact(&ev.target().value()));
                        }
                    >
                        {options}
                    </select>
                </label>
                <label class="flex flex-col gap-1">
                    <span class=LABEL>"Version"</span>
                    <input
                        id="stored-run-version"
                        type="text"
                        placeholder="1.0.0"
                        class=format!("{INPUT} w-32")
                        disabled=Signal::derive(move || !needs_version.get())
                        prop:value=move || version.get()
                        on:input:target=move |ev| version.set(ev.target().value())
                    />
                </label>
            </div>
            {resolution_note(&name, mode, version)}
        </section>
    }
    .into_any()
}

/// The line under the resolution controls: the request the current choice sends,
/// or why the version text does not fit the chosen form.
fn resolution_note(
    name: &str,
    mode: RwSignal<VersionResolution>,
    version: RwSignal<String>,
) -> AnyView {
    let name = name.to_owned();
    view! {
        <div class="mt-2 text-xs" data-resolution-note="">
            {move || {
                match resolve_version(mode.get(), &version.get()) {
                    Ok(None) => {
                        view! {
                            <span class="text-ink-muted">
                                "Sends "
                                <span class="font-mono text-ink">
                                    {format!("POST query/{name}")}
                                </span> " — no version, so the CDR runs the latest one."
                            </span>
                        }
                            .into_any()
                    }
                    Ok(Some(resolved)) => {
                        let path = format!("POST query/{name}/{resolved}");
                        let tail = if mode.get() == VersionResolution::Prefix {
                            " — a partial pattern, so the CDR runs the latest version matching it."
                        } else {
                            " — exactly that version."
                        };
                        view! {
                            <span class="text-ink-muted">
                                "Sends " <span class="font-mono text-ink">{path}</span> {tail}
                            </span>
                        }
                            .into_any()
                    }
                    Err(error) => {
                        view! { <span class="text-danger">{error.to_string()}</span> }.into_any()
                    }
                }
            }}
        </div>
    }
    .into_any()
}

// ── parameters ──────────────────────────────────────────────────────────────

/// One input per `$placeholder` the stored AQL declares, rendered from the
/// definition (so the fields are server-rendered too) and bound to the
/// setup-created `bindings` map — which is what keeps typed values across the
/// section's re-renders.
fn parameters_section(
    definition: Resource<Result<Option<LoadedQuery>, ViewerError>>,
    bindings: RwSignal<BTreeMap<String, String>>,
) -> AnyView {
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Parameters"</h2>
            <Transition fallback=move || {
                view! { <p class="text-sm text-ink-muted">"Reading parameters…"</p> }
            }>
                {move || Suspend::new(async move {
                    match definition.await {
                        Ok(Some((_, _, aql))) => parameter_inputs(&placeholders(&aql), bindings),
                        Ok(None) => ().into_any(),
                        Err(e) => crate::components::notice::inline_error(&e),
                    }
                })}
            </Transition>
        </section>
    }
    .into_any()
}

/// The parameter fields themselves, or the honest "none declared" note.
fn parameter_inputs(names: &[String], bindings: RwSignal<BTreeMap<String, String>>) -> AnyView {
    if names.is_empty() {
        return view! {
            <p class="text-sm text-ink-muted">
                "This query declares no parameters, so it runs as stored."
            </p>
        }
        .into_any();
    }
    let fields = names
        .iter()
        .map(|name| parameter_input(name, bindings))
        .collect::<Vec<_>>();
    view! {
        <div class="space-y-2">
            <div class="flex flex-wrap items-end gap-3">{fields}</div>
            <p class="text-xs text-ink-muted">
                "A value that reads as JSON is sent as that type — "
                <span class="font-mono">"38.5"</span> " as a number, "
                <span class="font-mono">"true"</span>
                " as a boolean; anything else is sent as text. Quote a value to force text ("
                <span class="font-mono">"\"0123\""</span>
                "). A field left blank is not sent at all."
            </p>
        </div>
    }
    .into_any()
}

/// One labelled parameter input. `data-stored-param` (the placeholder name) is
/// the stable E2E hook; the label shows the AQL spelling (`$name`) while the
/// wire sends the unprefixed name.
fn parameter_input(name: &str, bindings: RwSignal<BTreeMap<String, String>>) -> AnyView {
    let key = name.to_owned();
    let key_for_read = key.clone();
    let key_for_write = key.clone();
    view! {
        <label class="flex flex-col gap-1">
            <span class=format!("{LABEL} font-mono")>{format!("${name}")}</span>
            <input
                type="text"
                class=INPUT
                data-stored-param=key
                prop:value=move || {
                    bindings.with(|values| values.get(&key_for_read).cloned().unwrap_or_default())
                }
                on:input:target=move |ev| {
                    let value = ev.target().value();
                    bindings
                        .update(|values| {
                            values.insert(key_for_write.clone(), value);
                        });
                }
            />
        </label>
    }
    .into_any()
}

// ── run ─────────────────────────────────────────────────────────────────────

/// The Run control: dispatches the resolved `(name, version, parameters)` at the
/// first page, and stays disabled while the version text does not fit the chosen
/// resolution form (so a malformed pattern never reaches the CDR).
fn run_section(
    name: &str,
    mode: RwSignal<VersionResolution>,
    version: RwSignal<String>,
    bindings: RwSignal<BTreeMap<String, String>>,
    definition: LoadedQueryResource,
    ran: RwSignal<Option<StoredRun>>,
    offset: RwSignal<u32>,
) -> AnyView {
    let name = name.to_owned();
    let name_for_click = name.clone();
    let disabled = Signal::derive(move || {
        name.is_empty() || resolve_version(mode.get(), &version.get()).is_err()
    });
    let on_click = move |_| {
        // The button is disabled while this is an `Err`, so the guard is the
        // belt to that suspenders — a malformed pattern never reaches the CDR.
        let Ok(resolved) = resolve_version(mode.get_untracked(), &version.get_untracked()) else {
            return;
        };
        let parameters = bindings.with_untracked(|values| parameters_json(values).to_string());
        // Whether the request may carry its own row window: a stored definition
        // with an AQL `LIMIT`/`TOP` windows itself, and the two cannot be
        // combined. Read UNTRACKED in the event handler — never at render — so
        // no render-time code reads the definition resource (hydration rule);
        // a definition still in flight defaults to paged, matching the runner's
        // behaviour for a query it knows nothing about.
        let paged = match definition.get_untracked() {
            Some(Ok(Some((_, _, aql)))) => !carries_own_window(&aql),
            _ => true,
        };
        ran.set(Some((
            name_for_click.clone(),
            resolved.unwrap_or_default(),
            parameters,
            paged,
        )));
        offset.set(0);
    };
    view! {
        <section class=CARD_PAD>
            <button
                type="button"
                id="stored-run"
                class=BTN_PRIMARY
                disabled=disabled
                on:click=on_click
            >
                "Run"
            </button>
        </section>
    }
    .into_any()
}

// ── results ─────────────────────────────────────────────────────────────────

/// The `RESULT_SET` page in the shared results pane, with prev/next paging when
/// the request owns the row window. A pure read: a CDR refusal (a `404` for an
/// unmatched version, a `400` for a missing parameter) renders inline where the
/// rows would be.
fn results_section(
    results: Resource<Result<Option<ResultPage>, ViewerError>>,
    offset: RwSignal<u32>,
    ran: RwSignal<Option<StoredRun>>,
) -> AnyView {
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                let show_paging = ran.with(|run| run.as_ref().is_none_or(|(.., paged)| *paged));
                match results.await {
                    Ok(None) => ().into_any(),
                    Ok(Some(page)) => {
                        let controls = if show_paging {
                            paging_buttons(offset, page.rows.len())
                        } else {
                            view! {
                                <p class="mt-3 text-xs text-ink-muted">
                                    "This query sets its own row window (AQL LIMIT/TOP), so the run is not paged."
                                </p>
                            }
                                .into_any()
                        };
                        let body = results_view(&page, false);
                        view! {
                            <section class=CARD_PAD data-stored-results="">
                                <h2 class=CARD_TITLE>"Results"</h2>
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

#[cfg(test)]
mod tests {
    use crate::pages::query_stored::run_href;
    use crate::query_namespace::split_query_ref;

    #[test]
    fn run_href_targets_the_runner_route_with_the_encoded_reference() {
        assert_eq!(
            run_href("org.example::vitals", "1.0.0"),
            "/queries/stored?load=org.example%3A%3Avitals%401.0.0"
        );
    }

    #[test]
    fn run_href_round_trips_back_through_split_query_ref() {
        let href = run_href("org.example::c/name/value", "1.2.3");
        let value = href
            .strip_prefix("/queries/stored?load=")
            .expect("the helper always emits /queries/stored?load=<value>");
        let decoded = urlencoding::decode(value).expect("valid UTF-8 percent-encoding");
        assert_eq!(
            split_query_ref(&decoded),
            Some(("org.example::c/name/value".to_owned(), "1.2.3".to_owned()))
        );
    }
}
