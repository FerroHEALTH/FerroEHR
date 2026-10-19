// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The application shell.
//!
//! The session guard plus the persistent chrome around the routed `<Outlet/>`,
//! drawn by FerroUI: the skip link, the top bar (the lockup, the CDR status
//! pill carrying the declared deployment profile, the theme toggle, the
//! operator menu), the sandbox banner under it, the navigation (a column beside
//! the page, a drawer on a narrow screen), and the footer.
//!
//! The access drawer ("View scopes", in the operator menu) is the viewer's
//! effective-identity surface: the authenticated principal, the policy source
//! deciding what it may do, this session's scopes rendered as their parsed
//! SMART grants, and a free previewer for any scope string — all through
//! [`crate::scopes`] over the ONE master08 grammar the CDR's own gate parses
//! with. Capability is not authorization: the drawer says so, and the CDR stays
//! the enforcer.
//!
//! The shell is a layout, not a route: it is the `view` of the guarded
//! `ParentRoute` in [`crate::app`], and it renders the matched child screen
//! through `<Outlet/>`. Access is gated on a live viewer session — with no
//! session the guard redirects to `/login`. The theme is the server cookie the
//! switch posts to ([`crate::theme`]); no browser storage holds anything.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::tone::Tone;
use ferrohealth_ui::view::menu::{OperatorMenu, operator_menu};
use ferrohealth_ui::view::shell::{Footer, Shell, drawer_button, shell, topbar};
use ferrohealth_ui::view::status::{banner, pill};
use ferrohealth_ui::view::theme_switch::theme_toggle;
use ferrohealth_ui::widget::nav::{Entry, Group, sidebar};
use ferrohealth_ui::widget::side_panel::{SidePanel, side_panel, side_panel_trigger};
use ferrohealth_ui::widget::toast::{provide_toasts, toast_region};
use leptos::prelude::*;
use leptos::server_fn::ServerFn as _;
use leptos_router::components::{Outlet, Redirect};

use crate::auth::{Logout, SessionInfo, current_session, fetch_status};
use crate::components::brand::viewer_lockup;
use crate::components::scope_grants::{ScopePreviewer, capability_note, fact_row, grant_cards};
use crate::scopes::{grants_of, policy_source};
use crate::session_client::{session_ended_epoch, session_ended_signal, signed_out_url};

/// How often (ms) the topbar re-polls the CDR `{rest root}/status` health endpoint.
const HEALTH_POLL_MS: u64 = 30_000;

/// How long (ms) after the session's reported deadline the browser re-checks it.
///
/// One second past it, so the re-check reads a cookie the server has already
/// stopped honouring rather than racing it by a hair.
const EXPIRY_GRACE_MS: u64 = 1_000;

/// The id of the navigation, which the drawer button opens.
const SIDEBAR_ID: &str = "sidebar";

/// The id of the access-scopes side panel, which the operator menu opens.
const ACCESS_PANEL_ID: &str = "access-scopes";

/// The operator menu's "View scopes" button: FerroUI's secondary button at
/// the menu's full width, so it lines up with the sign-out under it.
const SCOPES_TRIGGER: &str = "fui-button fui-button-secondary w-full";

/// The id of the operator menu's popover.
const OPERATOR_MENU_ID: &str = "user-menu";

/// The demographics entry's href: the section's landing redirects to the
/// default kind's browser, and every `/demographics/…` path marks this entry.
const NAV_DEMOGRAPHICS: &str = "/demographics";

/// The clinical-data entries every deployment shows, in order.
const DATA: [Entry; 6] = [
    Entry {
        href: "/",
        label: "Dashboard",
        glyph: Glyph::LAYOUT_DASHBOARD,
        exact: true,
    },
    Entry {
        href: "/templates",
        label: "Templates",
        glyph: Glyph::FILE_CODE,
        exact: false,
    },
    Entry {
        href: "/queries",
        label: "Queries",
        glyph: Glyph::SEARCH_CODE,
        exact: false,
    },
    Entry {
        href: "/ehrs",
        label: "EHRs",
        glyph: Glyph::DATABASE,
        exact: false,
    },
    Entry {
        href: NAV_DEMOGRAPHICS,
        label: "Demographics",
        glyph: Glyph::USERS,
        exact: false,
    },
    Entry {
        href: "/terminology",
        label: "Terminology",
        glyph: Glyph::BOOK_A,
        exact: false,
    },
];

/// The probe-gated subscriptions entry, last in the data group.
const SUBSCRIPTIONS: Entry = Entry {
    href: "/subscriptions",
    label: "Subscriptions",
    glyph: Glyph::RADIO_TOWER,
    exact: false,
};

/// The probe-gated operations entry, first in the platform group.
const OPERATIONS: Entry = Entry {
    href: "/operations",
    label: "Operations",
    glyph: Glyph::GAUGE,
    exact: false,
};

/// The platform-about-itself entries every deployment shows, `System` last.
const AUDIT: Entry = Entry {
    href: "/audit",
    label: "Audit log",
    glyph: Glyph::SHIELD_CHECK,
    exact: false,
};

/// The system entry, last of all.
const SYSTEM: Entry = Entry {
    href: "/system",
    label: "System",
    glyph: Glyph::SERVER,
    exact: false,
};

/// The data group with the subscriptions entry.
const DATA_WITH_SUBSCRIPTIONS: [Entry; 7] = [
    DATA[0],
    DATA[1],
    DATA[2],
    DATA[3],
    DATA[4],
    DATA[5],
    SUBSCRIPTIONS,
];

/// The platform group with the operations entry.
const PLATFORM_WITH_OPERATIONS: [Entry; 3] = [OPERATIONS, AUDIT, SYSTEM];

/// The platform group without it.
const PLATFORM: [Entry; 2] = [AUDIT, SYSTEM];

/// The navigation's two groups, the gated entries in their group when the CDR
/// serves their surface: the DATA group (the clinical and definitional
/// sections a deployment's own data lives in), then the PLATFORM group (the
/// platform-about-itself screens), with `System` last.
fn nav_groups(subscriptions: bool, operations: bool) -> [Group; 2] {
    let data: &'static [Entry] = if subscriptions {
        &DATA_WITH_SUBSCRIPTIONS
    } else {
        &DATA
    };
    let platform: &'static [Entry] = if operations {
        &PLATFORM_WITH_OPERATIONS
    } else {
        &PLATFORM
    };
    [
        Group {
            id: "nav-data",
            title: "Clinical data",
            entries: data,
        },
        Group {
            id: "nav-platform",
            title: "Platform",
            entries: platform,
        },
    ]
}

/// The session-guarded application shell.
///
/// Loads the current session with a [`Resource`]; under `<Suspense>` a missing
/// session renders a [`Redirect`] to `/login`, and a present session renders
/// the full chrome + `<Outlet/>`. The topbar status pill reads a second
/// resource over `fetch_status`, re-polled every `HEALTH_POLL_MS` via
/// a browser interval (an effect, so never on the server).
///
/// The shell never stays interactive over a dead session, and three browser
/// timers/watchers see to it: the session resource is re-checked on the health
/// poll's tick, again on the session's own deadline
/// ([`crate::auth::SessionInfo::expires_in_secs`] plus `EXPIRY_GRACE_MS`), and
/// immediately when [`crate::session_client`]'s transport backstop reports that
/// some server function answered `Unauthenticated` or `CdrUnauthorized`. All
/// three land on the same signed-out URL ([`signed_out_url`]), which unmounts
/// the whole chrome, so there is no window in which menus and forms accept
/// input against an expired credential.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn AppShell() -> impl IntoView {
    let session = Resource::new(|| (), |()| current_session());
    let status = Resource::new(|| (), |()| fetch_status());
    // The two surface probes are created HERE, in component setup — never
    // inside a Suspend closure, which re-runs and would re-create the resource.
    // They gate the operations and subscriptions nav entries.
    let management = crate::management::management_gate();
    let subscriptions = crate::subscriptions::event_subscription_gate();
    let toasts = provide_toasts();

    // The signed-out destination, computed from the URL the user is on so
    // signing in again returns them to it. Read untracked: it is the
    // transition's argument, never a dependency of it.
    let location = leptos_router::hooks::use_location();
    let pathname = location.pathname;
    let search = location.search;
    let signed_out = move || signed_out_url(&pathname.get_untracked(), &search.get_untracked());

    // Whether a session has ever resolved as PRESENT in this shell, which is
    // what separates an expiry from a fresh visit to a guarded URL. Plain
    // storage rather than a signal: the gate writes it while rendering, and a
    // reactive write there would re-run the gate that wrote it.
    let seen_session = StoredValue::new(false);

    // Browser-only (an Effect never runs on the server pass): poll the CDR
    // health endpoint on an interval, and re-check the session on the same
    // tick — a revocation the browser never asked about surfaces within one
    // poll even if nothing else is happening.
    Effect::new(move |_| {
        match set_interval_with_handle(
            move || {
                status.refetch();
                session.refetch();
            },
            std::time::Duration::from_millis(HEALTH_POLL_MS),
        ) {
            Ok(handle) => on_cleanup(move || handle.clear()),
            Err(_refused) => leptos::logging::warn!("the browser refused the health poll timer"),
        }
    });

    // Browser-only: the session's reported deadline schedules its own
    // re-check, which catches an idle expiry the poll above cannot — a
    // throttled background tab, a machine that slept. Each freshly resolved
    // session replaces the pending timer with one at its own deadline.
    let expiry_check = StoredValue::new(None::<TimeoutHandle>);
    Effect::new(move |_| {
        let Some(Ok(Some(info))) = session.get() else {
            return;
        };
        if let Some(pending) = expiry_check.get_value() {
            pending.clear();
        }
        let delay = std::time::Duration::from_secs(u64::from(info.expires_in_secs))
            .saturating_add(std::time::Duration::from_millis(EXPIRY_GRACE_MS));
        match set_timeout_with_handle(move || session.refetch(), delay) {
            Ok(handle) => expiry_check.set_value(Some(handle)),
            Err(_refused) => leptos::logging::warn!("the browser refused the session expiry timer"),
        }
    });

    // Browser-only: the transport backstop. `SessionAwareClient` bumps the
    // session-ended counter when a server fn answers `Unauthenticated` or
    // `CdrUnauthorized`, including a CDR revocation this viewer's own cookie
    // knows nothing about. It RE-READS the session rather than signing out on
    // that alone (#3066): a detection names no session, so an answer to a
    // request issued under the previous cookie can land after a new sign-in.
    let session_ended = session_ended_signal();
    let session_ended_baseline = session_ended_epoch();
    Effect::new(move |_| {
        if session_ended.get() != session_ended_baseline {
            session.refetch();
        }
    });

    // The chrome (and the routed <Outlet/>) is created exactly ONCE, outside
    // any Suspend closure; the session gate below renders ONLY the redirect
    // decision. A Suspend closure re-runs on every notification of the
    // resources it awaits, and re-creating the Outlet re-creates every resource
    // the routed page owns, so server and client resource ids diverge and
    // hydration reads the wrong serialized slots.
    let gate = view! {
        <Suspense fallback=|| ()>
            {move || {
                Suspend::new(async move {
                    match session.await {
                        Ok(Some(_)) => {
                            seen_session.set_value(true);
                            ().into_any()
                        }
                        Ok(None) if seen_session.get_value() => {
                            // A session that WAS present and is now gone is an
                            // expiry: say so, and carry the screen the user was
                            // on. A first resolution with no session is just a
                            // guarded URL visited signed out.
                            view! { <Redirect path=signed_out() /> }
                                .into_any()
                        }
                        _ => view! { <Redirect path="/login" /> }.into_any(),
                    }
                })
            }}
        </Suspense>
    }
    .into_any();
    let chrome = authed_shell(session, status, management, subscriptions);
    view! {
        {gate}
        {chrome}
        {toast_region(toasts)}
    }
}

/// The CDR status pill: the status document's verdict, version and declared
/// profile, kept visible while the poll reloads it.
fn status_pill(status: Resource<Result<String, crate::error::ViewerError>>) -> AnyView {
    // <Transition>, not <Suspense>: the 30 s poll refetches the resource and
    // Transition keeps the previous pill visible during reload instead of
    // flashing the fallback every poll (book async/12).
    view! {
        <span id="cdr-status" class="fui-roomy-only">
            <Transition fallback=|| pill(
                Tone::Neutral,
                "Checking the CDR",
            )>
                {move || {
                    Suspend::new(async move {
                        let (tone, word) = match status.await {
                            Ok(body) => {
                                let doc = crate::deployment::StatusDocument::parse(&body);
                                let version = doc
                                    .as_ref()
                                    .and_then(|d| d.server_version.clone())
                                    .unwrap_or_default();
                                let profile = doc
                                    .as_ref()
                                    .and_then(crate::deployment::StatusDocument::profile)
                                    .map(|p| format!(" · {p}"))
                                    .unwrap_or_default();
                                match doc.as_ref().and_then(|d| d.status.as_deref()) {
                                    Some("UP") => {
                                        (Tone::Ok, format!("CDR UP · v{version}{profile}"))
                                    }
                                    _ => (Tone::Warn, "CDR DEGRADED".to_owned()),
                                }
                            }
                            Err(
                                crate::error::ViewerError::Unauthenticated
                                | crate::error::ViewerError::CdrUnauthorized(_),
                            ) => (Tone::Warn, "Session ended".to_owned()),
                            Err(_) => (Tone::Bad, "CDR DOWN".to_owned()),
                        };
                        pill(tone, &word).into_any()
                    })
                }}
            </Transition>
        </span>
    }
    .into_any()
}

/// The sandbox banner (#3264): persistent, under the topbar, on every
/// authenticated screen, so the tab says which deployment it is. Nothing
/// renders for production or for a server that declares no profile.
fn sandbox_banner(status: Resource<Result<String, crate::error::ViewerError>>) -> AnyView {
    view! {
        <Transition fallback=|| ()>
            {move || {
                Suspend::new(async move {
                    let notice = match status.await {
                        Ok(body) => {
                            crate::deployment::StatusDocument::parse(&body)
                                .and_then(|doc| doc.sandbox_notice())
                        }
                        Err(_) => None,
                    };
                    notice
                        .map(|notice| {
                            let rule = crate::deployment::SandboxNotice::RULE;
                            let rest = rule.strip_prefix("Sandbox deployment: ").unwrap_or(rule);
                            let gaps = notice.gaps_sentence();
                            let body = if gaps.is_empty() {
                                rest.to_owned()
                            } else {
                                format!("{rest} {gaps}")
                            };
                            view! {
                                <div id="deployment-notice">
                                    {banner(
                                        Tone::Warn,
                                        Glyph::TRIANGLE_ALERT,
                                        "Sandbox deployment",
                                        &body,
                                    )}
                                </div>
                            }
                        })
                })
            }}
        </Transition>
    }
    .into_any()
}

/// The operator menu: the identity, "View scopes" (which opens the access
/// drawer) and the sign-out, a plain form post to the logout server function.
fn user_menu(session: Resource<Result<Option<SessionInfo>, crate::error::ViewerError>>) -> AnyView {
    // Identity text resolves the session in a resource-free section (safe to
    // re-run — it allocates no resources, so ids cannot diverge).
    view! {
        <Suspense fallback=|| ()>
            {move || {
                Suspend::new(async move {
                    let identity = session
                        .await
                        .ok()
                        .flatten()
                        .map(|info| info.identity)
                        .unwrap_or_default();
                    let extra = view! {
                        {side_panel_trigger(
                            ACCESS_PANEL_ID,
                            SCOPES_TRIGGER,
                            Glyph::SHIELD_CHECK,
                            "View scopes",
                        )}
                    };
                    view! {
                        <span id="user-menu-trigger" class="contents">
                            {operator_menu(
                                OperatorMenu {
                                    id: OPERATOR_MENU_ID,
                                    name: identity,
                                    subtitle: "Signed in to this viewer",
                                    sign_out: Some(Logout::url().to_owned()),
                                },
                                extra,
                            )}
                        </span>
                    }
                        .into_any()
                })
            }}
        </Suspense>
    }
    .into_any()
}

/// The effective-access block and the drawer that holds it: who this session
/// is, what decides what it may do, and the scopes it carries rendered as their
/// parsed master08 grants. It resolves the session inside its own
/// resource-free Suspend and renders ONLY facts the session actually carries:
/// the viewer never invents a claim it was not given.
fn scopes_drawer(
    session: Resource<Result<Option<SessionInfo>, crate::error::ViewerError>>,
) -> AnyView {
    let effective_access = view! {
        <Suspense fallback=|| ()>
            {move || {
                Suspend::new(async move {
                    let info = session.await.ok().flatten();
                    let (identity, method) = info
                        .as_ref()
                        .map(|info| (info.identity.clone(), info.method.clone()))
                        .unwrap_or_default();
                    let scopes = info.map(|info| info.scopes).unwrap_or_default();
                    let policy = policy_source(&method);
                    let scope_view = if scopes.is_empty() {
                        // Deliberately NOT an empty state: an empty scope list is
                        // the complete answer to "what am I allowed to do" under
                        // Basic auth — the sentence IS the content.
                        view! {
                            <p class="text-sm text-ink-muted">
                                "No scopes — Basic authentication grants full viewer access."
                            </p>
                        }
                            .into_any()
                    } else {
                        grant_cards(grants_of(&scopes))
                    };
                    view! {
                        <div class="flex flex-col gap-3">
                            <div class="flex flex-col gap-1">
                                {fact_row("Identity", identity)}
                                {fact_row("Signed in with", policy.label.to_owned())}
                            </div>
                            <p class="text-xs leading-snug text-ink-muted">{policy.note}</p>
                            <div id="session-scopes" class="flex flex-col gap-2">
                                <h3 class="text-xs font-medium text-ink">
                                    "Scopes on this session"
                                </h3>
                                {scope_view}
                            </div>
                        </div>
                    }
                        .into_any()
                })
            }}
        </Suspense>
    }
    .into_any();

    // Stable body id: the E2E journeys read the panel's whole body (identity,
    // policy source, session grants, previewer).
    let body = view! {
        <div class="flex flex-col gap-4">
            {capability_note()} {effective_access} <hr class="border-edge" /> <ScopePreviewer />
        </div>
    }
    .into_any();
    side_panel(SidePanel {
        id: ACCESS_PANEL_ID,
        body_id: "access-drawer",
        title: "Access scopes",
        body,
    })
}

/// The navigation, its gated entries shown once the probes answer; until then
/// the entries every deployment has.
fn navigation(
    management: Resource<
        Result<crate::management::ManagementAvailability, crate::error::ViewerError>,
    >,
    subscriptions: Resource<
        Result<crate::subscriptions::SubscriptionAvailability, crate::error::ViewerError>,
    >,
) -> AnyView {
    view! {
        <Transition fallback=|| sidebar(
            SIDEBAR_ID,
            "Main",
            &nav_groups(false, false),
        )>
            {move || {
                Suspend::new(async move {
                    let operations = crate::management::renders_management_ops(&management.await);
                    let events = crate::subscriptions::renders_event_subscriptions(
                        &subscriptions.await,
                    );
                    sidebar(SIDEBAR_ID, "Main", &nav_groups(events, operations))
                })
            }}
        </Transition>
    }
    .into_any()
}

/// Builds the authenticated chrome (topbar, banner, navigation, footer,
/// scopes drawer) around the routed `<Outlet/>`, each section an
/// `.into_any()`-erased local.
fn authed_shell(
    session: Resource<Result<Option<SessionInfo>, crate::error::ViewerError>>,
    status: Resource<Result<String, crate::error::ViewerError>>,
    management: Resource<
        Result<crate::management::ManagementAvailability, crate::error::ViewerError>,
    >,
    subscriptions: Resource<
        Result<crate::subscriptions::SubscriptionAvailability, crate::error::ViewerError>,
    >,
) -> AnyView {
    let back = leptos_router::hooks::use_location()
        .pathname
        .get_untracked();
    let start = view! {
        {drawer_button(SIDEBAR_ID)}
        {viewer_lockup()}
    }
    .into_any();
    let end = view! {
        {status_pill(status)}
        {theme_toggle(crate::theme::THEME_ROUTE.to_owned(), back)}
        {user_menu(session)}
    }
    .into_any();
    let content = view! {
        <Outlet />
        {scopes_drawer(session)}
    }
    .into_any();
    shell(Shell {
        topbar: topbar(start, end).into_any(),
        banner: Some(sandbox_banner(status)),
        sidebar: navigation(management, subscriptions),
        content,
        footer: Footer {
            product: "FerroEHR Viewer",
            version: env!("CARGO_PKG_VERSION"),
            about: "/system#manufacturer",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{SCOPES_TRIGGER, nav_groups};

    #[test]
    fn the_scopes_trigger_is_ferrouis_secondary_button_at_full_width() {
        let secondary = ferrohealth_ui::view::button::Variant::Secondary.class();
        assert_eq!(SCOPES_TRIGGER, format!("{secondary} w-full"));
    }

    /// Renders the groups as the token the order assertions read: each
    /// group's heading, then its entries' labels.
    fn tokens(subscriptions: bool, operations: bool) -> Vec<String> {
        nav_groups(subscriptions, operations)
            .iter()
            .flat_map(|group| {
                std::iter::once(format!("[{}]", group.title))
                    .chain(group.entries.iter().map(|entry| entry.label.to_owned()))
            })
            .collect()
    }

    /// The decided sidebar order (owner call, issue #2577): the data group on
    /// top with its gated entry in place, then the platform group with
    /// `System` last.
    #[test]
    fn sidebar_renders_data_group_then_platform_group_with_system_last() {
        assert_eq!(
            tokens(true, true),
            [
                "[Clinical data]",
                "Dashboard",
                "Templates",
                "Queries",
                "EHRs",
                "Demographics",
                "Terminology",
                "Subscriptions",
                "[Platform]",
                "Operations",
                "Audit log",
                "System",
            ]
        );
    }

    /// The platform group's anchors are unconditional, so the leanest
    /// deployment (no event subscriptions, no management surface) still shows
    /// both groups with content.
    #[test]
    fn hiding_every_gated_entry_leaves_both_groups() {
        assert_eq!(
            tokens(false, false),
            [
                "[Clinical data]",
                "Dashboard",
                "Templates",
                "Queries",
                "EHRs",
                "Demographics",
                "Terminology",
                "[Platform]",
                "Audit log",
                "System",
            ]
        );
    }

    /// Only the dashboard is marked on its own path alone: every other entry
    /// marks the paths under it too, so a detail screen marks its section.
    #[test]
    fn only_the_dashboard_entry_is_exact() {
        for group in nav_groups(true, true) {
            for entry in group.entries {
                assert_eq!(entry.exact, entry.href == "/", "{}", entry.label);
            }
        }
    }
}
