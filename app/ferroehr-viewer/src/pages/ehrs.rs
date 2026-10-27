// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHRs list and New EHR (FerroUI `design/consoles/ferroehr/`, pages 10
//! and 11).
//!
//! The list is the newest EHRs read with AQL (`docs/specs/openehr/ITS-REST/`
//! `docs/query/`), which states no total, so it pages with Previous and Next
//! alone; its position is kept in the browser's session, never the URL
//! ([`crate::paging`]). The find field takes an EHR id, which opens that EHR,
//! or a subject id with its namespace (`GET /ehr?subject_id&subject_namespace`,
//! ITS-REST `ehr_get_by_subject`), and posts, since a subject id names a
//! patient. New EHR creates one with a subject or with none, with the CDR's
//! id or one given (`POST /ehr`, `PUT /ehr/{ehr_id}`). No openEHR spec governs
//! the viewer's pages: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::filter::{Find, find_form};
use ferrohealth_ui::table::model::{OpenWindow, StateSource};
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_open_pager};
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{Crumb, Header, page_heading};
use ferrohealth_ui::view::template::list::{
    ActiveFilters, FilterChip, ListCard, active_filters, list_card, list_empty,
};
use ferrohealth_ui::widget::action_menu::{Act, Action};
#[cfg(not(target_arch = "wasm32"))]
use ferrohealth_ui::widget::identifier::{Domain, Masked};
use leptos::prelude::*;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::bff::ehrs::{
    ResultPage, cell_text, create_ehr, find_ehr_by_subject, is_uuid, list_ehrs,
};
use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::flash::{Done, redirect_with};
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, tr, tr_with};
#[cfg(not(target_arch = "wasm32"))]
use crate::pages::common::is_status;
use crate::pages::common::{
    EditorDock, Refused, card_panel, dock_editor, docked, drawn, page_model, section, short,
};
use crate::pages::ehr::ehr_href;
use crate::paging::Listing;

/// The EHRs list's heading, with New EHR.
fn heading() -> AnyView {
    page_heading(
        Header {
            title: tr(Msg::NavEhrs).into(),
            subtitle: Some(tr(RecordsMsg::EhrsSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::PLUS),
            tr(RecordsMsg::NewEhr),
            Act::Link("/ehrs/new".to_owned()),
        )),
    )
    .into_any()
}

/// The list's columns.
fn columns() -> [Column; 2] {
    [
        Column::text("ehr", tr(RecordsMsg::ColumnEhr)).role(Role::Title),
        Column::text("created", tr(Msg::ColumnCreated)).role(Role::Meta),
    ]
}

/// The table of `rows`, each `(ehr id, created)`.
fn table(rows: &[(String, String)]) -> AnyView {
    let columns = columns();
    let body = rows
        .iter()
        .map(|(ehr_id, created)| {
            let href = ehr_href(ehr_id);
            let title = title_cell(Some(&href), short(ehr_id), None);
            let created = cell(&columns[1], crate::pages::common::ago(created));
            view! { <tr>{title} {created}</tr> }.into_any()
        })
        .collect_view()
        .into_any();
    let caption = tr(Msg::NavEhrs);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(body),
    })
    .into_any()
}

#[cfg(not(target_arch = "wasm32"))]
/// The rows of a result page of the list's AQL.
fn rows_of(page: &ResultPage) -> Vec<(String, String)> {
    page.rows
        .iter()
        .map(|row| {
            (
                row.first().map(cell_text).unwrap_or_default(),
                row.get(1).map(cell_text).unwrap_or_default(),
            )
        })
        .filter(|(ehr_id, _)| !ehr_id.is_empty())
        .collect()
}

/// The find toolbar: an EHR id or a subject id, and the subject id's
/// namespace. It posts, since a subject id names a patient.
fn toolbar(typed: &str, namespace: &str) -> AnyView {
    let namespace_field = view! {
        <div class="fui-list-find">
            <label for="ehrs-namespace" class="fui-sr-only">
                {tr(RecordsMsg::EhrsNamespace)}
            </label>
            <input
                id="ehrs-namespace"
                name="namespace"
                type="text"
                class="fui-input fui-mono"
                value=namespace.to_owned()
                placeholder=tr(RecordsMsg::EhrsNamespace)
                autocomplete="off"
                spellcheck="false"
            />
        </div>
        <button type="submit" class="fui-button fui-button-secondary">
            {ferrohealth_ui::icon::icon(Glyph::SEARCH, "16")}
            {tr(Msg::SearchSubmit)}
        </button>
    };
    let label = tr(RecordsMsg::EhrsFindLabel);
    find_form(
        Find {
            id: "ehrs-find",
            name: "find",
            label: &label,
            value: typed,
            source: StateSource::Session,
            action: Some("/ehrs"),
            fields: &[("intent".to_owned(), "find".to_owned())],
        },
        namespace_field,
    )
}

/// The chip of a find with its way back to the whole list.
fn chips(label: String) -> AnyView {
    active_filters(
        ActiveFilters {
            chips: vec![FilterChip {
                label,
                remove: "/ehrs".to_owned(),
            }],
            clear: "/ehrs".to_owned(),
            shown: None,
        },
        None,
    )
}

/// The card of the list, its pager in its foot.
fn card(body: AnyView, count: Option<u64>, pager: Option<AnyView>) -> AnyView {
    list_card(ListCard {
        title: tr(Msg::NavEhrs),
        count: count.map(ferrohealth_ui::view::template::list::HeadingCount::All),
        order: None,
        tabs: None,
        actions: None,
        body,
        pager,
    })
}

/// The EHRs list's model: the page of the list, or what a find found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum EhrsModel {
    /// The page of the list, each row `(ehr id, created)`, or the CDR's
    /// refusal of it.
    List(Result<Vec<(String, String)>, Refused>),
    /// A subject id was typed with no namespace.
    NeedsNamespace {
        /// What was typed.
        typed: String,
    },
    /// The EHR a subject names.
    Found {
        /// The subject's namespace.
        namespace: String,
        /// The chip naming the subject, its id masked.
        chip: String,
        /// The EHR, `None` when no EHR has the subject, or the CDR's refusal.
        found: Result<Option<String>, Refused>,
    },
    /// The find named an EHR id, so the post redirected to it.
    Left,
}

#[cfg(not(target_arch = "wasm32"))]
/// What a post finding an EHR found: the EHR id's page, the one EHR the
/// subject names, or that no EHR has it.
async fn found(typed: String, namespace: String) -> EhrsModel {
    let id = crate::uid::container_uid_of(&typed);
    if is_uuid(&id) {
        leptos_axum::redirect(&ehr_href(&id));
        return EhrsModel::Left;
    }
    if namespace.is_empty() {
        return EhrsModel::NeedsNamespace { typed };
    }
    let masked = Masked::new(
        Domain::new(namespace.clone()),
        &typed,
        ferrohealth_ui::widget::identifier::KEEP,
    );
    let chip = tr_with(
        RecordsMsg::EhrsSubjectChip,
        &[("subject", masked.drawn().into())],
    );
    let found = find_ehr_by_subject(typed, namespace.clone())
        .await
        .map_err(|error| Refused::new(tr(RecordsMsg::EhrsFindRefused), error));
    EhrsModel::Found {
        namespace,
        chip,
        found,
    }
}

/// Loads the EHRs list: the find a post asks, or the page of the list.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_list(
    /// The path and query of the page.
    href: String,
) -> Result<EhrsModel, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_list().await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_list`] reads, on the server.
async fn read_list() -> EhrsModel {
    if let Some(posted) = crate::posted::posted_for("find") {
        let typed = posted.text("find");
        let namespace = posted.text("namespace");
        if !typed.is_empty() {
            return found(typed, namespace).await;
        }
    }
    let window = OpenWindow::of(crate::paging::current(Listing::Ehrs), 0);
    EhrsModel::List(
        list_ehrs(window.start, window.size)
            .await
            .map(|page| rows_of(&page))
            .map_err(|error| Refused::new(tr(RecordsMsg::EhrsRefused), error)),
    )
}

/// The EHRs list's body.
fn list_view(model: EhrsModel) -> AnyView {
    match model {
        EhrsModel::List(Ok(rows)) => {
            let paging = crate::paging::current(Listing::Ehrs);
            let fields = crate::paging::fields(Listing::Ehrs);
            let held = u32::try_from(rows.len()).unwrap_or(u32::MAX);
            let window = OpenWindow::of(paging, held);
            let body = if rows.is_empty() && window.page == 1 {
                list_empty(
                    tr(RecordsMsg::EhrsEmpty),
                    Some((tr(RecordsMsg::EhrsCreateFirst), "/ehrs/new".to_owned())),
                )
            } else {
                table(&rows)
            };
            let pager = (window.page > 1 || window.has_next()).then(|| {
                post_open_pager(
                    PagerListing::new(tr(RecordsMsg::EhrsPages)),
                    window,
                    PostTo {
                        id: "ehrs-pager",
                        action: crate::paging::PAGING_ROUTE,
                        fields: &fields,
                    },
                )
            });
            view! {
                {toolbar("", "")}
                {card(body, None, pager)}
            }
            .into_any()
        }
        EhrsModel::List(Err(refusal)) => view! {
            {toolbar("", "")}
            {refusal.view()}
        }
        .into_any(),
        EhrsModel::NeedsNamespace { typed } => {
            let line = list_empty(tr(RecordsMsg::EhrsSubjectNeedsNamespace), None);
            view! {
                {toolbar(&typed, "")}
                {card(line, None, None)}
            }
            .into_any()
        }
        EhrsModel::Found {
            namespace,
            chip,
            found,
        } => match found {
            Ok(Some(ehr_id)) => view! {
                {toolbar("", &namespace)}
                {chips(chip)}
                {card(table(&[(ehr_id, String::new())]), Some(1), None)}
            }
            .into_any(),
            Ok(None) => view! {
                {toolbar("", &namespace)}
                {chips(chip)}
                {card(list_empty(tr(RecordsMsg::EhrsNoMatch), None), Some(0), None)}
            }
            .into_any(),
            Err(refusal) => view! {
                {toolbar("", &namespace)}
                {refusal.view()}
            }
            .into_any(),
        },
        EhrsModel::Left => ().into_any(),
    }
}

/// The EHRs list.
#[component]
pub fn EhrsPage() -> impl IntoView {
    let model = page_model!(load_list());
    let body = drawn(model, list_view);
    view! {
        <Title text=tr(Msg::NavEhrs) />
        {heading()}
        <div class="fui-list-page">{body}</div>
    }
}

/// The id of New EHR's form, which the docked bar submits.
const NEW_EHR_FORM: &str = "ehr-new";

/// What New EHR's form holds, as it was posted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Draft {
    /// The subject id.
    subject_id: String,
    /// The subject's namespace.
    namespace: String,
    /// The EHR id asked for, empty for the CDR's own.
    ehr_id: String,
}

/// New EHR's form: the subject and its namespace, and an optional EHR id.
fn new_form(draft: &Draft, ehr_id_error: Option<String>) -> AnyView {
    let subject = Field::new("s-id", tr(RecordsMsg::EhrNewSubjectId)).view(view! {
        <input
            id="s-id"
            name="subject_id"
            type="password"
            class="fui-input fui-mono"
            spellcheck="false"
            autocomplete="off"
            value=draft.subject_id.clone()
        />
    });
    let namespace = Field::new("s-ns", tr(RecordsMsg::EhrNewNamespace)).view(view! {
        <input
            id="s-ns"
            name="namespace"
            type="text"
            class="fui-input fui-mono"
            spellcheck="false"
            autocomplete="off"
            value=draft.namespace.clone()
        />
    });
    let invalid = ehr_id_error.is_some().then_some("true");
    let ehr_id = Field::new("e-id", tr(RecordsMsg::EhrNewEhrId))
        .help(tr(RecordsMsg::EhrNewEhrIdHelp))
        .holds_id()
        .after(ehr_id_error.map(|error| {
            view! {
                <p class="fui-help fui-error" id="e-id-error">
                    {error}
                </p>
            }
        }))
        .view(view! {
            <input
                id="e-id"
                name="ehr_id"
                type="text"
                class="fui-input fui-mono"
                spellcheck="false"
                autocomplete="off"
                placeholder=tr(RecordsMsg::EhrNewEhrIdPlaceholder)
                aria-invalid=invalid
                value=draft.ehr_id.clone()
            />
        });
    view! {
        <form id=NEW_EHR_FORM method="post" action="/ehrs/new">
            {card_panel(
                view! {
                    {section(
                        Some(tr(RecordsMsg::EhrNewSubject)),
                        None,
                        None,
                        view! {
                            {ferrohealth_ui::view::surface::section_intro(
                                tr(RecordsMsg::EhrNewSubjectIntro),
                            )}
                            <div class="fui-form-grid">{subject} {namespace}</div>
                        },
                    )}
                    {section(None, None, None, view! { <div class="fui-form-grid">{ehr_id}</div> })}
                },
            )}
        </form>
    }
    .into_any()
}

/// New EHR's model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum NewModel {
    /// The form.
    Form {
        /// What it holds.
        draft: Draft,
        /// Whether a create was asked for and did not happen.
        failed: bool,
        /// What is wrong with the EHR id asked for.
        ehr_id_error: Option<String>,
        /// The CDR's refusal of the create, with the id already in use when
        /// that is why.
        refused: Option<(Refused, Option<String>)>,
    },
    /// The post redirected.
    Left,
}

/// Loads New EHR: answers its post (Discard, Create).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_new(
    /// The path and query of the page.
    href: String,
) -> Result<NewModel, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_new().await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_new`] reads, on the server.
async fn read_new() -> NewModel {
    let posted = crate::posted::posted();
    if crate::pages::common::discarded(posted.as_ref()) {
        leptos_axum::redirect("/ehrs");
        return NewModel::Left;
    }
    let draft = posted.as_ref().map_or_else(Draft::default, |posted| Draft {
        subject_id: posted.text("subject_id"),
        namespace: posted.text("namespace"),
        ehr_id: posted.text("ehr_id"),
    });
    let mut failed = false;
    let mut ehr_id_error = None;
    let mut refused = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        let id = draft.ehr_id.trim().to_owned();
        failed = true;
        if !id.is_empty() && !is_uuid(&id) {
            ehr_id_error = Some(tr(RecordsMsg::EhrNewNotUuid));
        } else {
            match create_ehr(
                id.clone(),
                draft.subject_id.clone(),
                draft.namespace.clone(),
            )
            .await
            {
                Ok(created) => {
                    redirect_with(
                        &ehr_href(&created),
                        &Done::ok(tr(RecordsMsg::EhrCreated))
                            .line(crate::pages::common::shortened(&created)),
                    );
                    return NewModel::Left;
                }
                Err(error) => {
                    let conflict = is_status(&error, http::StatusCode::CONFLICT);
                    if conflict {
                        ehr_id_error = Some(tr(RecordsMsg::EhrNewInUse));
                    }
                    refused = Some((
                        Refused::new(tr(RecordsMsg::EhrNewRefused), error),
                        conflict.then_some(id),
                    ));
                }
            }
        }
    }
    NewModel::Form {
        draft,
        failed,
        ehr_id_error,
        refused,
    }
}

/// New EHR's form, its decision bar docked.
fn new_view(model: NewModel) -> AnyView {
    let NewModel::Form {
        draft,
        failed,
        ehr_id_error,
        refused,
    } = model
    else {
        return ().into_any();
    };
    let line = if failed {
        (Glyph::CIRCLE_X, tr(RecordsMsg::EhrNewNotCreated))
    } else {
        (Glyph::FILE_PLUS, tr(RecordsMsg::EhrNewNotYet))
    };
    let top = refused.map(|(refusal, in_use)| {
        let open = in_use.map(|id| {
            ferrohealth_ui::view::button::link_button(
                ehr_href(&id),
                ferrohealth_ui::view::button::Variant::Secondary,
                Some(Glyph::ARROW_RIGHT),
                tr_with(
                    RecordsMsg::EhrOpen,
                    &[("id", crate::pages::common::shortened(&id).into())],
                ),
            )
        });
        view! {
            {refusal.view()}
            <div class="fui-cluster">{open}</div>
        }
        .into_any()
    });
    dock_editor(EditorDock {
        form: NEW_EHR_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save: tr(RecordsMsg::EhrNewCreate),
        blocked: None,
    });
    view! {
        {top}
        {new_form(&draft, ehr_id_error)}
    }
    .into_any()
}

/// New EHR.
#[component]
pub fn NewEhrPage() -> impl IntoView {
    let dock_promise = crate::shell::promise_dock();
    let model = page_model!(load_new());
    let body = docked(model, dock_promise, new_view);
    let heading = page_heading(
        Header {
            title: tr(RecordsMsg::NewEhr).into(),
            subtitle: Some(tr(RecordsMsg::EhrNewSubtitle).into()),
            crumbs: vec![Crumb::new(tr(Msg::NavEhrs), "/ehrs")],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    view! {
        <Title text=tr(RecordsMsg::NewEhr) />
        {heading}
        <div class="fui-stack">{body}</div>
    }
}
