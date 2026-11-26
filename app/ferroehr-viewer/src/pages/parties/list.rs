// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Parties: the parties the CDR holds, a find by what their identities hold,
//! the open by uid, and the demographic tag index.
//!
//! The list is the CDR's AQL rooted at a demographic class
//! ([`crate::bff::demographics::listing`]); it states no total, so it pages
//! with Previous and Next alone, its position kept in the browser's session
//! ([`crate::paging`]), and the kind shown is the address's `list`. A find
//! names people, so it posts and is never in the address. A CDR that serves
//! no demographic root answers `400`, and the page then keeps the open by uid
//! and the tag index (ITS-REST `demographic_tags_get`). No openEHR spec
//! governs the viewer's pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::filter::{Find, find_form};
use ferrohealth_ui::table::model::{OpenWindow, StateSource};
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_open_pager};
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table, title_cell};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{Header, page_heading};
use ferrohealth_ui::view::tabs::{Tab, TabRow};
use ferrohealth_ui::view::template::list::{
    ActiveFilters, FilterChip, ListCard, active_filters, list_card, list_empty,
};

use super::{display_name, kind_options, kind_words};
use crate::bff::PAGE_SIZE;
use crate::bff::demographics::listing::{ListedParty, PartyRoot, list_parties};
use crate::bff::demographics::party::fetch_party;
use crate::bff::demographics::{PartyKind, party_href, resolve_party_kind};
use crate::bff::item_tags::read_tags;
use crate::error::ViewerError;
use crate::locale::parties::PartiesMsg;
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{attr, h, opt, t};
use crate::pages::common::{Refused, ago, info_note, is_status, short, shortened};
use crate::paging::Listing;
use crate::routes::Drawn;

/// One row of the demographic tag index: the tag and the party it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Indexed {
    /// The tag's key.
    key: String,
    /// Its value.
    value: String,
    /// The party's name.
    on: String,
    /// The party's page.
    href: Option<String>,
    /// The party's kind.
    kind: Option<PartyKind>,
}

/// The demographic tag index, each target named by asking the CDR which
/// party holds it.
async fn tag_index() -> Result<Vec<Indexed>, ViewerError> {
    let tags = read_tags("demographic/tags").await?;
    let mut named: Vec<(String, Option<PartyKind>, String)> = Vec::new();
    let mut rows = Vec::new();
    for tag in tags {
        let container = crate::uid::container_uid_of(&tag.target);
        let known = named.iter().find(|(uid, _, _)| *uid == container).cloned();
        let (kind, name) = if let Some((_, kind, name)) = known {
            (kind, name)
        } else {
            let kind = resolve_party_kind(container.clone())
                .await?
                .as_deref()
                .and_then(PartyKind::from_segment);
            let name = match kind {
                Some(kind) => fetch_party(kind.segment().to_owned(), container.clone())
                    .await?
                    .map_or_else(|| shortened(&container), |state| display_name(&state)),
                None => shortened(&container),
            };
            named.push((container.clone(), kind, name.clone()));
            (kind, name)
        };
        rows.push(Indexed {
            key: tag.key,
            value: tag.value,
            on: name,
            href: kind.map(|kind| party_href(kind, &container)),
            kind,
        });
    }
    Ok(rows)
}

/// The tag index as a table.
fn index_table(rows: &[Indexed]) -> Html {
    let columns = [
        Column::text("key", tr(RecordsMsg::TagsKey)).role(Role::Title),
        Column::text("value", tr(Msg::TagsValue)).role(Role::Sub),
        Column::text("on", tr(PartiesMsg::TagOn)).role(Role::Meta),
        Column::text("kind", tr(Msg::ColumnKind)).role(Role::Meta),
    ];
    let body = rows
        .iter()
        .map(|row| {
            let on = match &row.href {
                Some(href) => h(format!(
                    r#"<a class="fui-row-link" href="{}">{}</a>"#,
                    t(href),
                    t(&row.on)
                )),
                None => t(&row.on),
            };
            h(format!(
                r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}{}{}</tr>"#,
                t(&row.key),
                cell(&columns[1], t(&row.value)),
                cell(&columns[2], on),
                cell(&columns[3], t(row.kind.map(kind_words).unwrap_or_default()))
            ))
        })
        .collect();
    let caption = tr(PartiesMsg::TagIndex);
    data_table(DataTable::new(&caption, &columns, Body::Rows(body)))
}

/// Opens the party the form names: its page when its kind holds it, the page
/// of the kind that does when another holds it. `Err` is what the field says.
async fn open(kind: PartyKind, uid: &str) -> Result<String, Result<String, ViewerError>> {
    match fetch_party(kind.segment().to_owned(), uid.to_owned()).await {
        Ok(_) => return Ok(party_href(kind, uid)),
        Err(error) if !is_status(&error, http::StatusCode::NOT_FOUND) => return Err(Err(error)),
        Err(_) => {}
    }
    match resolve_party_kind(uid.to_owned()).await {
        Ok(Some(segment)) => match PartyKind::from_segment(&segment) {
            Some(other) => Ok(party_href(other, uid)),
            None => Err(Ok(tr(PartiesMsg::NoneHolds))),
        },
        Ok(None) => Err(Ok(tr_with(
            PartiesMsg::NoKindHolds,
            &[("kind", kind_words(kind).into())],
        ))),
        Err(error) => Err(Err(error)),
    }
}

/// The reader's word for a root.
fn root_words(root: PartyRoot) -> String {
    match root {
        PartyRoot::Every => tr(PartiesMsg::RootAll),
        PartyRoot::Kind(kind) => kind_words(kind),
    }
}

/// The address of the list of `root`.
fn root_href(root: PartyRoot) -> String {
    format!("/parties?list={}", root.key())
}

/// What the list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Listed {
    /// A page of the list, or what a find found, newest commit first.
    Rows(Vec<ListedParty>),
    /// The CDR serves no AQL rooted at a demographic class.
    Unserved,
    /// The CDR refused the list.
    Refused(Refused),
}

/// Parties' model: the list, the open form and the tag index.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PartiesModel {
    /// The class the list is rooted at.
    root: PartyRoot,
    /// The text a posted find asked for, empty for the whole list.
    find: String,
    /// The list.
    listed: Listed,
    /// The kind the open form asks for.
    kind: PartyKind,
    /// The id the open form asks for.
    uid: Option<String>,
    /// What the id field says is wrong with it.
    invalid: Option<String>,
    /// The CDR's refusal of the open.
    refused: Option<Refused>,
    /// The tag index, or the CDR's refusal of it.
    index: Result<Vec<Indexed>, Refused>,
}

/// The list of `root` as the request asks it: what a posted `find` found,
/// or the page the session keeps.
async fn read_listed(root: PartyRoot, find: &str) -> Listed {
    let (text, window) = if find.is_empty() {
        let window = OpenWindow::of(crate::paging::current(Listing::Parties), 0);
        (None, (window.start, window.size))
    } else {
        (Some(find), (0, PAGE_SIZE))
    };
    match list_parties(root, text, window.0, window.1).await {
        Ok(rows) => Listed::Rows(rows),
        Err(error) if is_status(&error, http::StatusCode::BAD_REQUEST) => Listed::Unserved,
        Err(error) => Listed::Refused(Refused::new(tr(PartiesMsg::ListRefused), error)),
    }
}

/// What the page reads, on the server; `None` when the party asked for
/// opened, so the page redirected to it.
async fn read_parties() -> Option<PartiesModel> {
    let root = crate::request::query("list")
        .as_deref()
        .map_or(PartyRoot::Every, PartyRoot::from_key);
    let kind = crate::request::query("kind")
        .as_deref()
        .and_then(PartyKind::from_segment)
        .unwrap_or(PartyKind::Person);
    let uid = crate::request::query("uid")
        .map(|uid| uid.trim().to_owned())
        .filter(|uid| !uid.is_empty());
    let (invalid, refused) = match &uid {
        Some(uid) => match open(kind, uid).await {
            Ok(href) => {
                crate::request::redirect(&href);
                return None;
            }
            Err(Ok(message)) => (Some(message), None),
            Err(Err(error)) => (None, Some(Refused::new(tr(PartiesMsg::OpenRefused), error))),
        },
        None => (None, None),
    };
    let find = crate::posted::posted_for("find")
        .map(|posted| posted.text("find").trim().to_owned())
        .unwrap_or_default();
    let listed = read_listed(root, &find).await;
    let index = tag_index()
        .await
        .map_err(|error| Refused::new(tr(PartiesMsg::TagsRefused), error));
    Some(PartiesModel {
        root,
        find,
        listed,
        kind,
        uid,
        invalid,
        refused,
        index,
    })
}

/// The list's columns.
fn columns() -> [Column; 3] {
    [
        Column::text("name", tr(PartiesMsg::ColumnName)).role(Role::Title),
        Column::text("kind", tr(Msg::ColumnKind)).role(Role::Meta),
        Column::text("committed", tr(PartiesMsg::ColumnCommitted)).role(Role::Meta),
    ]
}

/// The table of `rows`, each row one link to its party.
fn table(rows: &[ListedParty]) -> Html {
    let columns = columns();
    let body = rows
        .iter()
        .map(|party| {
            let href = party.kind.map(|kind| party_href(kind, &party.uid));
            let title = match &party.name {
                Some(name) => title_cell(None, href.as_deref(), t(name), Some(short(&party.uid))),
                None => title_cell(None, href.as_deref(), short(&party.uid), None),
            };
            let kind = cell(
                &columns[1],
                t(party.kind.map(kind_words).unwrap_or_default()),
            );
            let committed = cell(&columns[2], ago(&party.committed));
            h(format!("<tr>{title}{kind}{committed}</tr>"))
        })
        .collect();
    let caption = tr(Msg::NavParties);
    data_table(DataTable::new(&caption, &columns, Body::Rows(body)))
}

/// The find toolbar of the list of `root`. It posts, since a find names
/// people.
fn toolbar(root: PartyRoot, typed: &str) -> Html {
    let label = tr(PartiesMsg::FindLabel);
    let action = root_href(root);
    find_form(
        Find {
            id: "parties-find",
            name: "find",
            label: &label,
            value: typed,
            source: StateSource::Session,
            action: Some(&action),
            fields: &[("intent".to_owned(), "find".to_owned())],
        },
        Html::empty(),
    )
}

/// The kinds of party as tabs, `root` the one shown.
fn tabs(root: PartyRoot) -> TabRow {
    TabRow {
        label: tr(PartiesMsg::Roots),
        tabs: PartyRoot::ALL
            .into_iter()
            .map(|each| Tab::new(each.key(), root_href(each), root_words(each)))
            .collect(),
        current: root.key().to_owned(),
    }
}

/// The list's card: the table, the find's chip and the pager.
fn list_view(root: PartyRoot, find: &str, listed: Listed) -> Html {
    let rows = match listed {
        Listed::Rows(rows) => rows,
        Listed::Unserved => return info_note(t(tr(PartiesMsg::ListUnserved))),
        Listed::Refused(refusal) => return toolbar(root, find).then(refusal.view()),
    };
    let finding = !find.is_empty();
    let held = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    let paging = crate::paging::current(Listing::Parties);
    let window = OpenWindow::of(paging, held);
    let body = if rows.is_empty() && (finding || window.page == 1) {
        list_empty(
            tr(if finding {
                PartiesMsg::NoMatch
            } else {
                PartiesMsg::Empty
            }),
            None,
        )
    } else {
        table(&rows)
    };
    let pager = (!finding && (window.page > 1 || window.has_next())).then(|| {
        let fields = crate::paging::fields(Listing::Parties);
        post_open_pager(
            &current(),
            PagerListing::new(tr(PartiesMsg::Pages)),
            window,
            PostTo {
                id: "parties-pager",
                action: crate::paging::PAGING_ROUTE,
                fields: &fields,
            },
        )
    });
    let capped = (finding && held >= PAGE_SIZE).then(|| {
        info_note(t(tr_with(
            PartiesMsg::FindCapped,
            &[("count", u64::from(held).into())],
        )))
    });
    let chips = finding.then(|| {
        active_filters(
            &current(),
            ActiveFilters {
                chips: vec![FilterChip {
                    label: tr_with(PartiesMsg::FindChip, &[("text", find.to_owned().into())]),
                    remove: root_href(root),
                }],
                clear: root_href(root),
                shown: None,
            },
            None,
        )
    });
    let card = list_card(
        &current(),
        ListCard {
            title: tr(Msg::NavParties),
            count: None,
            order: None,
            tabs: Some(tabs(root)),
            actions: None,
            body,
            pager,
        },
    );
    Html::join([toolbar(root, ""), opt(chips), opt(capped), card])
}

/// The open-by-uid form.
fn open_form(kind: PartyKind, uid: Option<&str>, invalid: Option<&str>) -> Html {
    let mut id_field = Field::new("p-id", tr(PartiesMsg::Uid)).holds_id();
    if let Some(message) = invalid {
        id_field = id_field.after(h(format!(
            r#"<p id="p-id-error" class="fui-field-error">{}</p>"#,
            t(message)
        )));
    }
    let id_field = id_field.view(h(format!(
        r#"<input id="p-id" name="uid" type="text" class="fui-input fui-mono" spellcheck="false" autocomplete="off" value="{}"{}{}>"#,
        t(uid.unwrap_or_default()),
        attr("aria-invalid", invalid.is_some().then_some("true")),
        attr("aria-describedby", invalid.is_some().then_some("p-id-error"))
    )));
    h(format!(
        r#"<form class="fui-card fui-panel" method="get" action="/parties"><div class="fui-transform-options">{}{id_field}<div class="fui-transform-run"><button type="submit" class="fui-button fui-button-secondary">{}{}</button></div></div></form>"#,
        Field::new("p-kind", tr(Msg::ColumnKind)).view(h(format!(
            r#"<select id="p-kind" name="kind" class="fui-select">{}</select>"#,
            kind_options(kind)
        ))),
        ferrohealth_ui::icon::icon(Glyph::ARROW_RIGHT, "16"),
        t(tr(Msg::PartiesOpen))
    ))
}

/// Parties' body: the list, the open form and the tag index.
fn parties_view(model: Option<PartiesModel>) -> Html {
    let Some(PartiesModel {
        root,
        find,
        listed,
        kind,
        uid,
        invalid,
        refused,
        index,
    }) = model
    else {
        return Html::empty();
    };
    let list = list_view(root, &find, listed);
    let form = open_form(kind, uid.as_deref(), invalid.as_deref());
    let (index, total) = match index {
        Ok(rows) if rows.is_empty() => (list_empty(tr(PartiesMsg::NoTags), None), Some(0)),
        Ok(rows) => (index_table(&rows), u64::try_from(rows.len()).ok()),
        Err(refusal) => (refusal.view(), None),
    };
    let tags = list_card(
        &current(),
        ListCard {
            title: tr(PartiesMsg::TagsTitle),
            count: total.map(ferrohealth_ui::view::template::list::HeadingCount::All),
            order: None,
            tabs: None,
            actions: None,
            body: index,
            pager: None,
        },
    );
    let top = refused.map(|refusal| refusal.view());
    h(format!(
        r#"<div class="fui-list-page">{}{list}{}{form}{tags}</div>"#,
        info_note(t(tr(PartiesMsg::Note))),
        opt(top)
    ))
}

/// Parties.
pub async fn list_page() -> Drawn {
    let heading = page_heading(
        &current(),
        Header {
            title: tr(Msg::NavParties).into(),
            subtitle: Some(tr(PartiesMsg::Subtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::PLUS),
            tr(RecordsMsg::PartiesNew),
            Act::Link("/parties/new".to_owned()),
        )),
    );
    let body = parties_view(read_parties().await);
    Drawn::page(tr(Msg::NavParties), heading.then(body))
}
