// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Server: the CDR, what it is, how it runs and what it serves, as six tabs,
//! Overview, Health, Metrics, Logging, API and SMART (FerroUI
//! `design/consoles/ferroehr/`, pages 19.1 to 19.9).
//!
//! Overview reads the status document the shell already read and the
//! management surface's build information; Health, Metrics and Logging read
//! the management surface ([`crate::management`]), which is off by default,
//! so a CDR without it shows Overview, API and SMART alone. API lists the
//! OpenAPI documents the CDR serves for itself, by family, and SMART its
//! `/.well-known/smart-configuration`. No openEHR spec governs these
//! surfaces: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's OpenAPI and SMART JSON documents to list them"
)]

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, group_row,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::dialog::{Confirmation, confirm_dialog};
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::scope::scope_grants;
use ferrohealth_ui::view::surface::{
    Header, Stat, definitions, page_heading, stat_card, stat_grid,
};
use ferrohealth_ui::view::tabs::{Tab, binder};
use leptos::prelude::*;
use leptos_meta::Title;
use serde_json::Value;

use crate::flash::{Done, redirect_with};
use crate::locale::{Msg, tr, tr_with};
use crate::management::{
    fetch_build_info, fetch_headline_metrics, fetch_loggers, fetch_metric_detail,
    fetch_metric_names, fetch_readiness, format_metric, probe_management_api, reset_log_filter,
    set_log_filter,
};
use crate::pages::common::{
    awaited, count, document_section, info_note, panel, plain_section, refused, section, state_mark,
};

/// The id of Apply the log filter's confirmation.
const APPLY_DIALOG: &str = "apply-log-filter";

/// The id of the log filter's form.
const LOG_FORM: &str = "log-filter";

/// The tabs of Server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerTab {
    /// What it is and how it was built.
    Overview,
    /// Its dependencies.
    Health,
    /// Its counters.
    Metrics,
    /// Its live log filter.
    Logging,
    /// Its OpenAPI documents.
    Api,
    /// Its SMART discovery document.
    Smart,
}

impl ServerTab {
    /// Every tab, in order.
    const ALL: [Self; 6] = [
        Self::Overview,
        Self::Health,
        Self::Metrics,
        Self::Logging,
        Self::Api,
        Self::Smart,
    ];

    /// The tab's key.
    const fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Health => "health",
            Self::Metrics => "metrics",
            Self::Logging => "logging",
            Self::Api => "api",
            Self::Smart => "smart",
        }
    }

    /// The tab's page.
    fn href(self) -> String {
        match self {
            Self::Overview => "/server".to_owned(),
            other => format!("/server/{}", other.key()),
        }
    }

    /// The tab's label and the line under the tabs.
    fn words(self) -> (String, String) {
        let (label, line) = match self {
            Self::Overview => (Msg::ServerOverview, Msg::ServerOverviewLine),
            Self::Health => (Msg::ServerHealth, Msg::ServerHealthLine),
            Self::Metrics => (Msg::ServerMetrics, Msg::ServerMetricsLine),
            Self::Logging => (Msg::ServerLogging, Msg::ServerLoggingLine),
            Self::Api => (Msg::ServerApi, Msg::ServerApiLine),
            Self::Smart => (Msg::ServerSmart, Msg::ServerSmartLine),
        };
        (tr(label), tr(line))
    }

    /// Whether the tab reads the management surface.
    const fn managed(self) -> bool {
        matches!(self, Self::Health | Self::Metrics | Self::Logging)
    }
}

/// A tab of Server over `body`; a management tab on a CDR without the
/// management surface says so.
async fn server_page<F, Fut>(tab: ServerTab, body: F) -> AnyView
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = AnyView>,
{
    let managed = probe_management_api().await.is_ok_and(|availability| {
        availability == crate::management::ManagementAvailability::Available
    });
    let tabs = ServerTab::ALL
        .into_iter()
        .filter(|each| managed || !each.managed())
        .map(|each| Tab::new(each.key(), each.href(), each.words().0))
        .collect();
    let content = if tab.managed() && !managed {
        info_note(tr(Msg::ServerNoManagement))
    } else {
        body().await
    };
    let heading = page_heading(
        Header {
            title: tr(Msg::NavServer).into(),
            subtitle: Some(tr(Msg::ServerSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
        },
        ActionGroup::none(),
    );
    let line = tab.words().1;
    view! {
        <Title text=tr(Msg::NavServer) />
        {heading}
        <div class="fui-stack">
            {binder(tr(Msg::ServerSections), tabs, tab.key(), panel(view! {
                {plain_section(ferrohealth_ui::view::surface::section_intro(line))}
                {content}
            }))}
        </div>
    }
    .into_any()
}

/// Server: Overview.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn OverviewTab() -> impl IntoView {
    let page = awaited(server_page(ServerTab::Overview, || async {
        let status = crate::shell::status().await;
        let document = status
            .as_ref()
            .ok()
            .and_then(|body| crate::deployment::StatusDocument::parse(body));
        let state = match (&status, &document) {
            (Ok(_), Some(document)) if document.status.as_deref() == Some("UP") => {
                state_mark(FlowState::Up, &tr(Msg::ServerUp))
            }
            (Ok(_), Some(document)) => state_mark(
                FlowState::Degraded,
                &document
                    .status
                    .clone()
                    .unwrap_or_else(|| tr(Msg::ServerUnknown)),
            ),
            (Ok(_), None) => state_mark(FlowState::NotChecked, &tr(Msg::ServerUnknown)),
            (Err(_), _) => state_mark(FlowState::Unreachable, &tr(Msg::ServerUnreachable)),
        };
        let deployment = document
            .as_ref()
            .and_then(|document| document.deployment.clone());
        let sandbox = deployment
            .as_ref()
            .is_some_and(|deployment| deployment.profile == "sandbox");
        let facts = definitions(vec![
            (tr(Msg::ServerState), state),
            (
                tr(Msg::ServerDeployment),
                tr(if sandbox {
                    Msg::ServerSandbox
                } else {
                    Msg::ServerProduction
                })
                .into_any(),
            ),
            (
                tr(Msg::ServerVersion),
                document
                    .as_ref()
                    .and_then(|document| document.server_version.clone())
                    .unwrap_or_default()
                    .into_any(),
            ),
            (
                tr(Msg::ServerConformance),
                view! { <a href="/about">{tr(Msg::NavAbout)}</a> }.into_any(),
            ),
        ]);
        let gaps = deployment.filter(|_| sandbox).map(|deployment| {
            let columns = [Column::text("gap", tr(Msg::ServerSafeguard)).role(Role::Title)];
            let rows = deployment
                .gaps
                .iter()
                .map(|gap| {
                    view! {
                        <tr>
                            <th scope="row" data-role="title">
                                {state_mark(FlowState::Degraded, &crate::deployment::gap_text(gap))}
                            </th>
                        </tr>
                    }
                    .into_any()
                })
                .collect_view()
                .into_any();
            let caption = tr(Msg::ServerSafeguardsOff);
            section(
                Some(tr(Msg::ServerSafeguardsOff)),
                u64::try_from(deployment.gaps.len()).ok(),
                None,
                view! {
                    {ferrohealth_ui::view::surface::section_intro(tr(Msg::ServerSandboxIntro))}
                    {data_table(DataTable { caption: &caption, columns: &columns, sorting: Sorting::default(), body: Body::Rows(rows) })}
                },
            )
        });
        let build = match fetch_build_info().await {
            Ok(Some(info)) => {
                let mut parts = vec![section(
                    Some(tr(Msg::ServerBuild)),
                    None,
                    None,
                    definitions(
                        info.facts
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone().into_any()))
                            .collect(),
                    ),
                )];
                for group in &info.sections {
                    parts.push(section(
                        Some(group.key.clone()),
                        None,
                        None,
                        definitions(
                            group
                                .rows
                                .iter()
                                .map(|(key, value)| (key.clone(), value.clone().into_any()))
                                .collect(),
                        ),
                    ));
                }
                parts.collect_view().into_any()
            }
            Ok(None) => plain_section(info_note(tr(Msg::ServerNoBuildInfo))),
            Err(error) => plain_section(refused(tr(Msg::ServerBuildRefused), &error, None)),
        };
        view! {
            {section(Some(tr(Msg::ServerStatus)), None, None, facts)}
            {gaps}
            {build}
        }
        .into_any()
    }));
    view! { {page} }
}

/// The flow state of an indicator's status word.
fn flow_of(status: &str) -> FlowState {
    match status {
        "UP" | "up" => FlowState::Up,
        "DOWN" | "down" => FlowState::Unreachable,
        "OUT_OF_SERVICE" | "DEGRADED" | "degraded" => FlowState::Degraded,
        "DISABLED" | "OFF" | "off" => FlowState::Off,
        _ => FlowState::NotChecked,
    }
}

/// Server: Health.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn HealthTab() -> impl IntoView {
    let page = awaited(server_page(ServerTab::Health, || async {
        match fetch_readiness().await {
            Ok(readiness) => {
                let columns = [
                    Column::text("dependency", tr(Msg::ServerDependency)).role(Role::Title),
                    Column::text("state", tr(Msg::ServerState)).role(Role::Sub),
                    Column::text("detail", tr(Msg::ServerDetail)).role(Role::Meta),
                ];
                let rows = readiness
                    .components
                    .iter()
                    .map(|row| {
                        view! {
                            <tr>
                                <th scope="row" data-role="title">{row.name.clone()}</th>
                                {cell(&columns[1], state_mark(flow_of(&row.status), &row.status.clone()))}
                                {cell(&columns[2], row.detail.clone())}
                            </tr>
                        }
                        .into_any()
                    })
                    .collect_view()
                    .into_any();
                let caption = tr(Msg::ServerDependencies);
                section(
                    Some(tr(Msg::ServerDependencies)),
                    u64::try_from(readiness.components.len()).ok(),
                    None,
                    view! {
                        {(!readiness.detailed).then(|| info_note(tr(Msg::ServerPublicReadiness)))}
                        {data_table(DataTable { caption: &caption, columns: &columns, sorting: Sorting::default(), body: Body::Rows(rows) })}
                    },
                )
            }
            Err(error) => plain_section(refused(tr(Msg::ServerHealthRefused), &error, None)),
        }
    }));
    view! { {page} }
}

/// Server: Metrics.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn MetricsTab() -> impl IntoView {
    let picked = crate::request::query("metric").filter(|metric| !metric.is_empty());
    let page = awaited(server_page(ServerTab::Metrics, move || async move {
        let headline = match fetch_headline_metrics().await {
            Ok(Some(tiles)) => section(
                Some(tr(Msg::ServerNow)),
                None,
                None,
                stat_grid(
                    tiles
                        .into_iter()
                        .map(|tile| {
                            stat_card(Stat {
                                glyph: Glyph::ACTIVITY,
                                value: tile.value,
                                label: tile.label.into(),
                                href: None,
                            })
                        })
                        .collect_view(),
                ),
            ),
            Ok(None) => ().into_any(),
            Err(error) => plain_section(refused(tr(Msg::ServerMetricsRefused), &error, None)),
        };
        let names = match fetch_metric_names().await {
            Ok(Some(names)) => names,
            Ok(None) => Vec::new(),
            Err(error) => return view! { {headline} {plain_section(refused(tr(Msg::ServerMetricsRefused), &error, None))} }.into_any(),
        };
        let options = names
            .iter()
            .map(|name| view! { <option value=name.clone() selected=picked.as_deref() == Some(name.as_str())>{name.clone()}</option> })
            .collect_view();
        let pick = view! {
            <form method="get" action="/server/metrics" class="fui-transform-options">
                {Field::new("m-pick", tr(Msg::ServerMetric))
                    .holds_id()
                    .view(view! { <select id="m-pick" name="metric" class="fui-select">{options}</select> })}
                <div class="fui-transform-run">
                    <button type="submit" class="fui-button fui-button-secondary">
                        {ferrohealth_ui::icon::icon(Glyph::CHART_LINE, "16")}
                        {tr(Msg::ServerShow)}
                    </button>
                </div>
            </form>
        };
        let samples = match &picked {
            Some(metric) => match fetch_metric_detail(metric.clone()).await {
                Ok(Some(detail)) => {
                    let columns = [
                        Column::text("labels", tr(Msg::ServerLabels)).role(Role::Title),
                        Column::text("value", tr(Msg::TagsValue))
                            .numeric()
                            .role(Role::Sub),
                    ];
                    let rows = detail
                        .samples
                        .iter()
                        .map(|sample| {
                            view! {
                                <tr>
                                    <th scope="row" data-role="title"><code>{sample.labels.clone()}</code></th>
                                    {cell(&columns[1], format_metric(sample.value))}
                                </tr>
                            }
                            .into_any()
                        })
                        .collect_view()
                        .into_any();
                    let caption = tr_with(
                        Msg::ServerSamplesOf,
                        &[("metric", detail.name.clone().into())],
                    );
                    view! {
                        <p class="fui-small fui-muted">{detail.help.clone()}</p>
                        {data_table(DataTable { caption: &caption, columns: &columns, sorting: Sorting::default(), body: Body::Rows(rows) })}
                    }
                    .into_any()
                }
                Ok(None) => info_note(tr(Msg::ServerNoMetric)),
                Err(error) => refused(tr(Msg::ServerMetricsRefused), &error, None),
            },
            None => ().into_any(),
        };
        view! {
            {headline}
            {section(
                Some(tr(Msg::ServerRegistry)),
                None,
                None,
                view! {
                    <p class="fui-small fui-muted">{tr_with(Msg::ServerMetricCount, &[("count", count(names.len()))])}</p>
                    {pick}
                    {samples}
                },
            )}
        }
        .into_any()
    }));
    view! { {page} }
}

/// Server: Logging.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn LoggingTab() -> impl IntoView {
    let posted = crate::posted::posted();
    let page = awaited(server_page(ServerTab::Logging, move || async move {
        let mut top = None;
        match posted
            .as_ref()
            .map(crate::posted::Posted::intent)
            .as_deref()
        {
            Some("apply-filter") => {
                let filter = posted
                    .as_ref()
                    .map(|posted| posted.text("filter"))
                    .unwrap_or_default();
                match set_log_filter(filter.clone()).await {
                    Ok(_) => {
                        redirect_with(
                            "/server/logging",
                            &Done::ok(tr(Msg::ServerFilterApplied)).line(filter),
                        );
                        return ().into_any();
                    }
                    Err(error) => top = Some(refused(tr(Msg::ServerFilterRefused), &error, None)),
                }
            }
            Some("reset-filter") => match reset_log_filter().await {
                Ok(_) => {
                    redirect_with("/server/logging", &Done::ok(tr(Msg::ServerFilterReset)));
                    return ().into_any();
                }
                Err(error) => top = Some(refused(tr(Msg::ServerFilterRefused), &error, None)),
            },
            _ => {}
        }
        let loggers = match fetch_loggers().await {
            Ok(Some(loggers)) => loggers,
            Ok(None) => return info_note(tr(Msg::ServerNoLogging)),
            Err(error) => return refused(tr(Msg::ServerLoggingRefused), &error, None),
        };
        let typed = posted
            .as_ref()
            .map(|posted| posted.text("filter"))
            .filter(|filter| !filter.is_empty())
            .unwrap_or_else(|| loggers.filter.clone());
        let at_boot = loggers.filter == loggers.boot_filter;
        let facts = definitions(vec![
            (
                tr(Msg::ServerInEffect),
                view! { <code>{loggers.filter.clone()}</code> }.into_any(),
            ),
            (
                tr(Msg::ServerAtBoot),
                view! { <code>{loggers.boot_filter.clone()}</code> }.into_any(),
            ),
        ]);
        let form = view! {
            <form id=LOG_FORM method="post" action="/server/logging" class="fui-transform-options">
                {Field::new("lf", tr(Msg::ServerNewFilter))
                    .holds_id()
                    .view(view! { <input id="lf" name="filter" type="text" class="fui-input fui-mono" value=typed.clone() /> })}
                <div class="fui-transform-run fui-cluster">
                    <button type="button" class="fui-button fui-button-primary" command="show-modal" commandfor=APPLY_DIALOG aria-haspopup="dialog">
                        {ferrohealth_ui::icon::icon(Glyph::CHECK, "16")}
                        {tr(Msg::ServerApply)}
                    </button>
                    <button
                        type="submit"
                        name="intent"
                        value="reset-filter"
                        class="fui-button fui-button-secondary"
                        disabled=at_boot
                        title=at_boot.then(|| tr(Msg::ServerAtBootAlready))
                    >
                        {ferrohealth_ui::icon::icon(Glyph::ROTATE_CCW, "16")}
                        {tr(Msg::ServerReset)}
                    </button>
                </div>
            </form>
        };
        let dialog = confirm_dialog(
            Confirmation {
                id: APPLY_DIALOG,
                title: tr(Msg::ServerApplyTitle).into(),
                message: tr(Msg::ServerApplyMessage).into(),
            },
            view! {
                <button type="submit" form=LOG_FORM name="intent" value="apply-filter" class="fui-button fui-button-primary">
                    {ferrohealth_ui::icon::icon(Glyph::CHECK, "16")}
                    {tr(Msg::ServerApplyConfirm)}
                </button>
            },
        );
        view! {
            {top}
            {section(Some(tr(Msg::ServerFilter)), None, None, facts)}
            {section(Some(tr(Msg::ServerChangeFilter)), None, None, view! {
                {ferrohealth_ui::view::surface::section_intro(tr(Msg::ServerChangeFilterIntro))}
                {form}
            })}
            {dialog}
        }
        .into_any()
    }));
    view! { {page} }
}

/// Server: API.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn ApiTab() -> impl IntoView {
    let family = crate::request::query("family")
        .filter(|family| crate::bff::system::openapi_families().any(|(key, _)| key == family))
        .unwrap_or_else(|| "ehr".to_owned());
    let page = awaited(server_page(ServerTab::Api, move || async move {
        let options = crate::bff::system::openapi_families()
            .map(|(key, words)| view! { <option value=key selected=key == family>{words}</option> })
            .collect_view();
        let pick = view! {
            <form method="get" action="/server/api" class="fui-transform-options">
                {Field::new("api-f", tr(Msg::ServerApiFamily))
                    .view(view! { <select id="api-f" name="family" class="fui-select">{options}</select> })}
                <div class="fui-transform-run">
                    <button type="submit" class="fui-button fui-button-secondary">
                        {ferrohealth_ui::icon::icon(Glyph::SEARCH, "16")}
                        {tr(Msg::ServerShow)}
                    </button>
                </div>
            </form>
        };
        let words = crate::bff::system::openapi_families()
            .find(|(key, _)| *key == family)
            .map_or_else(String::new, |(_, words)| words);
        let listed = match crate::bff::system::fetch_openapi(family.clone()).await {
            Ok(body) => match serde_json::from_str::<Value>(&body) {
                Ok(document) => {
                    let groups = crate::bff::system::group_openapi_paths(&document);
                    let total: usize = groups.iter().map(|(_, rows)| rows.len()).sum();
                    let columns = [
                        Column::text("path", tr(Msg::ServerMethodPath)).role(Role::Title),
                        Column::text("summary", tr(Msg::ServerSummary)).role(Role::Sub),
                    ];
                    let mut rows = Vec::new();
                    for (tag, items) in groups {
                        rows.push(group_row(
                            &columns,
                            view! {
                                {tag}
                                <span class="fui-small fui-muted">{format!(" · {}", tr_with(Msg::ServerEndpointCount, &[("count", count(items.len()))]))}</span>
                            },
                        ));
                        for (method, path, summary) in items {
                            rows.push(
                                view! {
                                    <tr>
                                        <th scope="row" data-role="title">
                                            <code class="fui-method">{method}</code>
                                            " "
                                            <code class="fui-path">{path}</code>
                                        </th>
                                        {cell(&columns[1], summary)}
                                    </tr>
                                }
                                .into_any(),
                            );
                        }
                    }
                    let caption =
                        tr_with(Msg::ServerEndpointsOf, &[("family", words.clone().into())]);
                    view! {
                        {section(
                            Some(caption.clone()),
                            u64::try_from(total).ok(),
                            None,
                            data_table(DataTable { caption: &caption, columns: &columns, sorting: Sorting::default(), body: Body::Rows(rows.collect_view().into_any()) }),
                        )}
                        {document_section(tr(Msg::ServerOpenApi), Language::Json, crate::bff::pretty_body(&body, crate::format::ReprFormat::CanonicalJson), "openapi")}
                    }
                    .into_any()
                }
                Err(error) => plain_section(refused(
                    tr(Msg::ServerApiRefused),
                    &crate::error::ViewerError::Internal(error.to_string()),
                    None,
                )),
            },
            Err(error) => plain_section(refused(tr(Msg::ServerApiRefused), &error, None)),
        };
        view! {
            {plain_section(pick)}
            {listed}
        }
        .into_any()
    }));
    view! { {page} }
}

/// Server: SMART.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn SmartTab() -> impl IntoView {
    let page = awaited(server_page(ServerTab::Smart, || async {
        match crate::bff::system::fetch_smart_config().await {
            Ok(Some(body)) => {
                let document: Value = match serde_json::from_str(&body) {
                    Ok(document) => document,
                    Err(error) => {
                        return plain_section(refused(
                            tr(Msg::ServerSmartRefused),
                            &crate::error::ViewerError::Internal(error.to_string()),
                            None,
                        ));
                    }
                };
                let list = |key: &str| -> Vec<String> {
                    document
                        .get(key)
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let scopes = list("scopes_supported");
                let (resources, others): (Vec<String>, Vec<String>) = scopes
                    .iter()
                    .cloned()
                    .partition(|scope| scope.contains('/'));
                let grants = crate::pages::access::grants_of(&resources.join(" "));
                let facts = definitions(vec![
                    (
                        tr(Msg::ServerAlso),
                        others
                            .iter()
                            .map(|scope| view! { <code>{scope.clone()}</code> " " })
                            .collect_view()
                            .into_any(),
                    ),
                    (
                        tr(Msg::ServerGrantTypes),
                        list("grant_types_supported").join(", ").into_any(),
                    ),
                    (
                        tr(Msg::ServerPkce),
                        list("code_challenge_methods_supported")
                            .join(", ")
                            .into_any(),
                    ),
                ]);
                view! {
                    {section(
                        Some(tr(Msg::ServerScopes)),
                        u64::try_from(resources.len()).ok(),
                        None,
                        scope_grants(tr(Msg::ServerScopes), grants),
                    )}
                    {section(Some(tr(Msg::ServerSignIn)), None, None, facts)}
                    {document_section(
                        "/.well-known/smart-configuration".to_owned(),
                        Language::Json,
                        crate::bff::pretty_body(&body, crate::format::ReprFormat::CanonicalJson),
                        "smart-configuration",
                    )}
                }
                .into_any()
            }
            Ok(None) => plain_section(info_note(tr(Msg::ServerNoSmart))),
            Err(error) => plain_section(refused(tr(Msg::ServerSmartRefused), &error, None)),
        }
    }));
    view! { {page} }
}
