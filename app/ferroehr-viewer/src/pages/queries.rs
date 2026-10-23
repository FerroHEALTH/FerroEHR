// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stored queries: the CDR's stored queries grouped by the namespace of
//! their name, and one stored query with Definition, Run and Versions
//! (FerroUI `design/consoles/ferroehr/`, pages 18.1 to 18.7).
//!
//! A stored query is a qualified name and an immutable version (ITS-REST
//! `docs/query/Qualified_query_name.md`); Run sends the latest version, a
//! version prefix or an exact version, as the reader picks
//! ([`crate::query_namespace::VersionResolution`]). Deleting one version is
//! the CDR's admin operation. No openEHR spec governs the viewer's pages: our
//! own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, group_row, menu_cell, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::dialog::{Confirmation, danger_dialog};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::segmented::choice_group;
use ferrohealth_ui::view::surface::{Crumb, Header, definitions, page_heading};
use ferrohealth_ui::view::tabs::{Tab, binder};
use ferrohealth_ui::view::template::list::{
    ActiveFilters, FilterChip, ListCard, active_filters, list_card, list_empty,
};
use ferrohealth_ui::view::versions::{Standing, version_cell};
use ferrohealth_ui::widget::action_menu::{Act, Action, Item, Menu, row_menu};
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;

use crate::flash::{Done, redirect_with};
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{
    awaited, count, document_section, info_note, panel, plain_section, refused, section, when,
};
use crate::pages::results::{Answered, chart_asked, not_run, results, shown_choice};
use crate::queries_api::{
    StoredQueryRow, fetch_stored_query, list_stored_queries, run_stored_query,
};
use crate::query_namespace::{
    VersionResolution, bare_name_of, group_by_namespace, resolve_version,
};

/// The key the dashboard's namespace tile names the unqualified queries by.
pub const UNQUALIFIED: &str = "-";

/// The id of Delete version's confirmation dialog.
const DELETE_DIALOG: &str = "delete-stored-query";

/// The id of Run's form.
const RUN_FORM: &str = "stored-query-run";

/// The page of version `version` of the stored query `name`.
#[must_use]
pub fn stored_query_href(name: &str, version: &str) -> String {
    format!(
        "/queries/{}/{}",
        urlencoding::encode(name),
        urlencoding::encode(version)
    )
}

/// The Versions tab of the stored query `name`.
fn versions_href(name: &str) -> String {
    format!("/queries/{}/versions", urlencoding::encode(name))
}

/// Query's AQL tab with version `version` of `name` loaded.
fn open_in_query(name: &str, version: &str) -> String {
    format!(
        "/query/aql?name={}&version={}",
        urlencoding::encode(name),
        urlencoding::encode(version)
    )
}

/// The order of two semantic versions, newest first.
fn newest_first(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |version: &str| -> Vec<u64> {
        version
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    key(b).cmp(&key(a))
}

/// The parameters a query text names, as code, or a dash for none.
fn parameters(aql: &str) -> AnyView {
    let names = crate::aql_text::placeholders(aql);
    if names.is_empty() {
        return view! { <span class="fui-muted">{tr(Msg::RelationshipNone)}</span> }.into_any();
    }
    names
        .into_iter()
        .map(|name| view! { <code>{name}</code> " " })
        .collect_view()
        .into_any()
}

/// The stored queries grouped by namespace, as a table.
fn table(rows: &[StoredQueryRow], only: Option<&str>) -> (AnyView, usize, usize) {
    let columns = [
        Column::text("name", tr(Msg::QueryName)).role(Role::Title),
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Sub),
        Column::text("parameters", tr(Msg::QueryParameters)).role(Role::Meta),
        Column::text("menu", tr(Msg::ColumnActions)).role(Role::Menu),
    ];
    let mut body = Vec::new();
    let mut names = 0;
    let mut versions = 0;
    let mut index = 0_usize;
    for group in group_by_namespace(rows) {
        let key = group
            .namespace
            .clone()
            .unwrap_or_else(|| UNQUALIFIED.to_owned());
        if only.is_some_and(|only| only != key) {
            continue;
        }
        let mut distinct: Vec<&str> = group.members.iter().map(|row| row.name.as_str()).collect();
        distinct.sort_unstable();
        distinct.dedup();
        names += distinct.len();
        versions += group.members.len();
        let label = match &group.namespace {
            Some(namespace) => view! { <code>{namespace.clone()}</code> }.into_any(),
            None => tr(Msg::QueriesUnqualified).into_any(),
        };
        body.push(group_row(
            &columns,
            view! {
                {label}
                <span class="fui-small fui-muted">
                    {format!(
                        " · {}",
                        tr_with(Msg::QueriesGroupCount, &[("names", count(distinct.len())), ("versions", count(group.members.len()))]),
                    )}
                </span>
            },
        ));
        for row in &group.members {
            let older = group.members.iter().any(|other| {
                other.name == row.name && newest_first(&other.version, &row.version).is_lt()
            });
            let href = stored_query_href(&row.name, &row.version);
            let sub = older.then(|| {
                view! { <span class="fui-row-sub">{tr(Msg::QueriesOlderVersion)}</span> }.into_any()
            });
            let menu = row_menu(
                Menu::new(
                    format!("stored-{index}"),
                    tr_with(
                        Msg::TemplateActionsFor,
                        &[("name", bare_name_of(&row.name).to_owned().into())],
                    ),
                    vec![
                        Item::new(Action::new(
                            Some(Glyph::PLAY),
                            tr(Msg::QueryRun),
                            Act::Link(format!("{href}/run")),
                        )),
                        Item::new(Action::new(
                            Some(Glyph::SQUARE_TERMINAL),
                            tr(Msg::QueriesOpenInQuery),
                            Act::Link(open_in_query(&row.name, &row.version)),
                        )),
                    ],
                )
                .destructive(vec![Item::new(Action::new(
                    Some(Glyph::TRASH),
                    tr(Msg::QueriesDeleteVersion),
                    Act::Link(format!("{href}?confirm=delete")),
                ))]),
            );
            body.push(
                view! {
                    <tr>
                        {title_cell(Some(&href), bare_name_of(&row.name).to_owned(), sub)}
                        {cell(&columns[1], row.version.clone())}
                        {cell(&columns[2], parameters(&row.query))}
                        {menu_cell(menu)}
                    </tr>
                }
                .into_any(),
            );
            index = index.saturating_add(1);
        }
    }
    let caption = tr(Msg::NavStoredQueries);
    let table = data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(body.collect_view().into_any()),
    })
    .into_any();
    (table, names, versions)
}

/// Stored queries.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn StoredQueriesPage() -> impl IntoView {
    let only = crate::request::query("namespace").filter(|namespace| !namespace.is_empty());
    let heading = page_heading(
        Header {
            title: tr(Msg::NavStoredQueries).into(),
            subtitle: Some(tr(Msg::QueriesSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
        },
        ActionGroup::none(),
    );
    let body = awaited(async move {
        let write = ferrohealth_ui::view::button::link_button(
            "/query/aql".to_owned(),
            ferrohealth_ui::view::button::Variant::Ghost,
            Some(Glyph::SQUARE_TERMINAL),
            tr(Msg::DashboardWriteOne),
        );
        let (list, total) = match list_stored_queries().await {
            Ok(rows) if rows.is_empty() => {
                (list_empty(tr(Msg::DashboardNoStoredQuery), None), Some(0))
            }
            Ok(rows) => {
                let (table, names, _versions) = table(&rows, only.as_deref());
                (table, u64::try_from(names).ok())
            }
            Err(error) => (refused(tr(Msg::QueriesRefused), &error, None), None),
        };
        let chips = only.as_ref().map(|namespace| {
            let words = if namespace == UNQUALIFIED {
                tr(Msg::QueriesUnqualified)
            } else {
                namespace.clone()
            };
            active_filters(
                ActiveFilters {
                    chips: vec![FilterChip {
                        label: tr_with(Msg::QueriesNamespaceChip, &[("value", words.into())]),
                        remove: "/queries".to_owned(),
                    }],
                    clear: "/queries".to_owned(),
                    shown: None,
                },
                None,
            )
        });
        let card = list_card(ListCard {
            title: tr(Msg::NavStoredQueries),
            count: total,
            order: None,
            tabs: None,
            actions: Some(write),
            body: list,
            pager: None,
        });
        view! { <div class="fui-list-page">{chips} {card}</div> }.into_any()
    });
    view! {
        <Title text=tr(Msg::NavStoredQueries) />
        {heading}
        {body}
    }
}

/// The name and the version the route names.
fn params() -> (String, String) {
    use_params_map().with_untracked(|params| {
        (
            params.get("name").unwrap_or_default(),
            params.get("version").unwrap_or_default(),
        )
    })
}

/// The tabs of a stored query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryTab {
    /// The definition.
    Definition,
    /// Run it.
    Run,
    /// Its versions.
    Versions,
}

impl QueryTab {
    /// The tab's key.
    const fn key(self) -> &'static str {
        match self {
            Self::Definition => "definition",
            Self::Run => "run",
            Self::Versions => "versions",
        }
    }
}

/// A stored query's page: its header with Open in Query and Delete version,
/// its tabs over `body`, and the delete confirmation.
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
async fn query_page<F, Fut>(name: String, version: String, tab: QueryTab, body: F) -> AnyView
where
    F: FnOnce(Vec<StoredQueryRow>, String) -> Fut,
    Fut: Future<Output = AnyView>,
{
    let acted = match crate::posted::posted_for("delete-version") {
        Some(posted) => {
            match crate::admin::admin_delete_stored_query(name.clone(), posted.text("version"))
                .await
            {
                Ok(()) => {
                    redirect_with(
                        "/queries",
                        &Done::ok(tr(Msg::QueriesDeleted))
                            .line(format!("{name} {}", posted.text("version"))),
                    );
                    return ().into_any();
                }
                Err(error) => Some(refused(tr(Msg::QueriesDeleteRefused), &error, None)),
            }
        }
        None => None,
    };
    let rows = list_stored_queries().await;
    let mut versions: Vec<StoredQueryRow> = rows
        .as_ref()
        .map(|rows| {
            rows.iter()
                .filter(|row| row.name == name)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    versions.sort_by(|a, b| newest_first(&a.version, &b.version));
    let latest = versions
        .first()
        .map(|row| row.version.clone())
        .unwrap_or_default();
    let version = if version.is_empty() {
        latest.clone()
    } else {
        version
    };
    let older = !latest.is_empty() && version != latest;
    let mut tabs = vec![Tab::new(
        QueryTab::Definition.key(),
        stored_query_href(&name, &version),
        tr(Msg::QueriesDefinition),
    )];
    if !older {
        tabs.push(Tab::new(
            QueryTab::Run.key(),
            format!("{}/run", stored_query_href(&name, &version)),
            tr(Msg::QueryRun),
        ));
    }
    tabs.push(
        Tab::new(
            QueryTab::Versions.key(),
            versions_href(&name),
            tr(Msg::CompositionVersions),
        )
        .count(
            u64::try_from(versions.len()).unwrap_or(u64::MAX),
            tr(Msg::CompositionVersions),
        ),
    );
    let more =
        Menu::new("stored-more", tr(Msg::QueriesMore), Vec::new()).destructive(vec![Item::new(
            Action::new(
                Some(Glyph::TRASH),
                tr_with(
                    Msg::QueriesDeleteNamed,
                    &[("version", version.clone().into())],
                ),
                Act::Dialog(DELETE_DIALOG.to_owned()),
            ),
        )]);
    let heading = page_heading(
        Header {
            title: bare_name_of(&name).to_owned().into(),
            subtitle: Some(
                tr_with(
                    Msg::QueriesLine,
                    &[
                        ("name", name.clone().into()),
                        ("version", version.clone().into()),
                    ],
                )
                .into(),
            ),
            crumbs: vec![Crumb::new(tr(Msg::NavStoredQueries), "/queries")],
            mono: false,
            badge: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::SQUARE_TERMINAL),
            tr(Msg::QueriesOpenInQuery),
            Act::Link(open_in_query(&name, &version)),
        ))
        .more(more),
    );
    let confirm = (crate::request::query("confirm").as_deref() == Some("delete"))
        .then(|| delete_form(&name, &version));
    let dialog = danger_dialog(
        Confirmation {
            id: DELETE_DIALOG,
            title: tr_with(
                Msg::QueriesDeleteTitle,
                &[
                    ("name", bare_name_of(&name).to_owned().into()),
                    ("version", version.clone().into()),
                ],
            )
            .into(),
            message: tr(Msg::QueriesDeleteMessage).into(),
        },
        delete_button(&name, &version),
    );
    let panel = match rows {
        Ok(_) => body(versions, version.clone()).await,
        Err(error) => refused(tr(Msg::QueriesRefused), &error, None),
    };
    view! {
        <Title text=bare_name_of(&name).to_owned() />
        {heading}
        <div class="fui-stack">
            {acted}
            {confirm}
            {binder(tr(Msg::QueriesViews), tabs, tab.key(), panel)}
        </div>
        {dialog}
    }
    .into_any()
}

/// The form that deletes version `version` of `name`.
fn delete_button(name: &str, version: &str) -> AnyView {
    view! {
        <form method="post" action=stored_query_href(name, version)>
            <input type="hidden" name="intent" value="delete-version" />
            <input type="hidden" name="version" value=version.to_owned() />
            <button type="submit" class="fui-button fui-button-danger">
                {ferrohealth_ui::icon::icon(Glyph::TRASH, "16")}
                {tr_with(Msg::QueriesDeleteConfirm, &[("version", version.to_owned().into())])}
            </button>
        </form>
    }
    .into_any()
}

/// The delete confirmation drawn in the page, for a reader who came from a
/// list row's Delete version….
// NOTE: FerroUI `view::dialog::Confirmation::id` is `&'static str`, so a list draws no dialog per row.
fn delete_form(name: &str, version: &str) -> AnyView {
    crate::pages::common::warn_note(view! {
        <p>
            <strong>{tr_with(Msg::QueriesDeleteTitle, &[("name", bare_name_of(name).to_owned().into()), ("version", version.to_owned().into())])}</strong>
            " "
            {tr(Msg::QueriesDeleteMessage)}
        </p>
        <div class="fui-cluster">
            {delete_button(name, version)}
            <a class="fui-button fui-button-secondary" href=stored_query_href(name, version)>{tr(Msg::Cancel)}</a>
        </div>
    })
}

/// A stored query's Definition.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn StoredQueryPage() -> impl IntoView {
    let (name, version) = params();
    let page = awaited(query_page(
        name.clone(),
        version,
        QueryTab::Definition,
        move |versions, version| async move {
            let aql = match fetch_stored_query(name.clone(), version.clone()).await {
                Ok(aql) => aql,
                Err(error) => {
                    if crate::pages::common::is_status(&error, http::StatusCode::NOT_FOUND) {
                        crate::request::set_status(http::StatusCode::NOT_FOUND);
                    }
                    return panel(plain_section(refused(
                        tr(Msg::QueriesStoredRefused),
                        &error,
                        None,
                    )));
                }
            };
            let row = versions.iter().find(|row| row.version == version);
            let latest = versions.first().map(|row| row.version.clone());
            let older = latest.as_ref().filter(|latest| **latest != version).map(|latest| {
            plain_section(info_note(view! {
                {tr_with(Msg::QueriesReadingOlder, &[("version", version.clone().into()), ("latest", latest.clone().into())])}
                " "
                <a href=stored_query_href(&name, latest)>{tr_with(Msg::QueriesReadVersion, &[("version", latest.clone().into())])}</a>
            }))
        });
            let mut facts = vec![(tr(Msg::QueriesType), "AQL".into_any())];
            if let Some(row) = row.filter(|row| !row.saved.is_empty()) {
                facts.push((tr(Msg::QueriesSaved), when(&row.saved)));
            }
            facts.push((tr(Msg::QueryParameters), parameters(&aql)));
            panel(view! {
                {older}
                {section(Some(tr(Msg::PartyFacts)), None, None, definitions(facts))}
                {document_section(
                    tr_with(Msg::QueriesAqlTitle, &[("name", bare_name_of(&name).to_owned().into()), ("version", version.clone().into())]),
                    Language::Aql,
                    aql,
                    "stored-aql",
                )}
            })
        },
    ));
    view! { {page} }
}

/// A stored query's Run.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn StoredQueryRunTab() -> impl IntoView {
    let (name, version) = params();
    let posted = crate::posted::posted();
    let page = awaited(query_page(
        name.clone(),
        version,
        QueryTab::Run,
        move |_versions, version| async move {
            let aql = match fetch_stored_query(name.clone(), version.clone()).await {
                Ok(aql) => aql,
                Err(error) => {
                    return panel(plain_section(refused(
                        tr(Msg::QueriesStoredRefused),
                        &error,
                        None,
                    )));
                }
            };
            let mode = posted
                .as_ref()
                .map_or(VersionResolution::Latest, |posted| VersionResolution::from_str_or_exact(&posted.text("res")));
            let typed = posted
                .as_ref()
                .map(|posted| posted.text("run_version"))
                .unwrap_or_default();
            let names = crate::aql_text::placeholders(&aql);
            let values: Vec<(String, String)> = names
                .iter()
                .map(|param| {
                    (
                        param.clone(),
                        posted
                            .as_ref()
                            .map(|posted| posted.text(&format!("p-{param}")))
                            .unwrap_or_default(),
                    )
                })
                .collect();
            let bindings = values.iter().cloned().collect();
            let parameters_json = crate::aql_text::parameters_json(&bindings).to_string();
            let chart = chart_asked();
            let mut version_error = None;
            let answer = if posted
                .as_ref()
                .is_some_and(|posted| posted.intent() == "run")
            {
                match resolve_version(mode, &typed) {
                    Ok(sent) => Some(
                        match run_stored_query(
                            name.clone(),
                            sent.unwrap_or_default(),
                            parameters_json.clone(),
                            0,
                            true,
                        )
                        .await
                        {
                            Ok(page) => results(&Answered {
                                page: &page,
                                aql: &aql,
                                parameters: &parameters_json,
                                chart,
                            }),
                            Err(error) => section(
                                Some(tr(Msg::ResultsTitle)),
                                None,
                                None,
                                refused(tr(Msg::QueryRunRefused), &error, None),
                            ),
                        },
                    ),
                    Err(error) => {
                        version_error = Some(error.to_string());
                        Some(not_run(tr(Msg::QueriesFixVersion)))
                    }
                }
            } else {
                None
            };
            let modes: Vec<(&'static str, String)> = VersionResolution::ALL
                .iter()
                .map(|each| (each.as_str(), tr(each.label())))
                .collect();
            let version_field = (mode != VersionResolution::Latest).then(|| {
            Field::new("rv", tr(Msg::ColumnVersion))
                .after(version_error.clone().map(|error| view! { <p class="fui-field-error">{error}</p> }))
                .view(view! { <input id="rv" name="run_version" type="text" class="fui-input fui-mono" value=typed.clone() placeholder="1 or 1.1" /> })
        });
            let fields = values
            .iter()
            .map(|(param, value)| {
                let id = format!("p-{param}");
                Field::new(id.clone(), param.clone())
                    .help(tr(Msg::QueriesParamHelp))
                    .view(view! { <input id=id.clone() name=id type="text" class="fui-input fui-mono" spellcheck="false" value=value.clone() /> })
            })
            .collect_view();
            panel(view! {
                <form id=RUN_FORM method="post" action=format!("{}/run", stored_query_href(&name, &version))>
                    <input type="hidden" name="default-intent" value="run" />
                    {section(Some(tr(Msg::ColumnVersion)), None, None, view! {
                        {choice_group("res", tr(Msg::QueriesWhichVersion), &modes, mode.as_str())}
                        {version_field}
                    })}
                    {section(Some(tr(Msg::QueryParameters)), u64::try_from(names.len()).ok(), None, view! {
                        <div class="fui-form-grid">{fields}</div>
                        {shown_choice(chart)}
                        <div class="fui-transform-run">
                            <button type="submit" class="fui-button fui-button-primary">
                                {ferrohealth_ui::icon::icon(Glyph::PLAY, "16")}
                                {tr(Msg::QueryRun)}
                            </button>
                        </div>
                    })}
                    {answer.unwrap_or_else(|| not_run(tr(Msg::ResultsNotRun)))}
                </form>
            })
        },
    ));
    view! { {page} }
}

/// A stored query's Versions.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn StoredQueryVersionsTab() -> impl IntoView {
    let (name, _) = params();
    let page = awaited(query_page(
        name.clone(),
        String::new(),
        QueryTab::Versions,
        move |versions, _| async move {
            let columns = [
                Column::text("version", tr(Msg::ColumnVersion)).role(Role::Title),
                Column::text("saved", tr(Msg::QueriesSaved)).role(Role::Sub),
                Column::text("parameters", tr(Msg::QueryParameters)).role(Role::Meta),
            ];
            let rows = versions
            .iter()
            .enumerate()
            .map(|(place, row)| {
                let standing = if place == 0 { Standing::Current } else { Standing::Older };
                let href = stored_query_href(&name, &row.version);
                view! {
                    <tr>
                        {title_cell(Some(&href), version_cell(&row.version, standing), None)}
                        {cell(&columns[1], if row.saved.is_empty() { ().into_any() } else { when(&row.saved) })}
                        {cell(&columns[2], parameters(&row.query))}
                    </tr>
                }
                .into_any()
            })
            .collect_view()
            .into_any();
            let caption = tr(Msg::CompositionVersions);
            let table = data_table(DataTable {
                caption: &caption,
                columns: &columns,
                sorting: Sorting::default(),
                body: Body::Rows(rows),
            });
            panel(section(
                Some(tr(Msg::CompositionVersions)),
                u64::try_from(versions.len()).ok(),
                None,
                view! {
                    {ferrohealth_ui::view::surface::section_intro(tr(Msg::QueriesVersionsIntro))}
                    {table}
                },
            ))
        },
    ));
    view! { {page} }
}
