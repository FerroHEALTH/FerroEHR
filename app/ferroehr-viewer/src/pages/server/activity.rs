// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Server › Activity: the repository's change history counted per service
//! and interval ([`crate::bff::report`]).
//!
//! The service and the interval are a `GET` form, so a report is an address;
//! the contribution ids page by `?page=`, since they name no patient. No
//! openEHR spec governs the viewer: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::query::Query;
use ferrohealth_ui::table::model::{Paging, Sizes, Window};
use ferrohealth_ui::table::pager::{Listing, link_pager};
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, data_table};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::layout::Arrangement;
use ferrohealth_ui::view::surface::{Stat, stat_card};

use super::{ServerTab, Tabbed, load_tab, server_page};
use crate::bff::report::{Ask, Report, SERVICES, fetch_report};
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, current, tr};
use crate::markup::{flag, h, t};
use crate::pages::common::{Refused, date_range, info_note, is_status, plain_section, section};
use crate::routes::Drawn;

/// The route of the tab.
const ROUTE: &str = "/server/activity";

/// Activity's model: the ask the address holds and its report.
#[derive(Debug, Clone, PartialEq)]
struct Activity {
    /// The service and the interval asked for.
    ask: Ask,
    /// The report, when a service was asked for, or its refusal.
    report: Option<Result<Report, Refused>>,
}

/// What the page reads, on the server.
async fn read_activity() -> Tabbed<Activity> {
    load_tab(ServerTab::Activity, || async {
        let get = |key: &str| crate::request::query(key).unwrap_or_default();
        let ask = Ask {
            service: get("a_service"),
            from: get("from"),
            to: get("to"),
        };
        let report = if ask.service.is_empty() {
            None
        } else {
            Some(
                fetch_report(&ask)
                    .await
                    .map_err(|error| Refused::new(tr(ConsoleMsg::ActivityRefused), error)),
            )
        };
        Activity { ask, report }
    })
    .await
}

/// A service in the reader's words.
fn service_words(service: &str) -> String {
    tr(match service {
        "EHR" => ConsoleMsg::ActivityServiceEhr,
        "DEMOGRAPHIC" => ConsoleMsg::ActivityServiceDemographic,
        "DEFINITIONS" => ConsoleMsg::ActivityServiceDefinitions,
        "QUERY" => ConsoleMsg::ActivityServiceQuery,
        "ADMIN" => ConsoleMsg::ActivityServiceAdmin,
        "EHR_INDEX" => ConsoleMsg::ActivityServiceEhrIndex,
        "MESSAGE" => ConsoleMsg::ActivityServiceMessage,
        _ => ConsoleMsg::ActivityServiceSystemLog,
    })
}

/// The form that asks for a report, the refusal of the last ask beside its
/// fields.
fn ask_form(ask: &Ask, refusal: Option<&Refused>) -> Html {
    let options: Html = SERVICES
        .iter()
        .map(|service| {
            h(format!(
                r#"<option value="{service}"{}>{}</option>"#,
                flag("selected", ask.service.eq_ignore_ascii_case(service)),
                t(service_words(service))
            ))
        })
        .collect();
    let service = Field::new("r-service", tr(ConsoleMsg::ActivityService))
        .holds_id()
        .view(h(format!(
            r#"<select id="r-service" name="a_service" class="fui-select">{options}</select>"#
        )));
    let form = h(format!(
        r#"<form method="get" action="{ROUTE}" class="fui-transform-options">{service}{range}<div class="fui-transform-run"><button type="submit" class="fui-button fui-button-primary">{glyph}{count}</button></div></form>"#,
        range = date_range(
            "r-when",
            tr(ConsoleMsg::ActivityInterval),
            &ask.from,
            &ask.to
        ),
        glyph = ferrohealth_ui::icon::icon(Glyph::CHART_LINE, "16"),
        count = t(tr(ConsoleMsg::ActivityCount)),
    ));
    section(
        Some(tr(ConsoleMsg::ActivityAsk)),
        None,
        None,
        form.then(refusal.map(Refused::view).unwrap_or_default()),
    )
}

/// The link to a contribution the viewer can open: a demographic one opens
/// on its own page; any other names its EHR, which the report does not say.
fn contribution(service: &str, id: &str) -> Html {
    if service.eq_ignore_ascii_case("demographic") {
        h(format!(
            r#"<a href="/parties/contributions/{}"><code>{}</code></a>"#,
            urlencoding::encode(id),
            t(id)
        ))
    } else {
        h(format!("<code>{}</code>", t(id)))
    }
}

/// The report, drawn: the counts, then the contribution ids a page at a time.
fn report(ask: &Ask, report: &Report) -> Html {
    let tile = |value: u64, label: ConsoleMsg, glyph: Glyph| {
        stat_card(Stat {
            glyph,
            value: value.to_string(),
            label: tr(label).into(),
            href: None,
        })
    };
    let listed = u64::try_from(report.contributions.len()).unwrap_or(u64::MAX);
    let counts = section(
        Some(tr(ConsoleMsg::ActivityCounts)),
        None,
        None,
        Arrangement::StatGrid.wrap(Html::join([
            tile(
                report.contribution_count,
                ConsoleMsg::ActivityContributions,
                Glyph::ACTIVITY,
            ),
            tile(listed, ConsoleMsg::ActivityListed, Glyph::ACTIVITY),
            tile(
                report.versioned_compositions,
                ConsoleMsg::ActivityCompositions,
                Glyph::ACTIVITY,
            ),
            tile(
                report.composition_versions,
                ConsoleMsg::ActivityVersions,
                Glyph::ACTIVITY,
            ),
        ])),
    );
    let total = u32::try_from(report.contributions.len()).unwrap_or(u32::MAX);
    let paging = Paging::read(
        crate::request::query("page").as_deref(),
        crate::request::query("size").as_deref(),
    );
    let window = Window::over(total, paging);
    let columns = [Column::text("id", tr(ConsoleMsg::ActivityContribution)).role(Role::Title)];
    let rows = window
        .slice(&report.contributions)
        .iter()
        .map(|id| {
            h(format!(
                r#"<tr><th scope="row" data-role="title">{}</th></tr>"#,
                contribution(&ask.service, id)
            ))
        })
        .collect();
    let caption = tr(ConsoleMsg::ActivityContributionIds);
    let href = |paging: Paging| {
        let mut query = Query::parse(
            &crate::request::path_and_query()
                .split_once('?')
                .map(|(_, q)| q.to_owned())
                .unwrap_or_default(),
        );
        paging.write(&mut query, Sizes::STANDARD);
        let text = query.to_query_string();
        if text.is_empty() {
            ROUTE.to_owned()
        } else {
            format!("{ROUTE}?{text}")
        }
    };
    let ids = if report.contributions.is_empty() {
        info_note(t(tr(ConsoleMsg::ActivityNone)))
    } else {
        data_table(DataTable::new(&caption, &columns, Body::Rows(rows))).then(link_pager(
            &current(),
            Listing::new(caption.clone()),
            window,
            &href,
        ))
    };
    counts.then(section(Some(caption), Some(listed), None, ids))
}

/// Activity, drawn.
fn activity(model: Activity) -> Html {
    let Activity { ask, report: read } = model;
    match read {
        Some(Err(refusal)) if is_status(&refusal.error, http::StatusCode::METHOD_NOT_ALLOWED) => {
            plain_section(info_note(t(tr(ConsoleMsg::ServerNoAdmin))))
        }
        Some(Err(refusal)) => ask_form(&ask, Some(&refusal)),
        Some(Ok(answer)) => ask_form(&ask, None).then(report(&ask, &answer)),
        None => {
            ask_form(&ask, None).then(plain_section(info_note(t(tr(ConsoleMsg::ActivityPick)))))
        }
    }
}

/// Server › Activity.
pub async fn page() -> Drawn {
    let tabbed = read_activity().await;
    Drawn::page(
        tr(Msg::NavServer),
        server_page(ServerTab::Activity, tabbed, activity),
    )
}
