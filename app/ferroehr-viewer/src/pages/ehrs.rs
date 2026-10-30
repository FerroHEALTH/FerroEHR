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

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::filter::{Find, find_form};
use ferrohealth_ui::table::model::{OpenWindow, StateSource};
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_open_pager};
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::identifier::{Domain, Masked};
use ferrohealth_ui::view::surface::{Crumb, Header, page_heading};
use ferrohealth_ui::view::template::list::{
    ActiveFilters, FilterChip, ListCard, active_filters, list_card, list_empty,
};

use crate::bff::ehrs::{
    ResultPage, cell_text, create_ehr, find_ehr_by_subject, is_uuid, list_ehrs,
};
use crate::flash::{Done, redirect_with};
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{attr, h, opt, t};
use crate::pages::common::{
    EditorDock, Refused, card_panel, dock_editor, drawn, is_status, section, short,
};
use crate::pages::ehr::ehr_href;
use crate::paging::Listing;
use crate::routes::Drawn;

/// The EHRs list's heading, with New EHR.
fn heading() -> Html {
    page_heading(
        &current(),
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
}

/// The list's columns.
fn columns() -> [Column; 2] {
    [
        Column::text("ehr", tr(RecordsMsg::ColumnEhr)).role(Role::Title),
        Column::text("created", tr(Msg::ColumnCreated)).role(Role::Meta),
    ]
}

/// The table of `rows`, each `(ehr id, created)`.
fn table(rows: &[(String, String)]) -> Html {
    let columns = columns();
    let body = rows
        .iter()
        .map(|(ehr_id, created)| {
            let href = ehr_href(ehr_id);
            let title = title_cell(Some(&href), short(ehr_id), None);
            let created = cell(&columns[1], crate::pages::common::ago(created));
            h(format!("<tr>{title}{created}</tr>"))
        })
        .collect();
    let caption = tr(Msg::NavEhrs);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(body),
    })
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
fn toolbar(typed: &str, namespace: &str) -> Html {
    let namespace_field = h(format!(
        r#"<div class="fui-list-find"><label for="ehrs-namespace" class="fui-sr-only">{label}</label><input id="ehrs-namespace" name="namespace" type="text" class="fui-input fui-mono" value="{value}" placeholder="{label}" autocomplete="off" spellcheck="false"></div><button type="submit" class="fui-button fui-button-secondary">{glyph}{submit}</button>"#,
        label = t(tr(RecordsMsg::EhrsNamespace)),
        value = t(namespace),
        glyph = ferrohealth_ui::icon::icon(Glyph::SEARCH, "16"),
        submit = t(tr(Msg::SearchSubmit)),
    ));
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
fn chips(label: String) -> Html {
    active_filters(
        &current(),
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
fn card(body: Html, count: Option<u64>, pager: Option<Html>) -> Html {
    list_card(
        &current(),
        ListCard {
            title: tr(Msg::NavEhrs),
            count: count.map(ferrohealth_ui::view::template::list::HeadingCount::All),
            order: None,
            tabs: None,
            actions: None,
            body,
            pager,
        },
    )
}

/// The EHRs list's model: the page of the list, or what a find found.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// What a post finding an EHR found: the EHR id's page, the one EHR the
/// subject names, or that no EHR has it.
async fn found(typed: String, namespace: String) -> EhrsModel {
    let id = crate::uid::container_uid_of(&typed);
    if is_uuid(&id) {
        crate::request::redirect(&ehr_href(&id));
        return EhrsModel::Left;
    }
    if namespace.is_empty() {
        return EhrsModel::NeedsNamespace { typed };
    }
    let masked = Masked::new(
        Domain::new(namespace.clone()),
        &typed,
        ferrohealth_ui::view::identifier::KEEP,
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
/// [`crate::error::ViewerError::Unauthenticated`] without a viewer session.
async fn load_list() -> Result<EhrsModel, crate::error::ViewerError> {
    crate::session::require_session()?;
    if let Some(posted) = crate::posted::posted_for("find") {
        let typed = posted.text("find");
        let namespace = posted.text("namespace");
        if !typed.is_empty() {
            return Ok(found(typed, namespace).await);
        }
    }
    let window = OpenWindow::of(crate::paging::current(Listing::Ehrs), 0);
    Ok(EhrsModel::List(
        list_ehrs(window.start, window.size)
            .await
            .map(|page| rows_of(&page))
            .map_err(|error| Refused::new(tr(RecordsMsg::EhrsRefused), error)),
    ))
}

/// The EHRs list's body.
fn list_view(model: EhrsModel) -> Html {
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
                    &current(),
                    PagerListing::new(tr(RecordsMsg::EhrsPages)),
                    window,
                    PostTo {
                        id: "ehrs-pager",
                        action: crate::paging::PAGING_ROUTE,
                        fields: &fields,
                    },
                )
            });
            toolbar("", "").then(card(body, None, pager))
        }
        EhrsModel::List(Err(refusal)) => toolbar("", "").then(refusal.view()),
        EhrsModel::NeedsNamespace { typed } => {
            let line = list_empty(tr(RecordsMsg::EhrsSubjectNeedsNamespace), None);
            toolbar(&typed, "").then(card(line, None, None))
        }
        EhrsModel::Found {
            namespace,
            chip,
            found,
        } => match found {
            Ok(Some(ehr_id)) => Html::join([
                toolbar("", &namespace),
                chips(chip),
                card(table(&[(ehr_id, String::new())]), Some(1), None),
            ]),
            Ok(None) => Html::join([
                toolbar("", &namespace),
                chips(chip),
                card(list_empty(tr(RecordsMsg::EhrsNoMatch), None), Some(0), None),
            ]),
            Err(refusal) => toolbar("", &namespace).then(refusal.view()),
        },
        EhrsModel::Left => Html::empty(),
    }
}

/// The EHRs list.
pub async fn list_page() -> Drawn {
    let body = drawn(load_list().await, list_view);
    Drawn::page(
        tr(Msg::NavEhrs),
        heading().then(h(format!(r#"<div class="fui-list-page">{body}</div>"#))),
    )
}

/// The id of New EHR's form, which the docked bar submits.
const NEW_EHR_FORM: &str = "ehr-new";

/// What New EHR's form holds, as it was posted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Draft {
    /// The subject id.
    subject_id: String,
    /// The subject's namespace.
    namespace: String,
    /// The EHR id asked for, empty for the CDR's own.
    ehr_id: String,
}

/// A plain one-line text input `name` with the id `id`, holding `value`.
fn mono_input(id: &str, name: &str, kind: &str, value: &str, extra: Html) -> Html {
    h(format!(
        r#"<input id="{id}" name="{name}" type="{kind}" class="fui-input fui-mono" spellcheck="false" autocomplete="off" value="{value}""#,
        id = t(id),
        name = t(name),
        kind = t(kind),
        value = t(value),
    ))
    .then(extra)
    .then(h(">".to_owned()))
}

/// New EHR's form: the subject and its namespace, and an optional EHR id.
fn new_form(draft: &Draft, ehr_id_error: Option<String>) -> Html {
    let subject = Field::new("s-id", tr(RecordsMsg::EhrNewSubjectId)).view(mono_input(
        "s-id",
        "subject_id",
        "password",
        &draft.subject_id,
        Html::empty(),
    ));
    let namespace = Field::new("s-ns", tr(RecordsMsg::EhrNewNamespace)).view(mono_input(
        "s-ns",
        "namespace",
        "text",
        &draft.namespace,
        Html::empty(),
    ));
    let invalid = ehr_id_error.is_some().then_some("true");
    let mut ehr_id = Field::new("e-id", tr(RecordsMsg::EhrNewEhrId))
        .help(tr(RecordsMsg::EhrNewEhrIdHelp))
        .holds_id();
    if let Some(error) = ehr_id_error {
        ehr_id = ehr_id.after(h(format!(
            r#"<p class="fui-help fui-error" id="e-id-error">{}</p>"#,
            t(error)
        )));
    }
    let ehr_id = ehr_id.view(mono_input(
        "e-id",
        "ehr_id",
        "text",
        &draft.ehr_id,
        attr("placeholder", Some(tr(RecordsMsg::EhrNewEhrIdPlaceholder)))
            .then(attr("aria-invalid", invalid)),
    ));
    let subject_part = section(
        Some(tr(RecordsMsg::EhrNewSubject)),
        None,
        None,
        ferrohealth_ui::view::surface::section_intro(tr(RecordsMsg::EhrNewSubjectIntro)).then(h(
            format!(r#"<div class="fui-form-grid">{subject}{namespace}</div>"#),
        )),
    );
    let id_part = section(
        None,
        None,
        None,
        h(format!(r#"<div class="fui-form-grid">{ehr_id}</div>"#)),
    );
    h(format!(
        r#"<form id="{NEW_EHR_FORM}" method="post" action="/ehrs/new">{}</form>"#,
        card_panel(subject_part.then(id_part))
    ))
}

/// New EHR's model.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// [`crate::error::ViewerError::Unauthenticated`] without a viewer session.
async fn load_new() -> Result<NewModel, crate::error::ViewerError> {
    crate::session::require_session()?;
    let posted = crate::posted::posted();
    if crate::pages::common::discarded(posted.as_ref()) {
        crate::request::redirect("/ehrs");
        return Ok(NewModel::Left);
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
                    return Ok(NewModel::Left);
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
    Ok(NewModel::Form {
        draft,
        failed,
        ehr_id_error,
        refused,
    })
}

/// New EHR's form, its decision bar docked.
fn new_view(model: NewModel) -> Html {
    let NewModel::Form {
        draft,
        failed,
        ehr_id_error,
        refused,
    } = model
    else {
        return Html::empty();
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
        refusal.view().then(h(format!(
            r#"<div class="fui-cluster">{}</div>"#,
            opt(open)
        )))
    });
    dock_editor(EditorDock {
        form: NEW_EHR_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save: tr(RecordsMsg::EhrNewCreate),
        blocked: None,
    });
    opt(top).then(new_form(&draft, ehr_id_error))
}

/// New EHR.
pub async fn new_page() -> Drawn {
    let body = drawn(load_new().await, new_view);
    let heading = page_heading(
        &current(),
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
    Drawn::page(
        tr(RecordsMsg::NewEhr),
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}
