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
use leptos_router::hooks::use_params_map;

use crate::flash::{Done, redirect_with};
use crate::locale::{Msg, message, tr, tr_with};
use crate::pages::common::{
    EditorDock, ago, awaited, card_panel, discarded, dock_editor, info_note, refused, section,
    state_mark,
};
use crate::subscriptions::{
    SubscriptionPredicates, SubscriptionRow, create_event_subscription, delete_event_subscription,
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
        (true, true) => tr(Msg::SubEveryVersion),
        (false, true) => tr_with(Msg::SubEveryOfKind, &[("kind", what.into())]),
        (true, false) => tr_with(
            Msg::SubEveryChange,
            &[("change", words_of(&CHANGES, change).into())],
        ),
        (false, false) => tr_with(
            Msg::SubEveryChangeOfKind,
            &[
                ("kind", what.into()),
                ("change", words_of(&CHANGES, change).into()),
            ],
        ),
    };
    if !template.is_empty() {
        line = tr_with(
            Msg::SubUnderTemplate,
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
            subtitle: Some(tr(Msg::SubSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::PLUS),
            tr(Msg::SubNew),
            Act::Link("/subscriptions/new".to_owned()),
        )),
    )
    .into_any()
}

/// The note of a CDR without the subscription API.
fn off() -> AnyView {
    card_panel(view! {
        <div class="fui-panel-body fui-stack">
            <p>{tr(Msg::SubOff)}</p>
            <p class="fui-small fui-muted">{tr(Msg::SubOffLine)}</p>
        </div>
    })
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

/// Subscriptions.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn SubscriptionsPage() -> impl IntoView {
    let body = awaited(async move {
        let rows = match list_event_subscriptions().await {
            Ok(Some(rows)) => rows,
            Ok(None) => return off(),
            Err(error) => return refused(tr(Msg::SubRefused), &error, None),
        };
        let mut top = None;
        if let Some(posted) = crate::posted::posted_for("toggle") {
            let id = posted.text("id");
            if let Some(row) = rows.iter().find(|row| row.id == id) {
                match update_event_subscription(id.clone(), predicates_of(row, !row.enabled)).await
                {
                    Ok(_) => {
                        redirect_with(
                            "/subscriptions",
                            &Done::ok(tr(if row.enabled {
                                Msg::SubDisabled
                            } else {
                                Msg::SubEnabled
                            }))
                            .line(row.name.clone()),
                        );
                        return ().into_any();
                    }
                    Err(error) => top = Some(refused(tr(Msg::SubSaveRefused), &error, None)),
                }
            }
        }
        let columns = [
            Column::text("name", tr(Msg::NavSubscriptions)).role(Role::Title),
            Column::text("state", tr(Msg::ServerState)).role(Role::Sub),
            Column::text("created", tr(Msg::ColumnCreated)).role(Role::Meta),
            Column::text("menu", tr(Msg::ColumnActions)).role(Role::Menu),
        ];
        let list = if rows.is_empty() {
            list_empty(
                tr(Msg::SubEmpty),
                Some((tr(Msg::SubNew), "/subscriptions/new".to_owned())),
            )
        } else {
            let body = rows
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let href = edit_href(&row.name);
                    let sub = view! { <span class="fui-row-sub">{summary(&row.kind, &row.change_type, &row.template_id)}</span> }.into_any();
                    let state = if row.enabled {
                        state_mark(FlowState::Up, &tr(Msg::SubStateEnabled))
                    } else {
                        state_mark(FlowState::Off, &tr(Msg::SubStateDisabled))
                    };
                    let menu = row_menu(
                        Menu::new(
                            format!("subscription-{index}"),
                            tr_with(Msg::TemplateActionsFor, &[("name", row.name.clone().into())]),
                            vec![
                                Item::new(Action::new(Some(Glyph::PENCIL), tr(Msg::PartyEdit), Act::Link(href.clone()))),
                                Item::new(Action::new(
                                    Some(if row.enabled { Glyph::BAN } else { Glyph::CHECK }),
                                    tr(if row.enabled { Msg::SubDisable } else { Msg::SubEnable }),
                                    Act::Post {
                                        action: "/subscriptions".to_owned(),
                                        fields: vec![("intent".to_owned(), "toggle".to_owned()), ("id".to_owned(), row.id.clone())],
                                    },
                                )),
                            ],
                        )
                        .destructive(vec![Item::new(Action::new(
                            Some(Glyph::TRASH),
                            tr(Msg::SubDeleteRow),
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
            count: u64::try_from(rows.len()).ok(),
            order: None,
            tabs: None,
            actions: None,
            body: list,
            pager: None,
        });
        view! { <div class="fui-list-page">{top} {card}</div> }.into_any()
    });
    view! {
        <Title text=tr(Msg::NavSubscriptions) />
        {heading()}
        {body}
    }
}

/// The fields of a subscription as the form holds them.
#[derive(Debug, Clone, Default)]
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
#[expect(
    clippy::too_many_lines,
    reason = "one page part: the reads it waits on and the view it draws"
)]
async fn form_view(form: &Form, editing: bool, name_error: Option<String>) -> AnyView {
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
    let templates: Vec<String> = crate::bff::templates::list_templates()
        .await
        .map(|rows| rows.into_iter().map(|row| row.template_id).collect())
        .unwrap_or_default();
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
                            tr(Msg::SubQueue),
                            view! { <code>{format!("ferroehr.events.{}", form.name)}</code> }
                                .into_any(),
                        ),
                    ],
                )}
                <p class="fui-help">{tr(Msg::SubNameFixed)}</p>
            },
        )
    } else {
        let error = name_error
            .clone()
            .map(|error| view! { <p class="fui-field-error">{error}</p> });
        section(
            Some(tr(Msg::QueryName)),
            None,
            None,
            view! {
                <div class="fui-form-grid">
                    <div class="fui-span-2">
                        {Field::new("sn", tr(Msg::QueryName))
                            .label_hidden()
                            .help(tr(Msg::SubNameHelp))
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
                {select("sc", "change", tr(Msg::SubChangeType), &CHANGES, &form.change)}
            </div>
            <div class="fui-span-2">
                {Field::new("st", tr(Msg::FilterTemplate))
                    .view(
                        view! {
                            <select id="st" name="template" class="fui-select">
                                <option value="">{tr(Msg::SubAnyTemplate)}</option>
                                {template_options}
                            </select>
                        },
                    )}
            </div>
        </div>
    };
    let deliver = ferrohealth_ui::view::form::keyed_check_field(
        "enabled".to_owned(),
        tr(Msg::SubDeliver),
        tr(Msg::SubDeliverHelp),
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
        {section(Some(tr(Msg::SubSelects)), None, None, what)}
        {section(Some(tr(Msg::SubDelivery)), None, None, deliver)}
    })
}

/// New subscription.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn NewSubscriptionPage() -> impl IntoView {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        leptos_axum::redirect("/subscriptions");
    }
    let dock_promise = crate::shell::promise_dock();
    let page = awaited(async move {
        let _dock_promise = dock_promise;
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
                            &Done::ok(tr(Msg::SubCreated))
                                .line(tr_with(
                                    Msg::SubCreatedLine,
                                    &[("name", row.name.clone().into())],
                                ))
                                .link(tr(Msg::PartyEdit), edit_href(&row.name)),
                        );
                        return ().into_any();
                    }
                    Err(error) => {
                        if crate::pages::common::is_status(&error, http::StatusCode::CONFLICT) {
                            name_error = Some(tr_with(
                                Msg::SubNameTaken,
                                &[("name", form.name.clone().into())],
                            ));
                        } else {
                            top = Some(refused(tr(Msg::SubSaveRefused), &error, None));
                        }
                    }
                }
            } else {
                name_error = Some(tr(Msg::SubNameInvalid));
            }
        }
        dock_editor(EditorDock {
            form: EDIT_FORM.to_owned(),
            line: summary(&form.kind, &form.change, &form.template),
            glyph: Glyph::FUNNEL,
            save: tr(Msg::SubCreate),
            // NOTE: leptos-ui.md §15.8: a refused name stays editable and the save stays live, since
            // a page with no WebAssembly has no other way to send the corrected name.
            blocked: None,
        });
        let heading = editor_heading(
            message(Msg::SubEditorTitle),
            &Mode::Creating,
            Header {
                title: String::new().into(),
                subtitle: Some(tr(Msg::SubSubtitle).into()),
                crumbs: vec![Crumb::new(tr(Msg::NavSubscriptions), "/subscriptions")],
                mono: false,
                badge: None,
            },
            ActionGroup::none(),
        );
        view! {
            <Title text=tr(Msg::SubNew) />
            {heading}
            <div class="fui-stack">
                {top} <form id=EDIT_FORM method="post" action="/subscriptions/new">
                    {form_view(&form, false, name_error).await}
                </form>
            </div>
        }
        .into_any()
    });
    view! { {page} }
}

/// Edit a subscription.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn EditSubscriptionPage() -> impl IntoView {
    let name = use_params_map()
        .with_untracked(|params| params.get("name"))
        .unwrap_or_default();
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        leptos_axum::redirect("/subscriptions");
    }
    let dock_promise = crate::shell::promise_dock();
    let page = awaited(async move {
        let _dock_promise = dock_promise;
        let rows = match list_event_subscriptions().await {
            Ok(Some(rows)) => rows,
            Ok(None) => {
                return view! {
                    {heading()}
                    {off()}
                }
                .into_any();
            }
            Err(error) => return refused(tr(Msg::SubRefused), &error, None),
        };
        let Some(row) = rows.into_iter().find(|row| row.name == name) else {
            crate::request::set_status(http::StatusCode::NOT_FOUND);
            return view! {
                {heading()}
                {info_note(tr_with(Msg::SubNone, &[("name", name.clone().into())]))}
            }
            .into_any();
        };
        let form = Form::read(Some(&row));
        let mut top = None;
        match posted
            .as_ref()
            .map(crate::posted::Posted::intent)
            .as_deref()
        {
            Some("save") => {
                match update_event_subscription(row.id.clone(), form.predicates()).await {
                    Ok(_) => {
                        redirect_with(
                            "/subscriptions",
                            &Done::ok(tr(Msg::SubSaved)).line(row.name.clone()),
                        );
                        return ().into_any();
                    }
                    Err(error) => top = Some(refused(tr(Msg::SubSaveRefused), &error, None)),
                }
            }
            Some("delete-subscription") => match delete_event_subscription(row.id.clone()).await {
                Ok(()) => {
                    redirect_with(
                        "/subscriptions",
                        &Done::ok(tr(Msg::SubDeleted)).line(row.name.clone()),
                    );
                    return ().into_any();
                }
                Err(error) => top = Some(refused(tr(Msg::SubDeleteRefused), &error, None)),
            },
            _ => {}
        }
        dock_editor(EditorDock {
            form: EDIT_FORM.to_owned(),
            line: summary(&form.kind, &form.change, &form.template),
            glyph: Glyph::FUNNEL,
            save: tr(Msg::EditorSave),
            blocked: None,
        });
        let more = Menu::new("subscription-more", tr(Msg::SubMore), Vec::new()).destructive(vec![
            Item::new(Action::new(
                Some(Glyph::TRASH),
                tr(Msg::SubDelete),
                Act::Dialog(DELETE_DIALOG.to_owned()),
            )),
        ]);
        let heading = editor_heading(
            message(Msg::SubEditorTitle),
            &Mode::Editing(row.name.clone()),
            Header {
                title: String::new().into(),
                subtitle: Some(tr(Msg::SubEditSubtitle).into()),
                crumbs: vec![Crumb::new(tr(Msg::NavSubscriptions), "/subscriptions")],
                mono: false,
                badge: None,
            },
            ActionGroup::more_only(more),
        );
        let delete = view! {
            <form method="post" action=edit_href(&row.name)>
                <input type="hidden" name="intent" value="delete-subscription" />
                <button type="submit" class="fui-button fui-button-danger">
                    {ferrohealth_ui::icon::icon(Glyph::TRASH, "16")}
                    {tr(Msg::SubDeleteConfirm)}
                </button>
            </form>
        };
        let confirm = (crate::request::query("confirm").as_deref() == Some("delete")).then(|| {
            crate::pages::common::warn_note(view! {
                <p>
                    <strong>
                        {tr_with(Msg::SubDeleteTitle, &[("name", row.name.clone().into())])}
                    </strong>
                    " "
                    {tr(Msg::SubDeleteMessage)}
                </p>
                <div class="fui-cluster">
                    <form method="post" action=edit_href(&row.name)>
                        <input type="hidden" name="intent" value="delete-subscription" />
                        <button type="submit" class="fui-button fui-button-danger">
                            {tr(Msg::SubDeleteConfirm)}
                        </button>
                    </form>
                    <a class="fui-button fui-button-secondary" href=edit_href(&row.name)>
                        {tr(Msg::Cancel)}
                    </a>
                </div>
            })
        });
        let dialog = danger_dialog(
            Confirmation {
                id: DELETE_DIALOG,
                title: tr_with(Msg::SubDeleteTitle, &[("name", row.name.clone().into())]).into(),
                message: tr(Msg::SubDeleteMessage).into(),
            },
            delete,
        );
        view! {
            <Title text=tr_with(Msg::CompositionEditTitle, &[("name", row.name.clone().into())]) />
            {heading}
            <div class="fui-stack">
                {top} {confirm} <form id=EDIT_FORM method="post" action=edit_href(&row.name)>
                    {form_view(&form, true, None).await}
                </form>
            </div>
            {dialog}
        }
        .into_any()
    });
    view! { {page} }
}
