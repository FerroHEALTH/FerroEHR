// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The dashboard: four stat cards, compositions per day, and the matches of
//! each stored-query namespace, as the viewer's mockup draws it (FerroUI
//! `design/consoles/ferroehr/`, page 01).
//!
//! The counts come from fixed AQL and the template and stored-query lists
//! ([`crate::bff::dashboard`]), read on the server into one [`Dashboard`]
//! model the page draws; a namespace whose stored query did not run
//! shows no total rather than a short one, and a count the CDR answered in
//! another shape draws as unavailable with its reason, never as 0. A CDR that
//! does not answer turns the page into its refusal. No openEHR spec governs the viewer: our own
//! design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action};
use ferrohealth_ui::view::chart::{ActivityChart, activity_chart};
use ferrohealth_ui::view::surface::{Header, Stat, page_heading, stat_card, stat_card_marked};
use ferrohealth_ui::view::template::list::{ListCard, list_card, list_empty};

use crate::activity::ActivityPoint;
use crate::bff::dashboard::{Count, CountShape, NamespaceTile};
use crate::error::ViewerError;
use crate::locale::overview::OverviewMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, t};
use crate::pages::common::{refused, state_mark};
use crate::routes::Drawn;

/// One namespace tile and the link that opens Stored queries on it.
#[derive(Debug, Clone)]
pub struct NamespaceLink {
    /// The tile.
    tile: NamespaceTile,
    /// Stored queries, filtered to the tile's namespace.
    href: String,
}

/// What the dashboard draws, read from the CDR on the server.
#[derive(Debug, Clone)]
pub struct Dashboard {
    /// The EHRs the CDR holds.
    ehrs: Count,
    /// The compositions the CDR holds.
    compositions: Count,
    /// The operational templates the CDR holds.
    templates: u32,
    /// The stored queries, or `None` when the CDR refused the list.
    stored: Option<u32>,
    /// Compositions committed per day.
    trend: Result<Vec<ActivityPoint>, ViewerError>,
    /// The stored queries' matches by namespace.
    tiles: Result<Vec<NamespaceLink>, ViewerError>,
}

/// Reads the dashboard's model from the CDR.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session, and the
/// refusal of the headline counts, which leaves the page nothing to draw.
pub async fn load_dashboard() -> Result<Dashboard, ViewerError> {
    crate::session::require_session()?;
    let (ehrs, compositions, templates) = crate::bff::dashboard::dashboard_counts().await?;
    // NOTE: no openEHR spec governs this: our own design; a stored-query list
    // the CDR refused is an unknown count, drawn as the dash (R12), never 0.
    let stored = crate::queries_api::list_stored_queries()
        .await
        .ok()
        .map(|rows| u32::try_from(rows.len()).unwrap_or(u32::MAX));
    let trend = crate::bff::dashboard::commit_trend().await;
    let tiles = crate::bff::dashboard::namespace_tiles().await.map(|tiles| {
        tiles
            .into_iter()
            .map(|tile| NamespaceLink {
                href: namespace_href(&tile),
                tile,
            })
            .collect()
    });
    Ok(Dashboard {
        ehrs,
        compositions,
        templates,
        stored,
        trend,
        tiles,
    })
}

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
fn grouped(count: u64) -> String {
    ferrohealth_ui::i18n::number::grouped(&current(), count)
}

/// The mark of a count the CDR answered in another shape: unavailable, with
/// the reason in plain words.
pub(crate) fn unavailable(shape: CountShape) -> Html {
    ferrohealth_ui::view::state::state(
        ferrohealth_ui::view::flow::FlowState::Degraded.into(),
        tr(Msg::CountUnavailable),
        Some(shape.reason()),
    )
}

/// The stat card of `count`: its number, or the dash marked unavailable with
/// the reason the CDR's answer held no count.
fn count_card(glyph: Glyph, count: Count, label: String, href: &str) -> Html {
    let card = |value| Stat {
        glyph,
        value,
        label: label.into(),
        href: Some(href.to_owned()),
    };
    match count {
        Ok(count) => stat_card(card(grouped(count))),
        Err(shape) => stat_card_marked(card(tr(OverviewMsg::UnknownCount)), unavailable(shape)),
    }
}

/// The four stat cards.
fn stats(ehrs: Count, compositions: Count, templates: u32, stored: Option<u32>) -> Html {
    let cards = [
        count_card(Glyph::DATABASE, ehrs, tr(OverviewMsg::StatEhrs), "/ehrs"),
        count_card(
            Glyph::FILE_TEXT,
            compositions,
            tr(OverviewMsg::StatCompositions),
            "/query",
        ),
        stat_card(Stat {
            glyph: Glyph::FILE_CODE,
            value: grouped(u64::from(templates)),
            label: tr(OverviewMsg::StatTemplates).into(),
            href: Some("/templates".to_owned()),
        }),
        stat_card(Stat {
            glyph: crate::shell::STORED_QUERIES_GLYPH,
            value: stored.map_or_else(
                || tr(OverviewMsg::UnknownCount),
                |stored| grouped(u64::from(stored)),
            ),
            label: tr(OverviewMsg::StatStoredQueries).into(),
            href: Some("/queries".to_owned()),
        }),
    ]
    .into_iter()
    .collect::<Html>();
    h(format!(
        r#"<div class="fui-grid fui-grid-stats">{cards}</div>"#
    ))
}

/// How many days the activity chart covers.
const ACTIVITY_DAYS: u16 = 30;

/// The compositions-per-day card.
fn activity_card(points: Result<Vec<ActivityPoint>, ViewerError>) -> Html {
    let body = match points {
        Ok(points) if points.is_empty() => list_empty(
            tr(OverviewMsg::DashboardNoCompositions),
            Some((
                tr(OverviewMsg::DashboardCreateFirstEhr),
                "/ehrs/new".to_owned(),
            )),
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
                label: tr(OverviewMsg::DashboardPerDay),
                summary: tr_with(
                    OverviewMsg::DashboardPerDaySummary,
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
            h(format!(
                r#"<div class="fui-panel-body">{}<p class="fui-small fui-muted">{}</p></div>"#,
                activity_chart(&chart),
                t(tr(OverviewMsg::DashboardPerDayNote))
            ))
        }
        Err(error) => refused(tr(OverviewMsg::DashboardPerDayRefused), &error, None),
    };
    list_card(
        &current(),
        ListCard {
            title: tr(OverviewMsg::DashboardPerDay),
            count: None,
            order: None,
            tabs: None,
            actions: None,
            body,
            pager: None,
        },
    )
}

/// The matches-by-namespace card.
fn namespaces_card(tiles: Result<Vec<NamespaceLink>, ViewerError>) -> Html {
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
                Column::text("namespace", tr(OverviewMsg::ColumnNamespace)).role(Role::Title),
                Column::text("matches", tr(OverviewMsg::ColumnMatches))
                    .numeric()
                    .role(Role::Meta),
            ];
            let rows = tiles
                .iter()
                .map(|NamespaceLink { tile, href }| {
                    let sub = tr_with(
                        OverviewMsg::DashboardQueries,
                        &[("count", u64::from(tile.members).into())],
                    );
                    let matches = match tile.matches {
                        Some(matches) => t(tr_with(
                            OverviewMsg::DashboardMatches,
                            &[("count", u64::try_from(matches).unwrap_or(0).into())],
                        )),
                        None => state_mark(
                            ferrohealth_ui::view::flow::FlowState::Degraded,
                            &tr(OverviewMsg::DashboardNoTotal),
                        ),
                    };
                    let sub = h(format!(r#"<span class="fui-row-sub">{}</span>"#, t(sub)));
                    let title = title_cell(Some(href), t(&tile.label), Some(sub));
                    let matches = cell(&columns[1], matches);
                    h(format!("<tr>{title}{matches}</tr>"))
                })
                .collect();
            let total = u64::try_from(tiles.len()).unwrap_or_default();
            let caption = tr(OverviewMsg::DashboardByNamespace);
            (
                Some(total),
                data_table(DataTable {
                    caption: &caption,
                    columns: &columns,
                    sorting: Sorting::default(),
                    body: Body::Rows(rows),
                }),
            )
        }
        Err(error) => (
            None,
            refused(tr(OverviewMsg::DashboardNamespacesRefused), &error, None),
        ),
    };
    list_card(
        &current(),
        ListCard {
            title: tr(OverviewMsg::DashboardByNamespace),
            count: count.map(ferrohealth_ui::view::template::list::HeadingCount::All),
            order: None,
            tabs: None,
            actions: None,
            body,
            pager: None,
        },
    )
}

/// The page's heading.
fn heading(subtitle: String, actions: ActionGroup) -> Html {
    page_heading(
        &current(),
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
}

/// The dashboard.
pub async fn page() -> Drawn {
    let dashboard = match load_dashboard().await {
        Ok(dashboard) => dashboard,
        Err(error) => {
            let retry = ferrohealth_ui::view::button::link_button(
                "/".to_owned(),
                ferrohealth_ui::view::button::Variant::Primary,
                Some(Glyph::REFRESH_CW),
                tr(OverviewMsg::TryAgain),
            );
            let body =
                heading(tr(OverviewMsg::DashboardNoAnswer), ActionGroup::none()).then(h(format!(
                    r#"<div class="fui-stack">{}<div class="fui-cluster">{retry}</div></div>"#,
                    refused(tr(OverviewMsg::DashboardRefused), &error, None)
                )));
            return Drawn::page(tr(Msg::NavDashboard), body);
        }
    };
    let Dashboard {
        ehrs,
        compositions,
        templates,
        stored,
        trend,
        tiles,
    } = dashboard;
    let actions = if ehrs == Ok(0) && compositions == Ok(0) && templates == 0 {
        ActionGroup::primary(Action::new(
            Some(Glyph::UPLOAD),
            tr(OverviewMsg::UploadATemplate),
            Act::Link("/templates/upload".to_owned()),
        ))
    } else {
        ActionGroup::none()
    };
    let host = crate::request::settings().cdr_address;
    let subtitle = tr_with(OverviewMsg::DashboardSubtitle, &[("host", host.into())]);
    let body = heading(subtitle, actions).then(h(format!(
        r#"<div class="fui-stack fui-gap-section">{}<div class="fui-grid fui-grid-start">{}{}</div></div>"#,
        stats(ehrs, compositions, templates, stored),
        activity_card(trend),
        namespaces_card(tiles)
    )));
    Drawn::page(tr(Msg::NavDashboard), body)
}

#[cfg(test)]
mod tests {
    use super::stats;
    use crate::bff::dashboard::CountShape;
    use crate::locale::{Msg, tr};

    #[test]
    fn a_real_zero_draws_zero() {
        let html = stats(Ok(0), Ok(0), 0, Some(0)).as_str().to_owned();
        assert!(
            html.contains(r#"<div class="fui-stat-value">0</div>"#),
            "{html}"
        );
        assert!(!html.contains(&tr(Msg::CountUnavailable)), "{html}");
    }

    #[test]
    fn a_malformed_count_draws_unavailable_with_its_reason_never_zero() {
        let html = stats(
            Err(CountShape::NoRow),
            Err(CountShape::NotACount),
            3,
            Some(4),
        )
        .as_str()
        .to_owned();
        assert!(
            !html.contains(r#"<div class="fui-stat-value">0</div>"#),
            "{html}"
        );
        assert_eq!(
            html.matches(&tr(Msg::CountUnavailable)).count(),
            2,
            "{html}"
        );
        assert!(html.contains(&CountShape::NoRow.reason()), "{html}");
        assert!(html.contains(&CountShape::NotACount.reason()), "{html}");
    }
}
