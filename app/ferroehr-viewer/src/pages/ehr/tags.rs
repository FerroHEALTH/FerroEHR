// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Tags tab: every `ITEM_TAG` in the EHR, grouped by the object it
//! is set on (FerroUI `design/consoles/ferroehr/`, page 12.13).
//!
//! The list is the CDR's aggregate read (ITS-REST `ehr_tags_get`), which
//! spans the version containers and single versions of every taggable kind.
//! A tag is set on its object's own Tags tab; this list reads them all. The
//! key and value filters are a `GET` form sent to the CDR as its released
//! filters, so a filtered list keeps its address. No openEHR spec governs
//! the viewer's pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::filter::{Find, find_form};
use ferrohealth_ui::table::model::StateSource;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, group_row,
};
use ferrohealth_ui::view::template::list::{ActiveFilters, FilterChip, active_filters, list_empty};

use crate::bff::ehr_tags::{EhrTargetKind, list_ehr_tags, resolve_ehr_target};
use crate::bff::item_tags::{TagGroup, group_by_target};
use crate::error::ViewerError;
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{Refused, drawn, shortened};
use crate::pages::ehr::EhrModel;
use crate::pages::ehr::{EhrTab, ehr_id_param, ehr_view, tab_href};
use crate::routes::Drawn;

/// The filters the address names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Filters {
    /// The exact key.
    key: String,
    /// The exact value.
    value: String,
}

impl Filters {
    /// The filters of the request.
    fn read() -> Self {
        Self {
            key: crate::request::query("key").unwrap_or_default(),
            value: crate::request::query("value").unwrap_or_default(),
        }
    }

    /// The address of the tab with the filter `without` dropped.
    fn href_without(&self, ehr_id: &str, without: &str) -> String {
        let pairs: Vec<String> = [("key", &self.key), ("value", &self.value)]
            .into_iter()
            .filter(|(name, value)| *name != without && !value.is_empty())
            .map(|(name, value)| format!("{name}={}", urlencoding::encode(value)))
            .collect();
        let base = tab_href(ehr_id, EhrTab::Tags);
        if pairs.is_empty() {
            base
        } else {
            format!("{base}?{}", pairs.join("&"))
        }
    }
}

/// The filter toolbar: the key as the find field, the value beside it.
fn toolbar(ehr_id: &str, filters: &Filters) -> Html {
    let action = tab_href(ehr_id, EhrTab::Tags);
    let tools = h(format!(
        r#"<div class="fui-list-find"><label for="t-value" class="fui-sr-only">{label}</label><input id="t-value" name="value" type="text" class="fui-input" value="{value}" placeholder="{label}" autocomplete="off"></div><button type="submit" class="fui-button fui-button-secondary">{glyph}{apply}</button>"#,
        label = t(tr(Msg::TagsValue)),
        value = t(&filters.value),
        glyph = ferrohealth_ui::icon::icon(ferrohealth_ui::icon::Glyph::FUNNEL, "16"),
        apply = t(tr(Msg::FilterApply)),
    ));
    let label = tr(RecordsMsg::TagsKey);
    find_form(
        Find {
            id: "t-key",
            name: "key",
            label: &label,
            value: &filters.key,
            source: StateSource::Url,
            action: Some(&action),
            fields: &[],
        },
        tools,
    )
}

/// The active filters as chips.
fn chips(ehr_id: &str, filters: &Filters) -> Option<Html> {
    let chips: Vec<FilterChip> = [
        ("key", &filters.key, RecordsMsg::ChipKey),
        ("value", &filters.value, RecordsMsg::ChipValue),
    ]
    .into_iter()
    .filter(|(_, value, _)| !value.is_empty())
    .map(|(name, value, msg)| FilterChip {
        label: tr_with(msg, &[("value", value.clone().into())]),
        remove: filters.href_without(ehr_id, name),
    })
    .collect();
    (!chips.is_empty()).then(|| {
        active_filters(
            &current(),
            ActiveFilters {
                chips,
                clear: tab_href(ehr_id, EhrTab::Tags),
                shown: None,
            },
            None,
        )
    })
}

/// A tagged object as its group heading names it: what it is, where it opens,
/// and whether the tags sit on the whole object or one version.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    /// Its name.
    name: String,
    /// Its page, when the viewer has one.
    href: Option<String>,
    /// Which form the tags target.
    form: String,
}

/// One tag of a group.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tag {
    /// Its key.
    key: String,
    /// Its value, empty when it carries none.
    value: String,
    /// The path it targets, empty for the whole object.
    target_path: String,
}

/// The tags set on one object.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tagged {
    /// The object.
    target: Target,
    /// Its tags.
    tags: Vec<Tag>,
}

/// The tab's list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum List {
    /// No tag matches.
    Empty,
    /// The tags, grouped by their object.
    Groups(Vec<Tagged>),
    /// The CDR's refusal.
    Refused(Refused),
}

/// The tab's model: the filters it read and the list they found.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Panel {
    /// The filters.
    filters: Filters,
    /// The list.
    list: List,
}

/// Names the object a group of tags is set on, asking the CDR which kind it
/// is.
async fn target(ehr_id: &str, group: &TagGroup) -> Result<Target, ViewerError> {
    let container = crate::uid::container_uid_of(&group.target);
    let version =
        (container != group.target).then(|| crate::pages::common::version_number(&group.target));
    let kind = resolve_ehr_target(ehr_id.to_owned(), group.target.clone())
        .await?
        .and_then(|segment| EhrTargetKind::from_segment(&segment));
    let (name, versioned) = match kind {
        Some(EhrTargetKind::Composition) => (
            tr_with(
                RecordsMsg::TagsComposition,
                &[("id", shortened(&container).into())],
            ),
            tr(RecordsMsg::TagsVersionedComposition),
        ),
        Some(EhrTargetKind::EhrStatus) => (
            tr(RecordsMsg::TagsEhrStatus),
            tr(RecordsMsg::TagsVersionedStatus),
        ),
        Some(EhrTargetKind::Directory) => (
            tr(RecordsMsg::EhrTabDirectory),
            tr(RecordsMsg::TagsVersionedDirectory),
        ),
        None => (shortened(&container), tr(RecordsMsg::TagsUnknownObject)),
    };
    let form = match version {
        Some(number) => tr_with(RecordsMsg::TagsOneVersion, &[("version", number.into())]),
        None => versioned,
    };
    Ok(Target {
        name,
        href: kind.map(|kind| kind.href(ehr_id, &container)),
        form,
    })
}

/// The tags of `groups`, each group's object named.
async fn tagged(ehr_id: &str, groups: Vec<TagGroup>) -> Result<Vec<Tagged>, ViewerError> {
    let mut named = Vec::new();
    for group in groups {
        let target = target(ehr_id, &group).await?;
        let tags = group
            .tags
            .into_iter()
            .map(|tag| Tag {
                key: tag.key,
                value: tag.value,
                target_path: tag.target_path,
            })
            .collect();
        named.push(Tagged { target, tags });
    }
    Ok(named)
}

/// Loads the tab: the tags the request's filters find.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)).
async fn load(
    // The EHR the route names.
    ehr_id: String,
) -> Result<EhrModel<Panel>, ViewerError> {
    crate::session::require_session()?;
    Ok(read_page(ehr_id).await)
}

/// What [`load`] reads.
async fn read_page(ehr_id: String) -> EhrModel<Panel> {
    crate::pages::ehr::load_ehr(&ehr_id, async |_facts| {
        let filters = Filters::read();
        let tags = list_ehr_tags(
            ehr_id.clone(),
            filters.key.clone(),
            filters.value.clone(),
            String::new(),
        )
        .await;
        let list = match tags {
            Ok(tags) if tags.is_empty() => List::Empty,
            Ok(tags) => match tagged(&ehr_id, group_by_target(tags)).await {
                Ok(groups) => List::Groups(groups),
                Err(error) => List::Refused(Refused::new(tr(RecordsMsg::TagsRefused), error)),
            },
            Err(error) => List::Refused(Refused::new(tr(RecordsMsg::TagsRefused), error)),
        };
        Panel { filters, list }
    })
    .await
}

/// The tags, grouped by their object.
fn table(groups: Vec<Tagged>) -> Html {
    let columns = [
        Column::text("key", tr(RecordsMsg::TagsKey)).role(Role::Title),
        Column::text("value", tr(Msg::TagsValue)).role(Role::Sub),
        Column::text("path", tr(RecordsMsg::TagsTargetPath)).role(Role::Meta),
    ];
    let mut rows = Vec::new();
    for group in groups {
        let target = group.target;
        let name = match target.href {
            Some(href) => h(format!(r#"<a href="{}">{}</a>"#, t(href), t(&target.name))),
            None => t(&target.name),
        };
        rows.push(group_row(
            &columns,
            name.then(h(format!(
                r#"<span class="fui-small fui-muted">{}</span>"#,
                t(format!(" · {}", target.form))
            ))),
        ));
        for tag in group.tags {
            let path = if tag.target_path.is_empty() {
                h(format!(
                    r#"<span class="fui-muted">{}</span>"#,
                    t(tr(RecordsMsg::TagsWholeObject))
                ))
            } else {
                h(format!("<code>{}</code>", t(&tag.target_path)))
            };
            rows.push(h(format!(
                r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}{}</tr>"#,
                t(&tag.key),
                cell(&columns[1], t(&tag.value)),
                cell(&columns[2], path)
            )));
        }
    }
    let caption = tr(RecordsMsg::TagsCaption);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(Html::join(rows)),
    })
}

/// The tab's panel.
fn panel(ehr_id: &str, panel: Panel) -> Html {
    let Panel { filters, list } = panel;
    let list = match list {
        List::Empty => list_empty(
            tr(if filters.key.is_empty() && filters.value.is_empty() {
                RecordsMsg::TagsEmpty
            } else {
                RecordsMsg::TagsNoMatch
            }),
            None,
        ),
        List::Groups(groups) => table(groups),
        List::Refused(refusal) => refusal.view(),
    };
    h(format!(
        r#"<div class="fui-panel"><div class="fui-panel-body fui-stack"><p class="fui-small fui-muted">{}</p>{}{}</div>{list}</div>"#,
        t(tr(RecordsMsg::TagsIntro)),
        toolbar(ehr_id, &filters),
        opt(chips(ehr_id, &filters))
    ))
}

/// An EHR's Tags tab.
pub async fn page() -> Drawn {
    let ehr_id = ehr_id_param();
    let title = tr_with(RecordsMsg::TagsTitle, &[("id", shortened(&ehr_id).into())]);
    let body = drawn(load(ehr_id.clone()).await, |model| {
        ehr_view(&ehr_id, EhrTab::Tags, model, |found| panel(&ehr_id, found))
    });
    Drawn::page(title, body)
}
