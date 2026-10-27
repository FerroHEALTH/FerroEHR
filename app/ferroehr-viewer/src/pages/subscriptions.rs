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

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, menu_cell, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::dialog::{Confirmation, danger_dialog};
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{
    Crumb, Header, Mode, definitions, editor_heading, page_heading,
};
use ferrohealth_ui::view::template::list::{ListCard, list_card, list_empty};
use ferrohealth_ui::widget::action_menu::{Act, Action, Item, Menu, row_menu};
use leptos::prelude::*;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::flash::{Done, redirect_with};
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, message, tr, tr_with};
#[cfg(not(target_arch = "wasm32"))]
use crate::pages::common::discarded;
use crate::pages::common::{
    EditorDock, Refused, ago, card_panel, dock_editor, docked, drawn, info_note, page_model,
    section, state_mark,
};
use crate::subscriptions::SubscriptionRow;
#[cfg(not(target_arch = "wasm32"))]
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
fn heading() -> AnyView {
    page_heading(
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
    .into_any()
}

/// The note of a CDR without the subscription API.
fn off() -> AnyView {
    card_panel(view! {
        <div class="fui-panel-body fui-stack">
            <p>{tr(ConsoleMsg::SubOff)}</p>
            <p class="fui-small fui-muted">{tr(ConsoleMsg::SubOffLine)}</p>
        </div>
    })
}

#[cfg(not(target_arch = "wasm32"))]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Loads Subscriptions: the list, and the toggle a post asks for.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_list(
    /// The path and query of the page.
    href: String,
) -> Result<ListModel, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_list().await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_list`] reads, on the server.
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
fn list_view(rows: &[SubscriptionRow], top: Option<Refused>) -> AnyView {
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
                let sub = view! { <span class="fui-row-sub">{summary(&row.kind, &row.change_type, &row.template_id)}</span> }.into_any();
                let state = if row.enabled {
                    state_mark(FlowState::Up, &tr(ConsoleMsg::SubStateEnabled))
                } else {
                    state_mark(FlowState::Off, &tr(ConsoleMsg::SubStateDisabled))
                };
                let menu = row_menu(
                    Menu::new(
                        format!("subscription-{index}"),
                        tr_with(Msg::TemplateActionsFor, &[("name", row.name.clone().into())]),
                        vec![
                            Item::new(Action::new(Some(Glyph::PENCIL), tr(Msg::PartyEdit), Act::Link(href.clone()))),
                            Item::new(Action::new(
                                Some(if row.enabled { Glyph::BAN } else { Glyph::CHECK }),
                                tr(if row.enabled { ConsoleMsg::SubDisable } else { ConsoleMsg::SubEnable }),
                                Act::Post {
                                    action: "/subscriptions".to_owned(),
                                    fields: vec![("intent".to_owned(), "toggle".to_owned()), ("id".to_owned(), row.id.clone())],
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
                let name = view! { <code>{row.name.clone()}</code> }.into_any();
                view! {
                    <tr class:is-excluded=!row
                        .enabled>
                        {title_cell(Some(&href), name, Some(sub))} {cell(&columns[1], state)}
                        {cell(&columns[2], ago(&row.created_at))} {menu_cell(menu)}
                    </tr>
                }
                .into_any()
            })
            .collect_view()
            .into_any();
        let caption = tr(Msg::NavSubscriptions);
        data_table(DataTable {
            caption: &caption,
            columns: &columns,
            sorting: Sorting::default(),
            body: Body::Rows(body),
        })
        .into_any()
    };
    let card = list_card(ListCard {
        title: tr(Msg::NavSubscriptions),
        count: u64::try_from(rows.len())
            .ok()
            .map(ferrohealth_ui::view::template::list::HeadingCount::All),
        order: None,
        tabs: None,
        actions: None,
        body: list,
        pager: None,
    });
    let top = top.map(|refusal| refusal.view());
    view! { <div class="fui-list-page">{top} {card}</div> }.into_any()
}

/// Subscriptions.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn SubscriptionsPage() -> impl IntoView {
    let model = page_model!(load_list());
    let body = drawn(model, |model| match model {
        ListModel::Listed { rows, top } => list_view(&rows, top),
        ListModel::Off => off(),
        ListModel::Refused(refusal) => refusal.view(),
        ListModel::Left => ().into_any(),
    });
    view! {
        <Title text=tr(Msg::NavSubscriptions) />
        {heading()}
        {body}
    }
}

/// The fields of a subscription as the form holds them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[cfg(not(target_arch = "wasm32"))]
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
#[expect(
    clippy::too_many_lines,
    reason = "one page part: the reads it waits on and the view it draws"
)]
fn form_view(
    form: &Form,
    editing: bool,
    name_error: Option<&str>,
    templates: &[String],
) -> AnyView {
    let select = |id: &'static str,
                  name: &'static str,
                  label: String,
                  choices: &[(&str, Msg)],
                  chosen: &str| {
        let options = choices
            .iter()
            .map(|(value, words)| {
                view! {
                    <option value=(*value).to_owned() selected=*value == chosen>
                        {tr(*words)}
                    </option>
                }
            })
            .collect_view();
        Field::new(id, label).view(view! {
            <select id=id name=name class="fui-select">
                {options}
            </select>
        })
    };
    let template_options = templates
        .iter()
        .map(|template| {
            view! {
                <option value=template.clone() selected=*template == form.template>
                    {template.clone()}
                </option>
            }
        })
        .collect_view();
    let name = if editing {
        section(
            Some(tr(Msg::QueryName)),
            None,
            None,
            view! {
                <p>
                    <code>{form.name.clone()}</code>
                </p>
                {definitions(
                    vec![
                        (
                            tr(ConsoleMsg::SubQueue),
                            view! { <code>{format!("ferroehr.events.{}", form.name)}</code> }
                                .into_any(),
                        ),
                    ],
                )}
                <p class="fui-help">{tr(ConsoleMsg::SubNameFixed)}</p>
            },
        )
    } else {
        let error =
            name_error.map(|error| view! { <p class="fui-field-error">{error.to_owned()}</p> });
        section(
            Some(tr(Msg::QueryName)),
            None,
            None,
            view! {
                <div class="fui-form-grid">
                    <div class="fui-span-2">
                        {Field::new("sn", tr(Msg::QueryName))
                            .label_hidden()
                            .help(tr(ConsoleMsg::SubNameHelp))
                            .after(error)
                            .view(
                                view! {
                                    <input
                                        id="sn"
                                        name="name"
                                        type="text"
                                        class="fui-input fui-mono"
                                        spellcheck="false"
                                        placeholder="vital-signs-feed"
                                        required
                                        value=form.name.clone()
                                        aria-invalid=name_error.is_some().then_some("true")
                                    />
                                },
                            )}
                    </div>
                </div>
            },
        )
    };
    let what = view! {
        <div class="fui-form-grid">
            <div class="fui-span-2">
                {select("sk", "kind", tr(Msg::ColumnKind), &KINDS, &form.kind)}
            </div>
            <div class="fui-span-2">
                {select("sc", "change", tr(ConsoleMsg::SubChangeType), &CHANGES, &form.change)}
            </div>
            <div class="fui-span-2">
                {Field::new("st", tr(Msg::FilterTemplate))
                    .view(
                        view! {
                            <select id="st" name="template" class="fui-select">
                                <option value="">{tr(ConsoleMsg::SubAnyTemplate)}</option>
                                {template_options}
                            </select>
                        },
                    )}
            </div>
        </div>
    };
    let deliver = ferrohealth_ui::view::form::keyed_check_field(
        "enabled".to_owned(),
        tr(ConsoleMsg::SubDeliver),
        tr(ConsoleMsg::SubDeliverHelp),
        view! {
            <input type="hidden" name="enabled" value="off" />
            <input
                type="checkbox"
                id="enabled"
                name="enabled"
                class="fui-switch"
                checked=form.enabled
            />
        },
    );
    card_panel(view! {
        {name}
        {section(Some(tr(ConsoleMsg::SubSelects)), None, None, what)}
        {section(Some(tr(ConsoleMsg::SubDelivery)), None, None, deliver)}
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// The templates the editor offers; a refused read offers none.
async fn template_ids() -> Vec<String> {
    crate::bff::templates::list_templates()
        .await
        .map(|rows| rows.into_iter().map(|row| row.template_id).collect())
        .unwrap_or_default()
}

/// What the editor holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// New subscription's model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum NewModel {
    /// The editor.
    Editor(Box<Editor>),
    /// The post redirected.
    Left,
}

/// Loads New subscription: answers its post (Discard, Save) and reads the
/// templates it offers.
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
    if discarded(posted.as_ref()) {
        leptos_axum::redirect("/subscriptions");
        return NewModel::Left;
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
                    return NewModel::Left;
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
    NewModel::Editor(Box::new(Editor {
        form,
        top,
        name_error,
        templates: template_ids().await,
    }))
}

/// New subscription's editor, its decision bar docked.
fn new_editor(editor: Editor) -> AnyView {
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
        // NOTE: leptos-ui.md §15.8: a refused name stays editable and the save stays live, since
        // a page with no WebAssembly has no other way to send the corrected name.
        blocked: None,
    });
    let heading = editor_heading(
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
    view! {
        <Title text=tr(ConsoleMsg::SubNew) />
        {heading}
        <div class="fui-stack">
            {top} <form id=EDIT_FORM method="post" action="/subscriptions/new">
                {form_view(&form, false, name_error.as_deref(), &templates)}
            </form>
        </div>
    }
    .into_any()
}

/// New subscription.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn NewSubscriptionPage() -> impl IntoView {
    let dock_promise = crate::shell::promise_dock();
    let model = page_model!(load_new());
    let page = docked(model, dock_promise, |model| match model {
        NewModel::Editor(editor) => new_editor(*editor),
        NewModel::Left => ().into_any(),
    });
    view! { {page} }
}

/// Edit a subscription's model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum EditModel {
    /// The editor.
    Editor(Box<Editor>),
    /// The CDR serves no subscription API.
    Off,
    /// The CDR's refusal of the list.
    Refused(Refused),
    /// No subscription goes by this name.
    Missing(String),
    /// The post redirected.
    Left,
}

/// Loads Edit a subscription: finds it, answers its post (Discard, Save,
/// Delete) and reads the templates it offers.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_edit(
    /// The path and query of the page.
    href: String,
    /// The subscription the route names.
    name: String,
) -> Result<EditModel, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_edit(name).await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_edit`] reads, on the server.
async fn read_edit(name: String) -> EditModel {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        leptos_axum::redirect("/subscriptions");
        return EditModel::Left;
    }
    let rows = match list_event_subscriptions().await {
        Ok(Some(rows)) => rows,
        Ok(None) => return EditModel::Off,
        Err(error) => return EditModel::Refused(Refused::new(tr(ConsoleMsg::SubRefused), error)),
    };
    let Some(row) = rows.into_iter().find(|row| row.name == name) else {
        crate::request::set_status(http::StatusCode::NOT_FOUND);
        return EditModel::Missing(name);
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
                return EditModel::Left;
            }
            Err(error) => top = Some(Refused::new(tr(ConsoleMsg::SubSaveRefused), error)),
        },
        Some("delete-subscription") => match delete_event_subscription(row.id.clone()).await {
            Ok(()) => {
                redirect_with(
                    "/subscriptions",
                    &Done::ok(tr(ConsoleMsg::SubDeleted)).line(row.name.clone()),
                );
                return EditModel::Left;
            }
            Err(error) => top = Some(Refused::new(tr(ConsoleMsg::SubDeleteRefused), error)),
        },
        _ => {}
    }
    EditModel::Editor(Box::new(Editor {
        form,
        top,
        name_error: None,
        templates: template_ids().await,
    }))
}

/// Edit a subscription's editor, its decision bar docked.
fn edit_editor(editor: Editor) -> AnyView {
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
    let delete = view! {
        <form method="post" action=edit_href(&name)>
            <input type="hidden" name="intent" value="delete-subscription" />
            <button type="submit" class="fui-button fui-button-danger">
                {ferrohealth_ui::icon::icon(Glyph::TRASH, "16")}
                {tr(ConsoleMsg::SubDeleteConfirm)}
            </button>
        </form>
    };
    let confirm = (crate::request::query("confirm").as_deref() == Some("delete")).then(|| {
        crate::pages::common::warn_note(view! {
            <p>
                <strong>
                    {tr_with(ConsoleMsg::SubDeleteTitle, &[("name", name.clone().into())])}
                </strong>
                " "
                {tr(ConsoleMsg::SubDeleteMessage)}
            </p>
            <div class="fui-cluster">
                <form method="post" action=edit_href(&name)>
                    <input type="hidden" name="intent" value="delete-subscription" />
                    <button type="submit" class="fui-button fui-button-danger">
                        {tr(ConsoleMsg::SubDeleteConfirm)}
                    </button>
                </form>
                <a class="fui-button fui-button-secondary" href=edit_href(&name)>
                    {tr(Msg::Cancel)}
                </a>
            </div>
        })
    });
    let dialog = danger_dialog(
        Confirmation {
            id: DELETE_DIALOG.into(),
            title: tr_with(ConsoleMsg::SubDeleteTitle, &[("name", name.clone().into())]).into(),
            message: tr(ConsoleMsg::SubDeleteMessage).into(),
        },
        delete,
    );
    let top = top.map(|refusal| refusal.view());
    view! {
        <Title text=tr_with(Msg::CompositionEditTitle, &[("name", name.clone().into())]) />
        {heading}
        <div class="fui-stack">
            {top} {confirm} <form id=EDIT_FORM method="post" action=edit_href(&name)>
                {form_view(&form, true, None, &templates)}
            </form>
        </div>
        {dialog}
    }
    .into_any()
}

/// Edit a subscription.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn EditSubscriptionPage() -> impl IntoView {
    let dock_promise = crate::shell::promise_dock();
    let name = crate::routes::params()
        .with(|params| params.get("name"))
        .unwrap_or_default();
    let model = page_model!(load_edit(name));
    let page = docked(model, dock_promise, |model| match model {
        EditModel::Editor(editor) => edit_editor(*editor),
        EditModel::Off => view! {
            {heading()}
            {off()}
        }
        .into_any(),
        EditModel::Refused(refusal) => refusal.view(),
        EditModel::Missing(name) => view! {
            {heading()}
            {info_note(tr_with(ConsoleMsg::SubNone, &[("name", name.into())]))}
        }
        .into_any(),
        EditModel::Left => ().into_any(),
    });
    view! { {page} }
}
