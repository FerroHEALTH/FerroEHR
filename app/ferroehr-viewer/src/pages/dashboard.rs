// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The dashboard: four stat cards, compositions per day, and the matches of
//! each stored-query namespace, as the viewer's mockup draws it (FerroUI
//! `design/consoles/ferroehr/`, page 01).
//!
//! The counts come from fixed AQL and the template and stored-query lists
//! ([`crate::bff::dashboard`]); a namespace whose stored query did not run
//! shows no total rather than a short one. A CDR that does not answer turns
//! the page into its refusal. No openEHR spec governs the viewer: our own
//! design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::chart::{ActivityChart, activity_chart};
use ferrohealth_ui::view::surface::{Header, Stat, page_heading, stat_card};
use ferrohealth_ui::view::template::list::{ListCard, list_card, list_empty};
use ferrohealth_ui::widget::action_menu::{Act, Action};
use leptos::prelude::*;
use leptos_meta::Title;

use crate::bff::dashboard::{NamespaceTile, commit_trend, dashboard_counts, namespace_tiles};
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{awaited, refused, state_mark};

/// The link that opens Stored queries on the namespace of `tile`.
fn namespace_href(tile: &NamespaceTile) -> String {
    let unqualified = crate::query_namespace::group_label(None);
    let key = if tile.label == unqualified {
        crate::pages::queries::UNQUALIFIED.to_owned()
    } else {
        tile.label.clone()
    };
    format!("/queries?namespace={}", urlencoding::encode(&key))
}

/// A count grouped in the page's language.
fn grouped(count: i64) -> String {
    ferrohealth_ui::i18n::number::grouped(u64::try_from(count).unwrap_or_default())
}

/// The four stat cards.
fn stats(ehrs: i64, compositions: i64, templates: u32, stored: Option<u32>) -> AnyView {
    let cards = [
        Stat {
            glyph: Glyph::DATABASE,
            value: grouped(ehrs),
            label: tr(Msg::StatEhrs).into(),
            href: Some("/ehrs".to_owned()),
        },
        Stat {
            glyph: Glyph::FILE_TEXT,
            value: grouped(compositions),
            label: tr(Msg::StatCompositions).into(),
            href: Some("/query".to_owned()),
        },
        Stat {
            glyph: Glyph::FILE_CODE,
            value: grouped(i64::from(templates)),
            label: tr(Msg::StatTemplates).into(),
            href: Some("/templates".to_owned()),
        },
        Stat {
            glyph: crate::shell::STORED_QUERIES_GLYPH,
            value: stored.map_or_else(
                || tr(Msg::UnknownCount),
                |stored| grouped(i64::from(stored)),
            ),
            label: tr(Msg::StatStoredQueries).into(),
            href: Some("/queries".to_owned()),
        },
    ]
    .into_iter()
    .map(|stat| stat_card(stat).into_any())
    .collect_view();
    view! { <div class="fui-grid fui-grid-stats">{cards}</div> }.into_any()
}

/// How many days the activity chart covers.
const ACTIVITY_DAYS: u16 = 30;

/// The compositions-per-day card.
fn activity_card(
    points: Result<Vec<crate::activity::ActivityPoint>, crate::error::ViewerError>,
) -> AnyView {
    let body = match points {
        Ok(points) if points.is_empty() => list_empty(
            tr(Msg::DashboardNoCompositions),
            Some((tr(Msg::DashboardCreateFirstEhr), "/ehrs/new".to_owned())),
        ),
        Ok(points) => {
            let days = crate::pages::common::last_days(&points, ACTIVITY_DAYS);
            let total: u32 = days.counts.iter().map(|(_, count)| *count).sum();
            let most = days
                .counts
                .iter()
                .map(|(_, count)| *count)
                .max()
                .unwrap_or(0);
            let chart = ActivityChart {
                name: "compositions-per-day",
                label: tr(Msg::DashboardPerDay),
                summary: tr_with(
                    Msg::DashboardPerDaySummary,
                    &[
                        ("total", u64::from(total).into()),
                        ("most", u64::from(most).into()),
                        ("first", days.first.clone().into()),
                        ("last", days.last.clone().into()),
                    ],
                ),
                points: days.counts,
                ticks: days.ticks,
            };
            view! {
                <div class="fui-panel-body">
                    {activity_chart(&chart)}
                    <p class="fui-small fui-muted">{tr(Msg::DashboardPerDayNote)}</p>
                </div>
            }
            .into_any()
        }
        Err(error) => refused(tr(Msg::DashboardPerDayRefused), &error, None),
    };
    list_card(ListCard {
        title: tr(Msg::DashboardPerDay),
        count: None,
        order: None,
        tabs: None,
        actions: None,
        body,
        pager: None,
    })
}

/// The matches-by-namespace card.
fn namespaces_card(tiles: Result<Vec<NamespaceTile>, crate::error::ViewerError>) -> AnyView {
    let (count, body) = match tiles {
        Ok(tiles) if tiles.is_empty() => (
            None,
            list_empty(
                tr(Msg::DashboardNoStoredQuery),
                Some((tr(Msg::DashboardWriteOne), "/query/aql".to_owned())),
            ),
        ),
        Ok(tiles) => {
            let columns = [
                Column::text("namespace", tr(Msg::ColumnNamespace)).role(Role::Title),
                Column::text("matches", tr(Msg::ColumnMatches))
                    .numeric()
                    .role(Role::Meta),
            ];
            let rows = tiles
                .iter()
                .map(|tile| {
                    let sub = tr_with(
                        Msg::DashboardQueries,
                        &[("count", u64::from(tile.members).into())],
                    );
                    let matches = match tile.matches {
                        Some(matches) => tr_with(
                            Msg::DashboardMatches,
                            &[("count", u64::try_from(matches).unwrap_or(0).into())],
                        )
                        .into_any(),
                        None => state_mark(
                            ferrohealth_ui::view::flow::FlowState::Degraded,
                            &tr(Msg::DashboardNoTotal),
                        ),
                    };
                    let href = namespace_href(tile);
                    let label = tile.label.clone();
                    let sub = view! { <span class="fui-row-sub">{sub}</span> }.into_any();
                    let title = title_cell(Some(&href), label, Some(sub));
                    let matches = cell(&columns[1], matches);
                    view! { <tr>{title} {matches}</tr> }.into_any()
                })
                .collect_view()
                .into_any();
            let total = u64::try_from(tiles.len()).unwrap_or_default();
            let caption = tr(Msg::DashboardByNamespace);
            (
                Some(total),
                data_table(DataTable {
                    caption: &caption,
                    columns: &columns,
                    sorting: Sorting::default(),
                    body: Body::Rows(rows),
                })
                .into_any(),
            )
        }
        Err(error) => (
            None,
            refused(tr(Msg::DashboardNamespacesRefused), &error, None),
        ),
    };
    list_card(ListCard {
        title: tr(Msg::DashboardByNamespace),
        count: count.map(ferrohealth_ui::view::template::list::HeadingCount::All),
        order: None,
        tabs: None,
        actions: None,
        body,
        pager: None,
    })
}

/// The page's heading.
fn heading(subtitle: String, actions: ActionGroup) -> AnyView {
    page_heading(
        Header {
            title: tr(Msg::NavDashboard).into(),
            subtitle: Some(subtitle.into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        actions,
    )
    .into_any()
}

/// The dashboard.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn DashboardPage() -> impl IntoView {
    let body = awaited(async move {
        let host = crate::request::state()
            .map(|state| state.config.cdr.base_url.clone())
            .unwrap_or_default();
        let (ehrs, compositions, templates) = match dashboard_counts().await {
            Ok(counts) => counts,
            Err(error) => {
                let retry = ferrohealth_ui::view::button::link_button(
                    "/".to_owned(),
                    ferrohealth_ui::view::button::Variant::Primary,
                    Some(Glyph::REFRESH_CW),
                    tr(Msg::TryAgain),
                );
                return view! {
                    {heading(tr(Msg::DashboardNoAnswer), ActionGroup::none())}
                    <div class="fui-stack">
                        {refused(tr(Msg::DashboardRefused), &error, None)}
                        <div class="fui-cluster">{retry}</div>
                    </div>
                }
                .into_any();
            }
        };
        // NOTE: no openEHR spec governs this: our own design; a stored-query list
        // the CDR refused is an unknown count, drawn as the dash (R12), never 0.
        let stored = crate::queries_api::list_stored_queries()
            .await
            .ok()
            .map(|rows| u32::try_from(rows.len()).unwrap_or(u32::MAX));
        let trend = commit_trend().await;
        let tiles = namespace_tiles().await;
        let actions = if ehrs == 0 && compositions == 0 && templates == 0 {
            ActionGroup::primary(Action::new(
                Some(Glyph::UPLOAD),
                tr(Msg::UploadATemplate),
                Act::Link("/templates/upload".to_owned()),
            ))
        } else {
            ActionGroup::none()
        };
        let subtitle = tr_with(Msg::DashboardSubtitle, &[("host", host.into())]);
        view! {
            {heading(subtitle, actions)}
            <div class="fui-stack fui-gap-section">
                {stats(ehrs, compositions, templates, stored)}
                <div class="fui-grid fui-grid-start">
                    {activity_card(trend)} {namespaces_card(tiles)}
                </div>
            </div>
        }
        .into_any()
    });
    view! {
        <Title text=tr(Msg::NavDashboard) />
        {body}
    }
}
