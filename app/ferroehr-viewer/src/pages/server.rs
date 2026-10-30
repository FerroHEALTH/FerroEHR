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

use ferrohealth_health::document::State;
use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
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

use crate::bff::system::Endpoint;
use crate::routes::Drawn;
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{attr, flag, h, opt, t};
use crate::management::{
    BuildInfoView, LoggerView, MetricDetailView, MetricTile, ReadinessView, format_metric,
};
use crate::management::{
    fetch_build_info, fetch_headline_metrics, fetch_loggers, fetch_metric_detail,
    fetch_metric_names, fetch_readiness, probe_management_api, reset_log_filter, set_log_filter,
};
use crate::pages::common::{
    Refused, count, document_section, drawn, info_note, panel, plain_section, section, state_mark,
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
            Self::Overview => (ConsoleMsg::ServerOverview, ConsoleMsg::ServerOverviewLine),
            Self::Health => (ConsoleMsg::ServerHealth, ConsoleMsg::ServerHealthLine),
            Self::Metrics => (ConsoleMsg::ServerMetrics, ConsoleMsg::ServerMetricsLine),
            Self::Logging => (ConsoleMsg::ServerLogging, ConsoleMsg::ServerLoggingLine),
            Self::Api => (ConsoleMsg::ServerApi, ConsoleMsg::ServerApiLine),
            Self::Smart => (ConsoleMsg::ServerSmart, ConsoleMsg::ServerSmartLine),
        };
        (tr(label), tr(line))
    }

    /// Whether the tab reads the management surface.
    const fn managed(self) -> bool {
        matches!(self, Self::Health | Self::Metrics | Self::Logging)
    }
}

/// A tab's model: whether the CDR serves the management surface, and the
/// tab's own model, absent on a management tab the CDR does not serve.
#[derive(Debug, Clone, PartialEq)]
struct Tabbed<T> {
    /// Whether the management surface answers.
    managed: bool,
    /// The tab's own model.
    body: Option<T>,
}

/// Loads a tab of Server: probes the management surface, then loads `body`
/// unless the tab reads the surface and the CDR does not serve it.
async fn load_tab<T, F, Fut>(tab: ServerTab, body: F) -> Tabbed<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let managed = crate::management::renders_management_ops(&probe_management_api().await);
    let body = if tab.managed() && !managed {
        None
    } else {
        Some(body().await)
    };
    Tabbed { managed, body }
}

/// A tab of Server over the view `draw` makes of its model; a management tab
/// on a CDR without the management surface says so.
fn server_page<T>(tab: ServerTab, tabbed: Tabbed<T>, draw: impl FnOnce(T) -> Html) -> Html {
    let Tabbed { managed, body } = tabbed;
    let tabs = ServerTab::ALL
        .into_iter()
        .filter(|each| managed || !each.managed())
        .map(|each| Tab::new(each.key(), each.href(), each.words().0))
        .collect();
    let content =
        body.map_or_else(|| info_note(t(tr(ConsoleMsg::ServerNoManagement))), draw);
    let heading = page_heading(
        &current(),
        Header {
            title: tr(Msg::NavServer).into(),
            subtitle: Some(tr(ConsoleMsg::ServerSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let intro = if tab == ServerTab::Smart {
        smart_intro()
    } else {
        ferrohealth_ui::view::surface::section_intro(tab.words().1)
    };
    let binder = binder(
        &current(),
        tr(ConsoleMsg::ServerSections),
        tabs,
        tab.key(),
        panel(plain_section(intro).then(content)),
    );
    heading.then(h(format!(r#"<div class="fui-stack">{binder}</div>"#)))
}

/// The line under the SMART tab, its "Your access" a link to the page that
/// shows the scopes the session holds.
fn smart_intro() -> Html {
    const MARK: &str = "\u{1}";
    let line = tr_with(ConsoleMsg::ServerSmartLine, &[("access", MARK.into())]);
    let (before, after) = line.split_once(MARK).unwrap_or((line.as_str(), ""));
    h(format!(
        r#"<p class="fui-small fui-muted fui-section-intro">{}<a href="/access">{}</a>{}</p>"#,
        t(before),
        t(tr(Msg::YourAccess)),
        t(after)
    ))
}

/// How the CDR's status document says it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Running {
    /// `UP`.
    Up,
    /// Any other status word, or none.
    Degraded(Option<String>),
    /// A status that is no status document.
    Unknown,
    /// No status read.
    Unreachable,
}

/// Overview's model.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Overview {
    /// How the CDR runs.
    running: Running,
    /// Whether it runs the sandbox deployment profile.
    sandbox: bool,
    /// Its version, empty when it names none.
    version: String,
    /// The safeguards a sandbox runs without, as sentences.
    gaps: Vec<String>,
    /// The management surface's build information, or its refusal.
    build: Result<Option<BuildInfoView>, Refused>,
}

/// Loads Overview: the status the shell read and the build information.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
async fn load_overview(
) -> Result<Tabbed<Overview>, ViewerError> {
    crate::session::require_session()?;
    Ok(read_overview().await)
}

/// What [`load_overview`] reads, on the server.
async fn read_overview() -> Tabbed<Overview> {
    load_tab(ServerTab::Overview, || async {
        let status = crate::shell::status().await;
        let document = status
            .as_ref()
            .ok()
            .and_then(|body| crate::deployment::StatusDocument::parse(body));
        let running = match (&status, &document) {
            (Ok(_), Some(document)) if document.status.as_deref() == Some("UP") => Running::Up,
            (Ok(_), Some(document)) => Running::Degraded(document.status.clone()),
            (Ok(_), None) => Running::Unknown,
            (Err(_), _) => Running::Unreachable,
        };
        let deployment = document
            .as_ref()
            .and_then(|document| document.deployment.clone());
        let sandbox = deployment
            .as_ref()
            .is_some_and(|deployment| deployment.profile == "sandbox");
        let gaps = deployment
            .filter(|_| sandbox)
            .map(|deployment| {
                deployment
                    .gaps
                    .iter()
                    .map(String::as_str)
                    .map(crate::deployment::gap_text)
                    .collect()
            })
            .unwrap_or_default();
        Overview {
            running,
            sandbox,
            version: document
                .and_then(|document| document.server_version)
                .unwrap_or_default(),
            gaps,
            build: fetch_build_info()
                .await
                .map_err(|error| Refused::new(tr(ConsoleMsg::ServerBuildRefused), error)),
        }
    })
    .await
}

/// Overview, drawn.
fn overview(model: Overview) -> Html {
    let Overview {
        running,
        sandbox,
        version,
        gaps,
        build,
    } = model;
    let state = match running {
        Running::Up => state_mark(FlowState::Up, &tr(ConsoleMsg::ServerUp)),
        Running::Degraded(word) => state_mark(
            FlowState::Degraded,
            &word.unwrap_or_else(|| tr(ConsoleMsg::ServerUnknown)),
        ),
        Running::Unknown => state_mark(FlowState::NotChecked, &tr(ConsoleMsg::ServerUnknown)),
        Running::Unreachable => {
            state_mark(FlowState::Unreachable, &tr(ConsoleMsg::ServerUnreachable))
        }
    };
    let facts = definitions(vec![
        (tr(ConsoleMsg::ServerState), state),
        (
            tr(ConsoleMsg::ServerDeployment),
            t(tr(if sandbox {
                ConsoleMsg::ServerSandbox
            } else {
                ConsoleMsg::ServerProduction
            })),
        ),
        (tr(ConsoleMsg::ServerVersion), t(version)),
        (
            tr(ConsoleMsg::ServerConformance),
            h(format!(r#"<a href="/about">{}</a>"#, t(tr(Msg::NavAbout)))),
        ),
    ]);
    let gaps = (!gaps.is_empty()).then(|| {
        let columns = [Column::text("gap", tr(ConsoleMsg::ServerSafeguard)).role(Role::Title)];
        let total = u64::try_from(gaps.len()).ok();
        let rows = gaps
            .iter()
            .map(|gap| {
                h(format!(
                    r#"<tr><th scope="row" data-role="title">{}</th></tr>"#,
                    state_mark(FlowState::Degraded, gap)
                ))
            })
            .collect();
        let caption = tr(ConsoleMsg::ServerSafeguardsOff);
        section(
            Some(tr(ConsoleMsg::ServerSafeguardsOff)),
            total,
            None,
            ferrohealth_ui::view::surface::section_intro(tr(ConsoleMsg::ServerSandboxIntro)).then(
                data_table(DataTable {
                    caption: &caption,
                    columns: &columns,
                    sorting: Sorting::default(),
                    body: Body::Rows(rows),
                }),
            ),
        )
    });
    let build = match build {
        Ok(Some(info)) => {
            let mut parts = vec![section(
                Some(tr(ConsoleMsg::ServerBuild)),
                None,
                None,
                definitions(
                    info.facts
                        .iter()
                        .map(|(key, value)| (key.clone(), t(value)))
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
                            .map(|(key, value)| (key.clone(), t(value)))
                            .collect(),
                    ),
                ));
            }
            Html::join(parts)
        }
        Ok(None) => plain_section(info_note(t(tr(ConsoleMsg::ServerNoBuildInfo)))),
        Err(refusal) => plain_section(refusal.view()),
    };
    Html::join([
        section(Some(tr(ConsoleMsg::ServerStatus)), None, None, facts),
        opt(gaps),
        build,
    ])
}

/// Server: Overview.
/// Server › Overview.
pub async fn overview_page() -> Drawn {
    let page = drawn(load_overview().await, |tabbed| {
        server_page(ServerTab::Overview, tabbed, overview)
    });
    Drawn::page(tr(Msg::NavServer), page)
}

/// The word the health tab shows for an indicator's `state`.
const fn state_word(state: State) -> &'static str {
    match state {
        State::Up => "UP",
        State::Degraded => "DEGRADED",
        State::Down => "DOWN",
    }
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

/// Loads Health: the readiness document.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
async fn load_health(
) -> Result<Tabbed<Result<ReadinessView, Refused>>, ViewerError> {
    crate::session::require_session()?;
    Ok(read_health().await)
}

/// What [`load_health`] reads, on the server.
async fn read_health() -> Tabbed<Result<ReadinessView, Refused>> {
    load_tab(ServerTab::Health, || async {
        fetch_readiness()
            .await
            .map_err(|error| Refused::new(tr(ConsoleMsg::ServerHealthRefused), error))
    })
    .await
}

/// Server: Health.
/// Server › Health.
pub async fn health_page() -> Drawn {
    let page = drawn(load_health().await, |tabbed| {
        server_page(ServerTab::Health, tabbed, health)
    });
    Drawn::page(tr(Msg::NavServer), page)
}

/// Health, drawn from the readiness document or its refusal.
fn health(read: Result<ReadinessView, Refused>) -> Html {
    match read {
        Ok(readiness) => {
            let columns = [
                Column::text("dependency", tr(ConsoleMsg::ServerDependency)).role(Role::Title),
                Column::text("state", tr(ConsoleMsg::ServerState)).role(Role::Sub),
                Column::text("detail", tr(ConsoleMsg::ServerDetail)).role(Role::Meta),
            ];
            let rows = readiness
                .readiness
                .indicators
                .iter()
                .map(|row| {
                    let word = state_word(row.state);
                    h(format!(
                        r#"<tr><th scope="row" data-role="title">{}</th>{}{}</tr>"#,
                        t(&row.name),
                        cell(&columns[1], state_mark(flow_of(word), word)),
                        cell(&columns[2], t(row.detail.clone().unwrap_or_default()))
                    ))
                })
                .collect();
            let caption = tr(ConsoleMsg::ServerDependencies);
            section(
                Some(tr(ConsoleMsg::ServerDependencies)),
                u64::try_from(readiness.readiness.indicators.len()).ok(),
                None,
                opt((!readiness.detailed)
                    .then(|| info_note(t(tr(ConsoleMsg::ServerPublicReadiness)))))
                .then(data_table(DataTable {
                    caption: &caption,
                    columns: &columns,
                    sorting: Sorting::default(),
                    body: Body::Rows(rows),
                })),
            )
        }
        Err(refusal) => plain_section(refusal.view()),
    }
}

/// Metrics' model.
#[derive(Debug, Clone, PartialEq)]
struct Metrics {
    /// The headline tiles, or their refusal.
    headline: Result<Option<Vec<MetricTile>>, Refused>,
    /// The registered metric names, or their refusal.
    names: Result<Vec<String>, Refused>,
    /// The picked metric's samples, when one is picked and the names read.
    samples: Option<Result<Option<MetricDetailView>, Refused>>,
}

/// Loads Metrics: the headline, the names and the picked metric.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
async fn load_metrics(
) -> Result<Tabbed<Metrics>, ViewerError> {
    crate::session::require_session()?;
    Ok(read_metrics().await)
}

/// What [`load_metrics`] reads, on the server.
async fn read_metrics() -> Tabbed<Metrics> {
    load_tab(ServerTab::Metrics, || async {
        let refused = |error| Refused::new(tr(ConsoleMsg::ServerMetricsRefused), error);
        let headline = fetch_headline_metrics().await.map_err(refused);
        let names = fetch_metric_names()
            .await
            .map(Option::unwrap_or_default)
            .map_err(refused);
        let samples = match (&names, picked()) {
            (Ok(_), Some(metric)) => Some(fetch_metric_detail(metric).await.map_err(refused)),
            _ => None,
        };
        Metrics {
            headline,
            names,
            samples,
        }
    })
    .await
}

/// The metric the address picks.
fn picked() -> Option<String> {
    crate::request::query("metric").filter(|metric| !metric.is_empty())
}

/// Metrics, drawn.
#[expect(
    clippy::too_many_lines,
    reason = "one page part: the headline, the picker and the samples it draws"
)]
fn metrics(model: Metrics) -> Html {
    let Metrics {
        headline,
        names,
        samples,
    } = model;
    let headline = match headline {
        Ok(Some(tiles)) => section(
            Some(tr(ConsoleMsg::ServerNow)),
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
                    .collect(),
            ),
        ),
        Ok(None) => Html::empty(),
        Err(refusal) => plain_section(refusal.view()),
    };
    let names = match names {
        Ok(names) => names,
        Err(refusal) => {
            return headline.then(plain_section(refusal.view()));
        }
    };
    let picked = picked();
    let options: Html = names
        .iter()
        .map(|name| {
            h(format!(
                r#"<option value="{}"{}>{}</option>"#,
                t(name),
                flag("selected", picked.as_deref() == Some(name.as_str())),
                t(name)
            ))
        })
        .collect();
    let pick = h(format!(
        r#"<form method="get" action="/server/metrics" class="fui-transform-options">{}<div class="fui-transform-run"><button type="submit" class="fui-button fui-button-secondary">{}{}</button></div></form>"#,
        Field::new("m-pick", tr(ConsoleMsg::ServerMetric))
            .holds_id()
            .view(h(format!(
                r#"<select id="m-pick" name="metric" class="fui-select">{options}</select>"#
            ))),
        ferrohealth_ui::icon::icon(Glyph::CHART_LINE, "16"),
        t(tr(ConsoleMsg::ServerShow))
    ));
    let samples = match samples {
        Some(Ok(Some(detail))) => {
            let columns = [
                Column::text("labels", tr(ConsoleMsg::ServerLabels)).role(Role::Title),
                Column::text("value", tr(Msg::TagsValue))
                    .numeric()
                    .role(Role::Sub),
            ];
            let rows = detail
                .samples
                .iter()
                .map(|sample| {
                    h(format!(
                        r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}</tr>"#,
                        t(&sample.labels),
                        cell(&columns[1], t(format_metric(sample.value)))
                    ))
                })
                .collect();
            let caption = tr_with(
                ConsoleMsg::ServerSamplesOf,
                &[("metric", detail.name.clone().into())],
            );
            h(format!(
                r#"<p class="fui-small fui-muted">{}</p>"#,
                t(&detail.help)
            ))
            .then(data_table(DataTable {
                caption: &caption,
                columns: &columns,
                sorting: Sorting::default(),
                body: Body::Rows(rows),
            }))
        }
        Some(Ok(None)) => info_note(t(tr(ConsoleMsg::ServerNoMetric))),
        Some(Err(refusal)) => refusal.view(),
        None => Html::empty(),
    };
    headline.then(section(
        Some(tr(ConsoleMsg::ServerRegistry)),
        None,
        None,
        Html::join([
            h(format!(
                r#"<p class="fui-small fui-muted">{}</p>"#,
                t(tr_with(
                    ConsoleMsg::ServerMetricCount,
                    &[("count", count(names.len()))]
                ))
            )),
            pick,
            samples,
        ]),
    ))
}

/// Server: Metrics.
/// Server › Metrics.
pub async fn metrics_page() -> Drawn {
    let page = drawn(load_metrics().await, |tabbed| {
        server_page(ServerTab::Metrics, tabbed, metrics)
    });
    Drawn::page(tr(Msg::NavServer), page)
}

/// Logging's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Logging {
    /// The filters, and the refusal of a change the post asked for.
    Loggers {
        /// The refused change.
        top: Option<Refused>,
        /// The filters, `None` when the CDR serves none, or their refusal.
        loggers: Result<Option<LoggerView>, Refused>,
        /// The filter the form holds.
        typed: String,
    },
    /// The post redirected.
    Left,
}

/// Loads Logging: answers its post (Apply, Reset) and reads the filters.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
async fn load_logging(
) -> Result<Tabbed<Logging>, ViewerError> {
    crate::session::require_session()?;
    Ok(read_logging().await)
}

/// What [`load_logging`] reads, on the server.
async fn read_logging() -> Tabbed<Logging> {
    load_tab(ServerTab::Logging, || async {
        let posted = crate::posted::posted();
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
                            &Done::ok(tr(ConsoleMsg::ServerFilterApplied)).line(filter),
                        );
                        return Logging::Left;
                    }
                    Err(error) => {
                        top = Some(Refused::new(tr(ConsoleMsg::ServerFilterRefused), error));
                    }
                }
            }
            Some("reset-filter") => match reset_log_filter().await {
                Ok(_) => {
                    redirect_with(
                        "/server/logging",
                        &Done::ok(tr(ConsoleMsg::ServerFilterReset)),
                    );
                    return Logging::Left;
                }
                Err(error) => top = Some(Refused::new(tr(ConsoleMsg::ServerFilterRefused), error)),
            },
            _ => {}
        }
        let loggers = fetch_loggers()
            .await
            .map_err(|error| Refused::new(tr(ConsoleMsg::ServerLoggingRefused), error));
        let typed = posted
            .as_ref()
            .map(|posted| posted.text("filter"))
            .filter(|filter| !filter.is_empty())
            .or_else(|| {
                loggers
                    .as_ref()
                    .ok()
                    .and_then(Option::as_ref)
                    .map(|loggers| loggers.filter.clone())
            })
            .unwrap_or_default();
        Logging::Loggers {
            top,
            loggers,
            typed,
        }
    })
    .await
}

/// The confirmation of Apply, which posts the log form.
fn apply_dialog() -> Html {
    confirm_dialog(
        &current(),
        Confirmation {
            id: APPLY_DIALOG.into(),
            title: tr(ConsoleMsg::ServerApplyTitle).into(),
            message: tr(ConsoleMsg::ServerApplyMessage).into(),
        },
        h(format!(
            r#"<button type="submit" form="{LOG_FORM}" name="intent" value="apply-filter" class="fui-button fui-button-primary">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::CHECK, "16"),
            t(tr(ConsoleMsg::ServerApplyConfirm))
        )),
    )
}

/// Logging, drawn.
fn logging(model: Logging) -> Html {
    let (top, loggers, typed) = match model {
        Logging::Loggers {
            top,
            loggers,
            typed,
        } => (top, loggers, typed),
        Logging::Left => return Html::empty(),
    };
    let loggers = match loggers {
        Ok(Some(loggers)) => loggers,
        Ok(None) => return info_note(t(tr(ConsoleMsg::ServerNoLogging))),
        Err(refusal) => return refusal.view(),
    };
    let at_boot = loggers.filter == loggers.boot_filter;
    let facts = definitions(vec![
        (
            tr(ConsoleMsg::ServerInEffect),
            h(format!("<code>{}</code>", t(&loggers.filter))),
        ),
        (
            tr(ConsoleMsg::ServerAtBoot),
            h(format!("<code>{}</code>", t(&loggers.boot_filter))),
        ),
    ]);
    let form = h(format!(
        r#"<form id="{LOG_FORM}" method="post" action="/server/logging" class="fui-transform-options">{field}<div class="fui-transform-run fui-cluster"><button type="button" class="fui-button fui-button-primary" command="show-modal" commandfor="{APPLY_DIALOG}" aria-haspopup="dialog">{check}{apply}</button><button type="submit" name="intent" value="reset-filter" class="fui-button fui-button-secondary"{disabled}{title}>{rotate}{reset}</button></div></form>"#,
        field = Field::new("lf", tr(ConsoleMsg::ServerNewFilter))
            .holds_id()
            .view(h(format!(
                r#"<input id="lf" name="filter" type="text" class="fui-input fui-mono" value="{}">"#,
                t(&typed)
            ))),
        check = ferrohealth_ui::icon::icon(Glyph::CHECK, "16"),
        apply = t(tr(ConsoleMsg::ServerApply)),
        disabled = flag("disabled", at_boot),
        title = attr("title", at_boot.then(|| tr(ConsoleMsg::ServerAtBootAlready))),
        rotate = ferrohealth_ui::icon::icon(Glyph::ROTATE_CCW, "16"),
        reset = t(tr(ConsoleMsg::ServerReset)),
    ));
    let dialog = apply_dialog();
    let top = top.map(|refusal| refusal.view());
    Html::join([
        opt(top),
        section(Some(tr(ConsoleMsg::ServerFilter)), None, None, facts),
        section(
            Some(tr(ConsoleMsg::ServerChangeFilter)),
            None,
            None,
            ferrohealth_ui::view::surface::section_intro(tr(ConsoleMsg::ServerChangeFilterIntro))
                .then(form),
        ),
        dialog,
    ])
}

/// Server: Logging.
/// Server › Logging.
pub async fn logging_page() -> Drawn {
    let page = drawn(load_logging().await, |tabbed| {
        server_page(ServerTab::Logging, tabbed, logging)
    });
    Drawn::page(tr(Msg::NavServer), page)
}

/// One OpenAPI document as API lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Document {
    /// Its operations, grouped by tag.
    groups: Vec<(String, Vec<Endpoint>)>,
    /// The document pretty-printed.
    text: String,
}

/// API's model.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Api {
    /// The family shown.
    family: String,
    /// Every document the picker offers, as its slug and words.
    families: Vec<(String, String)>,
    /// The family's document, or its refusal.
    listed: Result<Document, Refused>,
}

/// Loads API: the document of the family the address picks.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
async fn load_api(
) -> Result<Tabbed<Api>, ViewerError> {
    crate::session::require_session()?;
    Ok(read_api().await)
}

/// What [`load_api`] reads, on the server.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's OpenAPI JSON document to list it"
)]
async fn read_api() -> Tabbed<Api> {
    load_tab(ServerTab::Api, || async {
        let family = crate::request::query("family")
            .filter(|family| crate::bff::system::openapi_families().any(|(key, _)| key == family))
            .unwrap_or_else(|| "ehr".to_owned());
        let families = crate::bff::system::openapi_families()
            .map(|(key, words)| (key.to_owned(), words))
            .collect();
        let refused = |error| Refused::new(tr(ConsoleMsg::ServerApiRefused), error);
        let listed = match crate::bff::system::fetch_openapi(family.clone()).await {
            Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(document) => Ok(Document {
                    groups: crate::bff::system::group_openapi_paths(&document),
                    text: crate::bff::pretty_body(&body, crate::format::ReprFormat::CanonicalJson),
                }),
                Err(error) => Err(refused(ViewerError::Internal(error.to_string()))),
            },
            Err(error) => Err(refused(error)),
        };
        Api {
            family,
            families,
            listed,
        }
    })
    .await
}

/// The form that picks the API family `family` shows of `families`.
fn family_pick(family: &str, families: &[(String, String)]) -> Html {
    let options: Html = families
        .iter()
        .map(|(key, words)| {
            h(format!(
                r#"<option value="{}"{}>{}</option>"#,
                t(key),
                flag("selected", *key == family),
                t(words)
            ))
        })
        .collect();
    h(format!(
        r#"<form method="get" action="/server/api" class="fui-transform-options">{}<div class="fui-transform-run"><button type="submit" class="fui-button fui-button-secondary">{}{}</button></div></form>"#,
        Field::new("api-f", tr(ConsoleMsg::ServerApiFamily)).view(h(format!(
            r#"<select id="api-f" name="family" class="fui-select">{options}</select>"#
        ))),
        ferrohealth_ui::icon::icon(Glyph::SEARCH, "16"),
        t(tr(ConsoleMsg::ServerShow))
    ))
}

/// API, drawn.
fn api(model: Api) -> Html {
    let Api {
        family,
        families,
        listed,
    } = model;
    let pick = family_pick(&family, &families);
    let words = families
        .into_iter()
        .find(|(key, _)| *key == family)
        .map_or_else(String::new, |(_, words)| words);
    let listed = match listed {
        Ok(Document { groups, text }) => {
            let total: usize = groups.iter().map(|(_, rows)| rows.len()).sum();
            let columns = [
                Column::text("path", tr(ConsoleMsg::ServerMethodPath)).role(Role::Title),
                Column::text("summary", tr(ConsoleMsg::ServerSummary)).role(Role::Sub),
            ];
            let mut rows = Vec::new();
            for (tag, items) in groups {
                rows.push(group_row(
                    &columns,
                    t(&tag).then(h(format!(
                        r#"<span class="fui-small fui-muted">{}</span>"#,
                        t(format!(
                            " · {}",
                            tr_with(
                                ConsoleMsg::ServerEndpointCount,
                                &[("count", count(items.len()))],
                            ),
                        ))
                    ))),
                ));
                for (method, path, summary) in items {
                    rows.push(h(format!(
                        r#"<tr><th scope="row" data-role="title"><code class="fui-method">{}</code> <code class="fui-path">{}</code></th>{}</tr>"#,
                        t(method),
                        t(path),
                        cell(&columns[1], t(summary))
                    )));
                }
            }
            let caption = tr_with(ConsoleMsg::ServerEndpointsOf, &[("family", words.into())]);
            section(
                Some(caption.clone()),
                u64::try_from(total).ok(),
                None,
                data_table(DataTable {
                    caption: &caption,
                    columns: &columns,
                    sorting: Sorting::default(),
                    body: Body::Rows(Html::join(rows)),
                }),
            )
            .then(document_section(
                tr(ConsoleMsg::ServerOpenApi),
                Language::Json,
                text,
                "openapi",
            ))
        }
        Err(refusal) => plain_section(refusal.view()),
    };
    plain_section(pick).then(listed)
}

/// Server: API.
/// Server › Api.
pub async fn api_page() -> Drawn {
    let page = drawn(load_api().await, |tabbed| {
        server_page(ServerTab::Api, tabbed, api)
    });
    Drawn::page(tr(Msg::NavServer), page)
}

/// The SMART configuration as the tab lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Smart {
    /// The resource scopes it supports.
    resources: Vec<String>,
    /// The other scopes it supports.
    others: Vec<String>,
    /// `grant_types_supported`.
    grant_types: Vec<String>,
    /// `code_challenge_methods_supported`.
    pkce: Vec<String>,
    /// The document pretty-printed.
    text: String,
}

/// Loads SMART: the CDR's discovery document, `None` when it serves none.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
async fn load_smart(
) -> Result<Tabbed<Result<Option<Smart>, Refused>>, ViewerError> {
    crate::session::require_session()?;
    Ok(read_smart().await)
}

/// What [`load_smart`] reads, on the server.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's SMART JSON document to list it"
)]
async fn read_smart() -> Tabbed<Result<Option<Smart>, Refused>> {
    use serde_json::Value;
    load_tab(ServerTab::Smart, || async {
        let refused = |error| Refused::new(tr(ConsoleMsg::ServerSmartRefused), error);
        let Some(body) = crate::bff::system::fetch_smart_config()
            .await
            .map_err(refused)?
        else {
            return Ok(None);
        };
        let document: Value = serde_json::from_str(&body)
            .map_err(|error| refused(ViewerError::Internal(error.to_string())))?;
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
        let (resources, others) = list("scopes_supported")
            .into_iter()
            .partition(|scope| scope.contains('/'));
        Ok(Some(Smart {
            resources,
            others,
            grant_types: list("grant_types_supported"),
            pkce: list("code_challenge_methods_supported"),
            text: crate::bff::pretty_body(&body, crate::format::ReprFormat::CanonicalJson),
        }))
    })
    .await
}

/// SMART, drawn.
fn smart(read: Result<Option<Smart>, Refused>) -> Html {
    match read {
        Ok(Some(Smart {
            resources,
            others,
            grant_types,
            pkce,
            text,
        })) => {
            let grants = crate::pages::access::grants_of(&resources.join(" "));
            let facts = definitions(vec![
                (
                    tr(ConsoleMsg::ServerAlso),
                    others
                        .iter()
                        .map(|scope| h(format!("<code>{}</code> ", t(scope))))
                        .collect(),
                ),
                (tr(ConsoleMsg::ServerGrantTypes), t(grant_types.join(", "))),
                (tr(ConsoleMsg::ServerPkce), t(pkce.join(", "))),
            ]);
            Html::join([
                section(
                    Some(tr(ConsoleMsg::ServerScopes)),
                    u64::try_from(resources.len()).ok(),
                    None,
                    scope_grants(&current(), &tr(ConsoleMsg::ServerScopes), grants),
                ),
                section(Some(tr(ConsoleMsg::ServerSignIn)), None, None, facts),
                document_section(
                    "/.well-known/smart-configuration".to_owned(),
                    Language::Json,
                    text,
                    "smart-configuration",
                ),
            ])
        }
        Ok(None) => plain_section(info_note(t(tr(ConsoleMsg::ServerNoSmart)))),
        Err(refusal) => plain_section(refusal.view()),
    }
}

/// Server: SMART.
/// Server › Smart.
pub async fn smart_page() -> Drawn {
    let page = drawn(load_smart().await, |tabbed| {
        server_page(ServerTab::Smart, tabbed, smart)
    });
    Drawn::page(tr(Msg::NavServer), page)
}
