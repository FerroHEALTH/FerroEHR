// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/terminology` screen — the terminology browser.
//!
//! One page over the CDR's terminology surface ([`crate::terminology`]): the
//! terminologies it serves and the selected one's descriptor, a term lookup, a
//! strict subsumption test, and a value set's members plus a membership test.
//!
//! NOTE: no openEHR spec governs the viewer — our own design / product
//! extension; the wire it reads is the CDR's own extension realizing SM
//! `I_TERMINOLOGY_SERVICE`
//! (`docs/specs/openehr/SM/docs/openehr_platform/master12-terminology_service.adoc`).
//!
//! Two shapes carry the screen. The **selection** is URL state
//! (`?terminology=…`), and the two `Resource`s it drives are created in
//! component SETUP — never inside a `Suspend`, which re-runs and would re-create
//! them. The three **lookups** are `Action`s: each writes its own answer in the
//! action's own async continuation.
//!
//! Everything here is a READ, so failures render inline and nothing toasts. A
//! `404` is never an error: the CDR answers it both for an absent
//! terminology/code/value set and for the whole extension being switched off, so
//! an absent answer is rendered as the state it is.

use leptos::component;
use leptos::prelude::*;
use leptos_meta::Title;

use crate::components::data_table::{CELL, CELL_MONO, ROW, table_shell, table_skeleton};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, INPUT, LABEL};
use crate::components::notice::inline_error;
use crate::components::page_header::PageHeader;
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::error::ViewerError;
use crate::terminology::{
    TerminologyDescriptor, TerminologyExtractView, check_subsumption, fetch_term,
    fetch_terminology_description, fetch_value_set, list_terminologies, validate_value_set_code,
};

/// The href that selects `terminology_id` on this screen.
///
/// The id is percent-encoded — a terminology id carrying `&`, `#` or `%`
/// would otherwise forge a second query parameter.
#[must_use]
pub fn select_href(terminology_id: &str) -> String {
    format!(
        "/terminology?terminology={}",
        urlencoding::encode(terminology_id)
    )
}

/// The terminology browser screen.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn TerminologyPage() -> impl IntoView {
    let query = leptos_router::hooks::use_query_map();
    // Reactive: selecting a terminology only changes the query string, so the
    // component body does NOT re-run — the descriptor resource has to track it.
    let selected = Signal::derive(move || {
        query
            .with(|q| q.get("terminology").unwrap_or_default())
            .trim()
            .to_owned()
    });
    let terminologies: Resource<Result<Option<Vec<String>>, ViewerError>> =
        Resource::new(|| (), |()| async move { list_terminologies().await });
    let descriptor: Resource<Result<Option<TerminologyDescriptor>, ViewerError>> = Resource::new(
        move || selected.get(),
        |id| async move {
            if id.is_empty() {
                Ok(None)
            } else {
                fetch_terminology_description(id).await
            }
        },
    );

    let body = browser_section(terminologies, descriptor, selected);

    view! {
        <Title text="Terminology" />
        <div id="terminology-screen" class="p-6">
            <PageHeader
                title="Terminology"
                subtitle="Browse the terminologies the CDR serves: describe one, define a code, expand a value set, and test membership or subsumption."
            />
            {body}
        </div>
    }
}

/// The whole screen body under ONE `<Transition>` over the terminology list.
///
/// The list is what decides whether the surface exists at all, so the disabled
/// state can be exactly what the requirement asks for — one empty-state card
/// instead of a page of cards that cannot work. The awaited resource's source
/// is `()`, so this `Suspend` resolves once and never re-runs; the descriptor
/// has its own nested `<Transition>` inside, and neither closure CREATES a
/// resource.
fn browser_section(
    terminologies: Resource<Result<Option<Vec<String>>, ViewerError>>,
    descriptor: Resource<Result<Option<TerminologyDescriptor>, ViewerError>>,
    selected: Signal<String>,
) -> AnyView {
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match terminologies.await {
                    Err(error) => inline_error(&error),
                    Ok(None) => disabled_card(),
                    Ok(Some(ids)) => enabled_body(ids, descriptor, selected),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The whole screen when the CDR does not serve the terminology surface.
///
/// A `404` on `GET /terminology` means the routes answer as if unmounted —
/// which is what `[terminology] api_enabled = false` does — so the honest
/// screen is one card naming the switch, never a broken browser.
fn disabled_card() -> AnyView {
    view! {
        <section id="terminology-disabled" class=CARD_PAD>
            <EmptyState
                icon=icondata_lu::LuBookX
                message="The terminology extension is disabled on this server"
                hint="The CDR answers its terminology routes as if unmounted. Set api_enabled = true under [terminology] in the CDR's configuration to browse terminologies, terms and value sets here."
            />
        </section>
    }
    .into_any()
}

/// The browser proper: the terminology list beside the selected descriptor,
/// then the three lookup cards.
fn enabled_body(
    ids: Vec<String>,
    descriptor: Resource<Result<Option<TerminologyDescriptor>, ViewerError>>,
    selected: Signal<String>,
) -> AnyView {
    let list = list_card(ids, selected);
    let facts = descriptor_card(descriptor, selected);
    let term = term_card(selected);
    let subsumption = subsumption_card(selected);
    let value_set = value_set_card(selected);
    view! {
        <div class="space-y-4">
            <div class="grid grid-cols-1 lg:grid-cols-3 gap-4 items-start">
                <section class=CARD_PAD>
                    <h2 class=CARD_TITLE>"Terminologies"</h2>
                    {list}
                </section>
                <div class="lg:col-span-2">{facts}</div>
            </div>
            {term}
            {subsumption}
            {value_set}
        </div>
    }
    .into_any()
}

/// The terminology list: one link per id, the selected one marked.
///
/// Plain anchors, not the router's `<A>`: the router intercepts every
/// same-origin anchor once hydrated, and before the WASM bundle loads the
/// browser follows the same href as an ordinary GET — so selecting a
/// terminology never depends on JavaScript being live.
fn list_card(ids: Vec<String>, selected: Signal<String>) -> AnyView {
    if ids.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuBookX
                message="No terminologies"
                hint="The CDR serves the terminology API but reports no terminology in it."
            />
        }
        .into_any();
    }
    let rows = ids
        .into_iter()
        .map(|id| {
            let href = select_href(&id);
            let hook = id.clone();
            let label = id.clone();
            // A derived `Signal<bool>` rather than a closure: the row reads it
            // from three attribute positions, and only a `Copy` handle can be
            // moved into all three.
            let is_selected = Signal::derive(move || selected.get() == id);
            view! {
                <li>
                    <a
                        href=href
                        data-terminology-id=hook
                        aria-current=move || if is_selected.get() { Some("true") } else { None }
                        class="block rounded-control px-2 py-1 text-sm font-mono"
                        class=(["bg-accent-subtle", "text-accent-ink"], is_selected)
                        class=(["text-ink", "hover:bg-sunken"], move || !is_selected.get())
                    >
                        {label}
                    </a>
                </li>
            }
        })
        .collect::<Vec<_>>();
    view! { <ul class="flex flex-col gap-0.5">{rows}</ul> }.into_any()
}

/// The selected terminology's descriptor card.
fn descriptor_card(
    descriptor: Resource<Result<Option<TerminologyDescriptor>, ViewerError>>,
    selected: Signal<String>,
) -> AnyView {
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Descriptor"</h2>
            // <Transition>, not <Suspense>: switching terminology refetches,
            // and the previous descriptor stays visible instead of flashing a
            // skeleton.
            <Transition fallback=table_skeleton>
                {move || Suspend::new(async move {
                    match descriptor.await {
                        Err(error) => inline_error(&error),
                        Ok(None) if selected.get_untracked().is_empty() => {
                            view! {
                                <p class="text-sm text-ink-muted">
                                    "Pick a terminology on the left to describe it."
                                </p>
                            }
                                .into_any()
                        }
                        Ok(None) => {
                            let id = selected.get_untracked();
                            view! {
                                <p class="text-sm text-ink-muted">
                                    "The CDR serves no terminology called "
                                    <span class="font-mono text-ink">{id}</span> "."
                                </p>
                            }
                                .into_any()
                        }
                        Ok(Some(descriptor)) => descriptor_facts(&descriptor),
                    }
                })}
            </Transition>
        </section>
    }
    .into_any()
}

/// One descriptor's facts, each rendered only when the CDR published it.
fn descriptor_facts(descriptor: &TerminologyDescriptor) -> AnyView {
    let name = (!descriptor.name.is_empty()).then(|| fact_row("Name", descriptor.name.clone()));
    let publisher = (!descriptor.publisher.is_empty())
        .then(|| fact_row("Publisher", descriptor.publisher.clone()));
    let uri = (!descriptor.uri.is_empty()).then(|| fact_row("URI", descriptor.uri.clone()));
    let versions = (!descriptor.available_versions.is_empty()).then(|| {
        fact_row(
            "Available versions",
            descriptor.available_versions.join(", "),
        )
    });
    let attributes = (!descriptor.attributes.is_empty())
        .then(|| fact_row("Attributes", descriptor.attributes.join(", ")));
    let id = descriptor.terminology_id.clone();
    view! {
        <div id="terminology-descriptor" class="flex flex-col gap-1">
            {fact_row("Terminology", id)}
            {name}
            {publisher}
            {uri}
            {versions}
            {attributes}
        </div>
    }
    .into_any()
}

/// One label/value row of the descriptor card.
fn fact_row(label: &'static str, value: String) -> AnyView {
    view! {
        <div class="grid grid-cols-1 gap-x-3 sm:grid-cols-[minmax(8rem,14rem)_1fr]">
            <span class="text-xs text-ink-muted">{label}</span>
            <span class="break-words text-sm text-ink">{value}</span>
        </div>
    }
    .into_any()
}

/// The term-lookup card: a code plus an optional effective date, answered by
/// the extract's terms (and relationships, when the CDR sent any) or by an
/// inline "unknown code" note.
fn term_card(selected: Signal<String>) -> AnyView {
    // UNCONTROLLED inputs read at dispatch: a controlled input resets to
    // its empty signal at hydration, wiping pre-WASM typing.
    let code_ref = NodeRef::<leptos::html::Input>::new();
    let at_date_ref = NodeRef::<leptos::html::Input>::new();
    let validation = RwSignal::new(Option::<String>::None);
    let lookup = Action::new(|(terminology, code, at_date): &(String, String, String)| {
        let terminology = terminology.clone();
        let code = code.clone();
        let at_date = at_date.clone();
        async move { fetch_term(terminology, code, at_date).await }
    });
    let on_click = move |_| {
        let terminology = selected.get_untracked();
        let code = input_value(code_ref);
        let at_date = input_value(at_date_ref);
        if let Some(message) = missing_selection(&terminology) {
            validation.set(Some(message));
            return;
        }
        if code.is_empty() {
            validation.set(Some("Type a term code to define.".to_owned()));
            return;
        }
        validation.set(None);
        lookup.dispatch((terminology, code, at_date));
    };
    let outcome = move || match lookup.value().get() {
        None => ().into_any(),
        Some(Err(error)) => inline_error(&error),
        Some(Ok(None)) => {
            let (terminology, code) = lookup
                .input()
                .get()
                .map(|(terminology, code, _)| (terminology, code))
                .unwrap_or_default();
            view! {
                <p id="terminology-term-absent" class="text-sm text-ink-muted">
                    "Unknown code "
                    <span class="font-mono text-ink">{code}</span>
                    " in "
                    <span class="font-mono text-ink">{terminology}</span>
                    "."
                </p>
            }
            .into_any()
        }
        Some(Ok(Some(extract))) => extract_panel(&extract, "term"),
    };

    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Define a term"</h2>
            <div class="flex flex-wrap items-end gap-3">
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="terminology-code">
                        "Term code"
                    </label>
                    <input
                        id="terminology-code"
                        type="text"
                        class=INPUT
                        placeholder="249"
                        node_ref=code_ref
                    />
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="terminology-at-date">
                        "Effective date (optional)"
                    </label>
                    <input
                        id="terminology-at-date"
                        type="text"
                        class=INPUT
                        placeholder="2026-01-01"
                        node_ref=at_date_ref
                    />
                </div>
                <button
                    id="terminology-term-lookup"
                    type="button"
                    class=BTN_PRIMARY
                    disabled=Signal::derive(move || lookup.pending().get())
                    on:click=on_click
                >
                    "Define"
                </button>
            </div>
            <div id="terminology-term-result" class="mt-3 text-sm">
                <Show when=move || lookup.pending().get()>
                    <span class="text-ink-muted">"Looking up…"</span>
                </Show>
                {move || validation.get().map(validation_note)}
                {outcome}
            </div>
        </section>
    }
    .into_any()
}

/// The subsumption card: two codes, one sentence.
#[expect(
    clippy::too_many_lines,
    reason = "one erased section: the two code fields, their validation and the verdict (rules §1)"
)]
fn subsumption_card(selected: Signal<String>) -> AnyView {
    let ref_ref = NodeRef::<leptos::html::Input>::new();
    let candidate_ref = NodeRef::<leptos::html::Input>::new();
    let validation = RwSignal::new(Option::<String>::None);
    let test = Action::new(
        |(terminology, reference, candidate): &(String, String, String)| {
            let terminology = terminology.clone();
            let reference = reference.clone();
            let candidate = candidate.clone();
            async move { check_subsumption(terminology, reference, candidate).await }
        },
    );
    let on_click = move |_| {
        let terminology = selected.get_untracked();
        let reference = input_value(ref_ref);
        let candidate = input_value(candidate_ref);
        if let Some(message) = missing_selection(&terminology) {
            validation.set(Some(message));
            return;
        }
        if reference.is_empty() || candidate.is_empty() {
            validation.set(Some(
                "Both a reference code and a candidate code are needed.".to_owned(),
            ));
            return;
        }
        validation.set(None);
        test.dispatch((terminology, reference, candidate));
    };
    let outcome = move || match test.value().get() {
        None => ().into_any(),
        Some(Err(error)) => inline_error(&error),
        Some(Ok(None)) => absent_terminology_note(test.input().get().map(|(id, _, _)| id)),
        Some(Ok(Some(subsumes))) => {
            let (reference, candidate) = test
                .input()
                .get()
                .map(|(_, reference, candidate)| (reference, candidate))
                .unwrap_or_default();
            let verb = if subsumes {
                " subsumes "
            } else {
                " does not subsume "
            };
            view! {
                <p id="terminology-subsumes-verdict" class="text-sm text-ink">
                    <span class="font-mono">{reference}</span>
                    {verb}
                    <span class="font-mono">{candidate}</span>
                    "."
                </p>
            }
            .into_any()
        }
    };

    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Test subsumption"</h2>
            <div class="flex flex-wrap items-end gap-3">
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="terminology-subsumes-ref">
                        "Reference code"
                    </label>
                    <input
                        id="terminology-subsumes-ref"
                        type="text"
                        class=INPUT
                        placeholder="249"
                        node_ref=ref_ref
                    />
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="terminology-subsumes-candidate">
                        "Candidate code"
                    </label>
                    <input
                        id="terminology-subsumes-candidate"
                        type="text"
                        class=INPUT
                        placeholder="532"
                        node_ref=candidate_ref
                    />
                </div>
                <button
                    id="terminology-subsumes-run"
                    type="button"
                    class=BTN_SECONDARY
                    disabled=Signal::derive(move || test.pending().get())
                    on:click=on_click
                >
                    "Test"
                </button>
            </div>
            <p class="mt-2 text-xs text-ink-muted">
                "Subsumption is strict: a code never subsumes itself."
            </p>
            <div class="mt-2 text-sm">
                <Show when=move || test.pending().get()>
                    <span class="text-ink-muted">"Testing…"</span>
                </Show>
                {move || validation.get().map(validation_note)}
                {outcome}
            </div>
        </section>
    }
    .into_any()
}

/// The refusal copy for an incomplete value-set form, or `None` when the form
/// is ready to dispatch.
///
/// `candidate` is `Some` only for the membership test, which needs both fields.
fn value_set_form_error(
    terminology: &str,
    value_set: &str,
    candidate: Option<&str>,
) -> Option<String> {
    if let Some(message) = missing_selection(terminology) {
        return Some(message);
    }
    let Some(candidate) = candidate else {
        return value_set
            .is_empty()
            .then(|| "Type a value set id to expand.".to_owned());
    };
    (value_set.is_empty() || candidate.is_empty())
        .then(|| "Both a value set id and a candidate code are needed.".to_owned())
}

/// The value-set card: expand a value set's members, then test one code's
/// membership in it.
#[expect(
    clippy::too_many_lines,
    reason = "one erased section: the value set's expansion and its membership test share the id field (rules §1)"
)]
fn value_set_card(selected: Signal<String>) -> AnyView {
    let id_ref = NodeRef::<leptos::html::Input>::new();
    let candidate_ref = NodeRef::<leptos::html::Input>::new();
    let validation = RwSignal::new(Option::<String>::None);
    let expand = Action::new(|(terminology, value_set): &(String, String)| {
        let terminology = terminology.clone();
        let value_set = value_set.clone();
        async move { fetch_value_set(terminology, value_set).await }
    });
    let validate = Action::new(
        |(terminology, value_set, candidate): &(String, String, String)| {
            let terminology = terminology.clone();
            let value_set = value_set.clone();
            let candidate = candidate.clone();
            async move {
                validate_value_set_code(terminology, value_set, candidate, String::new()).await
            }
        },
    );
    let on_expand = move |_| {
        let terminology = selected.get_untracked();
        let value_set = input_value(id_ref);
        if let Some(message) = value_set_form_error(&terminology, &value_set, None) {
            validation.set(Some(message));
            return;
        }
        validation.set(None);
        expand.dispatch((terminology, value_set));
    };
    let on_validate = move |_| {
        let terminology = selected.get_untracked();
        let value_set = input_value(id_ref);
        let candidate = input_value(candidate_ref);
        if let Some(message) = value_set_form_error(&terminology, &value_set, Some(&candidate)) {
            validation.set(Some(message));
            return;
        }
        validation.set(None);
        validate.dispatch((terminology, value_set, candidate));
    };
    let members = move || match expand.value().get() {
        None => ().into_any(),
        Some(Err(error)) => inline_error(&error),
        Some(Ok(None)) => {
            let (terminology, value_set) = expand.input().get().unwrap_or_default();
            view! {
                <p id="terminology-value-set-absent" class="text-sm text-ink-muted">
                    "Unknown value set "
                    <span class="font-mono text-ink">{value_set}</span>
                    " in "
                    <span class="font-mono text-ink">{terminology}</span>
                    "."
                </p>
            }
            .into_any()
        }
        Some(Ok(Some(extract))) => extract_panel(&extract, "value-set-member"),
    };
    let verdict = move || match validate.value().get() {
        None => ().into_any(),
        Some(Err(error)) => inline_error(&error),
        Some(Ok(None)) => absent_terminology_note(validate.input().get().map(|(id, _, _)| id)),
        Some(Ok(Some(valid))) => {
            let (value_set, candidate) = validate
                .input()
                .get()
                .map(|(_, value_set, candidate)| (value_set, candidate))
                .unwrap_or_default();
            let verb = if valid {
                " is a member of "
            } else {
                " is not a member of "
            };
            view! {
                <p id="terminology-value-set-verdict" class="text-sm text-ink">
                    <span class="font-mono">{candidate}</span>
                    {verb}
                    <span class="font-mono">{value_set}</span>
                    "."
                </p>
            }
            .into_any()
        }
    };

    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Expand a value set"</h2>
            <div class="flex flex-wrap items-end gap-3">
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="terminology-value-set-id">
                        "Value set id"
                    </label>
                    <input
                        id="terminology-value-set-id"
                        type="text"
                        class=INPUT
                        placeholder="audit_change_type"
                        node_ref=id_ref
                    />
                </div>
                <button
                    id="terminology-value-set-expand"
                    type="button"
                    class=BTN_PRIMARY
                    disabled=Signal::derive(move || expand.pending().get())
                    on:click=on_expand
                >
                    "Expand"
                </button>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="terminology-value-set-candidate">
                        "Candidate code"
                    </label>
                    <input
                        id="terminology-value-set-candidate"
                        type="text"
                        class=INPUT
                        placeholder="249"
                        node_ref=candidate_ref
                    />
                </div>
                <button
                    id="terminology-value-set-validate"
                    type="button"
                    class=BTN_SECONDARY
                    disabled=Signal::derive(move || validate.pending().get())
                    on:click=on_validate
                >
                    "Validate"
                </button>
            </div>
            <div class="mt-3 text-sm">
                <Show when=move || expand.pending().get() || validate.pending().get()>
                    <span class="text-ink-muted">"Asking the CDR…"</span>
                </Show>
                {move || validation.get().map(validation_note)}
                {verdict}
                {members}
            </div>
        </section>
    }
    .into_any()
}

/// One extract rendered as its terms table plus, when the CDR sent any, its
/// relationships. `hook` names the per-row `data-*` attribute so a journey can
/// address a term row and a value-set member row distinctly.
fn extract_panel(extract: &TerminologyExtractView, hook: &'static str) -> AnyView {
    if extract.terms.is_empty() && extract.relationships.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuSearchX
                message="Nothing in this extract"
                hint="The CDR answered with an extract that carries no term and no relationship."
            />
        }
        .into_any();
    }
    let version = (!extract.version.is_empty()).then(|| {
        let version = extract.version.clone();
        view! { <p class="mb-2 text-xs text-ink-muted">"Terminology version " {version}</p> }
    });
    let terms = (!extract.terms.is_empty()).then(|| terms_table(extract, hook));
    let relationships = (!extract.relationships.is_empty()).then(|| relationships_table(extract));
    view! { <div class="space-y-3">{version}{terms}{relationships}</div> }.into_any()
}

/// The extract's terms as a table: one `code — text` rubric per row, the
/// language it is written in, and whether it is the preferred term among
/// alternatives.
fn terms_table(extract: &TerminologyExtractView, hook: &'static str) -> AnyView {
    let rows = extract
        .terms
        .iter()
        .map(|term| {
            let code_hook = term.code.clone();
            let rubric = term.rubric();
            let language = term.language.clone();
            let preferred = if term.preferred { "preferred" } else { "" };
            view! {
                <tr class=ROW>
                    <td class=CELL_MONO data-term-code=code_hook>
                        {rubric}
                    </td>
                    <td class=CELL>{language}</td>
                    <td class=CELL>{preferred}</td>
                </tr>
            }
        })
        .collect_view()
        .into_any();
    // `data-extract` names which lookup produced the table (a term definition
    // or a value-set expansion); each row's term cell carries `data-term-code`.
    let table = table_shell(&["Term", "Language", "Preferred"], rows);
    view! { <div data-extract=hook>{table}</div> }.into_any()
}

/// The extract's relationships as a table.
fn relationships_table(extract: &TerminologyExtractView) -> AnyView {
    let rows = extract
        .relationships
        .iter()
        .map(|relationship| {
            let origin = relationship.origin_code.clone();
            let name = relationship.relation_name.clone();
            let targets = relationship.target_codes.join(", ");
            view! {
                <tr class=ROW>
                    <td class=CELL_MONO>{origin}</td>
                    <td class=CELL>{name}</td>
                    <td class=CELL_MONO>{targets}</td>
                </tr>
            }
        })
        .collect_view()
        .into_any();
    table_shell(&["Origin", "Relation", "Targets"], rows)
}

/// The note for a lookup the CDR answered `404` on because it knows no such
/// terminology (the membership and subsumption tests share it).
fn absent_terminology_note(terminology: Option<String>) -> AnyView {
    let terminology = terminology.unwrap_or_default();
    view! {
        <p class="text-sm text-ink-muted">
            "The CDR serves no terminology called "
            <span class="font-mono text-ink">{terminology}</span> "."
        </p>
    }
    .into_any()
}

/// The inline client-side validation bar every card on this screen shares.
fn validation_note(message: String) -> AnyView {
    view! {
        <p
            role="alert"
            class="rounded-control border border-danger/40 bg-danger-subtle px-3 py-2 text-danger"
        >
            {message}
        </p>
    }
    .into_any()
}

/// The copy for "no terminology is selected yet", or `None` when one is.
fn missing_selection(terminology: &str) -> Option<String> {
    terminology
        .is_empty()
        .then(|| "Pick a terminology on the left first.".to_owned())
}

/// The trimmed current value of an uncontrolled input.
fn input_value(field: NodeRef<leptos::html::Input>) -> String {
    field
        .get_untracked()
        .map(|element| element.value())
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{missing_selection, select_href};

    #[test]
    fn the_selection_href_percent_encodes_the_id() {
        assert_eq!(select_href("openehr"), "/terminology?terminology=openehr");
        assert_eq!(
            select_href("ISO_639-1"),
            "/terminology?terminology=ISO_639-1"
        );
        // A hostile id can never forge a second parameter.
        assert_eq!(
            select_href("a&b=c#d"),
            "/terminology?terminology=a%26b%3Dc%23d"
        );
    }

    #[test]
    fn a_lookup_without_a_selected_terminology_says_so() {
        assert!(missing_selection("").is_some());
        assert_eq!(missing_selection("openehr"), None);
    }
}
