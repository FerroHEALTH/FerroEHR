// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shell every signed-in page is drawn in: the top bar, the navigation,
//! the sandbox band, the page, its docked decision bar and the footer.
//!
//! The parts are FerroUI's (`view::shell`, `widget::nav`, `widget::menu`,
//! `view::status`, `view::search`): the top bar carries the search, the CDR's
//! status pill and the user menu, and no theme control (family rule 5). The
//! navigation follows the approved information architecture of the viewer's
//! mockup (FerroUI `design/consoles/ferroehr/`): Overview, Records,
//! Definitions, Queries and the Console group. Subscriptions is left out when
//! the CDR does not serve its API.
//!
//! The session is read from the sealed cookie with no CDR request; the
//! pre-render guard ([`crate::server::login_guard`]) has already sent a
//! request without one to the sign-in page. The status pill reads the CDR's
//! status document once, as the page is drawn: the viewer polls nothing in
//! the background, so an open tab's idle window measures what the reader
//! does (#3057). No openEHR spec governs the viewer: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::theme::Theme;
use ferrohealth_ui::tone::Tone;
use ferrohealth_ui::view::dock::{DecisionBar, decision_bar};
use ferrohealth_ui::view::search::{SearchAnswer, SearchBox, search_box};
use ferrohealth_ui::view::shell::{Footer, Shell, TopBar, brand_mark, lockup, shell, topbar};
use ferrohealth_ui::view::status::{Reach, StatusPill, banner, status_pill};
use ferrohealth_ui::widget::menu::{ThemeChoice, UserMenu, user_menu};
use ferrohealth_ui::widget::nav::{Entry, Group, Rail, Sidebar, sidebar};
use leptos::prelude::*;
use leptos_router::components::Outlet;

use crate::locale::{Msg, tr, tr_with};

/// The id of the navigation, which the top bar's Menu button opens.
const SIDEBAR_ID: &str = "sidebar";

/// The id of the user menu.
const USER_MENU_ID: &str = "user-menu";

/// The route Sign out posts to.
pub const SIGN_OUT_ROUTE: &str = "/logout";

/// The path the FerroEHR icon is served at for the lockup.
pub const MARK_PATH: &str = "/ferroehr-icon.svg";

/// The route the top bar's search posts to.
pub const FIND_ROUTE: &str = "/find";

/// The page's docked decision bar, which the page sets as it is drawn and
/// the shell draws at the foot of the main area (R4).
///
/// A page that decides its bar only after a read promises it first
/// ([`promise_dock`]); the shell draws the dock once every promise is kept or
/// dropped, so the bar never depends on which part of the page the server
/// happened to finish first.
#[derive(Debug, Clone, Default)]
pub struct Dock {
    /// The bar, once the page set one.
    bar: ArcRwSignal<Option<DecisionBar>>,
    /// The promises still open.
    pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Woken as each promise closes.
    closed: std::sync::Arc<tokio::sync::Notify>,
}

impl Dock {
    /// Docks `bar` at the foot of the page being drawn.
    pub fn set(&self, bar: DecisionBar) {
        self.bar.set(Some(bar));
    }

    /// The bar, once no promise is open.
    async fn settled(&self) -> Option<DecisionBar> {
        loop {
            let woken = self.closed.notified();
            if self.pending.load(std::sync::atomic::Ordering::Acquire) == 0 {
                return self.bar.get_untracked();
            }
            woken.await;
        }
    }
}

/// A page's promise to dock its bar, or to have none, by the time it drops.
#[derive(Debug)]
pub struct DockPromise(Option<Dock>);

impl Drop for DockPromise {
    fn drop(&mut self) {
        if let Some(dock) = self.0.take() {
            dock.pending
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            dock.closed.notify_waiters();
        }
    }
}

/// Promises the dock of the page being drawn: hold it in the future that
/// calls [`dock`], so the shell waits for it.
#[must_use]
pub fn promise_dock() -> DockPromise {
    let dock = use_context::<Dock>();
    if let Some(dock) = &dock {
        dock.pending
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    DockPromise(dock)
}

/// Docks `bar` at the foot of the page being drawn.
pub fn dock(bar: DecisionBar) {
    if let Some(dock) = use_context::<Dock>() {
        dock.set(bar);
    }
}

/// The CDR's status document, read once per page: the pill, the band and a
/// page that shows the deployment read the same answer.
#[derive(Debug, Clone, Default)]
pub struct Status(
    pub std::sync::Arc<tokio::sync::OnceCell<Result<String, crate::error::ViewerError>>>,
);

impl Status {
    /// The status document as the CDR answered it for this page.
    ///
    /// # Errors
    ///
    /// The CDR's refusal of the status read, or its being out of reach.
    pub async fn read(&self) -> Result<String, crate::error::ViewerError> {
        self.0
            .get_or_init(|| async { crate::auth::fetch_status().await })
            .await
            .clone()
    }
}

/// The CDR's status document for the page being drawn.
///
/// # Errors
///
/// The CDR's refusal of the status read, or its being out of reach.
pub async fn status() -> Result<String, crate::error::ViewerError> {
    match use_context::<Status>() {
        Some(status) => status.read().await,
        None => crate::auth::fetch_status().await,
    }
}

/// The viewer's product name and the lockup's accented word.
fn lockup_view() -> AnyView {
    lockup(
        "/",
        brand_mark(MARK_PATH, MARK_PATH),
        tr(Msg::BrandName),
        tr(Msg::BrandWord),
    )
    .into_any()
}

/// The footer of every page: the product, its version and About.
#[must_use]
pub fn footer() -> Footer {
    Footer {
        product: crate::app::product().into(),
        version: env!("CARGO_PKG_VERSION").into(),
        about: "/about".into(),
    }
}

/// One navigation entry.
fn entry(href: &'static str, label: Msg, glyph: Glyph, exact: bool) -> Entry {
    Entry {
        href: href.into(),
        label: tr(label).into(),
        glyph,
        exact,
        count: None,
    }
}

/// The navigation groups, Subscriptions among them when the CDR serves it.
fn groups(subscriptions: bool) -> Vec<Group> {
    let mut console = vec![
        entry("/server", Msg::NavServer, Glyph::SERVER, false),
        entry("/audit", Msg::NavAudit, Glyph::SHIELD_CHECK, false),
    ];
    if subscriptions {
        console.push(entry(
            "/subscriptions",
            Msg::NavSubscriptions,
            Glyph::RADIO_TOWER,
            false,
        ));
    }
    console.push(entry(
        "/configuration",
        Msg::NavConfiguration,
        Glyph::FILE_TEXT,
        false,
    ));
    console.push(entry("/about", Msg::NavAbout, Glyph::INFO, true));
    vec![
        Group {
            id: "nav-overview",
            title: tr(Msg::NavGroupOverview).into(),
            entries: vec![entry("/", Msg::NavDashboard, Glyph::LAYOUT_DASHBOARD, true)],
        },
        Group {
            id: "nav-records",
            title: tr(Msg::NavGroupRecords).into(),
            entries: vec![
                entry("/ehrs", Msg::NavEhrs, Glyph::DATABASE, false),
                entry("/parties", Msg::NavParties, Glyph::USERS, false),
            ],
        },
        Group {
            id: "nav-definitions",
            title: tr(Msg::NavGroupDefinitions).into(),
            entries: vec![
                entry("/templates", Msg::NavTemplates, Glyph::FILE_CODE, false),
                entry("/terminology", Msg::NavTerminology, Glyph::BOOK_A, false),
            ],
        },
        Group {
            id: "nav-queries",
            title: tr(Msg::NavGroupQueries).into(),
            entries: vec![
                entry("/queries", Msg::NavStoredQueries, Glyph::SEARCH_CODE, false),
                entry("/query", Msg::NavQuery, Glyph::SQUARE_TERMINAL, false),
            ],
        },
        Group {
            id: "nav-console",
            title: tr(Msg::NavGroupConsole).into(),
            entries: console,
        },
    ]
}

/// The navigation, with the page at `current` marked.
fn navigation(current: &str, subscriptions: bool) -> AnyView {
    sidebar(Sidebar {
        id: SIDEBAR_ID,
        label: tr(Msg::NavLabel),
        groups: groups(subscriptions),
        current: current.to_owned(),
        rail: Some(Rail {
            collapsed: crate::prefs::rail(),
            action: crate::prefs::SIDEBAR_ROUTE.to_owned(),
            back: crate::request::path_and_query(),
        }),
        nest: None,
    })
}

/// The CDR's status as the top bar's pill says it, from the status document.
fn pill(status: Result<String, crate::error::ViewerError>) -> AnyView {
    let state = crate::request::state();
    let address = state
        .as_ref()
        .map(|state| state.config.cdr.base_url.clone())
        .unwrap_or_default();
    let (reach, words, detail) = match status {
        Ok(body) => {
            let document = crate::deployment::StatusDocument::parse(&body);
            let version = document
                .as_ref()
                .and_then(|document| document.server_version.clone())
                .unwrap_or_else(|| tr(Msg::UnknownVersion));
            let profile = document
                .as_ref()
                .and_then(|document| document.profile().map(str::to_owned));
            let detail = match profile {
                Some(profile) => tr_with(
                    Msg::PillDetailProfile,
                    &[
                        ("version", version.into()),
                        ("address", address.into()),
                        ("profile", profile.into()),
                    ],
                ),
                None => tr_with(
                    Msg::PillDetail,
                    &[("version", version.into()), ("address", address.into())],
                ),
            };
            (Reach::Up, tr(Msg::PillUp), Some(detail))
        }
        Err(crate::error::ViewerError::CdrUnauthorized(_)) => {
            (Reach::Refused, tr(Msg::PillRefused), None)
        }
        Err(crate::error::ViewerError::CdrUnreachable(_)) => (
            Reach::Unreachable,
            tr(Msg::PillUnreachable),
            Some(tr_with(
                Msg::PillUnreachableDetail,
                &[("address", address.into())],
            )),
        ),
        Err(_) => (Reach::Unknown, tr(Msg::PillUnknown), None),
    };
    status_pill(StatusPill {
        reach,
        words,
        detail,
    })
    .into_any()
}

/// The sandbox band under the top bar, when the CDR declares the sandbox
/// deployment profile.
fn sandbox_band(status: Option<&Result<String, crate::error::ViewerError>>) -> Option<AnyView> {
    let body = status?.as_ref().ok()?;
    let document = crate::deployment::StatusDocument::parse(body)?;
    let notice = document.sandbox_notice()?;
    let line = if notice.gaps.is_empty() {
        tr(Msg::SandboxNoGaps)
    } else {
        tr_with(
            Msg::SandboxGaps,
            &[(
                "count",
                u64::try_from(notice.gaps.len()).unwrap_or(u64::MAX).into(),
            )],
        )
    };
    Some(banner(
        Tone::Warn,
        Glyph::TRIANGLE_ALERT,
        tr(Msg::SandboxTitle),
        &line,
    ))
}

/// The user menu: who is signed in, Your preferences, the theme, Keyboard
/// shortcuts and Sign out.
fn menu(session: Option<&crate::session::ViewerSession>) -> AnyView {
    let (who, line) = match session {
        Some(session) => {
            let how = match session.credential {
                crate::session::Credential::Basic { .. } => tr(Msg::SignedInBasic),
                crate::session::Credential::Bearer { .. } => tr(Msg::SignedInOidc),
            };
            (session.identity.clone(), how)
        }
        None => (tr(Msg::YourMenu), String::new()),
    };
    user_menu(UserMenu {
        id: USER_MENU_ID,
        label: who.clone(),
        name: Some(who.clone()),
        who,
        line,
        preferences: "/preferences".to_owned(),
        theme: ThemeChoice {
            current: crate::prefs::current().theme,
            action: crate::prefs::PREFERENCE_ROUTE.to_owned(),
            back: crate::request::path_and_query(),
        },
        choices: Vec::new(),
        shortcuts: Some("/preferences/keyboard".to_owned()),
        sign_out: Some(SIGN_OUT_ROUTE.to_owned()),
    })
}

/// The top bar's search box, its answer open under it when the page answers
/// a search.
fn search(answer: Option<SearchAnswer>, typed: String, disabled: Option<String>) -> AnyView {
    search_box(
        SearchBox {
            id: "find".to_owned(),
            name: "q".to_owned(),
            label: tr(Msg::SearchLabel),
            placeholder: tr(Msg::SearchPlaceholder),
            action: FIND_ROUTE.to_owned(),
            value: typed,
            submit: tr(Msg::SearchSubmit).into(),
            shortcut: Some('/'),
            busy: false,
            disabled,
            post: true,
        },
        answer,
    )
}

/// The shell of every signed-in page.
///
/// The top bar's pill, the band and the navigation wait for the CDR in their
/// own `Suspense`; the page itself is drawn once, outside them.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn AppShell() -> impl IntoView {
    let session = crate::request::session();
    let current = crate::request::path();
    let dock = Dock::default();
    provide_context(dock.clone());
    let status = Status::default();
    provide_context(status.clone());
    let typed = crate::posted::posted()
        .filter(|_| current == FIND_ROUTE)
        .map(|posted| posted.text("q"))
        .filter(|typed| !typed.is_empty());
    let end = {
        let status = status.clone();
        crate::pages::common::awaited(async move {
            let answer = status.read().await;
            view! {
                {pill(answer)}
                {menu(session.as_ref())}
            }
            .into_any()
        })
    };
    let search_part = {
        let status = status.clone();
        crate::pages::common::awaited(async move {
            let down = matches!(
                status.read().await,
                Err(crate::error::ViewerError::CdrUnreachable(_))
            );
            let disabled = down.then(|| tr(Msg::SearchWaits));
            search(None, typed.unwrap_or_default(), disabled)
        })
    };
    let band = {
        let status = status.clone();
        crate::pages::common::awaited(async move {
            let answer = status.read().await;
            sandbox_band(Some(&answer)).unwrap_or_else(|| ().into_any())
        })
    };
    let nav = {
        let current = current.clone();
        crate::pages::common::awaited(async move {
            let offered = crate::subscriptions::probe_event_subscriptions()
                .await
                .is_ok_and(|availability| availability == crate::subscriptions::SubscriptionAvailability::Available);
            navigation(&current, offered)
        })
    };
    let docked = {
        let dock = dock.clone();
        crate::pages::common::awaited(async move {
            dock.settled().await.map(decision_bar).into_any()
        })
    };
    shell(Shell {
        topbar: topbar(TopBar {
            sidebar: SIDEBAR_ID,
            lockup: lockup_view(),
            search: Some(search_part),
            end,
        }),
        banner: Some(band),
        sidebar: nav,
        content: view! { <Outlet /> }.into_any(),
        dock: Some(docked),
        footer: footer(),
    })
}

/// The theme a page is drawn in, for a part that previews it.
#[must_use]
pub fn theme_in_force() -> Theme {
    crate::prefs::current().theme
}
