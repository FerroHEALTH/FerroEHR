// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/operations` screen: the operator's panel over the CDR's operational
//! surfaces — dependency health, build + spec provenance, the metric registry,
//! and the live log-filter control.
//!
//! No openEHR spec governs the viewer, and none governs the surfaces it reads
//! here either — our own operational extension; the wire shapes live in
//! [`crate::management`], which carries the citations.
//!
//! **Two deliberate divisions of labour**, both to keep exactly one viewer
//! reader per claim:
//!
//! * *Health.* The application shell's topbar pill polls the product status
//!   document (`{rest root}/status`: is the API answering, at which version).
//!   The health card here reads the OTHER contract — the public
//!   `/health/readiness` indicators (database ping, migrations applied,
//!   component flags) — and the card says so on screen, so the two are never
//!   mistaken for the same claim.
//! * *Configuration.* `GET /management/env` and the Admin API's
//!   `GET /admin/config` serve the SAME snapshot (both are the binary's
//!   `FerroEhrConfig::to_redacted_json` value, the management route adding a
//!   second redaction pass), so a viewer here would be a duplicate reader of
//!   one claim. The single viewer stays on `/system` — it reads the API base
//!   URL the viewer is always configured for, whereas the management surface
//!   may sit on an internal listener the viewer cannot reach at all — and this
//!   screen cross-links to it.
//!
//! Each card resolves its `Result` INSIDE its own `<Suspense>` — an SSR'd
//! `ErrorBoundary` fallback mismatches at hydration in leptos 0.8 — so one
//! failing card never blanks the page, and a management endpoint the deployment
//! left off renders as a first-class absent state rather than an error.

use leptos::component;
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::components::A;
use leptos_router::hooks::use_query_map;

use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::data_table::{CELL, CELL_MONO, ROW, table_shell};
use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, INPUT, SELECT};
use crate::components::notice::inline_error;
use crate::components::page_header::PageHeader;
use crate::components::stat_card::StatCard;
use crate::components::surface::titled_card;
use crate::components::toast::toast_success;
use crate::error::ViewerError;
use crate::management::{
    BuildInfoView, LoggerView, MetricDetailView, MetricTile, ReadinessView, fetch_build_info,
    fetch_headline_metrics, fetch_loggers, fetch_metric_detail, fetch_metric_names,
    fetch_readiness, reset_log_filter, set_log_filter,
};

/// The log-filter apply action: the filter it was dispatched with, paired with
/// the CDR's answer, so both toasts can name the exact directives (the
/// mutation-feedback rule).
type ApplyFilterAction = Action<String, (String, Result<LoggerView, ViewerError>)>;

/// The log-filter reset action.
type ResetFilterAction = Action<(), Result<LoggerView, ViewerError>>;

/// The `/operations` screen.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn OperationsPage() -> impl IntoView {
    let toaster = thaw::ToasterInjection::expect_context();
    let query = use_query_map();

    let apply: ApplyFilterAction = Action::new(|filter: &String| {
        let filter = filter.clone();
        async move {
            let outcome = set_log_filter(filter.clone()).await;
            (filter, outcome)
        }
    });
    let reset: ResetFilterAction = Action::new(|(): &()| async move { reset_log_filter().await });

    // Both mutation outcomes toast, success and failure alike (the viewer's
    // one feedback rule): dispatching a toast is a side effect on the outside
    // world, so an Effect is its correct home and it never runs on the server
    // pass.
    Effect::new(move |_| match apply.value().get() {
        Some((filter, Ok(view))) => toast_success(
            toaster,
            "Log filter applied",
            &format!(
                "The CDR is now logging with `{}` (requested `{filter}`).",
                view.filter
            ),
        ),
        Some((filter, Err(error))) => crate::feedback::toast_write_failure(
            toaster,
            "Log filter not applied",
            &format!("the log filter `{filter}`"),
            &error,
        ),
        None => {}
    });
    Effect::new(move |_| match reset.value().get() {
        Some(Ok(view)) => toast_success(
            toaster,
            "Log filter reset",
            &format!("The CDR restored its boot filter `{}`.", view.filter),
        ),
        Some(Err(error)) => crate::feedback::toast_write_failure(
            toaster,
            "Log filter not reset",
            "the log filter",
            &error,
        ),
        None => {}
    });

    // The filter awaiting confirmation (`None` = the apply dialog is closed)
    // and the reset flag: one signal each, so trigger, Cancel, Esc and the
    // backdrop all write the same state (the ConfirmDialog contract).
    let pending_apply = RwSignal::new(Option::<String>::None);
    let pending_reset = RwSignal::new(false);

    let health = health_card();
    let build = build_info_card();
    let metrics = metrics_card(query);
    let config = config_card();
    let loggers = loggers_card(apply, reset, pending_apply, pending_reset);
    let dialogs = filter_dialogs(apply, reset, pending_apply, pending_reset);

    view! {
        <Title text="Operations" />
        <div class="p-6">
            <PageHeader
                title="Operations"
                subtitle="Dependency health, build provenance, the metric registry, and live log control."
            />
            <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 items-start">
                {health} {build} {metrics} {loggers} {config}
            </div>
            {dialogs}
        </div>
    }
}

/// The `<Suspense>` fallback shared by every data-backed card.
fn card_skeleton() -> impl IntoView {
    view! {
        <thaw::Skeleton>
            <thaw::SkeletonItem class="h-4 mb-2" />
            <thaw::SkeletonItem class="h-4 mb-2" />
            <thaw::SkeletonItem class="h-4" />
        </thaw::Skeleton>
    }
}

/// A status pill in the shared chip shape: `UP` reads neutral-good, `DEGRADED`
/// warns, anything else (`DOWN`, `UNKNOWN`) is the danger tone.
fn status_pill(status: &str) -> AnyView {
    let (dot, text_class) = match status {
        "UP" => ("bg-ok", "text-ink"),
        "DEGRADED" => ("bg-warn", "text-ink"),
        _ => ("bg-danger", "text-danger"),
    };
    let label = status.to_owned();
    view! {
        <span class=format!(
            "inline-flex items-center gap-1.5 rounded-full border border-edge bg-raised px-2.5 py-1 text-xs font-medium {text_class}",
        )>
            <span class=format!("h-1.5 w-1.5 rounded-full {dot}")></span>
            {label}
        </span>
    }
    .into_any()
}

// ── Health ──────────────────────────────────────────────────────────────────

/// The dependency-health card: the public `/health/readiness` aggregate plus
/// every indicator. A pure read, so a failure renders inline and never toasts.
fn health_card() -> AnyView {
    let resource = Resource::new(|| (), |()| async move { fetch_readiness().await });
    let body = view! {
        <Suspense fallback=card_skeleton>
            {move || {
                Suspend::new(async move {
                    match resource.await {
                        Ok(view) => readiness_body(view),
                        Err(e) => inline_error(&e),
                    }
                })
            }}
        </Suspense>
    }
    .into_any();
    titled_card("Dependency health", false, body)
}

/// Render a readiness aggregate: the overall pill, the one-line explanation of
/// how it differs from the topbar pill, and the per-indicator table.
fn readiness_body(view: ReadinessView) -> AnyView {
    let pill = status_pill(&view.status);
    let rows = view
        .components
        .into_iter()
        .map(|component| {
            let detail = if component.detail.is_empty() {
                "—".to_owned()
            } else {
                component.detail
            };
            let pill = status_pill(&component.status);
            view! {
                <tr class=ROW>
                    <td class=CELL_MONO>{component.name}</td>
                    <td class=CELL>{pill}</td>
                    <td class=CELL>{detail}</td>
                </tr>
            }
            .into_any()
        })
        .collect_view()
        .into_any();
    let table = table_shell(&["Indicator", "Status", "Detail"], rows);
    view! {
        <div id="ops-readiness">
            {pill}
            <p class="mt-2 mb-3 text-sm text-ink-muted">
                "The topbar pill reports the API itself (status document: reachable, and at which version). "
                "This is the other question — the CDR's readiness probe, one row per dependency it checks."
            </p> {table}
        </div>
    }
    .into_any()
}

// ── Build + spec provenance ─────────────────────────────────────────────────

/// The build-info card: `GET /management/info` (version, git commit, `rustc`,
/// and the pinned openEHR specification versions).
fn build_info_card() -> AnyView {
    let resource = Resource::new(|| (), |()| async move { fetch_build_info().await });
    let body = view! {
        <Suspense fallback=card_skeleton>
            {move || {
                Suspend::new(async move {
                    match resource.await {
                        Ok(Some(view)) => build_info_body(view),
                        Ok(None) => surface_absent("Build provenance"),
                        Err(e) => inline_error(&e),
                    }
                })
            }}
        </Suspense>
    }
    .into_any();
    titled_card("Build & spec provenance", false, body)
}

/// Render the build facts and, under their own heading, the pinned
/// specification versions.
fn build_info_body(view: BuildInfoView) -> AnyView {
    let definition_list = |rows: Vec<(String, String)>| {
        rows.into_iter()
            .map(|(key, value)| {
                view! {
                    <dt class="font-medium text-ink-muted">{key}</dt>
                    <dd class="font-mono break-all text-ink">{value}</dd>
                }
            })
            .collect_view()
            .into_any()
    };
    let facts = definition_list(view.facts);
    let spec = (!view.spec.is_empty()).then(|| {
        let rows = definition_list(view.spec);
        view! {
            <h3 class="mt-4 mb-1 text-xs font-semibold uppercase tracking-wide text-ink-muted">
                "openEHR specification pins"
            </h3>
            <dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">{rows}</dl>
        }
    });
    view! {
        <div id="ops-build-info">
            <dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">{facts}</dl>
            {spec}
        </div>
    }
    .into_any()
}

// ── Metrics ─────────────────────────────────────────────────────────────────

/// The metrics card: headline tiles plus the registry browser (pick a metric,
/// read its current samples). The selection lives in the URL (`?metric=…`), so
/// a view is shareable, refresh-safe, and works before WASM loads.
fn metrics_card(query: Memo<leptos_router::params::ParamsMap>) -> AnyView {
    let tiles = Resource::new(|| (), |()| async move { fetch_headline_metrics().await });
    let names = Resource::new(|| (), |()| async move { fetch_metric_names().await });
    let selected = move || query.read().get("metric").unwrap_or_default();
    let detail: Resource<Result<Option<MetricDetailView>, ViewerError>> =
        Resource::new(selected, |name| async move {
            if name.trim().is_empty() {
                return Ok(None);
            }
            fetch_metric_detail(name).await
        });

    let tiles_section = view! {
        <Suspense fallback=card_skeleton>
            {move || {
                Suspend::new(async move {
                    match tiles.await {
                        Ok(Some(tiles)) => tiles_body(tiles),
                        Ok(None) => surface_absent("Metrics"),
                        Err(e) => inline_error(&e),
                    }
                })
            }}
        </Suspense>
    }
    .into_any();

    let browser_section = view! {
        <Suspense fallback=|| ()>
            {move || {
                Suspend::new(async move {
                    match names.await {
                        Ok(Some(names)) if !names.is_empty() => {
                            let current = query.read_untracked().get("metric");
                            metric_picker(names, current.as_deref())
                        }
                        Ok(_) => ().into_any(),
                        Err(e) => inline_error(&e),
                    }
                })
            }}
        </Suspense>
    }
    .into_any();

    let detail_section = view! {
        <Suspense fallback=|| ()>
            {move || {
                Suspend::new(async move {
                    match detail.await {
                        Ok(Some(detail)) => metric_detail_body(detail),
                        Ok(None) => ().into_any(),
                        Err(e) => inline_error(&e),
                    }
                })
            }}
        </Suspense>
    }
    .into_any();

    let body = view! { <div>{tiles_section} {browser_section} {detail_section}</div> }.into_any();
    titled_card("Metrics", true, body)
}

/// The headline tiles: one [`StatCard`] per tracked metric, `—` where the
/// deployment records nothing.
fn tiles_body(tiles: Vec<MetricTile>) -> AnyView {
    let cards = tiles
        .into_iter()
        .map(|tile| {
            let value = tile.value;
            view! {
                <StatCard
                    label=tile.label
                    value=Signal::derive(move || value.clone())
                    icon=icondata_lu::LuGauge
                    href=format!("/operations?metric={}", urlencoding::encode(&tile.name))
                />
            }
            .into_any()
        })
        .collect_view()
        .into_any();
    view! { <div class="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-3">{cards}</div> }
        .into_any()
}

/// The registry picker: a GET `<Form>` submitting `?metric=…` back to this
/// screen (progressive enhancement — no WASM needed), pre-selected from the URL.
fn metric_picker(names: Vec<String>, current: Option<&str>) -> AnyView {
    let options = names
        .into_iter()
        .map(|name| {
            let selected = current == Some(name.as_str());
            let value = name.clone();
            view! {
                <option value=value selected=selected>
                    {name}
                </option>
            }
        })
        .collect_view();
    view! {
        <leptos_router::components::Form
            method="GET"
            action="/operations"
            attr:class="mt-4 flex flex-wrap items-end gap-2"
        >
            <label class="flex flex-col gap-1 text-xs text-ink-muted">
                "Metric" <select id="ops-metric" name="metric" class=format!("min-w-72 {SELECT}")>
                    {options}
                </select>
            </label>
            <button id="ops-metric-inspect" type="submit" class=BTN_SECONDARY>
                "Inspect"
            </button>
        </leptos_router::components::Form>
    }
    .into_any()
}

/// One metric's current samples: its type/help, then a row per sample.
fn metric_detail_body(detail: MetricDetailView) -> AnyView {
    let name = detail.name.clone();
    let kind = detail.kind.clone();
    let help = detail.help.clone();
    let rows = detail
        .samples
        .into_iter()
        .map(|sample| {
            let labels = if sample.labels.is_empty() {
                "—".to_owned()
            } else {
                sample.labels
            };
            let value = crate::management::format_metric(sample.value);
            view! {
                <tr class=ROW>
                    <td class=CELL_MONO>{labels}</td>
                    <td class="px-3 py-2 text-right tabular-nums">{value}</td>
                </tr>
            }
            .into_any()
        })
        .collect_view()
        .into_any();
    let table = table_shell(&["Labels", "Value"], rows);
    let subtitle = metric_subtitle(&kind, &help);
    view! {
        <div id="ops-metric-detail" class="mt-3">
            // A metric name is an identifier the operator will grep for, so it
            // renders verbatim in mono — never through the uppercase section-label
            // style, which would print `db_pool_connections` as a name that does
            // not exist.
            <h3 class="mb-1 font-mono text-sm font-semibold text-ink">{name}</h3>
            {(!subtitle.is_empty())
                .then(|| view! { <p class="mb-2 text-sm text-ink-muted">{subtitle}</p> })}
            {table}
        </div>
    }
    .into_any()
}

/// A metric's one-line subtitle from its (optional) Prometheus type and
/// (optional) `# HELP` text — no dangling separator when either is absent.
fn metric_subtitle(kind: &str, help: &str) -> String {
    match (kind.is_empty(), help.is_empty()) {
        (true, true) => String::new(),
        (false, true) => kind.to_owned(),
        (true, false) => help.to_owned(),
        (false, false) => format!("{kind} · {help}"),
    }
}

// ── Configuration (cross-link, deliberately not a second viewer) ────────────

/// The configuration card: a pointer at the ONE effective-configuration viewer.
///
/// `/management/env` and `/admin/config` serve the same redacted snapshot (see
/// the module doc), so this card deliberately fetches nothing — a second viewer
/// of one claim is exactly the duplication the viewer avoids.
fn config_card() -> AnyView {
    let body = view! {
        <div class="flex flex-col items-start gap-3">
            <p class="text-sm text-ink-muted">
                "The CDR's effective configuration, with every secret redacted. The CDR serves the "
                "same snapshot on its management surface and its admin API, so the viewer reads it "
                "in exactly one place — the System screen, over the API base URL it is always "
                "configured for."
            </p>
            <A href="/system" attr:class=BTN_SECONDARY>
                <leptos_icons::Icon icon=icondata_lu::LuSettings width="14" height="14" />
                " Open runtime configuration"
            </A>
        </div>
    }
    .into_any();
    titled_card("Runtime configuration", false, body)
}

// ── Log control ─────────────────────────────────────────────────────────────

/// The log-control card: the effective + boot filter, a field to swap the live
/// filter, and a reset. Both writes confirm in a modal first.
///
/// The affordances render only once the CDR answered with a mounted `loggers`
/// endpoint. Capability is not authorization: whether THIS session may change
/// the filter is the CDR's per-request answer (the endpoint's configured
/// access level — every management endpoint is `off` until named), and a
/// refusal arrives as the actionable failure toast, never as a silent no-op.
fn loggers_card(
    apply: ApplyFilterAction,
    reset: ResetFilterAction,
    pending_apply: RwSignal<Option<String>>,
    pending_reset: RwSignal<bool>,
) -> AnyView {
    let loggers: Resource<Result<Option<LoggerView>, ViewerError>> = Resource::new(
        move || (apply.version().get(), reset.version().get()),
        |_| async move { fetch_loggers().await },
    );
    let draft = RwSignal::new(String::new());
    let body = view! {
        <Transition fallback=card_skeleton>
            {move || {
                Suspend::new(async move {
                    match loggers.await {
                        Ok(Some(view)) => loggers_body(view, draft, pending_apply, pending_reset),
                        Ok(None) => surface_absent("Log control"),
                        Err(e) => inline_error(&e),
                    }
                })
            }}
        </Transition>
    }
    .into_any();
    titled_card("Log level", false, body)
}

/// Render the live filter, the boot filter, and the two write affordances.
fn loggers_body(
    view: LoggerView,
    draft: RwSignal<String>,
    pending_apply: RwSignal<Option<String>>,
    pending_reset: RwSignal<bool>,
) -> AnyView {
    let effective = view.filter.clone();
    let boot = view.boot_filter;
    view! {
        <div>
            <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
                <dt class="font-medium text-ink-muted">"effective"</dt>
                <dd id="ops-log-effective" class="font-mono break-all text-ink">
                    {effective}
                </dd>
                <dt class="font-medium text-ink-muted">"boot"</dt>
                <dd class="font-mono break-all text-ink">{boot}</dd>
            </dl>
            <div class="mt-3 flex flex-wrap items-end gap-2">
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "New filter directives"
                    <input
                        id="ops-log-filter"
                        type="text"
                        class=format!("w-72 {INPUT}")
                        placeholder="ferroehr=debug,sqlx=warn"
                        prop:value=move || draft.get()
                        on:input:target=move |ev| draft.set(ev.target().value())
                    />
                </label>
                <button
                    id="ops-log-apply"
                    type="button"
                    class=BTN_PRIMARY
                    disabled=move || draft.read().trim().is_empty()
                    on:click=move |_| pending_apply.set(Some(draft.read().trim().to_owned()))
                >
                    <leptos_icons::Icon
                        icon=icondata_lu::LuSlidersHorizontal
                        width="14"
                        height="14"
                    />
                    " Apply filter"
                </button>
                <button
                    id="ops-log-reset"
                    type="button"
                    class=BTN_SECONDARY
                    on:click=move |_| pending_reset.set(true)
                >
                    <leptos_icons::Icon icon=icondata_lu::LuRotateCcw width="14" height="14" />
                    " Reset to boot filter"
                </button>
            </div>
            <p class="mt-2 text-xs text-ink-muted">
                "Takes effect immediately, for this CDR instance only, until it restarts."
            </p>
        </div>
    }
    .into_any()
}

/// The screen's two confirmation modals, rendered once outside the card so a
/// refetch never re-creates them and each stays inert while nothing is pending.
fn filter_dialogs(
    apply: ApplyFilterAction,
    reset: ResetFilterAction,
    pending_apply: RwSignal<Option<String>>,
    pending_reset: RwSignal<bool>,
) -> AnyView {
    let apply_message = Signal::derive(move || {
        pending_apply.get().map_or_else(String::new, |filter| {
            format!(
                "Switch the CDR's live log filter to “{filter}”? Logging changes immediately for \
                 every request this instance serves; a verbose filter on a busy server produces a \
                 lot of output."
            )
        })
    });
    view! {
        <ConfirmDialog
            open=Signal::derive(move || pending_apply.get().is_some())
            title="Apply log filter"
            message=apply_message
            confirm_label="Apply filter"
            confirm_icon=icondata_lu::LuSlidersHorizontal
            confirm_id="ops-log-apply-confirm"
            on_cancel=Callback::new(move |()| pending_apply.set(None))
            on_confirm=Callback::new(move |()| {
                if let Some(filter) = pending_apply.get_untracked() {
                    apply.dispatch(filter);
                }
                pending_apply.set(None);
            })
        />
        <ConfirmDialog
            open=pending_reset
            title="Reset log filter"
            message=Signal::derive(|| {
                "Restore the CDR's boot log filter? Any filter applied since startup is discarded."
                    .to_owned()
            })
            confirm_label="Reset filter"
            confirm_icon=icondata_lu::LuRotateCcw
            confirm_id="ops-log-reset-confirm"
            on_cancel=Callback::new(move |()| pending_reset.set(false))
            on_confirm=Callback::new(move |()| {
                reset.dispatch(());
                pending_reset.set(false);
            })
        />
    }
    .into_any()
}

// ── The absent-surface state ────────────────────────────────────────────────

/// The first-class "this management endpoint is not mounted" state (the CDR
/// answered `404`): the management surface is off by default and each endpoint
/// is independently opt-in, so absence is a configuration fact to state, never
/// an error to report.
fn surface_absent(what: &'static str) -> AnyView {
    view! {
        <thaw::MessageBar intent=thaw::MessageBarIntent::Info>
            <thaw::MessageBarBody>
                {what}
                " is not available: the CDR does not serve this management endpoint. Enable it "
                "server-side ([management] — each endpoint takes its own access level)."
            </thaw::MessageBarBody>
        </thaw::MessageBar>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::metric_subtitle;

    /// The metric's type and its `# HELP` text are both optional in a
    /// Prometheus exposition, so the subtitle must never render a dangling
    /// separator (or an empty line) when one of them is absent.
    #[test]
    fn metric_subtitle_never_dangles_a_separator() {
        assert_eq!(metric_subtitle("", ""), "");
        assert_eq!(metric_subtitle("gauge", ""), "gauge");
        assert_eq!(
            metric_subtitle("", "requests in flight"),
            "requests in flight"
        );
        assert_eq!(
            metric_subtitle("gauge", "requests in flight"),
            "gauge · requests in flight"
        );
    }
}
