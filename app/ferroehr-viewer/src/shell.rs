// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shell every signed-in page is drawn in: the top bar, the navigation,
//! the sandbox band, the page, its docked decision bar, the toasts and the
//! footer.
//!
//! The parts are FerroUI's (`view::shell`, `view::nav`, `view::menu`,
//! `view::status`, `view::search`, `view::toast`): the top bar carries the
//! search, the CDR's status pill and the user menu, and no theme control
//! (family rule 5). The navigation follows the approved information
//! architecture of the viewer's mockup (FerroUI `design/consoles/ferroehr/`):
//! Overview, Records, Definitions, Queries and the Console group.
//! Subscriptions is left out when the CDR does not serve its API.
//!
//! Every page handler draws the shell around its page, with the page at the
//! request's path marked (FerroUI `docs/consoles.md`, "How a console
//! builds", step 2). The session is read from the sealed cookie with no CDR
//! request; the session guard ([`crate::server::login_guard`]) has already
//! sent a request without one to the sign-in page. The status pill reads the
//! CDR's status document once per page ([`status`]): the viewer polls
//! nothing in the background, so an open tab's idle window measures what the
//! reader does (#3057). No openEHR spec governs the viewer: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::theme::Theme;
use ferrohealth_ui::tone::Tone;
use ferrohealth_ui::view::dock::{DecisionBar, decision_bar};
use ferrohealth_ui::view::menu::{ThemeChoice, UserLink, UserMenu, user_menu};
use ferrohealth_ui::view::nav::{Entry, Group, Rail, Sidebar, sidebar};
use ferrohealth_ui::view::search::{SearchBox, search_box};
use ferrohealth_ui::view::shell::{
    Footer, Shell, TopBar, brand_mark, centred, lockup, shell, topbar,
};
use ferrohealth_ui::view::status::{BannerLink, Reach, StatusPill, banner, status_pill};
use ferrohealth_ui::view::toast::toast_region;

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

/// The route the panel under the search box asks as you type
/// ([`crate::pages::find::find_answer`]).
pub const FIND_ANSWER_ROUTE: &str = "/api/find_answer";

/// The glyph of Stored queries in the navigation, on the dashboard and in
/// the search's answers.
pub const STORED_QUERIES_GLYPH: Glyph = Glyph::SCROLL_TEXT;

/// Server › Overview, where the sandbox band's Details lead.
const SERVER_ROUTE: &str = "/server";

/// Docks `bar` at the foot of the page being drawn (R4).
pub fn dock(bar: DecisionBar) {
    let drawn = decision_bar(bar);
    crate::scope::with(|scope| scope.answer(|answer| answer.dock = Some(drawn)));
}

/// The CDR's status document for the page being drawn, read once: the
/// shell's reading and a page that shows the deployment read the same answer.
///
/// # Errors
///
/// The CDR's refusal of the status read, or its being out of reach.
pub async fn status() -> Result<String, crate::error::ViewerError> {
    match crate::scope::current() {
        Some(scope) => scope
            .status
            .get_or_init(crate::auth::fetch_status)
            .await
            .clone(),
        None => crate::auth::fetch_status().await,
    }
}

/// The viewer's product name and the lockup's accented word.
fn lockup_view() -> Html {
    lockup(
        "/",
        brand_mark(MARK_PATH, MARK_PATH),
        tr(Msg::BrandName),
        tr(Msg::BrandWord),
    )
}

/// The footer of every page: the product, its version and About.
#[must_use]
pub fn footer() -> Footer {
    Footer {
        product: tr(Msg::ProductName).into(),
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
        entry(SERVER_ROUTE, Msg::NavServer, Glyph::SERVER, false),
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
                entry("/terminology", Msg::NavTerminology, Glyph::BOOK_OPEN, false),
            ],
        },
        Group {
            id: "nav-queries",
            title: tr(Msg::NavGroupQueries).into(),
            entries: vec![
                entry(
                    "/queries",
                    Msg::NavStoredQueries,
                    STORED_QUERIES_GLYPH,
                    false,
                ),
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
fn navigation(current: &str, subscriptions: bool) -> Html {
    sidebar(
        &crate::locale::current(),
        Sidebar {
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
        },
    )
}

/// How the CDR answered the status read, as the pill says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillReach {
    /// The CDR answered.
    Up,
    /// The CDR refused the session's credential.
    Refused,
    /// The CDR is out of reach.
    Unreachable,
    /// The answer said nothing the pill can read.
    Unknown,
}

/// What the shell draws of the CDR's status document and of its
/// subscriptions surface, read by the server as it draws the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellReading {
    /// How the pill says the CDR answered.
    pub reach: PillReach,
    /// The pill's words.
    pub words: String,
    /// The pill's detail.
    pub detail: Option<String>,
    /// The sandbox band's line, when the CDR declares the sandbox profile.
    pub sandbox: Option<String>,
    /// Whether the CDR serves the subscriptions API.
    pub subscriptions: bool,
}

impl ShellReading {
    /// The shell's reading of the status read `status`, with `subscriptions`
    /// saying whether the CDR serves that API.
    fn of(status: &Result<String, crate::error::ViewerError>, subscriptions: bool) -> Self {
        let address = crate::request::settings().cdr_address;
        let document = status
            .as_ref()
            .ok()
            .and_then(|body| crate::deployment::StatusDocument::parse(body));
        let (reach, words, detail) = match status {
            Ok(_) => {
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
                (PillReach::Up, tr(Msg::PillUp), Some(detail))
            }
            Err(crate::error::ViewerError::CdrUnauthorized(_)) => {
                (PillReach::Refused, tr(Msg::PillRefused), None)
            }
            Err(crate::error::ViewerError::CdrUnreachable(_)) => (
                PillReach::Unreachable,
                tr(Msg::PillUnreachable),
                Some(tr_with(
                    Msg::PillUnreachableDetail,
                    &[("address", address.into())],
                )),
            ),
            Err(_) => (PillReach::Unknown, tr(Msg::PillUnknown), None),
        };
        let sandbox = document
            .as_ref()
            .and_then(crate::deployment::StatusDocument::sandbox_notice)
            .map(|notice| {
                if notice.gaps.is_empty() {
                    tr(Msg::SandboxNoGaps)
                } else {
                    tr_with(
                        Msg::SandboxGaps,
                        &[(
                            "count",
                            u64::try_from(notice.gaps.len()).unwrap_or(u64::MAX).into(),
                        )],
                    )
                }
            });
        Self {
            reach,
            words,
            detail,
            sandbox,
            subscriptions,
        }
    }
}

/// What the shell draws of the CDR: the status read once for the page
/// ([`status`]) and whether the CDR serves the subscriptions API.
async fn shell_reading() -> ShellReading {
    let status = status().await;
    let subscriptions = crate::subscriptions::renders_event_subscriptions(
        &crate::subscriptions::probe_event_subscriptions().await,
    );
    ShellReading::of(&status, subscriptions)
}

/// The top bar's pill for the shell's reading `status`.
fn pill(status: &ShellReading) -> Html {
    let reach = match status.reach {
        PillReach::Up => Reach::Up,
        PillReach::Refused => Reach::Refused,
        PillReach::Unreachable => Reach::Unreachable,
        PillReach::Unknown => Reach::Unknown,
    };
    status_pill(StatusPill {
        reach,
        words: status.words.clone(),
        detail: status.detail.clone(),
    })
}

/// The sandbox band under the top bar, saying `line` when the CDR declares
/// the sandbox deployment profile, its Details leading to Server › Overview
/// unless the page at `current` is that one.
fn sandbox_band(line: Option<&String>, current: &str) -> Option<Html> {
    let line = line?;
    Some(banner(
        Tone::Warn,
        Glyph::TRIANGLE_ALERT,
        tr(Msg::SandboxTitle),
        line,
        (current != SERVER_ROUTE).then(|| BannerLink {
            words: tr(Msg::SandboxDetails),
            href: SERVER_ROUTE.to_owned(),
        }),
    ))
}

/// The user menu: who is signed in, Your preferences, Your access, the
/// theme, Keyboard shortcuts and Sign out.
fn menu(session: Option<&crate::request::Signed>) -> Html {
    let (who, line) = match session {
        Some(session) => {
            let how = match session.method() {
                crate::request::Method::Basic => tr(Msg::SignedInBasic),
                crate::request::Method::Oidc => tr(Msg::SignedInOidc),
            };
            (session.identity.clone(), how)
        }
        None => (tr(Msg::YourMenu), String::new()),
    };
    user_menu(
        &crate::locale::current(),
        UserMenu {
            id: USER_MENU_ID,
            label: who.clone(),
            name: Some(who.clone()),
            who,
            line,
            preferences: "/preferences".to_owned(),
            links: vec![UserLink {
                words: tr(Msg::YourAccess),
                href: "/access".to_owned(),
                glyph: Glyph::KEY_ROUND,
            }],
            theme: ThemeChoice {
                current: crate::prefs::current().theme,
                action: crate::prefs::PREFERENCE_ROUTE.to_owned(),
                back: crate::request::path_and_query(),
            },
            choices: Vec::new(),
            shortcuts: Some("/preferences/keyboard".to_owned()),
            sign_out: Some(SIGN_OUT_ROUTE.to_owned()),
        },
    )
}

/// The top bar's search box, holding `typed` when the page answers a search.
fn search(typed: String, disabled: Option<String>) -> Html {
    search_box(
        &crate::locale::current(),
        &SearchBox {
            id: "find".to_owned(),
            name: "q".to_owned(),
            label: tr(Msg::SearchLabel),
            placeholder: tr(Msg::SearchPlaceholder),
            action: FIND_ROUTE.to_owned(),
            value: typed,
            submit: Some(tr(Msg::SearchSubmit).into()),
            shortcut: Some('/'),
            busy: false,
            disabled,
            post: true,
            suggest: Some(String::from(FIND_ANSWER_ROUTE)),
            suggest_from: 2,
            all: None,
            signed_out: Some(crate::session_client::signed_out_url("/", "")),
            glyphs: Vec::new(),
        },
    )
}

/// The flash toasts the page shows as it arrives.
fn toasts() -> Html {
    toast_region(&crate::locale::current(), crate::flash::take())
}

/// The page `content` inside the shell of a signed-in page, with the page
/// at the request's path marked and the bar the page docked at its foot.
pub async fn framed(content: Html) -> Html {
    let current = crate::request::path();
    let reading = shell_reading().await;
    let typed = crate::posted::posted()
        .filter(|_| current == FIND_ROUTE)
        .map(|posted| posted.text("q"))
        .filter(|typed| !typed.is_empty());
    let disabled = (reading.reach == PillReach::Unreachable).then(|| tr(Msg::SearchWaits));
    let end = pill(&reading).then(menu(crate::request::signed().as_ref()));
    let dock = crate::scope::with(|scope| scope.answer(|answer| answer.dock.take())).flatten();
    let locale = crate::locale::current();
    shell(
        &locale,
        Shell {
            topbar: topbar(
                &locale,
                TopBar {
                    sidebar: SIDEBAR_ID,
                    lockup: lockup_view(),
                    search: Some(search(typed.unwrap_or_default(), disabled)),
                    end,
                },
            ),
            banner: sandbox_band(reading.sandbox.as_ref(), &current),
            sidebar: navigation(&current, reading.subscriptions),
            content,
            dock,
            toasts: toasts(),
            footer: footer(),
        },
    )
}

/// The page `content` centred with no shell: the sign-in page and an error
/// page drawn for nobody signed in.
#[must_use]
pub fn centred_page(content: Html) -> Html {
    centred(&crate::locale::current(), content, toasts(), footer())
}

/// The theme a page is drawn in, for a part that previews it.
#[must_use]
pub fn theme_in_force() -> Theme {
    crate::prefs::current().theme
}
