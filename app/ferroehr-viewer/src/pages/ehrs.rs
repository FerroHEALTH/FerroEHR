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
use ferrohealth_ui::widget::identifier::{Domain, Masked};
use leptos::prelude::*;
use leptos_meta::Title;

use crate::bff::ehrs::{
    ResultPage, cell_text, create_ehr, ehr_detail_href, find_ehr_by_subject, is_uuid, list_ehrs,
};
use crate::flash::{Done, redirect_with};
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{
    EditorDock, awaited, card_panel, dock_editor, is_status, refused, section, short,
};
use crate::paging::Listing;

/// The EHRs list's heading, with New EHR.
fn heading() -> AnyView {
    page_heading(
        Header {
            title: tr(Msg::NavEhrs).into(),
            subtitle: Some(tr(Msg::EhrsSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::PLUS),
            tr(Msg::NewEhr),
            Act::Link("/ehrs/new".to_owned()),
        )),
    )
    .into_any()
}

/// The list's columns.
fn columns() -> [Column; 2] {
    [
        Column::text("ehr", tr(Msg::ColumnEhr)).role(Role::Title),
        Column::text("created", tr(Msg::ColumnCreated)).role(Role::Meta),
    ]
}

/// The table of `rows`, each `(ehr id, created)`.
fn table(rows: &[(String, String)]) -> AnyView {
    let columns = columns();
    let body = rows
        .iter()
        .map(|(ehr_id, created)| {
            let href = ehr_detail_href(ehr_id);
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
                {tr(Msg::EhrsNamespace)}
            </label>
            <input
                id="ehrs-namespace"
                name="namespace"
                type="text"
                class="fui-input fui-mono"
                value=namespace.to_owned()
                placeholder=tr(Msg::EhrsNamespace)
                autocomplete="off"
                spellcheck="false"
            />
        </div>
        <button type="submit" class="fui-button fui-button-secondary">
            {ferrohealth_ui::icon::icon(Glyph::SEARCH, "16")}
            {tr(Msg::SearchSubmit)}
        </button>
    };
    let label = tr(Msg::EhrsFindLabel);
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
        count,
        order: None,
        tabs: None,
        actions: None,
        body,
        pager,
    })
}

/// The page a post finding an EHR draws: the EHR id's page, the one EHR the
/// subject names, or the line that no EHR has it.
async fn found(typed: String, namespace: String) -> AnyView {
    let id = crate::uid::container_uid_of(&typed);
    if is_uuid(&id) {
        leptos_axum::redirect(&ehr_detail_href(&id));
        return ().into_any();
    }
    if namespace.is_empty() {
        let line = list_empty(tr(Msg::EhrsSubjectNeedsNamespace), None);
        return view! {
            {toolbar(&typed, &namespace)}
            {card(line, None, None)}
        }
        .into_any();
    }
    let masked = Masked::new(
        Domain::new(namespace.clone()),
        &typed,
        ferrohealth_ui::widget::identifier::KEEP,
    );
    let chip = tr_with(Msg::EhrsSubjectChip, &[("subject", masked.drawn().into())]);
    match find_ehr_by_subject(typed.clone(), namespace.clone()).await {
        Ok(Some(ehr_id)) => view! {
            {toolbar("", &namespace)}
            {chips(chip)}
            {card(table(&[(ehr_id, String::new())]), Some(1), None)}
        }
        .into_any(),
        Ok(None) => view! {
            {toolbar("", &namespace)}
            {chips(chip)}
            {card(list_empty(tr(Msg::EhrsNoMatch), None), Some(0), None)}
        }
        .into_any(),
        Err(error) => view! {
            {toolbar("", &namespace)}
            {refused(tr(Msg::EhrsFindRefused), &error, None)}
        }
        .into_any(),
    }
}

/// The EHRs list.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn EhrsPage() -> impl IntoView {
    let posted = crate::posted::posted_for("find");
    let paging = crate::paging::current(Listing::Ehrs);
    let fields = crate::paging::fields(Listing::Ehrs);
    let body = awaited(async move {
        if let Some(posted) = posted {
            let typed = posted.text("find");
            let namespace = posted.text("namespace");
            if !typed.is_empty() {
                return found(typed, namespace).await;
            }
        }
        let window = OpenWindow::of(paging, 0);
        match list_ehrs(window.start, window.size).await {
            Ok(page) => {
                let rows = rows_of(&page);
                let held = u32::try_from(rows.len()).unwrap_or(u32::MAX);
                let window = OpenWindow::of(paging, held);
                let body = if rows.is_empty() && window.page == 1 {
                    list_empty(
                        tr(Msg::EhrsEmpty),
                        Some((tr(Msg::EhrsCreateFirst), "/ehrs/new".to_owned())),
                    )
                } else {
                    table(&rows)
                };
                let pager = (window.page > 1 || window.has_next()).then(|| {
                    post_open_pager(
                        PagerListing::new(tr(Msg::EhrsPages)),
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
            Err(error) => view! {
                {toolbar("", "")}
                {refused(tr(Msg::EhrsRefused), &error, None)}
            }
            .into_any(),
        }
    });
    view! {
        <Title text=tr(Msg::NavEhrs) />
        {heading()}
        <div class="fui-list-page">{body}</div>
    }
}

/// The id of New EHR's form, which the docked bar submits.
const NEW_EHR_FORM: &str = "ehr-new";

/// What New EHR's form holds, as it was posted.
#[derive(Debug, Clone, Default)]
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
    let subject = Field::new("s-id", tr(Msg::EhrNewSubjectId)).view(view! {
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
    let namespace = Field::new("s-ns", tr(Msg::EhrNewNamespace)).view(view! {
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
    let ehr_id = Field::new("e-id", tr(Msg::EhrNewEhrId))
        .help(tr(Msg::EhrNewEhrIdHelp))
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
                placeholder=tr(Msg::EhrNewEhrIdPlaceholder)
                aria-invalid=invalid
                value=draft.ehr_id.clone()
            />
        });
    view! {
        <form id=NEW_EHR_FORM method="post" action="/ehrs/new">
            {card_panel(
                view! {
                    {section(
                        Some(tr(Msg::EhrNewSubject)),
                        None,
                        None,
                        view! {
                            {ferrohealth_ui::view::surface::section_intro(
                                tr(Msg::EhrNewSubjectIntro),
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

/// New EHR.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn NewEhrPage() -> impl IntoView {
    let posted = crate::posted::posted();
    if crate::pages::common::discarded(posted.as_ref()) {
        leptos_axum::redirect("/ehrs");
    }
    let dock_promise = crate::shell::promise_dock();
    let body = awaited(async move {
        let _dock_promise = dock_promise;
        let draft = posted.as_ref().map_or_else(Draft::default, |posted| Draft {
            subject_id: posted.text("subject_id"),
            namespace: posted.text("namespace"),
            ehr_id: posted.text("ehr_id"),
        });
        let mut top = None;
        let mut ehr_id_error = None;
        let mut line = (Glyph::FILE_PLUS, tr(Msg::EhrNewNotYet));
        if posted
            .as_ref()
            .is_some_and(|posted| posted.intent() == "save")
        {
            let id = draft.ehr_id.trim().to_owned();
            if !id.is_empty() && !is_uuid(&id) {
                ehr_id_error = Some(tr(Msg::EhrNewNotUuid));
                line = (Glyph::CIRCLE_X, tr(Msg::EhrNewNotCreated));
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
                            &ehr_detail_href(&created),
                            &Done::ok(tr(Msg::EhrCreated))
                                .line(crate::pages::common::shortened(&created)),
                        );
                        return ().into_any();
                    }
                    Err(error) => {
                        line = (Glyph::CIRCLE_X, tr(Msg::EhrNewNotCreated));
                        if is_status(&error, http::StatusCode::CONFLICT) {
                            ehr_id_error = Some(tr(Msg::EhrNewInUse));
                        }
                        let open = is_status(&error, http::StatusCode::CONFLICT).then(|| {
                            ferrohealth_ui::view::button::link_button(
                                ehr_detail_href(&id),
                                ferrohealth_ui::view::button::Variant::Secondary,
                                Some(Glyph::ARROW_RIGHT),
                                tr_with(
                                    Msg::EhrOpen,
                                    &[("id", crate::pages::common::shortened(&id).into())],
                                ),
                            )
                        });
                        top = Some(
                            view! {
                                {refused(tr(Msg::EhrNewRefused), &error, None)}
                                <div class="fui-cluster">{open}</div>
                            }
                            .into_any(),
                        );
                    }
                }
            }
        }
        dock_editor(EditorDock {
            form: NEW_EHR_FORM.to_owned(),
            line: line.1,
            glyph: line.0,
            save: tr(Msg::EhrNewCreate),
            blocked: None,
        });
        view! {
            {top}
            {new_form(&draft, ehr_id_error)}
        }
        .into_any()
    });
    let heading = page_heading(
        Header {
            title: tr(Msg::NewEhr).into(),
            subtitle: Some(tr(Msg::EhrNewSubtitle).into()),
            crumbs: vec![Crumb::new(tr(Msg::NavEhrs), "/ehrs")],
            mono: false,
            badge: None,
        },
        ActionGroup::none(),
    );
    view! {
        <Title text=tr(Msg::NewEhr) />
        {heading}
        <div class="fui-stack">{body}</div>
    }
}
