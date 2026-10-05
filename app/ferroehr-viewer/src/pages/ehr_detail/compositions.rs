// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail Compositions tab: the AQL-driven composition list, its
//! URL-borne row filters, and the "Commit composition" form.
//!
//! The filters (template, a context-start-time window, composer) live in the
//! query string — `?template=`/`?from=`/`?to=`/`?composer=`, submitted by a GET
//! `<Form>` — so a filtered view is shareable, refresh-safe and reproducible
//! before the WASM bundle loads. They are read in SETUP and feed the list
//! resource's source, so a filter change refetches exactly once and a
//! submission drops `?offset=`, putting the reader on the first page of the new
//! result set. The statement they produce is assembled by the pure,
//! component-free [`composition_filter`](super::composition_filter): operator
//! input is bound as AQL `query_parameters`, never concatenated into the query.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::prelude::*;
use leptos::server;
use leptos_router::components::A;
use leptos_router::params::ParamsMap;
use serde_json::Value;

#[cfg(feature = "ssr")]
use crate::uid::uid_value_of;

use crate::components::data_table::{CELL, CELL_MONO, ROW, table_shell, table_skeleton};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, INPUT, LABEL, SELECT, TEXTAREA};
use crate::components::format_view::PaneView;
use crate::components::surface::{CARD_PAD, CARD_TITLE, WELL};
use crate::components::toast::toast_success;
use crate::error::ViewerError;
use crate::format::ReprFormat;
use crate::pages::ehr_detail::composition_filter::CompositionFilter;
use crate::pages::ehrs::{ResultPage, cell_text, ehr_detail_href, paging_controls};
// Server-side helpers, compiled only where the #[server] bodies exist.
#[cfg(feature = "ssr")]
use crate::pages::ehr_detail::composition_filter::composition_query;
#[cfg(feature = "ssr")]
use crate::pages::ehrs::{aql_request_body, parse_result_set};

/// List an EHR's compositions newest-first, one page at `offset`, narrowed by
/// whichever filters are filled.
///
/// Every argument is operator input and every one of them travels as an AQL
/// `query_parameters` binding — see
/// [`composition_query`](super::composition_filter::composition_query), which
/// builds the statement and the bindings together.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when a date bound is neither a date nor an
/// instant; CDR transport errors pass through; a non-2xx CDR answer normalizes
/// via [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the result set is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_compositions(
    /// The EHR whose compositions to list.
    ehr_id: String,
    /// Substring of the template id to keep; empty means every template.
    template: String,
    /// Inclusive lower bound on the context start time; empty means unbounded.
    from: String,
    /// Inclusive upper bound on the context start time; empty means unbounded.
    to: String,
    /// Substring of the composer's name to keep; empty means every composer.
    composer: String,
    /// First row of the page to return.
    offset: u32,
) -> Result<ResultPage, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let filter = CompositionFilter::new(&template, &from, &to, &composer);
    let query = composition_query(&ehr_id, &filter)?;
    let url = state.cdr.rest_v1("query/aql");
    let body = aql_request_body(&query.aql, &query.parameters, offset);
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    parse_result_set(&response.body, offset)
}

/// The EHR's compositions with NO filter applied.
///
/// The viewer's composition PICKERS (the Commit tab's amend selector, the
/// directory editor's item selector) offer every composition in the EHR, not
/// the view the tab happens to be filtered to — this names that intent at the
/// call site instead of four empty strings.
///
/// `ehr_id` names the EHR and `offset` the page's first row.
///
/// # Errors
/// Whatever [`list_compositions`] returns.
pub async fn all_compositions(ehr_id: String, offset: u32) -> Result<ResultPage, ViewerError> {
    list_compositions(
        ehr_id,
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        offset,
    )
    .await
}

/// Commit a new COMPOSITION to the EHR (`POST /ehr/{ehr_id}/composition`). The
/// `format` picks the `Content-Type` (canonical JSON `application/json`,
/// canonical XML `application/xml`, FLAT `application/openehr.wt.flat+json`);
/// a FLAT commit additionally requires the `openehr-template-id` header.
/// `Accept: application/json` + `Prefer: return=representation` yields a
/// canonical composition body whose `uid.value` is returned as the new
/// version uid.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty body or a FLAT commit without a
/// template id; CDR transport errors pass through; a non-2xx CDR answer (its
/// validation diagnostics, which the UI renders verbatim, included)
/// normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn commit_composition(
    /// The EHR to commit into.
    ehr_id: String,
    /// Which representation the body is written in.
    format: ReprFormat,
    /// The template the composition is built from (required by the simplified formats).
    template_id: String,
    /// The composition document to commit, as text.
    body: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    if body.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "the composition body is empty".to_owned(),
        ));
    }
    let template_id = template_id.trim();
    let mut headers: Vec<(&str, &str)> = vec![("Prefer", "return=representation")];
    if matches!(format, ReprFormat::Flat) {
        if template_id.is_empty() {
            return Err(ViewerError::Invalid(
                "a template id is required to commit a FLAT composition".to_owned(),
            ));
        }
        headers.push(("openehr-template-id", template_id));
    }
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}/composition", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            format.media_type(),
            "application/json",
            &headers,
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    Ok(uid_value_of(&response.body))
}

/// Compositions tab: the filter bar, `list_compositions` (AQL) → a paged table
/// whose uid cells open the composition viewer's RENDERED clinical reading
/// (under `<Transition>` so paging keeps old rows visible), plus a "Commit
/// composition" form below it. A successful commit bumps the commit action's
/// version — a source of the list resource — refetching the table (never
/// fetch-in-effect).
pub(super) fn compositions_section(
    ehr_id: Signal<String>,
    offset: Signal<u32>,
    selected: Memo<String>,
) -> AnyView {
    let toaster = thaw::ToasterInjection::expect_context();
    // URL state read in SETUP: the filter values feed the resource source,
    // and the paging links carry every one of them across.
    let query = leptos_router::hooks::use_query_map();
    let filter = Signal::derive(move || {
        query.with(|q| {
            CompositionFilter::new(
                &q.get("template").unwrap_or_default(),
                &q.get("from").unwrap_or_default(),
                &q.get("to").unwrap_or_default(),
                &q.get("composer").unwrap_or_default(),
            )
        })
    });
    let commit = Action::new(
        |(ehr_id, format, template_id, body): &(String, ReprFormat, String, String)| {
            let ehr_id = ehr_id.clone();
            let format = *format;
            let template_id = template_id.clone();
            let body = body.clone();
            async move { commit_composition(ehr_id, format, template_id, body).await }
        },
    );
    // Both outcomes toast (an outside-world side-effect; the viewer's
    // mutation-feedback rule); the CDR's validation diagnostic ALSO stays
    // inline in the form, where the pasted body is.
    Effect::new(move |_| match commit.value().get() {
        Some(Ok(uid)) => {
            let detail = if uid.is_empty() {
                "The composition was committed.".to_owned()
            } else {
                format!("New version {uid}")
            };
            toast_success(toaster, "Composition committed", &detail);
        }
        Some(Err(error)) => crate::feedback::toast_write_failure(
            toaster,
            "Commit failed",
            "the composition",
            &error,
        ),
        None => {}
    });
    let resource = Resource::new(
        move || {
            let version = commit.version().get();
            (selected.get() == "compositions")
                .then(|| (ehr_id.get(), filter.get(), offset.get(), version))
        },
        |active| async move {
            match active {
                Some((id, filter, offset, _)) => list_compositions(
                    id,
                    filter.template,
                    filter.from,
                    filter.to,
                    filter.composer,
                    offset,
                )
                .await
                .map(Some),
                None => Ok(None),
            }
        },
    );
    let table = view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(Some(page)) => {
                        compositions_table(
                            &page,
                            &ehr_id.get(),
                            filter.with_untracked(CompositionFilter::is_empty),
                            query,
                        )
                    }
                    Ok(None) => ().into_any(),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();
    let filters = filter_bar(ehr_id, query);
    let form = commit_form(ehr_id, commit);
    view! { <div>{filters} {table} {form}</div> }.into_any()
}

/// The row-filter bar: a GET `<Form>` whose fields ARE the URL
/// state.
///
/// Submitting navigates to `/ehrs/{id}?tab=compositions&…`, which drops
/// `?offset=` — a new filter set starts at its own first page — and keeps
/// working with no JavaScript at all (pre-hydration the browser submits it
/// natively; the router takes over once WASM loads).
///
/// Each field shows what the URL says, through the attribute/property pair the
/// viewer uses wherever server HTML and live state must agree: the static
/// `value` attribute is the server pass, so a shared link arrives with the boxes
/// already filled; the `prop:value` binding follows the address bar afterwards,
/// so **Clear** and the back button empty and refill them. Typing changes
/// nothing the binding reads, so a draft survives until the next navigation.
fn filter_bar(ehr_id: Signal<String>, query: Memo<ParamsMap>) -> AnyView {
    let initial = move |key: &str| query.read_untracked().get(key).unwrap_or_default();
    let current = move |key: &'static str| {
        Signal::derive(move || query.with(|q| q.get(key).unwrap_or_default()))
    };
    let action = move || ehr_detail_href(&ehr_id.get());
    let clear = move || format!("{}?tab=compositions", ehr_detail_href(&ehr_id.get()));
    view! {
        <leptos_router::components::Form
            method="GET"
            action=action
            attr:class="mb-4"
            attr:id="compositions-filter"
        >
            // The tab this form belongs to travels with it: a GET <Form>
            // submits its own fields as the whole query string, so without
            // this the filtered result would land back on the Status tab.
            <input type="hidden" name="tab" value="compositions" />
            <div class="flex flex-wrap items-end gap-2">
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "Template"
                    <input
                        id="composition-filter-template"
                        type="text"
                        name="template"
                        class=format!("w-56 {INPUT}")
                        placeholder="part of the template id"
                        value=initial("template")
                        prop:value=current("template")
                    />
                </label>
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "From (UTC date)"
                    <input
                        id="composition-filter-from"
                        type="date"
                        name="from"
                        class=format!("w-44 {INPUT}")
                        value=initial("from")
                        prop:value=current("from")
                    />
                </label>
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "To (UTC date)"
                    <input
                        id="composition-filter-to"
                        type="date"
                        name="to"
                        class=format!("w-44 {INPUT}")
                        value=initial("to")
                        prop:value=current("to")
                    />
                </label>
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "Composer"
                    <input
                        id="composition-filter-composer"
                        type="text"
                        name="composer"
                        class=format!("w-48 {INPUT}")
                        placeholder="part of the composer's name"
                        value=initial("composer")
                        prop:value=current("composer")
                    />
                </label>
                <button id="composition-filter-apply" type="submit" class=BTN_PRIMARY>
                    <leptos_icons::Icon icon=icondata_lu::LuFunnel width="14" height="14" />
                    "Filter"
                </button>
                <a id="composition-filter-clear" href=clear class=BTN_SECONDARY>
                    "Clear"
                </a>
            </div>
            <p class="mt-2 text-xs text-ink-muted">
                "Template and composer match anywhere in the value; the dates bound the composition's context start time, each covering its whole UTC day."
            </p>
        </leptos_router::components::Form>
    }
    .into_any()
}

/// The "Commit composition" form: a format select, a template-id input shown
/// only for FLAT (its `openehr-template-id` header is required there — kept in
/// the DOM and toggled with `class:hidden` so the server and client view
/// structure stay identical), a large body textarea, and a Commit button
/// dispatching the shared `commit` action.
fn commit_form(
    ehr_id: Signal<String>,
    commit: Action<(String, ReprFormat, String, String), Result<String, ViewerError>>,
) -> AnyView {
    let format = RwSignal::new(ReprFormat::CanonicalJson);
    let template_id = RwSignal::new(String::new());
    let body = RwSignal::new(String::new());
    let is_flat = move || format.get() == ReprFormat::Flat;
    let on_commit = move |_| {
        commit.dispatch((ehr_id.get(), format.get(), template_id.get(), body.get()));
    };
    view! {
        <section class=format!("{CARD_PAD} mt-4")>
            <h2 class=CARD_TITLE>"Commit composition"</h2>
            <div class="flex flex-col gap-3">
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="commit-format">
                            "Format"
                        </label>
                        <select
                            id="commit-format"
                            class=SELECT
                            prop:value=move || format_value(format.get())
                            on:change=move |ev| {
                                format.set(format_from_value(&event_target_value(&ev)));
                            }
                        >
                            <option value="json">"Canonical JSON"</option>
                            <option value="xml">"Canonical XML"</option>
                            <option value="flat">"FLAT"</option>
                        </select>
                    </div>
                    <div class="flex flex-col gap-1" class:hidden=move || !is_flat()>
                        <label class=LABEL r#for="commit-template-id">
                            "Template id"
                        </label>
                        <input
                            id="commit-template-id"
                            type="text"
                            class=INPUT
                            placeholder="template id (required for FLAT)"
                            prop:value=move || template_id.get()
                            on:input:target=move |ev| template_id.set(ev.target().value())
                        />
                    </div>
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="commit-body">
                        "Composition"
                    </label>
                    <textarea
                        id="commit-body"
                        class=format!("{TEXTAREA} min-h-[16rem]")
                        placeholder="paste the composition document (JSON, XML, or FLAT)…"
                        prop:value=move || body.get()
                        on:input:target=move |ev| body.set(ev.target().value())
                    >
                        {body.get_untracked()}
                    </textarea>
                </div>
                <div class="flex items-center gap-3">
                    <button
                        id="commit-submit"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=Signal::derive(move || commit.pending().get())
                        on:click=on_commit
                    >
                        "Commit"
                    </button>
                    <Show when=move || commit.pending().get()>
                        <span class="text-sm text-ink-muted">"Committing…"</span>
                    </Show>
                </div>
                {commit_feedback(commit)}
            </div>
        </section>
    }
    .into_any()
}

/// The commit action's failure pane: the CDR's validation diagnostics
/// verbatim in a scrollable WELL (they are long and precious — a `<pre>`, not
/// a one-line error). Success is a toast (see [`compositions_section`]).
fn commit_feedback(
    commit: Action<(String, ReprFormat, String, String), Result<String, ViewerError>>,
) -> AnyView {
    view! {
        {move || match commit.value().get() {
            Some(Err(error)) => {
                view! {
                    <div class=WELL>
                        <pre class="overflow-auto max-h-[40vh] whitespace-pre-wrap font-mono text-xs text-danger">
                            {error.to_string()}
                        </pre>
                    </div>
                }
                    .into_any()
            }
            _ => ().into_any(),
        }}
    }
    .into_any()
}

/// The `<select>` option value for a committable format.
fn format_value(format: ReprFormat) -> &'static str {
    match format {
        ReprFormat::CanonicalXml => "xml",
        ReprFormat::Flat => "flat",
        _ => "json",
    }
}

/// The committable format for a `<select>` option value (unknown → canonical
/// JSON).
fn format_from_value(value: &str) -> ReprFormat {
    match value {
        "xml" => ReprFormat::CanonicalXml,
        "flat" => ReprFormat::Flat,
        _ => ReprFormat::CanonicalJson,
    }
}

/// Render one page of compositions: a table whose uid cell opens the
/// composition viewer's rendered clinical reading (the versioned-object id —
/// any `::system::version` suffix stripped for the link, the full uid kept
/// visible), plus paging that preserves the filters.
///
/// `unfiltered` picks the empty state's copy: nothing to show is a different
/// fact from nothing matching the filters, and the reader needs to know which
/// one they are looking at.
fn compositions_table(
    page: &ResultPage,
    ehr_id: &str,
    unfiltered: bool,
    query: Memo<ParamsMap>,
) -> AnyView {
    if page.rows.is_empty() {
        let (message, hint) = if unfiltered {
            (
                "No compositions in this EHR",
                "Commit one with the form above, or through the CDR's REST API.",
            )
        } else {
            (
                "No compositions match these filters",
                "Widen the date window, shorten the template or composer text, or clear the filters.",
            )
        };
        return view! { <EmptyState icon=icondata_lu::LuFileText message=message hint=hint /> }
            .into_any();
    }
    let rows = page.rows.clone();
    let ehr_id_owned = ehr_id.to_owned();
    let body = view! {
        <For
            each=move || rows.clone()
            key=|row| row.first().map(cell_text).unwrap_or_default()
            let:row
        >
            {composition_row(&row, &ehr_id_owned)}
        </For>
    }
    .into_any();
    let paging = paging_controls(
        page.offset,
        page.rows.len(),
        &ehr_detail_href(ehr_id),
        query,
    );
    view! {
        {table_shell(&["Composition", "Name", "Template", "Started", "Composer"], body)}
        {paging}
    }
    .into_any()
}

/// One composition row: the uid cell opens the viewer at the versioned-object
/// id, in the RENDERED clinical view (`?view=` — the composition viewer's
/// deep-linkable pane mode); the full uid stays visible.
///
/// Both path segments are percent-encoded — an id carrying `/`, `#`, `?` or `%`
/// would otherwise address a different route.
fn composition_row(row: &[Value], ehr_id: &str) -> AnyView {
    let uid = row.first().map(cell_text).unwrap_or_default();
    let vo_id = urlencoding::encode(&crate::uid::container_uid_of(&uid)).into_owned();
    let ehr = urlencoding::encode(ehr_id);
    let cells = row
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let text = cell_text(value);
            if i == 0 {
                let href = format!(
                    "/ehrs/{ehr}/compositions/{vo_id}?view={}",
                    PaneView::Rendered.param()
                );
                view! {
                    <td class=CELL_MONO>
                        <A href=href attr:class="text-accent hover:underline">
                            {text}
                        </A>
                    </td>
                }
                .into_any()
            } else {
                view! { <td class=CELL>{text}</td> }.into_any()
            }
        })
        .collect::<Vec<_>>();
    view! { <tr class=ROW>{cells}</tr> }.into_any()
}

#[cfg(test)]
mod tests {
    use super::{format_from_value, format_value};
    use crate::format::ReprFormat;

    #[test]
    fn format_value_round_trips() {
        for format in [
            ReprFormat::CanonicalJson,
            ReprFormat::CanonicalXml,
            ReprFormat::Flat,
        ] {
            assert_eq!(format_from_value(format_value(format)), format);
        }
        // An unknown value falls back to canonical JSON.
        assert_eq!(format_from_value("bogus"), ReprFormat::CanonicalJson);
    }
}
