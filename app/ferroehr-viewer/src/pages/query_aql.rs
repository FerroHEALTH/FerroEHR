// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/queries/aql` screen: the raw AQL editor.
//!
//! A plain-text AQL editor with BFF-local validation, an optional JSON
//! parameter-bindings pane, a paged result table, and a save-as-stored-query
//! row. Seeds its editor from the `?aql=` query parameter (the "open in raw
//! editor" hand-off from the point-and-click builder), which the router decodes
//! for us. No openEHR spec governs the viewer — our own design / product
//! extension; the wire it drives IS spec-bound (ITS-REST Query/Definition APIs).
//!
//! No new `#[server]` fn: the screen reuses [`validate_aql`], [`run_aql`] and
//! [`store_query`], each of which guards its own session, and it reuses the
//! shared render helpers from [`crate::pages::query_builder`] and
//! [`crate::pages::ehrs`].

use leptos::component;
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_query_map;

use crate::components::data_table::table_skeleton;
use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, LABEL, TEXTAREA};
use crate::components::page_header::{Crumb, PageHeader};
use crate::components::surface::CARD_PAD;
use crate::components::toast::toast_success;
use crate::error::ViewerError;
use crate::pages::ehrs::ResultPage;
use crate::pages::query_builder::{
    SaveAction, SaveFields, export_forms, paging_buttons, results_view, save_as_fields,
};
use crate::queries_api::{fetch_stored_query, run_aql, store_query, validate_aql};
use crate::query_namespace::{next_minor, qualify, split_qualified, split_query_ref};

/// One stored query as `?load=` fetched it: its qualified name, the version it
/// was read at, and its AQL. Named because the resource, its status section,
/// and the seeding effect all speak it — and because the version is a
/// first-class member of it, not an afterthought that can be dropped.
pub(crate) type LoadedQuery = (String, String, String);

/// The resource shape [`loaded_query_resource`] returns — named so consumers
/// (the stored-query runner's dispatch path) can take it as a parameter.
pub(crate) type LoadedQueryResource = Resource<Result<Option<LoadedQuery>, ViewerError>>;

/// Build the link INTO this screen that pre-fills the editor with `aql`.
///
/// The AQL becomes one query-string value, percent-encoded with the
/// `urlencoding` crate. AQL is full of URL-reserved characters — a space, the
/// `/` in every path, `#`, `&`, `=`, `'`, `%` — any one of which would truncate
/// the value or split it into extra parameters. The router percent-DEcodes
/// query params (`ParamsMap::insert` → `Url::unescape`), which is why
/// [`QueryAqlPage`]'s `?aql=` read needs no decode of its own. NOTE: no openEHR
/// spec governs the viewer's internal links — our own design/extension.
pub(crate) fn aql_href(aql: &str) -> String {
    format!("/queries/aql?aql={}", urlencoding::encode(aql))
}

/// Build the link INTO this screen that seeds the editor from the stored query
/// `name@version` — the `?load=` hand-off, encoded as one query-string value by
/// [`crate::query_namespace::load_href`] and parsed back by
/// [`split_query_ref`].
pub(crate) fn load_href(name: &str, version: &str) -> String {
    crate::query_namespace::load_href("/queries/aql", name, version)
}

/// The raw AQL editor screen.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn QueryAqlPage() -> impl IntoView {
    let query_map = use_query_map();
    // Deterministic from the URL (decoded by the router), so hydration-safe: the
    // server pass and the client hydration read the same `?aql=` value.
    let initial_aql = query_map.with_untracked(|m| m.get("aql").unwrap_or_default());
    let aql = RwSignal::new(initial_aql);
    let params = RwSignal::new(String::new());
    // The two halves of the stored query's qualified name (`namespace::name`,
    // the namespace optional — see `crate::query_namespace`).
    let save_namespace = RwSignal::new(String::new());
    let save_name = RwSignal::new(String::new());
    // The store version (empty = server-assigned). Seeding it from a loaded
    // query is what makes "load, edit, save" produce a NEW version instead of
    // colliding with the immutable one it came from.
    let save_version = RwSignal::new(String::new());
    let save_fields = SaveFields {
        namespace: save_namespace,
        name: save_name,
        version: save_version,
    };
    let offset = RwSignal::new(0_u32);
    let ran = RwSignal::new(None::<(String, String)>);

    // The "open in editor" hand-off from /queries: `?load=name@version` fetches
    // the stored query and seeds the editor. `load` is URL-derived, identical on
    // the server pass and the client hydration (hydration-safe), so the status
    // section only renders when the param is actually present.
    let has_load = query_map.with_untracked(|m| m.get("load").is_some_and(|s| !s.is_empty()));
    let load_resource = loaded_query_resource(query_map);
    seed_editor_from_loaded_query(load_resource, aql, save_fields);

    let validate_action: Action<String, Result<(), ViewerError>> = Action::new(|aql: &String| {
        let aql = aql.clone();
        async move { validate_aql(aql).await }
    });
    let save_action: SaveAction = Action::new(|input: &crate::pages::query_builder::SaveInput| {
        let (name, version, aql) = input.clone();
        async move { store_query(name, version, aql).await }
    });
    // Both outcomes toast (rules: Effect = sync with the outside world; no
    // signal is written — and the viewer's mutation-feedback rule). The
    // CDR's diagnostic ALSO stays inline (save_feedback), beside the AQL
    // it rejected.
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
    let results: Resource<Result<Option<ResultPage>, ViewerError>> = Resource::new(
        move || (ran.get(), offset.get()),
        |(ran, off)| async move {
            match ran {
                Some((aql, parameters)) => run_aql(aql, parameters, off).await.map(Some),
                None => Ok(None),
            }
        },
    );

    let load_status = if has_load {
        load_status_section(load_resource)
    } else {
        ().into_any()
    };
    let editor = editor_section(aql, validate_action);
    let parameters = parameters_section(params);
    let run_save = run_save_section(aql, params, save_fields, save_action, ran, offset);
    // Export tracks the editor + parameter panes — the same signals Run uses.
    let export_aql = Signal::derive(move || aql.get());
    let export_params = Signal::derive(move || params.get());
    let results_pane = results_section(results, offset, export_aql, export_params);

    view! {
        <Title text="AQL editor" />
        <div class="p-6 space-y-4">
            <PageHeader
                title="Raw AQL"
                subtitle="Write and run AQL directly, bind parameters, and save it as a stored query."
                crumbs=vec![Crumb::new("Queries", "/queries")]
            />
            {load_status}
            {editor}
            {parameters}
            {run_save}
            {results_pane}
        </div>
    }
}

/// The `?load=name@version` fetch: the stored query the hand-off from
/// `/queries` names, or `None` when the parameter is absent or malformed.
///
/// The version is CARRIED through, never dropped — re-saving a loaded query has
/// to be able to address a version, and the version it came from is what the
/// proposed next one is derived from
/// ([`seed_editor_from_loaded_query`]).
pub(crate) fn loaded_query_resource(
    query_map: Memo<leptos_router::params::ParamsMap>,
) -> Resource<Result<Option<LoadedQuery>, ViewerError>> {
    Resource::new(
        move || query_map.with(|m| m.get("load").unwrap_or_default()),
        |load| async move {
            match split_query_ref(&load) {
                Some((name, version)) => {
                    let text = fetch_stored_query(name.clone(), version.clone()).await?;
                    Ok(Some((name, version, text)))
                }
                None => Ok(None),
            }
        },
    )
}

/// Seed the editor and the save fields from a loaded stored query, exactly once
/// and client-side.
///
/// A resource-reading `Effect`, kept deliberately: the targets — the editor
/// `aql` and the save fields —
/// render in ALWAYS-MOUNTED sections outside the load `<Transition>`, so a
/// seed inside that `Suspend` writes them during the server pass and again
/// during hydration replay, changing already-serialized reactive text mid-walk
/// — reproduced live as tachys' unrecoverable-hydration panic on
/// `/queries/aql?load=…`. An `Effect` runs only after hydration completes, so
/// the seed can never diverge the two passes. The one-shot `StoredValue` guard
/// keeps it from re-firing, and the editor is only filled while still
/// untouched, so back-navigation never clobbers edits in progress.
///
/// The qualified name is SPLIT back into namespace + bare name (which is what
/// pre-fills the namespace field), and the version field is seeded with the
/// NEXT patch after the loaded version — the loaded `(name, version)` pair is
/// immutable on the CDR, so re-storing the same triple can only 409; proposing
/// the successor makes the common "load, tweak, save" loop one keystroke. A
/// version that is not a bumpable triple leaves the field empty — the
/// server-assigned store is then the honest default, and the field's own note
/// says so.
fn seed_editor_from_loaded_query(
    load_resource: Resource<Result<Option<LoadedQuery>, ViewerError>>,
    aql: RwSignal<String>,
    fields: SaveFields,
) {
    let seeded = StoredValue::new(false);
    Effect::new(move |_| {
        if seeded.get_value() {
            return;
        }
        if let Some(Ok(Some((qualified, version, text)))) = load_resource.get() {
            if aql.with_untracked(String::is_empty) {
                aql.set(text);
            }
            let (namespace, name) = split_qualified(&qualified);
            fields.namespace.set(namespace);
            fields.name.set(name);
            fields.version.set(next_minor(&version).unwrap_or_default());
            seeded.set_value(true);
        }
    });
}

/// The `?load=` hand-off status: a house-pattern `<Transition>` reporting the
/// loaded query (or the CDR error inline). The editor itself is seeded by the
/// one-shot effect in the page component; this only surfaces load progress.
fn load_status_section(
    load_resource: Resource<Result<Option<LoadedQuery>, ViewerError>>,
) -> AnyView {
    view! {
        <Transition fallback=move || {
            view! { <p class="text-sm text-ink-muted">"Loading stored query…"</p> }
        }>
            {move || Suspend::new(async move {
                match load_resource.await {
                    Ok(Some((qualified, version, _))) => {
                        let builder_href = crate::pages::query_builder::load_href(
                            &qualified,
                            &version,
                        );
                        let run_href = crate::pages::query_stored::run_href(&qualified, &version);
                        view! {
                            <div class="space-y-1">
                                <p class="text-sm text-ink-muted">
                                    "Loaded stored query "
                                    <span class="font-mono text-ink">{qualified}</span>
                                    " at version " <span class="font-mono text-ink">{version}</span>
                                    ". Saving stores the version in the field below — that version is "
                                    "immutable, so the next one is proposed."
                                </p>
                                <div class="flex flex-wrap items-center gap-3 text-sm">
                                    <a
                                        href=builder_href
                                        class="text-accent hover:underline"
                                        data-open-in-builder=""
                                    >
                                        "Open in builder"
                                    </a>
                                    <a
                                        href=run_href
                                        class="text-accent hover:underline"
                                        data-run-stored=""
                                    >
                                        "Run with parameters"
                                    </a>
                                </div>
                            </div>
                        }
                            .into_any()
                    }
                    Ok(None) => ().into_any(),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The AQL editor: a controlled `<textarea>` plus a BFF-local Validate button
/// and its diagnostic.
fn editor_section(
    aql: RwSignal<String>,
    validate_action: Action<String, Result<(), ViewerError>>,
) -> AnyView {
    let validate_click = move |_| {
        validate_action.dispatch(aql.get_untracked());
    };
    view! {
        <section class=CARD_PAD>
            <div class="space-y-2">
                <label class=LABEL r#for="aql-editor">
                    "AQL"
                </label>
                <textarea
                    id="aql-editor"
                    class=format!("{TEXTAREA} h-40")
                    placeholder="SELECT c FROM EHR e CONTAINS COMPOSITION c"
                    prop:value=move || aql.get()
                    on:input:target=move |ev| aql.set(ev.target().value())
                >
                    {aql.get_untracked()}
                </textarea>
                <div class="flex items-center gap-3">
                    <button
                        type="button"
                        class=BTN_SECONDARY
                        disabled=Signal::derive(move || aql.with(String::is_empty))
                        on:click=validate_click
                    >
                        "Validate"
                    </button>
                    {validate_feedback(validate_action)}
                </div>
            </div>
        </section>
    }
    .into_any()
}

/// The Validate action's inline result (parser diagnostic on failure).
fn validate_feedback(validate_action: Action<String, Result<(), ViewerError>>) -> AnyView {
    view! {
        <div class="text-sm">
            <Show when=move || validate_action.pending().get()>
                <span class="text-ink-muted">"Validating…"</span>
            </Show>
            {move || match validate_action.value().get() {
                Some(Ok(())) => {
                    view! {
                        <span class="rounded-control bg-ok-subtle px-2 py-0.5 text-ok">
                            "AQL is valid."
                        </span>
                    }
                        .into_any()
                }
                Some(Err(error)) => crate::components::notice::inline_error(&error),
                None => ().into_any(),
            }}
        </div>
    }
    .into_any()
}

/// The parameter-bindings pane: a JSON object bound as AQL `query_parameters`.
fn parameters_section(params: RwSignal<String>) -> AnyView {
    view! {
        <section class=CARD_PAD>
            <div class="space-y-2">
                <label class=LABEL r#for="aql-params">
                    "Parameters (JSON object)"
                </label>
                <textarea
                    id="aql-params"
                    class=format!("{TEXTAREA} h-24")
                    placeholder="{\"ehr_id\": \"...\"}"
                    prop:value=move || params.get()
                    on:input:target=move |ev| params.set(ev.target().value())
                >
                    {params.get_untracked()}
                </textarea>
            </div>
        </section>
    }
    .into_any()
}

/// The Run + Save surface: Run executes the AQL with the parameter bindings at
/// the first page; Save stores it under the qualified name composed from the
/// namespace + name fields ([`save_as_fields`]).
fn run_save_section(
    aql: RwSignal<String>,
    params: RwSignal<String>,
    fields: SaveFields,
    save_action: SaveAction,
    ran: RwSignal<Option<(String, String)>>,
    offset: RwSignal<u32>,
) -> AnyView {
    let empty_aql = Signal::derive(move || aql.with(String::is_empty));
    // The namespace stays optional (the spec makes it optional), so only the
    // AQL and the query name gate the Save button.
    let save_disabled = Signal::derive(move || {
        aql.with(String::is_empty)
            || fields.name.with(String::is_empty)
            || fields.version_is_unstorable()
    });
    let run_click = move |_| {
        ran.set(Some((aql.get_untracked(), params.get_untracked())));
        offset.set(0);
    };
    let save_click = move |_| {
        save_action.dispatch((
            qualify(
                &fields.namespace.get_untracked(),
                &fields.name.get_untracked(),
            ),
            fields.version_arg(),
            aql.get_untracked(),
        ));
    };
    let save_fields = save_as_fields("aql", fields);
    view! {
        <section class=CARD_PAD>
            <div class="space-y-3">
                <div class="flex flex-wrap items-end gap-3">
                    <button type="button" class=BTN_PRIMARY disabled=empty_aql on:click=run_click>
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
                </div>
                {save_feedback(save_action)}
            </div>
        </section>
    }
    .into_any()
}

/// The Save action's inline feedback: a pending hint and the CDR error verbatim.
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

/// The result table under a `<Transition>`, with local prev/next paging.
fn results_section(
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
                        let controls = paging_buttons(offset, page.rows.len());
                        let body = results_view(&page, false);
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

#[cfg(test)]
mod tests {
    use crate::pages::query_aql::{aql_href, load_href};
    use crate::query_namespace::split_query_ref;

    #[test]
    fn load_href_leaves_an_unreserved_qualified_ref_alone() {
        assert_eq!(
            load_href("my_query", "1.0.0"),
            "/queries/aql?load=my_query%401.0.0"
        );
    }

    #[test]
    fn load_href_escapes_the_qualified_ref_as_one_value() {
        // A qualified stored-query name carries `/` and `::`.
        assert_eq!(
            load_href("org.example::c/name/value", "1.2.3"),
            "/queries/aql?load=org.example%3A%3Ac%2Fname%2Fvalue%401.2.3"
        );
        // A `&` or `=` in the name must not become an extra parameter.
        assert_eq!(load_href("a&b=c", "1"), "/queries/aql?load=a%26b%3Dc%401");
    }

    #[test]
    fn load_href_round_trips_back_through_split_query_ref() {
        // The router decodes `?load=` before this screen reads it, so decoding
        // the emitted value must hand `split_query_ref` the original pair.
        for (name, version) in [
            ("my_query", "1.0.0"),
            ("org.example::c/name/value", "1.2.3"),
            ("a&b=c", "1"),
            ("blodtryk_målinger", "2.0.0"),
        ] {
            let href = load_href(name, version);
            let value = href
                .strip_prefix("/queries/aql?load=")
                .expect("the builder always emits /queries/aql?load=<value>");
            let decoded = urlencoding::decode(value).expect("valid UTF-8 percent-encoding");
            assert_eq!(
                split_query_ref(&decoded),
                Some((name.to_owned(), version.to_owned()))
            );
        }
    }

    #[test]
    fn aql_href_escapes_every_reserved_character_in_the_query_text() {
        assert_eq!(aql_href("Aa0-_.~"), "/queries/aql?aql=Aa0-_.~");
        // Space, ampersand, equals, percent, hash, question mark, plus, slash.
        assert_eq!(
            aql_href(" &=%#?+/"),
            "/queries/aql?aql=%20%26%3D%25%23%3F%2B%2F"
        );
        // Multi-byte UTF-8 escapes per byte, uppercase hex.
        assert_eq!(aql_href("é"), "/queries/aql?aql=%C3%A9");
    }

    #[test]
    fn aql_href_round_trips_a_real_query() {
        let aql = "SELECT c/uid/value FROM COMPOSITION c WHERE c/name/value = 'a b'";
        let href = aql_href(aql);
        assert_eq!(
            href,
            "/queries/aql?aql=SELECT%20c%2Fuid%2Fvalue%20FROM%20COMPOSITION%20c%20WHERE%20c%2Fname%2Fvalue%20%3D%20%27a%20b%27"
        );
        let value = href
            .strip_prefix("/queries/aql?aql=")
            .expect("the builder always emits /queries/aql?aql=<value>");
        // Nothing that could truncate the value or forge a second parameter.
        assert!(!value.contains(['?', '&', '=', '/', ' ', '#']));
        assert_eq!(
            urlencoding::decode(value).expect("valid UTF-8 percent-encoding"),
            aql
        );
    }
}
