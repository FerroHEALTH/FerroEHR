// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Subscriptions: a message on the event queue for each committed version
//! that matches, the list and its editor (FerroUI
//! `design/consoles/ferroehr/`, pages 21.1 to 21.8).
//!
//! A subscription is a name and a flat predicate record, the kind, the
//! change type and the template, each empty meaning any, with a flag that
//! keeps it but stops its delivery ([`crate::subscriptions`]). The group is
//! the CDR's own extension and off by default; a CDR without it answers
//! `404` and the page says so. No openEHR spec governs eventing: our own
//! design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, menu_cell, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action, Item, Menu, row_menu};
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, danger_form_dialog};
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{
    Crumb, Header, Mode, definitions, editor_heading, page_heading,
};
use ferrohealth_ui::view::template::list::{ListCard, list_card, list_empty};

use crate::flash::{Done, redirect_with};
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, current, message, tr, tr_with};
use crate::markup::{attr, flag, h, opt, t};
use crate::pages::common::discarded;
use crate::pages::common::{
    EditorDock, Refused, ago, card_panel, dock_editor, info_note, section, state_mark,
};
use crate::pages::form;
use crate::routes::Drawn;
use crate::subscriptions::SubscriptionRow;
use crate::subscriptions::{
    SubscriptionPredicates, create_event_subscription, delete_event_subscription,
    list_event_subscriptions, name_is_valid, update_event_subscription,
};

/// The id of Delete subscription's confirmation dialog.
const DELETE_DIALOG: &str = "delete-subscription";

/// The id of the editor's form.
const EDIT_FORM: &str = "subscription-edit";

/// The kinds a subscription selects.
const KINDS: [(&str, Msg); 4] = [
    ("", Msg::SubAnyKind),
    ("COMPOSITION", Msg::SubComposition),
    ("EHR_STATUS", Msg::SubEhrStatus),
    ("FOLDER", Msg::SubFolder),
];

/// The change types a subscription selects (openEHR `audit_change_type`).
const CHANGES: [(&str, Msg); 5] = [
    ("", Msg::SubAnyChange),
    ("249", Msg::ChangeCreation),
    ("250", Msg::ChangeAmendment),
    ("251", Msg::ChangeModification),
    ("523", Msg::ChangeDeleted),
];

/// The words of a predicate value among `choices`, the value itself when it
/// is not one of them.
fn words_of(choices: &[(&str, Msg)], value: &str) -> String {
    choices
        .iter()
        .find(|(key, _)| *key == value)
        .map_or_else(|| value.to_owned(), |(_, words)| tr(*words))
}

/// One sentence for what a subscription selects, the same in its row and its
/// editor's dock.
fn summary(kind: &str, change: &str, template: &str) -> String {
    let what = words_of(&KINDS, kind);
    let mut line = match (kind.is_empty(), change.is_empty()) {
        (true, true) => tr(ConsoleMsg::SubEveryVersion),
        (false, true) => tr_with(ConsoleMsg::SubEveryOfKind, &[("kind", what.into())]),
        (true, false) => tr_with(
            ConsoleMsg::SubEveryChange,
            &[("change", words_of(&CHANGES, change).into())],
        ),
        (false, false) => tr_with(
            ConsoleMsg::SubEveryChangeOfKind,
            &[
                ("kind", what.into()),
                ("change", words_of(&CHANGES, change).into()),
            ],
        ),
    };
    if !template.is_empty() {
        line = tr_with(
            ConsoleMsg::SubUnderTemplate,
            &[
                ("line", line.into()),
                ("template", template.to_owned().into()),
            ],
        );
    }
    line
}

/// The editor of one subscription.
fn edit_href(name: &str) -> String {
    format!("/subscriptions/{}/edit", urlencoding::encode(name))
}

/// The heading of Subscriptions.
fn heading() -> Html {
    page_heading(
        &current(),
        Header {
            title: tr(Msg::NavSubscriptions).into(),
            subtitle: Some(tr(ConsoleMsg::SubSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::PLUS),
            tr(ConsoleMsg::SubNew),
            Act::Link("/subscriptions/new".to_owned()),
        )),
    )
}

/// The note of a CDR without the subscription API.
fn off() -> Html {
    card_panel(h(format!(
        r#"<div class="fui-panel-body fui-stack"><p>{}</p><p class="fui-small fui-muted">{}</p></div>"#,
        t(tr(ConsoleMsg::SubOff)),
        t(tr(ConsoleMsg::SubOffLine))
    )))
}

/// The predicates of a subscription row, with `enabled` set.
fn predicates_of(row: &SubscriptionRow, enabled: bool) -> SubscriptionPredicates {
    SubscriptionPredicates {
        kind: row.kind.clone(),
        change_type: row.change_type.clone(),
        template_id: row.template_id.clone(),
        enabled,
    }
}

/// Subscriptions' model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ListModel {
    /// The subscriptions, and the CDR's refusal of a toggle.
    Listed {
        /// The subscriptions.
        rows: Vec<SubscriptionRow>,
        /// The refused toggle, when the post was one.
        top: Option<Refused>,
    },
    /// The CDR serves no subscription API.
    Off,
    /// The CDR's refusal of the list.
    Refused(Refused),
    /// The post redirected.
    Left,
}

/// What the page reads.
async fn read_list() -> ListModel {
    let rows = match list_event_subscriptions().await {
        Ok(Some(rows)) => rows,
        Ok(None) => return ListModel::Off,
        Err(error) => return ListModel::Refused(Refused::new(tr(ConsoleMsg::SubRefused), error)),
    };
    let mut top = None;
    if let Some(posted) = crate::posted::posted_for("toggle") {
        let id = posted.text("id");
        if let Some(row) = rows.iter().find(|row| row.id == id) {
            match update_event_subscription(id.clone(), predicates_of(row, !row.enabled)).await {
                Ok(_) => {
                    redirect_with(
                        "/subscriptions",
                        &Done::ok(tr(if row.enabled {
                            ConsoleMsg::SubDisabled
                        } else {
                            ConsoleMsg::SubEnabled
                        }))
                        .line(row.name.clone()),
                    );
                    return ListModel::Left;
                }
                Err(error) => top = Some(Refused::new(tr(ConsoleMsg::SubSaveRefused), error)),
            }
        }
    }
    ListModel::Listed { rows, top }
}

/// The subscriptions as a list card.
#[expect(
    clippy::too_many_lines,
    reason = "one listing: its rows with their menus, its empty state and its card"
)]
fn list_view(rows: &[SubscriptionRow], top: Option<Refused>) -> Html {
    let columns = [
        Column::text("name", tr(Msg::NavSubscriptions)).role(Role::Title),
        Column::text("state", tr(ConsoleMsg::ServerState)).role(Role::Sub),
        Column::text("created", tr(Msg::ColumnCreated)).role(Role::Meta),
        Column::text("menu", tr(Msg::ColumnActions)).role(Role::Menu),
    ];
    let list = if rows.is_empty() {
        list_empty(
            tr(ConsoleMsg::SubEmpty),
            Some((tr(ConsoleMsg::SubNew), "/subscriptions/new".to_owned())),
        )
    } else {
        let body = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let href = edit_href(&row.name);
                let sub = h(format!(
                    r#"<span class="fui-row-sub">{}</span>"#,
                    t(summary(&row.kind, &row.change_type, &row.template_id))
                ));
                let state = if row.enabled {
                    state_mark(FlowState::Up, &tr(ConsoleMsg::SubStateEnabled))
                } else {
                    state_mark(FlowState::Off, &tr(ConsoleMsg::SubStateDisabled))
                };
                let menu = row_menu(
                    &current(),
                    &Menu::new(
                        format!("subscription-{index}"),
                        tr_with(
                            Msg::TemplateActionsFor,
                            &[("name", row.name.clone().into())],
                        ),
                        vec![
                            Item::new(Action::new(
                                Some(Glyph::PENCIL),
                                tr(Msg::PartyEdit),
                                Act::Link(href.clone()),
                            )),
                            Item::new(Action::new(
                                Some(if row.enabled {
                                    Glyph::BAN
                                } else {
                                    Glyph::CHECK
                                }),
                                tr(if row.enabled {
                                    ConsoleMsg::SubDisable
                                } else {
                                    ConsoleMsg::SubEnable
                                }),
                                Act::Post {
                                    action: "/subscriptions".to_owned(),
                                    fields: vec![
                                        ("intent".to_owned(), "toggle".to_owned()),
                                        ("id".to_owned(), row.id.clone()),
                                    ],
                                },
                            )),
                        ],
                    )
                    .destructive(vec![Item::new(Action::new(
                        Some(Glyph::TRASH),
                        tr(ConsoleMsg::SubDeleteRow),
                        Act::Link(format!("{href}?confirm=delete")),
                    ))]),
                );
                let name = h(format!("<code>{}</code>", t(&row.name)));
                h(format!(
                    "<tr{}>{}{}{}{}</tr>",
                    attr("class", (!row.enabled).then_some("is-excluded")),
                    title_cell(Some(&href), name, Some(sub)),
                    cell(&columns[1], state),
                    cell(&columns[2], ago(&row.created_at)),
                    menu_cell(menu)
                ))
            })
            .collect();
        let caption = tr(Msg::NavSubscriptions);
        data_table(DataTable {
            caption: &caption,
            columns: &columns,
            sorting: Sorting::default(),
            body: Body::Rows(body),
        })
    };
    let card = list_card(
        &current(),
        ListCard {
            title: tr(Msg::NavSubscriptions),
            count: u64::try_from(rows.len())
                .ok()
                .map(ferrohealth_ui::view::template::list::HeadingCount::All),
            order: None,
            tabs: None,
            actions: None,
            body: list,
            pager: None,
        },
    );
    let top = top.map(|refusal| refusal.view());
    h(format!(
        r#"<div class="fui-list-page">{}{card}</div>"#,
        opt(top)
    ))
}

/// Subscriptions.
/// Subscriptions.
pub async fn page() -> Drawn {
    let body = match read_list().await {
        ListModel::Listed { rows, top } => list_view(&rows, top),
        ListModel::Off => off(),
        ListModel::Refused(refusal) => refusal.view(),
        ListModel::Left => Html::empty(),
    };
    Drawn::page(tr(Msg::NavSubscriptions), heading().then(body))
}

/// The fields of a subscription as the form holds them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Form {
    /// Its name.
    name: String,
    /// The kind it selects.
    kind: String,
    /// The change type it selects.
    change: String,
    /// The template it selects.
    template: String,
    /// Whether it delivers.
    enabled: bool,
}

impl Form {
    /// The form as posted, or as `row` holds it.
    fn read(row: Option<&SubscriptionRow>) -> Self {
        match crate::posted::posted() {
            Some(posted) => Self {
                name: row.map_or_else(|| posted.text("name"), |row| row.name.clone()),
                kind: posted.text("kind"),
                change: posted.text("change"),
                template: posted.text("template"),
                enabled: posted.checked("enabled"),
            },
            None => row.map_or_else(
                || Self {
                    enabled: true,
                    ..Self::default()
                },
                |row| Self {
                    name: row.name.clone(),
                    kind: row.kind.clone(),
                    change: row.change_type.clone(),
                    template: row.template_id.clone(),
                    enabled: row.enabled,
                },
            ),
        }
    }

    /// The predicates the form sets.
    fn predicates(&self) -> SubscriptionPredicates {
        SubscriptionPredicates {
            kind: self.kind.clone(),
            change_type: self.change.clone(),
            template_id: self.template.clone(),
            enabled: self.enabled,
        }
    }
}

/// The editor's sections: the name (fixed once created), what it selects,
/// and its delivery.
fn form_view(form: &Form, editing: bool, name_error: Option<&str>, templates: &[String]) -> Html {
    let option = |value: &str, words: &str, selected: bool| {
        h(format!(
            r#"<option value="{}"{}>{}</option>"#,
            t(value),
            flag("selected", selected),
            t(words)
        ))
    };
    let select = |id: &'static str,
                  name: &'static str,
                  label: String,
                  choices: &[(&str, Msg)],
                  chosen: &str| {
        let options: Html = choices
            .iter()
            .map(|(value, words)| option(value, &tr(*words), *value == chosen))
            .collect();
        Field::new(id, label).view(h(format!(
            r#"<select id="{id}" name="{name}" class="fui-select">{options}</select>"#
        )))
    };
    let template_options: Html = templates
        .iter()
        .map(|template| option(template, template, *template == form.template))
        .collect();
    let name = if editing {
        section(
            Some(tr(Msg::QueryName)),
            None,
            None,
            Html::join([
                h(format!("<p><code>{}</code></p>", t(&form.name))),
                definitions(vec![(
                    tr(ConsoleMsg::SubQueue),
                    h(format!(
                        "<code>{}</code>",
                        t(format!("ferroehr.events.{}", form.name))
                    )),
                )]),
                h(format!(
                    r#"<p class="fui-help">{}</p>"#,
                    t(tr(ConsoleMsg::SubNameFixed))
                )),
            ]),
        )
    } else {
        let mut field = Field::new("sn", tr(Msg::QueryName))
            .label_hidden()
            .help(tr(ConsoleMsg::SubNameHelp));
        if let Some(error) = name_error {
            field = field.after(h(format!(r#"<p class="fui-field-error">{}</p>"#, t(error))));
        }
        let field = field.view(h(format!(
            r#"<input id="sn" name="name" type="text" class="fui-input fui-mono" spellcheck="false" placeholder="vital-signs-feed" required value="{}"{}>"#,
            t(&form.name),
            attr("aria-invalid", name_error.is_some().then_some("true"))
        )));
        section(
            Some(tr(Msg::QueryName)),
            None,
            None,
            h(format!(
                r#"<div class="fui-form-grid"><div class="fui-span-2">{field}</div></div>"#
            )),
        )
    };
    let what = h(format!(
        r#"<div class="fui-form-grid"><div class="fui-span-2">{}</div><div class="fui-span-2">{}</div><div class="fui-span-2">{}</div></div>"#,
        select("sk", "kind", tr(Msg::ColumnKind), &KINDS, &form.kind),
        select(
            "sc",
            "change",
            tr(ConsoleMsg::SubChangeType),
            &CHANGES,
            &form.change
        ),
        Field::new("st", tr(Msg::FilterTemplate)).view(h(format!(
            r#"<select id="st" name="template" class="fui-select"><option value="">{}</option>{template_options}</select>"#,
            t(tr(ConsoleMsg::SubAnyTemplate))
        ))),
    ));
    let deliver = ferrohealth_ui::view::form::keyed_check_field(
        "enabled".to_owned(),
        tr(ConsoleMsg::SubDeliver),
        tr(ConsoleMsg::SubDeliverHelp),
        h(format!(
            r#"<input type="hidden" name="enabled" value="off"><input type="checkbox" id="enabled" name="enabled" class="fui-switch"{}>"#,
            flag("checked", form.enabled)
        )),
    );
    card_panel(Html::join([
        name,
        section(Some(tr(ConsoleMsg::SubSelects)), None, None, what),
        section(Some(tr(ConsoleMsg::SubDelivery)), None, None, deliver),
    ]))
}

/// The templates the editor offers; a refused read offers none.
async fn template_ids() -> Vec<String> {
    crate::bff::templates::list_templates()
        .await
        .map(|rows| rows.into_iter().map(|row| row.template_id).collect())
        .unwrap_or_default()
}

/// What the editor holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Editor {
    /// The form.
    form: Form,
    /// The CDR's refusal of a save or a delete.
    top: Option<Refused>,
    /// Why the name was refused.
    name_error: Option<String>,
    /// The templates it offers.
    templates: Vec<String>,
}

/// What the page reads.
async fn read_new() -> form::Form<Editor> {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        crate::request::redirect("/subscriptions");
        return form::Form::Left;
    }
    let form = Form::read(None);
    let mut top = None;
    let mut name_error = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        if name_is_valid(&form.name) {
            match create_event_subscription(form.name.clone(), form.predicates()).await {
                Ok(row) => {
                    redirect_with(
                        "/subscriptions",
                        &Done::ok(tr(ConsoleMsg::SubCreated))
                            .line(tr_with(
                                ConsoleMsg::SubCreatedLine,
                                &[("name", row.name.clone().into())],
                            ))
                            .link(tr(Msg::PartyEdit), edit_href(&row.name)),
                    );
                    return form::Form::Left;
                }
                Err(error) => {
                    if crate::pages::common::is_status(&error, http::StatusCode::CONFLICT) {
                        name_error = Some(tr_with(
                            ConsoleMsg::SubNameTaken,
                            &[("name", form.name.clone().into())],
                        ));
                    } else {
                        top = Some(Refused::new(tr(ConsoleMsg::SubSaveRefused), error));
                    }
                }
            }
        } else {
            name_error = Some(tr(ConsoleMsg::SubNameInvalid));
        }
    }
    form::Form::Shown(Box::new(Editor {
        form,
        top,
        name_error,
        templates: template_ids().await,
    }))
}

/// New subscription's editor, its decision bar docked.
fn new_editor(editor: Editor) -> Html {
    let Editor {
        form,
        top,
        name_error,
        templates,
    } = editor;
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line: summary(&form.kind, &form.change, &form.template),
        glyph: Glyph::FUNNEL,
        save: tr(ConsoleMsg::SubCreate),
        // NOTE: FerroUI console-ui.md §5: a refused name stays editable and the save stays live,
        // since a page with no script has no other way to send the corrected name.
        blocked: None,
    });
    let heading = editor_heading(
        &current(),
        message(ConsoleMsg::SubEditorTitle),
        &Mode::Creating,
        Header {
            title: String::new().into(),
            subtitle: Some(tr(ConsoleMsg::SubSubtitle).into()),
            crumbs: vec![Crumb::new(tr(Msg::NavSubscriptions), "/subscriptions")],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let top = top.map(|refusal| refusal.view());
    heading.then(h(format!(
        r#"<div class="fui-stack">{}<form id="{EDIT_FORM}" method="post" action="/subscriptions/new">{}</form></div>"#,
        opt(top),
        form_view(&form, false, name_error.as_deref(), &templates)
    )))
}

/// New subscription.
pub async fn new_page() -> Drawn {
    let page = match read_new().await {
        form::Form::Shown(editor) => new_editor(*editor),
        form::Form::Refused(refusal) => refusal.view(),
        form::Form::Left => Html::empty(),
    };
    Drawn::page(tr(ConsoleMsg::SubNew), page)
}

/// Why Edit subscription draws no form.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Absent {
    /// The CDR serves no subscription API.
    Off,
    /// No subscription goes by this name.
    Missing(String),
}

/// What the page reads.
async fn read_edit(name: String) -> form::Form<Editor, Absent> {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        crate::request::redirect("/subscriptions");
        return form::Form::Left;
    }
    let rows = match list_event_subscriptions().await {
        Ok(Some(rows)) => rows,
        Ok(None) => return form::Form::Not(Absent::Off),
        Err(error) => return form::Form::Refused(Refused::new(tr(ConsoleMsg::SubRefused), error)),
    };
    let Some(row) = rows.into_iter().find(|row| row.name == name) else {
        crate::request::set_status(http::StatusCode::NOT_FOUND);
        return form::Form::Not(Absent::Missing(name));
    };
    let form = Form::read(Some(&row));
    let mut top = None;
    match posted
        .as_ref()
        .map(crate::posted::Posted::intent)
        .as_deref()
    {
        Some("save") => match update_event_subscription(row.id.clone(), form.predicates()).await {
            Ok(_) => {
                redirect_with(
                    "/subscriptions",
                    &Done::ok(tr(ConsoleMsg::SubSaved)).line(row.name.clone()),
                );
                return form::Form::Left;
            }
            Err(error) => top = Some(Refused::new(tr(ConsoleMsg::SubSaveRefused), error)),
        },
        Some("delete-subscription") => match delete_event_subscription(row.id.clone()).await {
            Ok(()) => {
                redirect_with(
                    "/subscriptions",
                    &Done::ok(tr(ConsoleMsg::SubDeleted)).line(row.name.clone()),
                );
                return form::Form::Left;
            }
            Err(error) => top = Some(Refused::new(tr(ConsoleMsg::SubDeleteRefused), error)),
        },
        _ => {}
    }
    form::Form::Shown(Box::new(Editor {
        form,
        top,
        name_error: None,
        templates: template_ids().await,
    }))
}

/// Edit a subscription's editor, its decision bar docked.
fn edit_editor(editor: Editor) -> (String, Html) {
    let Editor {
        form,
        top,
        templates,
        ..
    } = editor;
    let name = form.name.clone();
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line: summary(&form.kind, &form.change, &form.template),
        glyph: Glyph::FUNNEL,
        save: tr(Msg::EditorSave),
        blocked: None,
    });
    let more =
        Menu::new("subscription-more", tr(ConsoleMsg::SubMore), Vec::new()).destructive(vec![
            Item::new(Action::new(
                Some(Glyph::TRASH),
                tr(ConsoleMsg::SubDelete),
                Act::Dialog(DELETE_DIALOG.to_owned()),
            )),
        ]);
    let heading = editor_heading(
        &current(),
        message(ConsoleMsg::SubEditorTitle),
        &Mode::Editing(name.clone()),
        Header {
            title: String::new().into(),
            subtitle: Some(tr(ConsoleMsg::SubEditSubtitle).into()),
            crumbs: vec![Crumb::new(tr(Msg::NavSubscriptions), "/subscriptions")],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::more_only(more),
    );
    let delete_fields = r#"<input type="hidden" name="intent" value="delete-subscription">"#;
    let confirm = (crate::request::query("confirm").as_deref() == Some("delete")).then(|| {
        crate::pages::common::warn_note(h(format!(
            r#"<p><strong>{title}</strong> {message}</p><div class="fui-cluster"><form method="post" action="{href}">{delete_fields}<button type="submit" class="fui-button fui-button-danger">{confirm}</button></form><a class="fui-button fui-button-secondary" href="{href}">{cancel}</a></div>"#,
            title = t(tr_with(
                ConsoleMsg::SubDeleteTitle,
                &[("name", name.clone().into())]
            )),
            message = t(tr(ConsoleMsg::SubDeleteMessage)),
            href = t(edit_href(&name)),
            confirm = t(tr(ConsoleMsg::SubDeleteConfirm)),
            cancel = t(tr(Msg::Cancel)),
        )))
    });
    let dialog = danger_form_dialog(
        &current(),
        Confirmation {
            id: DELETE_DIALOG.into(),
            title: tr_with(ConsoleMsg::SubDeleteTitle, &[("name", name.clone().into())]).into(),
            message: tr(ConsoleMsg::SubDeleteMessage).into(),
        },
        ConfirmForm {
            action: edit_href(&name),
            fields: h(delete_fields.to_owned()),
        },
        h(format!(
            r#"<button type="submit" class="fui-button fui-button-danger">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::TRASH, "16"),
            t(tr(ConsoleMsg::SubDeleteConfirm))
        )),
    );
    let top = top.map(|refusal| refusal.view());
    let body = Html::join([
        heading,
        h(format!(
            r#"<div class="fui-stack">{}{}<form id="{EDIT_FORM}" method="post" action="{}">{}</form></div>"#,
            opt(top),
            opt(confirm),
            t(edit_href(&name)),
            form_view(&form, true, None, &templates)
        )),
        dialog,
    ]);
    (
        tr_with(Msg::CompositionEditTitle, &[("name", name.into())]),
        body,
    )
}

/// Edit a subscription.
pub async fn edit_page() -> Drawn {
    let name = crate::routes::params().get("name").unwrap_or_default();
    let (title, page) = match read_edit(name).await {
        form::Form::Shown(editor) => edit_editor(*editor),
        form::Form::Not(Absent::Off) => (tr(Msg::NavSubscriptions), heading().then(off())),
        form::Form::Refused(refusal) => (tr(Msg::NavSubscriptions), refusal.view()),
        form::Form::Not(Absent::Missing(name)) => (
            tr(Msg::NavSubscriptions),
            heading().then(info_note(t(tr_with(
                ConsoleMsg::SubNone,
                &[("name", name.into())],
            )))),
        ),
        form::Form::Left => (String::new(), Html::empty()),
    };
    Drawn::page(title, page)
}
