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
#[cfg(not(target_arch = "wasm32"))]
use leptos_router::hooks::use_params_map;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::flash::{Done, redirect_with};
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{
    Refused, count, document_section, drawn, info_note, page_model, panel, plain_section, section,
    when,
};
use crate::pages::results::Answer;
#[cfg(not(target_arch = "wasm32"))]
use crate::queries_api::{
    StoredQueryRow, fetch_stored_query, list_stored_queries, run_stored_query,
};
use crate::query_namespace::{VersionResolution, bare_name_of};
#[cfg(not(target_arch = "wasm32"))]
use crate::query_namespace::{group_by_namespace, resolve_version};

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

#[cfg(not(target_arch = "wasm32"))]
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
fn parameters(names: &[String]) -> AnyView {
    if names.is_empty() {
        return view! { <span class="fui-muted">{tr(Msg::RelationshipNone)}</span> }.into_any();
    }
    names
        .iter()
        .map(|name| {
            view! {
                <code>{name.clone()}</code>
                " "
            }
        })
        .collect_view()
        .into_any()
}

/// One version of a stored query, as a listing shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Member {
    /// Its qualified name.
    name: String,
    /// Its version.
    version: String,
    /// Whether a newer version of the same name is listed.
    older: bool,
    /// The parameters its text names.
    parameters: Vec<String>,
}

/// The stored queries of one namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Group {
    /// The namespace, `None` for the unqualified ones.
    namespace: Option<String>,
    /// How many distinct names it holds.
    names: usize,
    /// Its versions, in the CDR's order.
    members: Vec<Member>,
}

/// The stored queries the listing shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Listing {
    /// The CDR holds none.
    Empty,
    /// The groups the filter keeps, and the distinct names in them.
    Groups {
        /// The groups.
        groups: Vec<Group>,
        /// The distinct names.
        names: usize,
    },
    /// The CDR's refusal.
    Refused(Refused),
}

/// Stored queries' model: the namespace the address keeps and the listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Listed {
    /// The namespace kept, if any.
    only: Option<String>,
    /// The listing.
    listing: Listing,
}

#[cfg(not(target_arch = "wasm32"))]
/// The stored queries grouped by namespace, the groups `only` keeps.
fn grouped(rows: &[StoredQueryRow], only: Option<&str>) -> Listing {
    let mut groups = Vec::new();
    let mut names = 0;
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
        let members = group
            .members
            .iter()
            .map(|row| Member {
                name: row.name.clone(),
                version: row.version.clone(),
                older: group.members.iter().any(|other| {
                    other.name == row.name && newest_first(&other.version, &row.version).is_lt()
                }),
                parameters: crate::aql_text::placeholders(&row.query),
            })
            .collect();
        groups.push(Group {
            namespace: group.namespace,
            names: distinct.len(),
            members,
        });
    }
    Listing::Groups { groups, names }
}

#[cfg(not(target_arch = "wasm32"))]
/// Loads Stored queries: the listing, kept to the namespace the address names.
async fn load_listing() -> Listed {
    let only = crate::request::query("namespace").filter(|namespace| !namespace.is_empty());
    let listing = match list_stored_queries().await {
        Ok(rows) if rows.is_empty() => Listing::Empty,
        Ok(rows) => grouped(&rows, only.as_deref()),
        Err(error) => Listing::Refused(Refused::new(tr(Msg::QueriesRefused), error)),
    };
    Listed { only, listing }
}

/// The stored queries grouped by namespace, as a table.
fn table(groups: Vec<Group>) -> AnyView {
    let columns = [
        Column::text("name", tr(Msg::QueryName)).role(Role::Title),
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Sub),
        Column::text("parameters", tr(Msg::QueryParameters)).role(Role::Meta),
        Column::text("menu", tr(Msg::ColumnActions)).role(Role::Menu),
    ];
    let mut body = Vec::new();
    let mut index = 0_usize;
    for group in groups {
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
                        tr_with(
                            Msg::QueriesGroupCount,
                            &[
                                ("names", count(group.names)),
                                ("versions", count(group.members.len())),
                            ],
                        ),
                    )}
                </span>
            },
        ));
        for row in &group.members {
            let href = stored_query_href(&row.name, &row.version);
            let sub = row.older.then(|| {
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
                        {cell(&columns[2], parameters(&row.parameters))} {menu_cell(menu)}
                    </tr>
                }
                .into_any(),
            );
            index = index.saturating_add(1);
        }
    }
    let caption = tr(Msg::NavStoredQueries);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(body.collect_view().into_any()),
    })
    .into_any()
}

/// Stored queries' list page.
fn listed(listed: Listed) -> AnyView {
    let Listed { only, listing } = listed;
    let write = ferrohealth_ui::view::button::link_button(
        "/query/aql".to_owned(),
        ferrohealth_ui::view::button::Variant::Ghost,
        Some(Glyph::SQUARE_TERMINAL),
        tr(Msg::DashboardWriteOne),
    );
    let (list, total) = match listing {
        Listing::Empty => (list_empty(tr(Msg::DashboardNoStoredQuery), None), Some(0)),
        Listing::Groups { groups, names } => (table(groups), u64::try_from(names).ok()),
        Listing::Refused(refusal) => (refusal.view(), None),
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
        count: total.map(ferrohealth_ui::view::template::list::HeadingCount::All),
        order: None,
        tabs: None,
        actions: Some(write),
        body: list,
        pager: None,
    });
    view! { <div class="fui-list-page">{chips} {card}</div> }.into_any()
}

/// Stored queries.
#[component]
pub fn StoredQueriesPage() -> impl IntoView {
    let heading = page_heading(
        Header {
            title: tr(Msg::NavStoredQueries).into(),
            subtitle: Some(tr(Msg::QueriesSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let model = page_model!(load_listing());
    let body = drawn(model, listed);
    view! {
        <Title text=tr(Msg::NavStoredQueries) />
        {heading}
        {body}
    }
}

#[cfg(not(target_arch = "wasm32"))]
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

/// A stored query's page around the panel of one tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Frame<T> {
    /// The qualified name.
    name: String,
    /// The version shown: the one the route names, or the latest.
    version: String,
    /// Whether a newer version exists.
    older: bool,
    /// How many versions the CDR holds.
    versions: usize,
    /// The refusal of a Delete version the page answers.
    acted: Option<Refused>,
    /// Whether the address asks to confirm a delete.
    confirm: bool,
    /// The tab's panel, or the refusal of the listing.
    panel: Result<T, Refused>,
}

/// A stored query's page: the deleted version's redirect, or the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Page<T> {
    /// The version was deleted and the reader sent to Stored queries.
    Deleted,
    /// The page.
    Shown(Box<Frame<T>>),
}

#[cfg(not(target_arch = "wasm32"))]
/// Loads a stored query's page: the Delete version it answers, its
/// versions, and the panel `body` loads from them and the version shown.
async fn load_page<T, F>(version: String, body: F) -> Page<T>
where
    F: AsyncFnOnce(String, Vec<StoredQueryRow>, String) -> T,
{
    let (name, _) = params();
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
                    return Page::Deleted;
                }
                Err(error) => Some(Refused::new(tr(Msg::QueriesDeleteRefused), error)),
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
    let confirm = crate::request::query("confirm").as_deref() == Some("delete");
    let count = versions.len();
    let panel = match rows {
        Ok(_) => Ok(body(name.clone(), versions, version.clone()).await),
        Err(error) => Err(Refused::new(tr(Msg::QueriesRefused), error)),
    };
    Page::Shown(Box::new(Frame {
        name,
        version,
        older,
        versions: count,
        acted,
        confirm,
        panel,
    }))
}

/// A stored query's page: its header with Open in Query and Delete version,
/// its tabs over the panel `body` draws, and the delete confirmation.
#[expect(
    clippy::too_many_lines,
    reason = "one page frame: the header, the tabs and the delete confirmation"
)]
fn page_view<T>(
    page: Page<T>,
    tab: QueryTab,
    body: impl FnOnce(&str, &str, T) -> AnyView,
) -> AnyView {
    let Page::Shown(frame) = page else {
        return ().into_any();
    };
    let Frame {
        name,
        version,
        older,
        versions,
        acted,
        confirm,
        panel,
    } = *frame;
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
            u64::try_from(versions).unwrap_or(u64::MAX),
            tr(Msg::CompositionVersions),
        ),
    );
    let more = Menu::new(
        "stored-more",
        tr(Msg::QueriesMore),
        vec![Item::new(Action::new(
            Some(Glyph::COPY),
            tr(Msg::QueriesCopyName),
            Act::Copy(name.clone()),
        ))],
    )
    .destructive(vec![Item::new(Action::new(
        Some(Glyph::TRASH),
        tr_with(
            Msg::QueriesDeleteNamed,
            &[("version", version.clone().into())],
        ),
        Act::Dialog(DELETE_DIALOG.to_owned()),
    ))]);
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
            subtitle_view: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::SQUARE_TERMINAL),
            tr(Msg::QueriesOpenInQuery),
            Act::Link(open_in_query(&name, &version)),
        ))
        .more(more),
    );
    let confirm = confirm.then(|| delete_form(&name, &version));
    let dialog = danger_dialog(
        Confirmation {
            id: DELETE_DIALOG.into(),
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
    let acted = acted.map(|refusal| refusal.view());
    let panel = match panel {
        Ok(found) => body(&name, &version, found),
        Err(refusal) => refusal.view(),
    };
    view! {
        <Title text=bare_name_of(&name).to_owned() />
        {heading}
        <div class="fui-stack">
            {acted} {confirm} {binder(tr(Msg::QueriesViews), tabs, tab.key(), panel)}
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
            <strong>
                {tr_with(
                    Msg::QueriesDeleteTitle,
                    &[
                        ("name", bare_name_of(name).to_owned().into()),
                        ("version", version.to_owned().into()),
                    ],
                )}
            </strong>
            " "
            {tr(Msg::QueriesDeleteMessage)}
        </p>
        <div class="fui-cluster">
            {delete_button(name, version)}
            <a class="fui-button fui-button-secondary" href=stored_query_href(name, version)>
                {tr(Msg::Cancel)}
            </a>
        </div>
    })
}

/// A stored query's Definition panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Definition {
    /// The CDR's refusal of the version.
    Refused(Refused),
    /// The version.
    Shown {
        /// Its AQL.
        aql: String,
        /// The latest version, when this one is older.
        latest: Option<String>,
        /// When it was saved, empty when the listing does not say.
        saved: String,
        /// The parameters its text names.
        parameters: Vec<String>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
/// Loads the Definition: the version's AQL and its facts.
async fn load_definition() -> Page<Definition> {
    let (_, version) = params();
    load_page(version, async |name, versions, version| {
        let aql = match fetch_stored_query(name.clone(), version.clone()).await {
            Ok(aql) => aql,
            Err(error) => {
                if crate::pages::common::is_status(&error, http::StatusCode::NOT_FOUND) {
                    crate::request::set_status(http::StatusCode::NOT_FOUND);
                }
                return Definition::Refused(Refused::new(tr(Msg::QueriesStoredRefused), error));
            }
        };
        let saved = versions
            .iter()
            .find(|row| row.version == version)
            .map(|row| row.saved.clone())
            .unwrap_or_default();
        let latest = versions
            .first()
            .map(|row| row.version.clone())
            .filter(|latest| *latest != version);
        let parameters = crate::aql_text::placeholders(&aql);
        Definition::Shown {
            aql,
            latest,
            saved,
            parameters,
        }
    })
    .await
}

/// The Definition panel, drawn.
fn definition(name: &str, version: &str, found: Definition) -> AnyView {
    let (aql, latest, saved, names) = match found {
        Definition::Refused(refusal) => return panel(plain_section(refusal.view())),
        Definition::Shown {
            aql,
            latest,
            saved,
            parameters,
        } => (aql, latest, saved, parameters),
    };
    let older = latest.map(|latest| {
        plain_section(info_note(view! {
            {tr_with(
                Msg::QueriesReadingOlder,
                &[("version", version.to_owned().into()), ("latest", latest.clone().into())],
            )}
            " "
            <a href=stored_query_href(
                name,
                &latest,
            )>{tr_with(Msg::QueriesReadVersion, &[("version", latest.clone().into())])}</a>
        }))
    });
    let mut facts = vec![(tr(Msg::QueriesType), "AQL".into_any())];
    if !saved.is_empty() {
        facts.push((tr(Msg::QueriesSaved), when(&saved)));
    }
    facts.push((tr(Msg::QueryParameters), parameters(&names)));
    panel(view! {
        {older}
        {section(Some(tr(Msg::PartyFacts)), None, None, definitions(facts))}
        {document_section(
            tr_with(
                Msg::QueriesAqlTitle,
                &[
                    ("name", bare_name_of(name).to_owned().into()),
                    ("version", version.to_owned().into()),
                ],
            ),
            Language::Aql,
            aql,
            "stored-aql",
        )}
    })
}

/// A stored query's Definition.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn StoredQueryPage() -> impl IntoView {
    let model = page_model!(load_definition());
    drawn(model, |page| {
        page_view(page, QueryTab::Definition, definition)
    })
}

/// A stored query's Run form as the post leaves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RunForm {
    /// Which version a run sends.
    mode: VersionResolution,
    /// The version typed.
    typed: String,
    /// Each parameter the text names and its value.
    values: Vec<(String, String)>,
    /// Why the version typed does not fit the choice.
    version_error: Option<String>,
    /// What the run answered, when one was asked.
    answer: Option<Answer>,
}

/// A stored query's Run panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Run {
    /// The CDR's refusal of the version.
    Refused(Refused),
    /// The form.
    Form(Box<RunForm>),
}

#[cfg(not(target_arch = "wasm32"))]
/// Loads Run: the form as the post leaves it, and the run it asks.
async fn load_run() -> Page<Run> {
    let (_, version) = params();
    let posted = crate::posted::posted();
    load_page(version, async move |name, _versions, version| {
        let aql = match fetch_stored_query(name.clone(), version.clone()).await {
            Ok(aql) => aql,
            Err(error) => {
                return Run::Refused(Refused::new(tr(Msg::QueriesStoredRefused), error));
            }
        };
        let mode = posted.as_ref().map_or(VersionResolution::Latest, |posted| {
            VersionResolution::from_str_or_exact(&posted.text("res"))
        });
        let typed = posted
            .as_ref()
            .map(|posted| posted.text("run_version"))
            .unwrap_or_default();
        let values: Vec<(String, String)> = crate::aql_text::placeholders(&aql)
            .into_iter()
            .map(|param| {
                let value = posted
                    .as_ref()
                    .map(|posted| posted.text(&format!("p-{param}")))
                    .unwrap_or_default();
                (param, value)
            })
            .collect();
        let bindings = values.iter().cloned().collect();
        let parameters_json = crate::aql_text::parameters_json(&bindings).to_string();
        let mut version_error = None;
        let answer = if posted
            .as_ref()
            .is_some_and(|posted| posted.intent() == "run")
        {
            match resolve_version(mode, &typed) {
                Ok(sent) => Some(Answer::of(
                    run_stored_query(
                        name.clone(),
                        sent.unwrap_or_default(),
                        parameters_json.clone(),
                        0,
                        true,
                    )
                    .await,
                    &aql,
                    &parameters_json,
                )),
                Err(error) => {
                    version_error = Some(error.to_string());
                    Some(Answer::NotRun(tr(Msg::QueriesFixVersion)))
                }
            }
        } else {
            None
        };
        Run::Form(Box::new(RunForm {
            mode,
            typed,
            values,
            version_error,
            answer,
        }))
    })
    .await
}

/// The Run panel, drawn.
fn run(name: &str, version: &str, found: Run) -> AnyView {
    let RunForm {
        mode,
        typed,
        values,
        version_error,
        answer,
    } = match found {
        Run::Refused(refusal) => return panel(plain_section(refusal.view())),
        Run::Form(form) => *form,
    };
    let modes: Vec<(&'static str, String)> = VersionResolution::ALL
        .iter()
        .map(|each| (each.as_str(), tr(each.label())))
        .collect();
    // NOTE: leptos-ui.md §15.8: the version field shows whatever the choice, so a page with no
    // WebAssembly can pick Prefix or Exact and type the version in one post.
    let version_field = Field::new("rv", tr(Msg::ColumnVersion))
        .help(tr(Msg::QueriesVersionHelp))
        .after(version_error.map(|error| view! { <p class="fui-field-error">{error}</p> }))
        .view(view! {
            <input
                id="rv"
                name="run_version"
                type="text"
                class="fui-input fui-mono"
                value=typed.clone()
                placeholder="1 or 1.1"
            />
        });
    let sends = match mode {
        VersionResolution::Latest => format!("POST /query/{name}"),
        VersionResolution::Prefix | VersionResolution::Exact => {
            format!("POST /query/{name}/{}", typed.trim())
        }
    };
    let sends = view! { <p class="fui-small fui-muted">{tr(Msg::QueriesSends)} " " <code>{sends}</code></p> };
    let names = values.len();
    let fields = values
        .into_iter()
        .map(|(param, value)| {
            let id = format!("p-{param}");
            Field::new(id.clone(), param)
                .help(tr(Msg::QueriesParamHelp))
                .view(view! {
                    <input
                        id=id.clone()
                        name=id
                        type="text"
                        class="fui-input fui-mono"
                        spellcheck="false"
                        value=value
                    />
                })
        })
        .collect_view();
    panel(view! {
        <form id=RUN_FORM method="post" action=format!("{}/run", stored_query_href(name, version))>
            <input type="hidden" name="default-intent" value="run" />
            {section(
                Some(tr(Msg::ColumnVersion)),
                None,
                None,
                view! {
                    {choice_group("res", tr(Msg::QueriesWhichVersion), &modes, mode.as_str())}
                    {version_field}
                    {sends}
                },
            )}
            {section(
                Some(tr(Msg::QueryParameters)),
                u64::try_from(names).ok(),
                None,
                view! {
                    <div class="fui-form-grid">{fields}</div>
                    <div class="fui-transform-run">
                        <button type="submit" class="fui-button fui-button-primary">
                            {ferrohealth_ui::icon::icon(Glyph::PLAY, "16")}
                            {tr(Msg::QueryRun)}
                        </button>
                    </div>
                },
            )}
            {Answer::view(answer)}
        </form>
    })
}

/// A stored query's Run.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn StoredQueryRunTab() -> impl IntoView {
    let model = page_model!(load_run());
    drawn(model, |page| page_view(page, QueryTab::Run, run))
}

/// One version of a stored query, as Versions lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct VersionRow {
    /// The version.
    version: String,
    /// When it was saved, empty when the listing does not say.
    saved: String,
    /// The parameters its text names.
    parameters: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
/// Loads Versions: every version of the stored query, newest first.
async fn load_versions() -> Page<Vec<VersionRow>> {
    load_page(String::new(), async |_name, versions, _version| {
        versions
            .into_iter()
            .map(|row| VersionRow {
                parameters: crate::aql_text::placeholders(&row.query),
                version: row.version,
                saved: row.saved,
            })
            .collect()
    })
    .await
}

/// The Versions panel, drawn.
fn versions(name: &str, _version: &str, versions: Vec<VersionRow>) -> AnyView {
    let columns = [
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Title),
        Column::text("saved", tr(Msg::QueriesSaved)).role(Role::Sub),
        Column::text("parameters", tr(Msg::QueryParameters)).role(Role::Meta),
    ];
    let total = versions.len();
    let rows = versions
        .into_iter()
        .enumerate()
        .map(|(place, row)| {
            let standing = if place == 0 {
                Standing::Current
            } else {
                Standing::Older
            };
            let href = stored_query_href(name, &row.version);
            view! {
                <tr>
                    {title_cell(Some(&href), version_cell(&row.version, standing), None)}
                    {cell(
                        &columns[1],
                        if row.saved.is_empty() { ().into_any() } else { when(&row.saved) },
                    )} {cell(&columns[2], parameters(&row.parameters))}
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
        u64::try_from(total).ok(),
        None,
        view! {
            {ferrohealth_ui::view::surface::section_intro(tr(Msg::QueriesVersionsIntro))}
            {table}
        },
    ))
}

/// A stored query's Versions.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn StoredQueryVersionsTab() -> impl IntoView {
    let model = page_model!(load_versions());
    drawn(model, |page| page_view(page, QueryTab::Versions, versions))
}
