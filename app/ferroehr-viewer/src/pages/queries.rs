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
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, group_row, menu_cell, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action, Item, Menu, row_menu};
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, danger_form_dialog};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::segmented::choice_group;
use ferrohealth_ui::view::surface::{Crumb, Header, definitions, page_heading};
use ferrohealth_ui::view::tabs::{Tab, binder};
use ferrohealth_ui::view::template::list::{
    ActiveFilters, FilterChip, ListCard, active_filters, list_card, list_empty,
};
use ferrohealth_ui::view::versions::{Standing, version_cell};

use crate::flash::{Done, redirect_with};
use crate::locale::queries::QueriesMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    Refused, count, document_section, info_note, panel, plain_section, section, when,
};
use crate::pages::results::Answer;
use crate::queries_api::{
    StoredQueryRow, fetch_stored_query, list_stored_queries, run_stored_query,
};
use crate::query_namespace::{VersionResolution, bare_name_of};
use crate::query_namespace::{group_by_namespace, resolve_version};
use crate::routes::Drawn;

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
fn parameters(names: &[String]) -> Html {
    if names.is_empty() {
        return h(format!(
            r#"<span class="fui-muted">{}</span>"#,
            t(tr(Msg::RelationshipNone))
        ));
    }
    names
        .iter()
        .map(|name| h(format!("<code>{}</code> ", t(name))))
        .collect()
}

/// One stored query of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct Group {
    /// The namespace, `None` for the unqualified ones.
    namespace: Option<String>,
    /// How many distinct names it holds.
    names: usize,
    /// Its versions, in the CDR's order.
    members: Vec<Member>,
}

/// The stored queries the listing shows.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct Listed {
    /// The namespace kept, if any.
    only: Option<String>,
    /// The listing.
    listing: Listing,
}

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

/// What the page reads.
async fn read_listing() -> Listed {
    let only = crate::request::query("namespace").filter(|namespace| !namespace.is_empty());
    let listing = match list_stored_queries().await {
        Ok(rows) if rows.is_empty() => Listing::Empty,
        Ok(rows) => grouped(&rows, only.as_deref()),
        Err(error) => Listing::Refused(Refused::new(tr(QueriesMsg::QueriesRefused), error)),
    };
    Listed { only, listing }
}

/// The stored queries grouped by namespace, as a table.
fn table(groups: Vec<Group>) -> Html {
    let columns = [
        Column::text("name", tr(Msg::QueryName)).role(Role::Title),
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Sub),
        Column::text("parameters", tr(QueriesMsg::QueryParameters)).role(Role::Meta),
        Column::text("menu", tr(Msg::ColumnActions)).role(Role::Menu),
    ];
    let mut body = Vec::new();
    let mut index = 0_usize;
    for group in groups {
        let label = match &group.namespace {
            Some(namespace) => h(format!("<code>{}</code>", t(namespace))),
            None => t(tr(QueriesMsg::QueriesUnqualified)),
        };
        body.push(group_row(
            &columns,
            label.then(h(format!(
                r#"<span class="fui-small fui-muted">{}</span>"#,
                t(format!(
                    " · {}",
                    tr_with(
                        QueriesMsg::QueriesGroupCount,
                        &[
                            ("names", count(group.names)),
                            ("versions", count(group.members.len())),
                        ],
                    ),
                ))
            ))),
        ));
        for row in &group.members {
            let href = stored_query_href(&row.name, &row.version);
            let sub = row.older.then(|| {
                h(format!(
                    r#"<span class="fui-row-sub">{}</span>"#,
                    t(tr(QueriesMsg::QueriesOlderVersion))
                ))
            });
            let menu = row_menu(
                &current(),
                &Menu::new(
                    format!("stored-{index}"),
                    tr_with(
                        Msg::TemplateActionsFor,
                        &[("name", bare_name_of(&row.name).to_owned().into())],
                    ),
                    vec![
                        Item::new(Action::new(
                            Some(Glyph::PLAY),
                            tr(QueriesMsg::QueryRun),
                            Act::Link(format!("{href}/run")),
                        )),
                        Item::new(Action::new(
                            Some(Glyph::SQUARE_TERMINAL),
                            tr(QueriesMsg::QueriesOpenInQuery),
                            Act::Link(open_in_query(&row.name, &row.version)),
                        )),
                    ],
                )
                .destructive(vec![Item::new(Action::new(
                    Some(Glyph::TRASH),
                    tr(QueriesMsg::QueriesDeleteVersion),
                    Act::Link(format!("{href}?confirm=delete")),
                ))]),
            );
            body.push(h(format!(
                "<tr>{}{}{}{}</tr>",
                title_cell(Some(&href), t(bare_name_of(&row.name)), sub),
                cell(&columns[1], t(&row.version)),
                cell(&columns[2], parameters(&row.parameters)),
                menu_cell(menu)
            )));
            index = index.saturating_add(1);
        }
    }
    let caption = tr(Msg::NavStoredQueries);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(Html::join(body)),
    })
}

/// Stored queries' list page.
fn listed(listed: Listed) -> Html {
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
            tr(QueriesMsg::QueriesUnqualified)
        } else {
            namespace.clone()
        };
        active_filters(
            &current(),
            ActiveFilters {
                chips: vec![FilterChip {
                    label: tr_with(QueriesMsg::QueriesNamespaceChip, &[("value", words.into())]),
                    remove: "/queries".to_owned(),
                }],
                clear: "/queries".to_owned(),
                shown: None,
            },
            None,
        )
    });
    let card = list_card(
        &current(),
        ListCard {
            title: tr(Msg::NavStoredQueries),
            count: total.map(ferrohealth_ui::view::template::list::HeadingCount::All),
            order: None,
            tabs: None,
            actions: Some(write),
            body: list,
            pager: None,
        },
    );
    h(format!(
        r#"<div class="fui-list-page">{}{card}</div>"#,
        opt(chips)
    ))
}

/// The stored queries, grouped by namespace.
pub async fn list_page() -> Drawn {
    let heading = page_heading(
        &current(),
        Header {
            title: tr(Msg::NavStoredQueries).into(),
            subtitle: Some(tr(QueriesMsg::QueriesSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let body = listed(read_listing().await);
    Drawn::page(tr(Msg::NavStoredQueries), heading.then(body))
}

/// The stored query and version the route names.
fn params() -> (String, String) {
    let params = crate::routes::params();
    (
        params.get("name").unwrap_or_default(),
        params.get("version").unwrap_or_default(),
    )
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
enum Page<T> {
    /// The version was deleted and the reader sent to Stored queries.
    Deleted,
    /// The page.
    Shown(Box<Frame<T>>),
}

/// Loads a stored query's page: the Delete version it answers, its
/// versions, and the panel `body` loads from them and the version shown.
async fn load_page<T, F>(name: String, version: String, body: F) -> Page<T>
where
    F: AsyncFnOnce(String, Vec<StoredQueryRow>, String) -> T,
{
    let acted = match crate::posted::posted_for("delete-version") {
        Some(posted) => {
            match crate::admin::admin_delete_stored_query(name.clone(), posted.text("version"))
                .await
            {
                Ok(()) => {
                    redirect_with(
                        "/queries",
                        &Done::ok(tr(QueriesMsg::QueriesDeleted))
                            .line(format!("{name} {}", posted.text("version"))),
                    );
                    return Page::Deleted;
                }
                Err(error) => Some(Refused::new(tr(QueriesMsg::QueriesDeleteRefused), error)),
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
        Err(error) => Err(Refused::new(tr(QueriesMsg::QueriesRefused), error)),
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
fn page_view<T>(page: Page<T>, tab: QueryTab, body: impl FnOnce(&str, &str, T) -> Html) -> Drawn {
    let Page::Shown(frame) = page else {
        return Drawn::default();
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
        tr(QueriesMsg::QueriesDefinition),
    )];
    if !older {
        tabs.push(Tab::new(
            QueryTab::Run.key(),
            format!("{}/run", stored_query_href(&name, &version)),
            tr(QueriesMsg::QueryRun),
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
        tr(QueriesMsg::QueriesMore),
        vec![Item::new(Action::new(
            Some(Glyph::COPY),
            tr(QueriesMsg::QueriesCopyName),
            Act::Copy(name.clone()),
        ))],
    )
    .destructive(vec![Item::new(Action::new(
        Some(Glyph::TRASH),
        tr_with(
            QueriesMsg::QueriesDeleteNamed,
            &[("version", version.clone().into())],
        ),
        Act::Dialog(DELETE_DIALOG.to_owned()),
    ))]);
    let heading = page_heading(
        &current(),
        Header {
            title: bare_name_of(&name).to_owned().into(),
            subtitle: Some(
                tr_with(
                    QueriesMsg::QueriesLine,
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
            tr(QueriesMsg::QueriesOpenInQuery),
            Act::Link(open_in_query(&name, &version)),
        ))
        .more(more),
    );
    let confirm = confirm.then(|| delete_form(&name, &version));
    let dialog = danger_form_dialog(
        &current(),
        Confirmation {
            id: DELETE_DIALOG.into(),
            title: tr_with(
                QueriesMsg::QueriesDeleteTitle,
                &[
                    ("name", bare_name_of(&name).to_owned().into()),
                    ("version", version.clone().into()),
                ],
            )
            .into(),
            message: tr(QueriesMsg::QueriesDeleteMessage).into(),
        },
        ConfirmForm {
            action: stored_query_href(&name, &version),
            fields: delete_fields(&version),
        },
        delete_submit(&version),
    );
    let acted = acted.map(|refusal| refusal.view());
    let panel = match panel {
        Ok(found) => body(&name, &version, found),
        Err(refusal) => refusal.view(),
    };
    let binder = binder(
        &current(),
        tr(QueriesMsg::QueriesViews),
        tabs,
        tab.key(),
        panel,
    );
    let body = Html::join([
        heading,
        h(format!(
            r#"<div class="fui-stack">{}{}{binder}</div>"#,
            opt(acted),
            opt(confirm)
        )),
        dialog,
    ]);
    Drawn::page(bare_name_of(&name).to_owned(), body)
}

/// The form that deletes version `version` of `name`.
fn delete_button(name: &str, version: &str) -> Html {
    h(format!(
        r#"<form method="post" action="{}">{}{}</form>"#,
        t(stored_query_href(name, version)),
        delete_fields(version),
        delete_submit(version)
    ))
}

/// The fields of a version's delete: its intent and the version.
fn delete_fields(version: &str) -> Html {
    h(format!(
        r#"<input type="hidden" name="intent" value="delete-version"><input type="hidden" name="version" value="{}">"#,
        t(version)
    ))
}

/// The submit of a version's delete.
fn delete_submit(version: &str) -> Html {
    h(format!(
        r#"<button type="submit" class="fui-button fui-button-danger">{}{}</button>"#,
        ferrohealth_ui::icon::icon(Glyph::TRASH, "16"),
        t(tr_with(
            QueriesMsg::QueriesDeleteConfirm,
            &[("version", version.to_owned().into())],
        ))
    ))
}

/// The delete confirmation drawn in the page, for a reader who came from a
/// list row's Delete version….
// NOTE: FerroUI `view::dialog::Confirmation::id` is `&'static str`, so a list draws no dialog per row.
fn delete_form(name: &str, version: &str) -> Html {
    crate::pages::common::warn_note(h(format!(
        r#"<p><strong>{title}</strong> {message}</p><div class="fui-cluster">{button}<a class="fui-button fui-button-secondary" href="{href}">{cancel}</a></div>"#,
        title = t(tr_with(
            QueriesMsg::QueriesDeleteTitle,
            &[
                ("name", bare_name_of(name).to_owned().into()),
                ("version", version.to_owned().into()),
            ],
        )),
        message = t(tr(QueriesMsg::QueriesDeleteMessage)),
        button = delete_button(name, version),
        href = t(stored_query_href(name, version)),
        cancel = t(tr(Msg::Cancel)),
    )))
}

/// A stored query's definition, or why it could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// What the page reads.
async fn read_definition(name: String, version: String) -> Page<Definition> {
    load_page(name, version, async |name, versions, version| {
        let aql = match fetch_stored_query(name.clone(), version.clone()).await {
            Ok(aql) => aql,
            Err(error) => {
                if crate::pages::common::is_status(&error, http::StatusCode::NOT_FOUND) {
                    crate::request::set_status(http::StatusCode::NOT_FOUND);
                }
                return Definition::Refused(Refused::new(
                    tr(QueriesMsg::QueriesStoredRefused),
                    error,
                ));
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
fn definition(name: &str, version: &str, found: Definition) -> Html {
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
        plain_section(info_note(h(format!(
            r#"{} <a href="{}">{}</a>"#,
            t(tr_with(
                QueriesMsg::QueriesReadingOlder,
                &[
                    ("version", version.to_owned().into()),
                    ("latest", latest.clone().into()),
                ],
            )),
            t(stored_query_href(name, &latest)),
            t(tr_with(
                QueriesMsg::QueriesReadVersion,
                &[("version", latest.clone().into())],
            ))
        ))))
    });
    let mut facts = vec![(tr(QueriesMsg::QueriesType), t("AQL"))];
    if !saved.is_empty() {
        facts.push((tr(QueriesMsg::QueriesSaved), when(&saved)));
    }
    facts.push((tr(QueriesMsg::QueryParameters), parameters(&names)));
    panel(Html::join([
        opt(older),
        section(Some(tr(Msg::PartyFacts)), None, None, definitions(facts)),
        document_section(
            tr_with(
                QueriesMsg::QueriesAqlTitle,
                &[
                    ("name", bare_name_of(name).to_owned().into()),
                    ("version", version.to_owned().into()),
                ],
            ),
            Language::Aql,
            aql,
            "stored-aql",
        ),
    ]))
}

/// A stored query's Definition tab.
pub async fn page() -> Drawn {
    let (name, version) = params();
    page_view(
        read_definition(name, version).await,
        QueryTab::Definition,
        definition,
    )
}

/// A stored query's Run form as the post leaves it.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
enum Run {
    /// The CDR's refusal of the version.
    Refused(Refused),
    /// The form.
    Form(Box<RunForm>),
}

/// What the page reads.
async fn read_run(name: String, version: String) -> Page<Run> {
    let posted = crate::posted::posted();
    load_page(name, version, async move |name, _versions, version| {
        let aql = match fetch_stored_query(name.clone(), version.clone()).await {
            Ok(aql) => aql,
            Err(error) => {
                return Run::Refused(Refused::new(tr(QueriesMsg::QueriesStoredRefused), error));
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
                    Some(Answer::NotRun(tr(QueriesMsg::QueriesFixVersion)))
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
fn run(name: &str, version: &str, found: Run) -> Html {
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
    // NOTE: FerroUI console-ui.md §1: the version field shows whatever the choice, so a page
    // with no script can pick Prefix or Exact and type the version in one post.
    let mut version_field =
        Field::new("rv", tr(Msg::ColumnVersion)).help(tr(QueriesMsg::QueriesVersionHelp));
    if let Some(error) = version_error {
        version_field =
            version_field.after(h(format!(r#"<p class="fui-field-error">{}</p>"#, t(error))));
    }
    let version_field = version_field.view(h(format!(
        r#"<input id="rv" name="run_version" type="text" class="fui-input fui-mono" value="{}" placeholder="1 or 1.1">"#,
        t(&typed)
    )));
    let sends = match mode {
        VersionResolution::Latest => format!("POST /query/{name}"),
        VersionResolution::Prefix | VersionResolution::Exact => {
            format!("POST /query/{name}/{}", typed.trim())
        }
    };
    let sends = h(format!(
        r#"<p class="fui-small fui-muted">{} <code>{}</code></p>"#,
        t(tr(QueriesMsg::QueriesSends)),
        t(sends)
    ));
    let names = values.len();
    let fields: Html = values
        .into_iter()
        .map(|(param, value)| {
            let id = format!("p-{param}");
            Field::new(id.clone(), param)
                .help(tr(QueriesMsg::QueriesParamHelp))
                .view(h(format!(
                    r#"<input id="{id}" name="{id}" type="text" class="fui-input fui-mono" spellcheck="false" value="{}">"#,
                    t(value),
                    id = t(&id),
                )))
        })
        .collect();
    let version_section = section(
        Some(tr(Msg::ColumnVersion)),
        None,
        None,
        Html::join([
            choice_group(
                "res",
                tr(QueriesMsg::QueriesWhichVersion),
                &modes,
                mode.as_str(),
            ),
            version_field,
            sends,
        ]),
    );
    let parameters = section(
        Some(tr(QueriesMsg::QueryParameters)),
        u64::try_from(names).ok(),
        None,
        h(format!(
            r#"<div class="fui-form-grid">{fields}</div><div class="fui-transform-run"><button type="submit" class="fui-button fui-button-primary">{}{}</button></div>"#,
            ferrohealth_ui::icon::icon(Glyph::PLAY, "16"),
            t(tr(QueriesMsg::QueryRun))
        )),
    );
    panel(h(format!(
        r#"<form id="{RUN_FORM}" method="post" action="{}"><input type="hidden" name="default-intent" value="run">{version_section}{parameters}{}</form>"#,
        t(format!("{}/run", stored_query_href(name, version))),
        Answer::view(answer)
    )))
}

/// A stored query's Run tab.
pub async fn run_page() -> Drawn {
    let (name, version) = params();
    page_view(read_run(name, version).await, QueryTab::Run, run)
}

/// One version of a stored query, as Versions lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionRow {
    /// The version.
    version: String,
    /// When it was saved, empty when the listing does not say.
    saved: String,
    /// The parameters its text names.
    parameters: Vec<String>,
}

/// What the page reads.
async fn read_versions(name: String) -> Page<Vec<VersionRow>> {
    load_page(name, String::new(), async |_name, versions, _version| {
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
fn versions(name: &str, _version: &str, versions: Vec<VersionRow>) -> Html {
    let columns = [
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Title),
        Column::text("saved", tr(QueriesMsg::QueriesSaved)).role(Role::Sub),
        Column::text("parameters", tr(QueriesMsg::QueryParameters)).role(Role::Meta),
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
            let saved = if row.saved.is_empty() {
                Html::empty()
            } else {
                when(&row.saved)
            };
            h(format!(
                "<tr>{}{}{}</tr>",
                title_cell(
                    Some(&href),
                    version_cell(&current(), &row.version, standing),
                    None
                ),
                cell(&columns[1], saved),
                cell(&columns[2], parameters(&row.parameters))
            ))
        })
        .collect();
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
        ferrohealth_ui::view::surface::section_intro(tr(QueriesMsg::QueriesVersionsIntro))
            .then(table),
    ))
}

/// A stored query's Versions.
/// A stored query's Versions tab.
pub async fn versions_page() -> Drawn {
    let (name, _) = params();
    page_view(read_versions(name).await, QueryTab::Versions, versions)
}
