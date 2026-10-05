// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared E2E journey harness: env-gated `WebDriver` setup (skip-with-reason
//! when the stack isn't up), step screenshots, explicit waits, and the
//! standing browser-console gate — every journey fails on any console
//! error (the cheapest hydration-bug detector).
//!
//! Two lanes drive this harness, and they are not equals. The IMAGE lane is
//! the authoritative one: it runs the shipped OCI artifact, whose WASM is the
//! release build. The HOST lane serves a debug-profile WASM bundle an order of
//! magnitude larger, trading hydration latency for compile speed so a change
//! can be driven in seconds — which is why [`Harness::wait_hydrated`] carries
//! its own, far longer budget than the element waits around it.

#![allow(
    clippy::panic,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "the shared e2e fixture panics when a configured stack cannot be \
              driven, and the skip-with-reason lines ARE this suite's report \
              (the clippy.toml test scoping reaches only #[test] fns, not \
              integration-binary helper modules)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

use std::future::Future;
use std::time::Duration;

use thirtyfour::components::SelectElement;
use thirtyfour::error::WebDriverErrorInner;
use thirtyfour::prelude::*;

/// The budget every ordinary element wait allows.
const WAIT: Duration = Duration::from_secs(15);

/// The budget one steady-state `WebDriver` command gets ([`bounded`]).
///
/// Two thirds of [`WAIT`], so a command that never answers fails INSIDE the
/// poll loop that issued it and still leaves that loop a third of its budget
/// to report the failure — instead of running out thirtyfour's 120 s
/// per-request default and inflating the whole journey.
const COMMAND_BUDGET: Duration = Duration::from_secs(10);

/// The interval every poll loop here re-reads its condition at.
const POLL_STEP: Duration = Duration::from_millis(200);

/// The budget [`Harness::wait_hydrated`] allows, four times [`WAIT`]: the host
/// lane's debug WASM is ~91 MB and the browser has to fetch, compile and run it
/// before the marker appears, which is comfortably slower than any wait that
/// only observes a rendered page (module docs).
const HYDRATION_WAIT: Duration = Duration::from_mins(1);

/// Everything a journey needs.
pub(crate) struct Harness {
    /// The `WebDriver` session, private so every journey command goes
    /// through a helper that bounds it ([`bounded`]) and reads a failure to
    /// answer as a failure ([`is_absence`]).
    driver: WebDriver,
    /// The viewer origin (`http://…`).
    pub(crate) base: String,
    shots_dir: String,
    journey: &'static str,
    /// Whether this session runs with JavaScript enabled — hydrated pages
    /// only exist in that mode, so [`Harness::goto`] waits for the shell's
    /// hydration marker exactly when one will ever appear.
    js: bool,
}

/// Environment lookup for a journey credential/URL.
#[expect(
    clippy::disallowed_methods,
    reason = "the E2E harness is configured by the environment the CI job / scripts/ui-e2e.sh exports; there is no viewer config tree on the test side"
)]
pub(crate) fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

impl Harness {
    /// Start a journey: returns `None` (with a printed reason) when the
    /// harness env is absent, so plain `cargo nextest run` stays green.
    ///
    /// # Panics
    /// When the stack env is set but the browser session cannot start —
    /// that IS a failure, not a skip.
    pub(crate) async fn start(journey: &'static str) -> Option<Self> {
        let (Some(base), Some(webdriver_url)) =
            (env("UI_E2E_BASE_URL"), env("UI_E2E_WEBDRIVER_URL"))
        else {
            println!(
                "SKIP {journey}: UI_E2E_BASE_URL/UI_E2E_WEBDRIVER_URL unset (run scripts/ui-e2e.sh)"
            );
            return None;
        };
        let shots_dir =
            env("UI_E2E_SHOTS_DIR").unwrap_or_else(|| "target/ui-e2e/screenshots".to_owned());
        std::fs::create_dir_all(&shots_dir).expect("screenshot dir");

        let mut caps = DesiredCapabilities::chrome();
        caps.add_arg("--headless=new").expect("caps");
        caps.add_arg("--window-size=1440,900").expect("caps");
        // Image-mode OIDC: the composed viewer advertises the in-network
        // issuer host (`keycloak`); the browser resolves it to the host-
        // mapped port. A no-op in host mode (nothing references the name).
        caps.add_arg("--host-resolver-rules=MAP keycloak 127.0.0.1")
            .expect("caps");
        caps.set_logging_prefs("browser", thirtyfour::LoggingPrefsLogLevel::All)
            .expect("logging prefs");
        // NOTE: session creation keeps thirtyfour's own 120 s request timeout —
        // a budget short enough for a steady-state command is exceeded here
        // under nextest parallelism; those commands are bounded by `bounded`.
        let driver = WebDriver::builder(&webdriver_url, caps)
            .await
            .expect("webdriver session (is chromedriver up?)");
        Some(Self {
            driver,
            base,
            shots_dir,
            journey,
            js: true,
        })
    }

    /// Navigate to a viewer path.
    ///
    /// # Panics
    /// On navigation failure (journeys are assertive end-to-end).
    pub(crate) async fn goto(&self, path: &str) {
        // Sweep the browser console BEFORE leaving the page: a SEVERE entry
        // is attributed to the page that produced it, not discovered by the
        // end-of-journey sweep with no locality (`get_log` drains, so the
        // final sweep still covers everything after the last navigation).
        let leaving = bounded("current_url()", self.driver.current_url())
            .await
            .map(|u| u.to_string())
            .unwrap_or_default();
        let entries = bounded("get_log(browser)", self.driver.get_log("browser"))
            .await
            .expect("browser log (chromedriver legacy endpoint)");
        let severe: Vec<String> = entries
            .into_iter()
            .filter(|e| e.level == "SEVERE")
            .map(|e| e.message)
            .filter(|m| !m.contains("Failed to load resource"))
            .collect();
        assert!(
            severe.is_empty(),
            "browser console has SEVERE entries on {leaving} (before navigating to {path}):\n{}",
            severe.join("\n")
        );
        bounded("goto()", self.driver.goto(format!("{}{path}", self.base)))
            .await
            .expect("navigate");
        // Every full navigation restarts hydration, and any first click or
        // file selection landing before it completes is silently lost —
        // unrecoverably for same-value re-sends (#2285). Waiting here makes
        // every journey's first interaction land on live listeners; the
        // no-JS sessions skip it, since their pages never hydrate.
        if self.js {
            self.wait_hydrated().await;
        }
    }

    /// Explicit wait: the first element matching `css`, within [`WAIT`].
    ///
    /// # Panics
    /// When the element never appears — with the selector in the message.
    pub(crate) async fn wait_css(&self, css: &str) -> Element {
        self.wait_css_for(css, WAIT).await
    }

    /// [`Self::wait_css`] with an explicit budget, so the hydration wait can be
    /// long without lengthening every other wait.
    ///
    /// # Panics
    /// When the element never appears — with the selector in the message.
    async fn wait_css_for(&self, css: &str, budget: Duration) -> Element {
        match bounded_for(
            &format!("query(css={css}).wait()"),
            budget + COMMAND_BUDGET,
            self.driver
                .query(By::Css(css))
                .wait(budget, Duration::from_millis(200))
                .first(),
        )
        .await
        {
            Ok(inner) => Element::new(inner, css),
            Err(e) => {
                // Failure evidence: where the browser actually was, and what
                // its console says. The console half is what separates "the
                // screen simply has not got there yet" from "the client
                // runtime is dead": an unrecoverable hydration error traps the
                // WASM module, after which no client-side navigation completes
                // and the page only moves on a full reload. Without it a
                // timeout reports a missing selector and hides its cause.
                let url = bounded("current_url()", self.driver.current_url())
                    .await
                    .map(|u| u.to_string())
                    .unwrap_or_default();
                let path = format!("{}/{}-fail.png", self.shots_dir, self.journey);
                drop(
                    bounded(
                        "screenshot()",
                        self.driver.screenshot(std::path::Path::new(&path)),
                    )
                    .await,
                );
                let console: Vec<String> =
                    bounded("get_log(browser)", self.driver.get_log("browser"))
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|entry| entry.level == "SEVERE")
                        .map(|entry| entry.message)
                        .collect();
                let console = if console.is_empty() {
                    "  (no SEVERE console entries)".to_owned()
                } else {
                    console
                        .iter()
                        .map(|m| format!("  {m}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                panic!("waiting for `{css}` at {url}: {e}\nbrowser console:\n{console}");
            }
        }
    }

    /// Start a journey with JavaScript DISABLED (the progressive-enhancement
    /// contract: SSR + plain HTML forms must work before WASM ever loads).
    ///
    /// # Panics
    /// When the stack env is set but the browser session cannot start.
    pub(crate) async fn start_without_javascript(journey: &'static str) -> Option<Self> {
        let (Some(base), Some(webdriver_url)) =
            (env("UI_E2E_BASE_URL"), env("UI_E2E_WEBDRIVER_URL"))
        else {
            println!(
                "SKIP {journey}: UI_E2E_BASE_URL/UI_E2E_WEBDRIVER_URL unset (run scripts/ui-e2e.sh)"
            );
            return None;
        };
        let shots_dir =
            env("UI_E2E_SHOTS_DIR").unwrap_or_else(|| "target/ui-e2e/screenshots".to_owned());
        std::fs::create_dir_all(&shots_dir).expect("screenshot dir");

        let mut caps = DesiredCapabilities::chrome();
        caps.add_arg("--headless=new").expect("caps");
        caps.add_arg("--window-size=1440,900").expect("caps");
        // Image-mode OIDC: the composed viewer advertises the in-network
        // issuer host (`keycloak`); the browser resolves it to the host-
        // mapped port. A no-op in host mode (nothing references the name).
        caps.add_arg("--host-resolver-rules=MAP keycloak 127.0.0.1")
            .expect("caps");
        // Chrome content-settings: 2 = block JavaScript.
        caps.add_experimental_option(
            "prefs",
            serde_json::json!({"profile.managed_default_content_settings.javascript": 2}),
        )
        .expect("prefs");
        // NOTE: session creation keeps thirtyfour's own 120 s request timeout —
        // a budget short enough for a steady-state command is exceeded here
        // under nextest parallelism; those commands are bounded by `bounded`.
        let driver = WebDriver::builder(&webdriver_url, caps)
            .await
            .expect("webdriver session (is chromedriver up?)");
        Some(Self {
            driver,
            base,
            shots_dir,
            journey,
            js: false,
        })
    }

    /// Wait until the current URL no longer contains `fragment`.
    ///
    /// # Panics
    /// When the URL still matches after 15 s.
    pub(crate) async fn wait_url_not_contains(&self, fragment: &str) {
        for _ in 0..75 {
            let url = bounded("current_url()", self.driver.current_url())
                .await
                .expect("current url");
            if !url.as_str().contains(fragment) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let url = bounded("current_url()", self.driver.current_url())
            .await
            .expect("current url");
        panic!("URL still contains `{fragment}` (last: {url})");
    }

    /// Explicit wait on an `XPath` (same budget + failure evidence as
    /// [`Self::wait_css`]).
    ///
    /// # Panics
    /// When the element never appears.
    pub(crate) async fn wait_xpath(&self, xpath: &str) -> Element {
        match bounded_for(
            &format!("query(xpath={xpath}).wait()"),
            WAIT + COMMAND_BUDGET,
            self.driver
                .query(By::XPath(xpath))
                .wait(WAIT, Duration::from_millis(200))
                .first(),
        )
        .await
        {
            Ok(inner) => Element::new(inner, xpath),
            Err(e) => {
                let url = bounded("current_url()", self.driver.current_url())
                    .await
                    .map(|u| u.to_string())
                    .unwrap_or_default();
                let path = format!("{}/{}-fail.png", self.shots_dir, self.journey);
                drop(
                    bounded(
                        "screenshot()",
                        self.driver.screenshot(std::path::Path::new(&path)),
                    )
                    .await,
                );
                panic!("waiting for xpath `{xpath}` at {url}: {e}");
            }
        }
    }

    /// Explicit wait on an `XPath` that additionally requires the element to be
    /// CLICKABLE — displayed and enabled — before returning it.
    ///
    /// A control the viewer disables until its form is valid is already
    /// PRESENT, so [`Self::wait_xpath`] hands it back and the click is
    /// INTERCEPTED by whatever sits above it. That is an error rather than a
    /// lost interaction, so the re-click loop other journeys use for
    /// pre-hydration clicks does not cover it — the condition has to be part of
    /// the wait.
    ///
    /// # Panics
    /// When the element never becomes clickable.
    pub(crate) async fn wait_clickable_xpath(&self, xpath: &str) -> Element {
        match bounded_for(
            &format!("query(xpath={xpath}).and_clickable().wait()"),
            WAIT + COMMAND_BUDGET,
            self.driver
                .query(By::XPath(xpath))
                .and_clickable()
                .wait(WAIT, Duration::from_millis(200))
                .first(),
        )
        .await
        {
            Ok(inner) => Element::new(inner, xpath),
            Err(e) => {
                let url = bounded("current_url()", self.driver.current_url())
                    .await
                    .map(|u| u.to_string())
                    .unwrap_or_default();
                let path = format!("{}/{}-fail.png", self.shots_dir, self.journey);
                drop(
                    bounded(
                        "screenshot()",
                        self.driver.screenshot(std::path::Path::new(&path)),
                    )
                    .await,
                );
                panic!("waiting for xpath `{xpath}` to become clickable at {url}: {e}");
            }
        }
    }

    /// Wait until the current URL contains `fragment` (redirect chains).
    ///
    /// # Panics
    /// When the URL never matches within [`WAIT`].
    pub(crate) async fn wait_url_contains(&self, fragment: &str) {
        self.wait_url_contains_for(fragment, WAIT).await;
    }

    /// [`Self::wait_url_contains`] with an explicit budget, for a transition
    /// the browser makes on its OWN schedule rather than in answer to a click
    /// — the session-expiry journeys wait out a whole poll interval.
    ///
    /// # Panics
    /// When the URL never matches within `budget`.
    pub(crate) async fn wait_url_contains_for(&self, fragment: &str, budget: Duration) {
        let step = Duration::from_millis(200);
        let mut waited = Duration::ZERO;
        loop {
            let url = bounded("current_url()", self.driver.current_url())
                .await
                .expect("current url");
            if url.as_str().contains(fragment) {
                return;
            }
            assert!(
                waited < budget,
                "URL never contained `{fragment}` (last: {url})"
            );
            tokio::time::sleep(step).await;
            waited += step;
        }
    }

    /// Wait until no toast card is on screen (a visible `thaw` toast overlays
    /// the bottom-right corner and INTERCEPTS clicks on buttons underneath —
    /// an explicit condition, not a sleep). Toasts auto-dismiss; bounded wait.
    ///
    /// # Panics
    /// When a toast is still visible after 15 s, or when the `WebDriver` stops
    /// answering — a stalled driver used to look exactly like a cleared screen.
    pub(crate) async fn wait_toasts_cleared(&self) {
        for _ in 0..75 {
            if count_matching(self, TOAST_CARD).await == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("a toast never cleared (it would intercept the next click)");
    }

    /// Wait until client hydration has completed on the current page (the
    /// shell stamps `data-hydrated` on `<body>` from a browser-only effect).
    /// Required before driving any control whose handler exists only
    /// hydrated — a click or file selection landing earlier is silently
    /// lost, and a same-value re-send fires no later event (#2285).
    ///
    /// This one wait gets [`HYDRATION_WAIT`] rather than [`WAIT`]: it is the
    /// only condition whose latency is dominated by the WASM bundle's size,
    /// which differs by an order of magnitude between the two lanes (module
    /// docs).
    pub(crate) async fn wait_hydrated(&self) {
        self.wait_css_for("body[data-hydrated]", HYDRATION_WAIT)
            .await;
    }

    /// Failure evidence at a journey-defined point, for a panic that would
    /// otherwise preempt the next `goto`'s console sweep: a screenshot, the
    /// DRAINED browser console printed to the test log (hydration errors and
    /// panics land there), and any visible message-bar text — returned as one
    /// line for the panic message.
    pub(crate) async fn evidence_dump(&self, slug: &str) -> String {
        let url = bounded("current_url()", self.driver.current_url())
            .await
            .map(|u| u.to_string())
            .unwrap_or_default();
        let path = format!("{}/{}-{slug}.png", self.shots_dir, self.journey);
        drop(
            bounded(
                "screenshot()",
                self.driver.screenshot(std::path::Path::new(&path)),
            )
            .await,
        );
        let entries = bounded("get_log(browser)", self.driver.get_log("browser"))
            .await
            .unwrap_or_default();
        let mut severe = 0usize;
        for entry in &entries {
            if entry.level == "SEVERE" {
                severe += 1;
            }
            println!("console[{}]: {}", entry.level, entry.message);
        }
        // Evidence gathering, so this one read stays best-effort: panicking
        // here would destroy the report the caller is about to print.
        let bar = match bounded(
            "find(.thaw-message-bar)",
            self.driver.find(By::Css(".thaw-message-bar")),
        )
        .await
        {
            Ok(inner) => bounded("text(.thaw-message-bar)", inner.text())
                .await
                .unwrap_or_default(),
            Err(_) => String::new(),
        };
        format!(
            "at {url}; {} console entries ({severe} SEVERE — printed above); message bar: {bar:?}",
            entries.len()
        )
    }

    /// Numbered step screenshot: `{journey}-{step}-{slug}.png`.
    ///
    /// # Panics
    /// On capture/IO failure.
    pub(crate) async fn shot(&self, step: u8, slug: &str) {
        let path = format!("{}/{}-{step:02}-{slug}.png", self.shots_dir, self.journey);
        bounded(
            "screenshot()",
            self.driver.screenshot(std::path::Path::new(&path)),
        )
        .await
        .expect("screenshot");
    }

    /// The standing browser-console gate: read the browser log (thirtyfour's
    /// legacy-log support over chromedriver) and fail on any SEVERE entry
    /// (hydration errors and panics land there). Network 4xx from
    /// deliberate negative steps can be allowed by substring.
    ///
    /// # Panics
    /// When the log contains a SEVERE entry not covered by `allowed`.
    pub(crate) async fn assert_console_clean(&self, allowed: &[&str]) {
        let entries = bounded("get_log(browser)", self.driver.get_log("browser"))
            .await
            .expect("browser log (chromedriver legacy endpoint)");
        let severe: Vec<String> = entries
            .into_iter()
            .filter(|e| e.level == "SEVERE")
            .map(|e| format!("[ts={}] {}", e.timestamp, e.message))
            .filter(|m| !allowed.iter().any(|a| m.contains(a)))
            .collect();
        let at = bounded("current_url()", self.driver.current_url())
            .await
            .map(|u| u.to_string())
            .unwrap_or_default();
        assert!(
            severe.is_empty(),
            "browser console has SEVERE entries (last page: {at}):\n{}",
            severe.join("\n")
        );
    }

    /// The browser's current URL.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer.
    pub(crate) async fn current_url(&self) -> String {
        bounded("current_url()", self.driver.current_url())
            .await
            .expect("current url")
            .to_string()
    }

    /// The current page's tab title, as the browser renders it.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer.
    pub(crate) async fn title(&self) -> String {
        bounded("title()", self.driver.title())
            .await
            .expect("tab title")
    }

    /// The current page's serialized DOM source.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer.
    pub(crate) async fn page_source(&self) -> String {
        bounded("source()", self.driver.source())
            .await
            .expect("page source")
    }

    /// Write a full-window PNG of the current page to `path`.
    ///
    /// # Panics
    /// When the capture or the write fails.
    pub(crate) async fn screenshot_to(&self, path: &std::path::Path) {
        bounded("screenshot()", self.driver.screenshot(path))
            .await
            .expect("write the screenshot");
    }

    /// Every cookie the browser holds for the viewer origin.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer.
    pub(crate) async fn cookies(&self) -> Vec<Cookie> {
        bounded("get_all_cookies()", self.driver.get_all_cookies())
            .await
            .expect("the browser's cookies for the viewer origin")
    }

    /// Delete the cookie named `name` from the browser.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer.
    pub(crate) async fn delete_cookie(&self, name: &str) {
        bounded(
            &format!("delete_cookie({name})"),
            self.driver.delete_cookie(name.to_owned()),
        )
        .await
        .expect("delete the cookie");
    }

    /// End the session (screenshots + console gate are per-journey calls).
    pub(crate) async fn finish(self) {
        bounded("quit()", self.driver.quit()).await.expect("quit");
    }
}

/// One element handle, together with the selector that found it.
///
/// A journey never holds a raw `WebElement`: every element command it can
/// issue goes through this type, which bounds the request ([`bounded`]) and
/// names the selector in its failure — the type-level form of the private
/// `driver` field on [`Harness`]. Nothing outside this module can build one,
/// so the guarantee cannot be routed around.
///
/// Two tiers. The assertive methods panic on anything but success, because a
/// journey that asked for a click either got an answer or it did not, and a
/// driver that stalled is never "the button was inert". The `read_*` probes
/// exist for a poll loop or a branch a journey takes either way, and collapse
/// ONLY an absence ([`is_absence`]) to `None`.
pub(crate) struct Element {
    inner: WebElement,
    /// The selector this handle was found by, which IS the failure message —
    /// so no call site needs an `expect` string of its own.
    what: String,
}

impl Element {
    /// Wrap a freshly found handle with the selector that found it.
    fn new(inner: WebElement, what: &str) -> Self {
        Self {
            inner,
            what: what.to_owned(),
        }
    }

    /// Click it.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn click(&self) {
        element_command(&format!("click({})", self.what), self.inner.click()).await;
    }

    /// Send `keys` to it.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn send_keys(&self, keys: impl Into<TypingData>) {
        element_command(
            &format!("send_keys({})", self.what),
            self.inner.send_keys(keys),
        )
        .await;
    }

    /// Clear it with `WebDriver`'s own element-clear command.
    ///
    /// [`clear_field`] is the one to reach for on a field whose value the
    /// viewer binds — this command moves the DOM value without firing the
    /// events that binding listens for.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn clear(&self) {
        element_command(&format!("clear({})", self.what), self.inner.clear()).await;
    }

    /// Scroll it into the viewport.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn scroll_into_view(&self) {
        element_command(
            &format!("scroll_into_view({})", self.what),
            self.inner.scroll_into_view(),
        )
        .await;
    }

    /// Its rendered text.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn text(&self) -> String {
        element_command(&format!("text({})", self.what), self.inner.text()).await
    }

    /// The value of its `name` ATTRIBUTE — what the markup carries.
    ///
    /// An absent attribute is a defect here, not an empty string: a journey
    /// asking for `data-visible` is asserting about a hook the viewer is
    /// supposed to render, and `""` would let a missing hook pass as a value.
    /// [`Self::read_attr`] is the probe for an attribute that legitimately
    /// may not be there.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer, or the attribute is absent.
    pub(crate) async fn attr(&self, name: &str) -> String {
        let label = format!("attr({}[{name}])", self.what);
        element_command(&label, self.inner.attr(name.to_owned()))
            .await
            .unwrap_or_else(|| panic!("`{}` carries no `{name}` attribute", self.what))
    }

    /// The value of its `name` PROPERTY — a control's LIVE state, which a
    /// typed-in value moves and the rendered attribute does not.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer, or the property is absent.
    pub(crate) async fn prop(&self, name: &str) -> String {
        let label = format!("prop({}.{name})", self.what);
        element_command(&label, self.inner.prop(name.to_owned()))
            .await
            .unwrap_or_else(|| panic!("`{}` carries no `{name}` property", self.what))
    }

    /// Whether it is enabled.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success — a stalled
    /// driver is never "the control is inert".
    pub(crate) async fn enabled(&self) -> bool {
        element_command(
            &format!("is_enabled({})", self.what),
            self.inner.is_enabled(),
        )
        .await
    }

    /// Whether it is displayed.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn displayed(&self) -> bool {
        element_command(
            &format!("is_displayed({})", self.what),
            self.inner.is_displayed(),
        )
        .await
    }

    /// Whether it is selected (a checkbox, a radio, an `<option>`).
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn selected(&self) -> bool {
        element_command(
            &format!("is_selected({})", self.what),
            self.inner.is_selected(),
        )
        .await
    }

    /// The RESOLVED value of the CSS `property` — what the browser computed,
    /// not what the stylesheet declared.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn css_value(&self, property: &str) -> String {
        element_command(
            &format!("css_value({}.{property})", self.what),
            self.inner.css_value(property.to_owned()),
        )
        .await
    }

    /// The first element matching `by` INSIDE this one.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success, absence
    /// included — a journey reaching into a subtree is asserting the child is
    /// there ([`Self::read_find`] is the probe).
    pub(crate) async fn find(&self, by: By) -> Self {
        let what = format!("{} {by:?}", self.what);
        let inner = element_command(&format!("find({what})"), self.inner.find(by)).await;
        Self::new(inner, &what)
    }

    /// Every element matching `by` inside this one, empty when none match.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but an absence
    /// ([`is_absence`]): a query that was never answered is not an empty
    /// subtree.
    pub(crate) async fn find_all(&self, by: By) -> Vec<Self> {
        let what = format!("{} {by:?}", self.what);
        let found = element_probe(&format!("find_all({what})"), self.inner.find_all(by))
            .await
            .unwrap_or_default();
        found
            .into_iter()
            .enumerate()
            .map(|(index, inner)| Self::new(inner, &format!("{what}[{index}]")))
            .collect()
    }

    /// Select the `<option>` whose value is `value`.
    ///
    /// thirtyfour's `SelectElement` issues its own requests — building one
    /// reads the tag name, and each selection reads the options and clicks —
    /// so it is wrapped here rather than handed to a journey: outside this
    /// type those requests would carry thirtyfour's 120 s default alone.
    ///
    /// # Panics
    /// When the element is not a `<select>`, or the option cannot be chosen.
    pub(crate) async fn select_by_value(&self, value: &str) {
        let select = element_command(
            &format!("SelectElement::new({})", self.what),
            SelectElement::new(&self.inner),
        )
        .await;
        element_command(
            &format!("select_by_value({}, {value})", self.what),
            select.select_by_value(value),
        )
        .await;
    }

    /// Select the `<option>` at `index` ([`Self::select_by_value`] for the
    /// bounding rationale).
    ///
    /// # Panics
    /// When the element is not a `<select>`, or the option cannot be chosen.
    pub(crate) async fn select_by_index(&self, index: u32) {
        let select = element_command(
            &format!("SelectElement::new({})", self.what),
            SelectElement::new(&self.inner),
        )
        .await;
        element_command(
            &format!("select_by_index({}, {index})", self.what),
            select.select_by_index(index),
        )
        .await;
    }

    /// Its rendered text, or `None` when the handle went stale.
    ///
    /// The probe tier's one collapse, reasoned here for all four `read_*`
    /// methods: the handle was found a moment ago, so the only "not there"
    /// this read can meet is a subtree that re-rendered between the find and
    /// the read, which `WebDriver` reports as `stale element reference`
    /// ([`is_absence`]) and a poll loop must treat as "not yet". Every other
    /// failure is the driver not ANSWERING, and stays a panic — a probe that
    /// swallowed it would spend its whole budget reporting an empty page.
    pub(crate) async fn read_text(&self) -> Option<String> {
        element_probe(&format!("text({})", self.what), self.inner.text()).await
    }

    /// Its `name` attribute, or `None` when absent or the handle went stale
    /// ([`Self::read_text`] reasons the collapse).
    pub(crate) async fn read_attr(&self, name: &str) -> Option<String> {
        element_probe(
            &format!("attr({}[{name}])", self.what),
            self.inner.attr(name.to_owned()),
        )
        .await
        .flatten()
    }

    /// Its `name` property, or `None` when absent or the handle went stale
    /// ([`Self::read_text`] reasons the collapse).
    pub(crate) async fn read_prop(&self, name: &str) -> Option<String> {
        element_probe(
            &format!("prop({}.{name})", self.what),
            self.inner.prop(name.to_owned()),
        )
        .await
        .flatten()
    }

    /// The first element matching `by` inside this one, or `None` when
    /// nothing matches or the handle went stale ([`Self::read_text`] reasons
    /// the collapse).
    pub(crate) async fn read_find(&self, by: By) -> Option<Self> {
        let what = format!("{} {by:?}", self.what);
        element_probe(&format!("find({what})"), self.inner.find(by))
            .await
            .map(|inner| Self::new(inner, &what))
    }

    /// Whether it is enabled, or `None` when the handle went stale
    /// ([`Self::read_text`] reasons the collapse).
    pub(crate) async fn read_enabled(&self) -> Option<bool> {
        element_probe(
            &format!("is_enabled({})", self.what),
            self.inner.is_enabled(),
        )
        .await
    }

    /// Whether it is displayed, or `None` when the handle went stale
    /// ([`Self::read_text`] reasons the collapse).
    pub(crate) async fn read_displayed(&self) -> Option<bool> {
        element_probe(
            &format!("is_displayed({})", self.what),
            self.inner.is_displayed(),
        )
        .await
    }
}

/// The assertive tier's one implementation: run `command` under
/// [`COMMAND_BUDGET`] and panic on anything but success, naming the selector.
///
/// A free function rather than a method, so a plain unit test can prove the
/// tier's decision without a browser session.
///
/// # Panics
/// On any `WebDriver` failure, an absence included.
async fn element_command<T>(label: &str, command: impl Future<Output = WebDriverResult<T>>) -> T {
    match bounded(label, command).await {
        Ok(value) => value,
        Err(error) => panic!("the WebDriver could not answer `{label}`: {error}"),
    }
}

/// The probe tier's one implementation: `None` on an absence
/// ([`is_absence`]), panic on anything else ([`Element::read_text`] reasons
/// the collapse).
///
/// # Panics
/// On any `WebDriver` failure that is not an absence.
async fn element_probe<T>(
    label: &str,
    command: impl Future<Output = WebDriverResult<T>>,
) -> Option<T> {
    match bounded(label, command).await {
        Ok(value) => Some(value),
        Err(error) if is_absence(&error) => None,
        Err(error) => panic!("the WebDriver could not answer `{label}`: {error}"),
    }
}

/// Run one steady-state `WebDriver` command under [`COMMAND_BUDGET`].
///
/// thirtyfour bounds a request only by its 120 s per-request default, which
/// outlasts every poll loop here: a driver that never answers reads as a page
/// that never changed, and the journey reports an inflated run time instead of
/// the stall. Bounding the await is ours to do — the transport cannot be
/// bounded instead, because thirtyfour's `request_timeout` covers the
/// `NewSession` request too, has no post-build setter, and a budget short
/// enough for a steady-state command is exceeded by session creation under
/// nextest parallelism (that is why [`Harness::start`] keeps the default).
///
/// A stall is reported as an ordinary command failure — a
/// `WebDriverErrorInner::Timeout` naming the command and the budget — so every
/// caller keeps the error path it already had: [`is_absence`] refuses it (a
/// stall is never an absence) and the panic names the command, while an
/// `expect`-based call site prints it verbatim.
///
/// A free function rather than a method: it needs nothing from the harness,
/// and a plain unit test can then prove the bound without a browser session.
async fn bounded<T>(
    what: &str,
    command: impl Future<Output = WebDriverResult<T>>,
) -> WebDriverResult<T> {
    bounded_for(what, COMMAND_BUDGET, command).await
}

/// [`bounded`] with an explicit budget, for thirtyfour's OWN polling queries.
///
/// `ElementQuery::wait` is a poll loop, not one command, so it is given its own
/// budget plus [`COMMAND_BUDGET`]: the loop is designed to finish within its
/// budget, and the grace is the one command it may be inside when the driver
/// stops answering.
async fn bounded_for<T>(
    what: &str,
    budget: Duration,
    command: impl Future<Output = WebDriverResult<T>>,
) -> WebDriverResult<T> {
    match tokio::time::timeout(budget, command).await {
        Ok(answer) => answer,
        Err(_) => Err(WebDriverError::Timeout(format!(
            "`{what}` did not answer within {budget:?} (the harness command budget)"
        ))),
    }
}

/// Whether a `WebDriver` error means the element is simply NOT THERE — the
/// only answer a polling probe may read as "not yet".
///
/// `no such element` is genuine absence, and a stale handle is a re-rendering
/// subtree detaching the element between the find and the read. Everything
/// else — a request that never came back, a dead session, a rejected
/// selector — is the driver failing to ANSWER the question, and a probe that
/// swallows it spends its whole budget reporting an empty page.
fn is_absence(error: &WebDriverError) -> bool {
    matches!(
        error.as_inner(),
        WebDriverErrorInner::NoSuchElement(_) | WebDriverErrorInner::StaleElementReference(_)
    )
}

/// The first element matching `by`, labelled `what`, or `None` when nothing
/// matches.
///
/// The one find every CSS-keyed helper below goes through, so each command
/// has a single implementation and every failure names the selector the
/// caller passed rather than `By`'s `Debug` form.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
async fn find_first(h: &Harness, by: By, what: &str) -> Option<Element> {
    match bounded(&format!("find({what})"), h.driver.find(by)).await {
        Ok(inner) => Some(Element::new(inner, what)),
        Err(error) if is_absence(&error) => None,
        Err(error) => panic!("the WebDriver could not answer `find({what})`: {error}"),
    }
}

/// [`find_first`] for a CSS selector, which labels itself.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
async fn find_css(h: &Harness, css: &str) -> Option<Element> {
    find_first(h, By::Css(css), css).await
}

/// Whether `by` currently matches an element.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence
/// ([`is_absence`]): a failure to observe is never an observation of nothing.
pub(crate) async fn is_present_by(h: &Harness, by: By) -> bool {
    let what = format!("{by:?}");
    find_first(h, by, &what).await.is_some()
}

/// [`is_present_by`] for a CSS selector — the ordinary "is this on the page"
/// probe.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn is_present(h: &Harness, css: &str) -> bool {
    is_present_by(h, By::Css(css)).await
}

/// Whether `css` matches a currently VISIBLE element.
///
/// thaw's dialog is never removed from the DOM: `leptos_transition_group`'s
/// `CSSTransition` hides it with `display: none`, so a closed dialog is still
/// findable. Openness is therefore visibility, never mere presence.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn is_visible(h: &Harness, css: &str) -> bool {
    match find_css(h, css).await {
        Some(element) => element.read_displayed().await == Some(true),
        None => false,
    }
}

/// Whether ANY element matching `css` is displayed.
///
/// [`is_visible`] answers for the FIRST match, which is the wrong question
/// about a dialog: a page mounts one surface per dialog and thaw's `Teleport`
/// never unmounts one after its first open, so a journey that has opened two
/// leaves a closed surface ahead of the open one in document order. "Is a
/// modal up" has to look at all of them or it answers about the wrong one.
pub(crate) async fn is_any_visible(h: &Harness, css: &str) -> bool {
    for element in find_all(h, css).await {
        if element.read_displayed().await == Some(true) {
            return true;
        }
    }
    false
}

/// thaw's modal surface — the panel itself.
const DIALOG_SURFACE: &str = ".thaw-dialog-surface";

/// thaw's modal backdrop: a full-viewport element rendered under every open
/// dialog and kept displayed while the dialog fades out.
const DIALOG_BACKDROP: &str = ".thaw-dialog-surface__backdrop";

/// thaw's modal surface MID-OPEN: `leptos_transition_group`'s `CSSTransition`
/// adds `{name}-enter-active` when the surface starts appearing and removes it
/// when the transition ends, and thaw's `DialogSurface` names its transition
/// `fade-in-scale-up-transition` (a 250 ms opacity + scale).
const DIALOG_ENTERING: &str = ".thaw-dialog-surface.fade-in-scale-up-transition-enter-active";

/// Establish the precondition every page-level click has: nothing is covering
/// the page.
///
/// The backdrop spans the viewport, so a click issued while one is up lands on
/// it instead of the control and `WebDriver` reports `ElementClickIntercepted`.
/// Two situations produce one: a dialog that is still fading out, and a dialog
/// that is deliberately still open because the CDR refused what it sent (the
/// upload dialog keeps its diagnostic beside the input that caused it). Waiting
/// alone answers only the first, so an open surface is dismissed through the
/// dialog's own Escape path — the same exit a person takes — and the wait then
/// covers the fade.
///
/// # Panics
/// When a backdrop is still up after 15 s.
pub(crate) async fn clear_dialog_overlay(h: &Harness) {
    if is_any_visible(h, DIALOG_SURFACE).await {
        drop(
            bounded(
                "action_chain(Escape).perform()",
                h.driver.action_chain().send_keys(Key::Escape).perform(),
            )
            .await,
        );
    }
    wait_hidden(h, DIALOG_BACKDROP).await;
}

/// Wait until the dialog holding `control_css` is open AND STILL: that control
/// visible, with no dialog anywhere mid-enter.
///
/// Takes a control INSIDE the target dialog rather than matching the surface
/// class, because a page mounts one surface per dialog and thaw's `Teleport`
/// mounts each subtree on its first open and never unmounts it. A journey that
/// uploads and then deletes leaves two surfaces in the DOM, and a bare
/// `.thaw-dialog-surface` match answers for whichever comes first — which is
/// the CLOSED one, so the wait could never be satisfied.
///
/// Visibility alone is not readiness either: a control clicked while the
/// surface is still scaling up is clicked at coordinates it has already left,
/// which `WebDriver` reports as a perfectly successful click on nothing.
/// [`DIALOG_ENTERING`] is the transition's own in-flight marker and is checked
/// across the page, so "this control is visible and nothing is entering" is
/// observable rather than timed.
///
/// # Panics
/// When the dialog is not open and settled within 15 s.
pub(crate) async fn wait_dialog_settled(h: &Harness, control_css: &str) {
    for _ in 0..75 {
        if is_visible(h, control_css).await && !is_present(h, DIALOG_ENTERING).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let url = bounded("current_url()", h.driver.current_url())
        .await
        .expect("current url");
    panic!("the dialog holding `{control_css}` never became open and settled (at {url})");
}

/// The submit button of the Template Manager's one upload dialog.
const UPLOAD_SUBMIT: &str = "#template-upload-submit";

/// The refusal diagnostic, scoped to the MODAL: the template listings render
/// their own `MessageBar` for an empty result, which is not this one.
const UPLOAD_DIAGNOSTIC: &str = ".thaw-dialog-surface .thaw-message-bar";

/// The in-flight line the upload dialog renders while its action is pending
/// (`src/components/upload_dialog.rs`). It sits inside a `Show`, so its
/// presence IS the pending state; matched by its rendered text because the
/// line carries no id of its own.
const UPLOAD_PENDING_LINE: &str =
    "//div[contains(@class, 'thaw-dialog-surface')]//span[normalize-space(text())='Uploading…']";

/// Every toast card currently on screen; each upload outcome raises one.
const TOAST_CARD: &str = ".thaw-toast-body";

/// What the upload dialog says about a dispatch, read at one instant.
///
/// The submit click's post-condition is a COMPARISON of two of these, because
/// only some of the observables are unconditionally absent beforehand: a
/// refusal diagnostic and a toast can both survive from an earlier attempt.
#[derive(Debug)]
struct UploadDialogState {
    /// Whether the modal surface is on screen — an accepted upload closes it.
    open: bool,
    /// Whether the submit button is live, or `None` when it is not in the DOM.
    submit_enabled: Option<bool>,
    /// Whether the "Uploading…" line is rendered.
    pending: bool,
    /// The refusal diagnostic rendered inside the dialog, if any.
    diagnostic: Option<String>,
    /// How many toast cards are up.
    toasts: usize,
}

impl UploadDialogState {
    /// Read every observable once.
    ///
    /// # Panics
    /// When the `WebDriver` fails to answer any of them ([`is_absence`]).
    async fn read(h: &Harness) -> Self {
        Self {
            open: is_visible(h, DIALOG_SURFACE).await,
            submit_enabled: read_enabled(h, UPLOAD_SUBMIT).await,
            pending: is_present_by(h, By::XPath(UPLOAD_PENDING_LINE)).await,
            diagnostic: read_text(h, UPLOAD_DIAGNOSTIC).await,
            toasts: count_matching(h, TOAST_CARD).await,
        }
    }

    /// Whether this state carries a consequence of the submit dispatch that
    /// `before` did not already carry.
    ///
    /// Each disjunct is something only a dispatch produces: the dialog closing
    /// is the accepted-upload continuation, the pending line and the button
    /// going inert are the action's own in-flight state, a moved diagnostic is
    /// a fresh refusal, and a new toast is either outcome's feedback.
    fn dispatched_since(&self, before: &Self) -> bool {
        (before.open && !self.open)
            || self.pending
            || (before.submit_enabled == Some(true) && self.submit_enabled == Some(false))
            || self.diagnostic != before.diagnostic
            || self.toasts > before.toasts
    }
}

/// The submit click's post-condition: poll until the dialog shows that the
/// dispatch happened.
///
/// # Panics
/// When nothing moves within 15 s — the click was answered, the handler never
/// ran, and the panic says so HERE rather than leaving a downstream read to
/// blame the CDR for a row nothing ever sent it.
async fn wait_upload_dispatched(h: &Harness, before: &UploadDialogState) {
    for _ in 0..75 {
        if UploadDialogState::read(h).await.dispatched_since(before) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let evidence = h.evidence_dump("upload-click-not-dispatched").await;
    panic!(
        "the click on `{UPLOAD_SUBMIT}` did not dispatch the upload: 15 s on, the dialog \
         is still open with its source loaded and its submit button live, nothing is \
         pending, no new diagnostic and no toast — the click was answered but the \
         handler never ran, so nothing was sent to the CDR ({evidence})"
    );
}

/// Upload `path` through the Template Manager's one upload dialog: open it
/// from the page-header trigger, choose the file, and send it.
///
/// Both template families share this control (#2955), so every family's seed
/// helper drives this one routine.
///
/// Four conditions, no sleeps, each the exact fact the next step needs:
/// nothing covers the page ([`clear_dialog_overlay`] — a refused upload keeps
/// its dialog open on purpose, so re-entry with a modal up is normal, #3134);
/// the modal is open and settled ([`wait_dialog_settled`]); the submit button
/// is live ([`wait_enabled`]), which is inert until the chosen file has been
/// read into the dialog's source editor and so means exactly "the file
/// arrived"; and the send actually DISPATCHED ([`wait_upload_dispatched`]).
///
/// # Panics
/// On any interaction failure, and when the submit click dispatches nothing.
pub(crate) async fn upload_via_dialog(h: &Harness, path: &str) {
    clear_dialog_overlay(h).await;
    h.wait_css("#template-upload-open").await.click().await;
    wait_dialog_settled(h, UPLOAD_SUBMIT).await;
    h.wait_css("#template-upload-picker input[type=file]")
        .await
        .send_keys(path)
        .await;
    wait_enabled(h, UPLOAD_SUBMIT).await;
    let before = UploadDialogState::read(h).await;
    h.wait_css(UPLOAD_SUBMIT).await.click().await;
    wait_upload_dispatched(h, &before).await;
}

/// Whether the control at `css` is enabled, or `None` when nothing matches.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn read_enabled(h: &Harness, css: &str) -> Option<bool> {
    find_css(h, css).await?.read_enabled().await
}

/// The text of the first element matching `css`, or `None` when nothing
/// matches.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn read_text(h: &Harness, css: &str) -> Option<String> {
    find_css(h, css).await?.read_text().await
}

/// [`find_all`] with the label each handle carries into its own failures.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
async fn find_all_labelled(h: &Harness, by: By, what: &str) -> Vec<Element> {
    let found = match bounded(&format!("find_all({what})"), h.driver.find_all(by)).await {
        Ok(found) => found,
        Err(error) if is_absence(&error) => Vec::new(),
        Err(error) => panic!("the WebDriver could not answer `find_all({what})`: {error}"),
    };
    found
        .into_iter()
        .enumerate()
        .map(|(index, inner)| Element::new(inner, &format!("{what}[{index}]")))
        .collect()
}

/// Every element matching `css`, empty when nothing matches — the ordinary
/// "every match on the page" read.
///
/// A query that was never ANSWERED is not an empty page, which is why the
/// panic below is not an empty vector.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn find_all(h: &Harness, css: &str) -> Vec<Element> {
    find_all_labelled(h, By::Css(css), css).await
}

/// The DOM property `property` of the first element matching `css`, or `None`
/// when nothing matches.
///
/// A control's LIVE value is a property; [`read_attr`] answers about the
/// attribute the server rendered, which a typed-in value never moves.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn read_prop(h: &Harness, css: &str, property: &str) -> Option<String> {
    find_css(h, css).await?.read_prop(property).await
}

/// How many elements match `css`.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn count_matching(h: &Harness, css: &str) -> usize {
    find_all(h, css).await.len()
}

/// Whether an element matching `css` shows up within `budget`.
///
/// The short-budget probe for a branch a journey takes either way (a capture
/// that is skipped when a screen is not served, a click retried until its
/// target renders); a condition the journey REQUIRES is waited for by
/// [`Harness::wait_css`] instead, which fails with evidence.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn appears_within(h: &Harness, css: &str, budget: Duration) -> bool {
    let step = Duration::from_millis(200);
    let mut waited = Duration::ZERO;
    loop {
        if is_present(h, css).await {
            return true;
        }
        if waited >= budget {
            return false;
        }
        tokio::time::sleep(step).await;
        waited += step;
    }
}

/// Poll `condition` every [`POLL_STEP`] until it holds, for at most `budget`;
/// returns whether it did.
///
/// The one poll loop every journey condition is written over, so a journey
/// file needs no timer of its own — which is what lets
/// `scripts/checks/e2e-waits.sh` refuse `sleep` there outright.
pub(crate) async fn poll_until_for(
    budget: Duration,
    mut condition: impl AsyncFnMut() -> bool,
) -> bool {
    let mut waited = Duration::ZERO;
    loop {
        if condition().await {
            return true;
        }
        if waited >= budget {
            return false;
        }
        tokio::time::sleep(POLL_STEP).await;
        waited += POLL_STEP;
    }
}

/// [`poll_until_for`] over [`WAIT`] — the ordinary journey condition.
pub(crate) async fn poll_until(condition: impl AsyncFnMut() -> bool) -> bool {
    poll_until_for(WAIT, condition).await
}

/// Poll `probe` every [`POLL_STEP`] until it answers `Some`, for at most
/// `budget`, and hand that answer back.
///
/// [`poll_until_for`] for a condition that also PRODUCES the value the
/// journey went looking for (the id in a URL, the settled text of a row).
pub(crate) async fn poll_some_for<T>(
    budget: Duration,
    mut probe: impl AsyncFnMut() -> Option<T>,
) -> Option<T> {
    let mut waited = Duration::ZERO;
    loop {
        if let Some(value) = probe().await {
            return Some(value);
        }
        if waited >= budget {
            return None;
        }
        tokio::time::sleep(POLL_STEP).await;
        waited += POLL_STEP;
    }
}

/// [`poll_some_for`] over [`WAIT`].
pub(crate) async fn poll_some<T>(probe: impl AsyncFnMut() -> Option<T>) -> Option<T> {
    poll_some_for(WAIT, probe).await
}

/// Let the page sit for `duration` with nothing expected to happen, printing
/// `why`.
///
/// The one wait here that is NOT a condition, and it lives in the harness so
/// the journey files can stay sleep-free: a journey that asserts a window
/// passes WITHOUT something happening — a timer the previous shell left
/// behind failing to fire — has no observable to poll for, and turning the
/// absence into a condition would only be a sleep wearing a wait's name.
pub(crate) async fn dwell(duration: Duration, why: &str) {
    println!("dwell {duration:?}: {why}");
    tokio::time::sleep(duration).await;
}

/// The red, green and blue channels of a serialised CSS colour, whatever
/// spelling it arrived in.
///
/// Every CSSOM colour serialization writes the three channels first —
/// `rgb(15, 23, 42)` for a colour the cascade resolved as opaque,
/// `rgba(15, 23, 42, 1)` when the winning declaration carried an explicit
/// alpha channel — so reading the first three numbers reads the PAINT and
/// ignores the spelling. Alpha is deliberately not compared: it is the one
/// component the two spellings disagree about, and a wait on a themed surface
/// is a wait on its colour.
fn colour_channels(value: &str) -> Option<[u32; 3]> {
    let mut numbers = value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(str::parse::<u32>);
    let mut channels = [0u32; 3];
    for channel in &mut channels {
        *channel = numbers.next()?.ok()?;
    }
    Some(channels)
}

/// Poll until the RESOLVED colour of the CSS `property` on the first element
/// matching `css` is `expected`.
///
/// Resolved rather than declared: `WebDriver`'s Get Element CSS Value returns
/// the computed value, which is what makes a token swap observable at all —
/// the declaration never changes, only what `var(--…)` resolves to.
///
/// Colour rather than text: the computed value comes back in the CSSOM
/// spelling the winning declaration produced, and this viewer's `<html>`
/// background reaches Chrome 152 as `rgba(15, 23, 42, 1)` where the same paint
/// declared without an alpha channel would read `rgb(15, 23, 42)`. Comparing
/// [`colour_channels`] compares what was painted; comparing the strings would
/// compare the serialization and fail on a browser that chose the other one.
///
/// # Panics
/// When it never does within [`WAIT`], reporting the last value seen, or when
/// `expected` is not a colour [`colour_channels`] can read.
pub(crate) async fn wait_css_colour(h: &Harness, css: &str, property: &str, expected: &str) {
    let want = colour_channels(expected)
        .unwrap_or_else(|| panic!("`{expected}` is not a CSS colour with three channels"));
    let mut last = String::new();
    let settled = poll_until(async || match find_css(h, css).await {
        Some(element) => {
            last = element.css_value(property).await;
            colour_channels(&last) == Some(want)
        }
        None => false,
    })
    .await;
    assert!(
        settled,
        "`{css}`'s resolved `{property}` never became `{expected}` (last: `{last}`)"
    );
}

/// Poll until the CSS `property` of `css` STOPS moving: two samples
/// [`POLL_STEP`] apart that agree.
///
/// The animation half of a theme flip. `wait_css_colour` answers "the token
/// swapped"; a surface carrying a colour transition is still somewhere
/// between its two colours at that instant, and a capture taken then freezes
/// a half-themed frame. Two equal samples is the observable end of the
/// transition — an explicit condition, where a fixed settle is a guess that
/// is either too short on a loaded machine or wasted time on a quiet one.
///
/// Returns at once when nothing matches `css`: a page carrying no animated
/// surface has nothing to settle.
///
/// # Panics
/// When the value is still moving after [`WAIT`].
pub(crate) async fn wait_css_value_still(h: &Harness, css: &str, property: &str) {
    let mut last = String::new();
    let settled = poll_until(async || {
        let Some(before) = find_css(h, css).await else {
            return true;
        };
        let first = before.css_value(property).await;
        tokio::time::sleep(POLL_STEP).await;
        let Some(after) = find_css(h, css).await else {
            return true;
        };
        last = after.css_value(property).await;
        first == last
    })
    .await;
    assert!(
        settled,
        "`{css}`'s `{property}` never stopped moving (last: `{last}`)"
    );
}

/// Poll until the control at `css` is present and ENABLED.
///
/// The viewer keeps an edit form inert until the document it edits has been
/// seeded into it, so this is the condition that makes typing (or saving) safe:
/// input accepted earlier would be replaced by the seed and the save would then
/// commit the pre-seed draft.
///
/// # Panics
/// When it never becomes enabled within 15 s, or when the `WebDriver` stops
/// answering — a stalled driver used to look exactly like an unseeded form.
pub(crate) async fn wait_enabled(h: &Harness, css: &str) {
    for _ in 0..75 {
        if read_enabled(h, css).await == Some(true) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{css}` never became enabled — its form was never seeded");
}

/// Poll until no element matches `css` — the assert-gone half of a delete.
///
/// # Panics
/// When something still matches after [`WAIT`], with the page it was on, or
/// when the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn wait_css_absent(h: &Harness, css: &str) {
    for _ in 0..75 {
        if count_matching(h, css).await == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let url = bounded("current_url()", h.driver.current_url())
        .await
        .expect("current url");
    panic!("`{css}` never disappeared (at {url})");
}

/// Wait until some element's text contains `needle` (a toast title, a status
/// line), returning whether it appeared.
///
/// # Panics
/// When the `WebDriver` answers with anything but an absence ([`is_absence`]):
/// "the page never said it" is a claim about the page, so it may only be made
/// on an answer.
pub(crate) async fn wait_text(h: &Harness, needle: &str) -> bool {
    let xpath = format!("//*[contains(normalize-space(.), '{needle}')]");
    for _ in 0..75 {
        if is_present_by(h, By::XPath(&xpath)).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// Poll until the text of the element at `css` contains `fragment` — the "the
/// CDR actually applied it" assertion, an explicit condition rather than a
/// sleep.
///
/// # Panics
/// When it never does within [`WAIT`], reporting what it said instead, or when
/// the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn wait_text_contains(h: &Harness, css: &str, fragment: &str) {
    let mut last = String::new();
    for _ in 0..75 {
        if let Some(text) = read_text(h, css).await {
            if text.contains(fragment) {
                return;
            }
            last = text;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{css}` never contained `{fragment}` (last text: {last})");
}

/// Wait until the element at `css` has text ending in `suffix` (a version
/// number the screen must have caught up to).
///
/// # Panics
/// When it never does within [`WAIT`], reporting the last text seen, or when
/// the `WebDriver` answers with anything but an absence ([`is_absence`]).
pub(crate) async fn wait_text_suffix(h: &Harness, css: &str, suffix: &str) {
    let mut last = String::new();
    for _ in 0..75 {
        if let Some(text) = read_text(h, css).await {
            last = text;
            if last.trim_end().ends_with(suffix) {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{css}` never ended in `{suffix}` (last text: `{last}`)");
}

/// Type `text` into the field at `css`, clearing whatever is there first.
///
/// # Panics
/// On any interaction failure.
pub(crate) async fn retype(h: &Harness, css: &str, text: &str) {
    let field = h.wait_css(css).await;
    field.clear().await;
    field.send_keys(text).await;
}

/// Click `css` until `target_css` shows up, returning whether it did (the
/// pre-hydration-click precedent; re-clicking an "open this version" button is
/// idempotent).
///
/// # Panics
/// On a click failure, and when the `WebDriver` answers the probe with
/// anything but an absence ([`is_absence`]) — five rounds of a dead driver
/// would otherwise report "the target never appeared".
pub(crate) async fn click_until_css(h: &Harness, css: &str, target_css: &str) -> bool {
    for _ in 0..5 {
        h.wait_css(css).await.click().await;
        if poll_until_for(Duration::from_secs(5), async || {
            is_present(h, target_css).await
        })
        .await
        {
            return true;
        }
    }
    false
}

/// Poll until `css` is no longer visible.
///
/// # Panics
/// When it is still visible after 15 s.
pub(crate) async fn wait_hidden(h: &Harness, css: &str) {
    for _ in 0..75 {
        if !is_any_visible(h, css).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let url = bounded("current_url()", h.driver.current_url())
        .await
        .expect("current url");
    panic!("`{css}` never hid (at {url})");
}

/// Poll until `css` is VISIBLE — the mirror of [`wait_hidden`].
///
/// thaw keeps a closed dialog in the DOM, so "the dialog opened" is a
/// visibility condition and never mere presence ([`is_visible`]).
///
/// # Panics
/// When it is still not visible after 15 s.
pub(crate) async fn wait_visible(h: &Harness, css: &str) {
    for _ in 0..75 {
        if is_visible(h, css).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let url = bounded("current_url()", h.driver.current_url())
        .await
        .expect("current url");
    panic!("`{css}` never became visible (at {url})");
}

/// Empty the field at `css` BY TYPING.
///
/// Deliberately not `WebDriver`'s element-clear command: measured on the
/// event-subscription journey, clearing that way empties the DOM value without
/// the viewer's `on:input` listener ever running, so the form's state keeps
/// the old value and the save sends it back — a green screen and a wrong wire.
/// Backspacing is what a person does, and it fires the events the binding
/// listens for. ([`retype`] is unaffected: the keystrokes it sends after
/// clearing re-deliver the whole value.)
///
/// # Panics
/// On any interaction failure, or when the field is not empty afterwards.
pub(crate) async fn clear_field(h: &Harness, css: &str) {
    let field = h.wait_css(css).await;
    let held = field.prop("value").await;
    let mut keys = String::from(Key::End.value());
    keys.extend(std::iter::repeat_n(
        Key::Backspace.value(),
        held.chars().count(),
    ));
    field.send_keys(keys).await;
    let left = field.prop("value").await;
    assert!(
        left.is_empty(),
        "`{css}` still reads `{left}` after erasing"
    );
}

/// One attempt at reading `attribute` off the first element matching `css`,
/// or `None`.
///
/// `None` covers both "nothing matches yet" and a STALE element handle: a
/// re-rendering table detaches the handle between the find and the read, and
/// `attr` then answers `stale element reference` — a retry, never a failure.
pub(crate) async fn read_attr(h: &Harness, css: &str, attribute: &str) -> Option<String> {
    find_css(h, css).await?.read_attr(attribute).await
}

/// `attribute` of the first element matching `css`, waited for — the rendered
/// window's identity.
///
/// # Panics
/// When nothing readable matches within the wait budget.
pub(crate) async fn wait_attr(h: &Harness, css: &str, attribute: &str) -> String {
    for _ in 0..75 {
        if let Some(value) = read_attr(h, css, attribute).await {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let url = bounded("current_url()", h.driver.current_url())
        .await
        .expect("current url");
    panic!("no element matched `{css}` with a readable `{attribute}` (at {url})");
}

/// Poll until `attribute` of the first element matching `css` is no longer
/// `previous`, and return the new value.
///
/// This is the content-moved condition (never a sleep): a paging link is a real
/// navigation and a `<Transition>` keeps the previous rows on screen while the
/// next window loads.
///
/// # Panics
/// When it has not changed after 15 s.
pub(crate) async fn wait_attr_change(
    h: &Harness,
    css: &str,
    attribute: &str,
    previous: &str,
) -> String {
    for _ in 0..75 {
        if let Some(current) = read_attr(h, css, attribute).await
            && current != previous
        {
            return current;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let url = bounded("current_url()", h.driver.current_url())
        .await
        .expect("current url");
    panic!("`{css}`'s `{attribute}` never moved off `{previous}` (at {url})");
}

/// Drive one action through its confirmation MODAL: click the trigger, wait for
/// the dialog to become visible, then click its confirm button. Explicit
/// conditions, never a sleep.
///
/// The trigger click carries a bounded retry (the login-submit precedent): a
/// click landing before hydration attaches the listener is simply lost, and
/// re-clicking is safe precisely because the dialog is not open yet.
///
/// # Panics
/// When the dialog never opens.
pub(crate) async fn confirm_in_dialog(h: &Harness, trigger_css: &str, confirm_id: &str) {
    // A visible toast overlays the bottom-right corner and intercepts clicks.
    h.wait_toasts_cleared().await;
    // A modal backdrop overlays the whole viewport and does the same (#3134).
    clear_dialog_overlay(h).await;
    let trigger = h.wait_css(trigger_css).await;
    let confirm_css = format!("#{confirm_id}");
    let mut opened = false;
    for attempt in 0..10 {
        // Retries exist for a pre-hydration click that does nothing. Once a
        // dialog surface is actually up, clicking the trigger again would be
        // swallowed by the modal and surface as `ElementClickIntercepted` — a
        // confusing report for what is really "the dialog opened, but not with
        // the confirm id this call expects". Stop and let the assertion say so.
        if attempt > 0 && is_any_visible(h, DIALOG_SURFACE).await {
            break;
        }
        trigger.click().await;
        opened = poll_until_for(Duration::from_secs(2), async || {
            is_visible(h, &confirm_css).await
        })
        .await;
        if opened {
            break;
        }
    }
    assert!(
        opened,
        "`{trigger_css}` never opened the `{confirm_css}` dialog — either the click \
         never landed (pre-hydration) or a dialog opened whose confirm id is not \
         `{confirm_id}`"
    );
    // The confirm button is visible from the first frame of the 250 ms enter
    // transition, and a click landing mid-scale can miss the moving target
    // without the driver reporting anything (#3200).
    wait_dialog_settled(h, &confirm_css).await;
    h.wait_css(&confirm_css).await.click().await;
    // The dialog hides on confirm — that it hid proves the click landed. The
    // backdrop outlives the button by the length of the fade, so the journey's
    // next click needs it gone too (#3134).
    wait_hidden(h, &confirm_css).await;
    wait_hidden(h, DIALOG_BACKDROP).await;
}

/// Log in through the Basic form (journeys that need a session).
///
/// # Panics
/// When the login flow does not land on the dashboard.
pub(crate) async fn login_basic(h: &Harness) {
    let user = env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned());
    let pass = env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned());
    login_basic_as(h, &user, &pass).await;
}

/// [`login_basic`] with explicit credentials — for journeys that need a
/// specific dev user (the audit screens require the CDR's admin role:
/// `UI_E2E_ADMIN_USER`/`UI_E2E_ADMIN_PASS`, defaulting to the quickstart
/// `ferroehr-admin`/`ferroehr` Basic user).
///
/// # Panics
/// When the login flow does not land on the dashboard.
pub(crate) async fn login_basic_as(h: &Harness, user: &str, pass: &str) {
    let user = user.to_owned();
    let pass = pass.to_owned();
    h.goto("/login").await;
    h.wait_css("#login-username").await.send_keys(&user).await;
    h.wait_css("#login-password").await.send_keys(&pass).await;
    // Submit with a bounded retry: a click landing exactly while hydration
    // swaps the form can be lost (the ActionForm is valid both pre- and
    // post-hydration, but the swap instant is a real race). Each attempt
    // gets a short bounded wait; leaving /login ends the loop.
    let mut attempts = 0;
    loop {
        h.wait_css("button[type=submit]").await.click().await;
        let off_login = poll_until_for(Duration::from_secs(3), async || {
            !h.current_url().await.contains("/login")
        })
        .await;
        if off_login {
            break;
        }
        attempts += 1;
        if attempts >= 3 {
            let path = format!("{}/{}-login-stuck.png", h.shots_dir, h.journey);
            drop(
                bounded(
                    "screenshot()",
                    h.driver.screenshot(std::path::Path::new(&path)),
                )
                .await,
            );
            let log = bounded("get_log(browser)", h.driver.get_log("browser"))
                .await
                .unwrap_or_default();
            for entry in &log {
                println!("console[{}]: {}", entry.level, entry.message);
            }
            panic!("login submit did not leave /login after 3 attempts");
        }
    }
    // The shell footer is the authenticated-chrome marker.
    h.wait_css("footer").await;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use thirtyfour::error::{WebDriverError, WebDriverErrorInfo, WebDriverErrorInner};

    use super::{
        COMMAND_BUDGET, WAIT, bounded, colour_channels, element_command, element_probe, is_absence,
    };

    /// The two failures a probe is allowed to read as "not there yet"
    /// ([`is_absence`]).
    fn absences() -> [WebDriverError; 2] {
        [
            WebDriverError::NoSuchElement(WebDriverErrorInfo::new("no such element".to_owned())),
            WebDriverError::StaleElementReference(WebDriverErrorInfo::new(
                "stale element reference".to_owned(),
            )),
        ]
    }

    /// The bound is proven, not asserted: a command that never answers fails
    /// within its budget, names itself, and is not read as an absence.
    ///
    /// Time is paused, so the runtime auto-advances the clock while the future
    /// is idle — the measured elapsed is the virtual budget and the test
    /// itself takes no wall time
    /// (<https://docs.rs/tokio/latest/tokio/time/fn.pause.html>).
    #[tokio::test(start_paused = true)]
    async fn a_command_that_never_answers_fails_inside_the_poll_budget() {
        let started = tokio::time::Instant::now();
        let answer: Result<(), _> = bounded(
            "find(.never-answers)",
            std::future::pending::<Result<(), WebDriverError>>(),
        )
        .await;

        let error = answer.expect_err("a future that never resolves must not report success");
        let elapsed = started.elapsed();
        assert!(
            matches!(error.as_inner(), WebDriverErrorInner::Timeout(_)),
            "a stall is reported as a timeout, not as {error}"
        );
        assert!(
            error.to_string().contains("find(.never-answers)"),
            "the failure names the command it bounded: {error}"
        );
        assert!(
            !is_absence(&error),
            "a stall is never an absence — a probe must not read it as `not yet`"
        );
        assert_eq!(elapsed, COMMAND_BUDGET, "the bound is the command budget");
        assert!(
            elapsed < WAIT,
            "the bound fires inside the poll budget that issued the command"
        );
    }

    /// The element tier is bounded too: a command issued on an element that
    /// never answers panics inside the poll budget, naming the selector that
    /// found the handle rather than an anonymous driver failure.
    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "click(#status-save)")]
    async fn an_element_command_that_never_answers_panics_naming_the_selector() {
        element_command(
            "click(#status-save)",
            std::future::pending::<Result<(), WebDriverError>>(),
        )
        .await;
    }

    /// The probe tier collapses an absence and NOTHING else: both answers
    /// [`is_absence`] admits become `None`, so a poll loop reads them as "not
    /// yet".
    #[tokio::test(start_paused = true)]
    async fn an_element_probe_reads_an_absence_as_not_yet() {
        for absence in absences() {
            assert!(is_absence(&absence), "{absence} must count as an absence");
            let answer: Option<()> = element_probe("text([data-staged][0])", async move {
                Err::<(), WebDriverError>(absence)
            })
            .await;
            assert!(
                answer.is_none(),
                "an absence is the one failure a probe may read as `not yet`"
            );
        }
    }

    /// A stall is never an absence — the failure this whole harness exists to
    /// stop reading as an empty page. The probe panics rather than answering
    /// `None`, so a poll loop cannot spend its budget on a dead driver.
    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "is_enabled(#aql-save-submit)")]
    async fn an_element_probe_never_reads_a_stall_as_not_yet() {
        let answer: Option<bool> = element_probe(
            "is_enabled(#aql-save-submit)",
            std::future::pending::<Result<bool, WebDriverError>>(),
        )
        .await;
        panic!("a stalled driver must not answer `{answer:?}`");
    }

    /// The two CSSOM spellings of one paint read as one colour, and a
    /// different paint does not — the comparison a themed capture waits on.
    #[test]
    fn one_colour_reads_the_same_in_both_cssom_spellings() {
        let dark = Some([15, 23, 42]);
        assert_eq!(colour_channels("rgb(15, 23, 42)"), dark);
        assert_eq!(
            colour_channels("rgba(15, 23, 42, 1)"),
            dark,
            "an explicit alpha channel is the spelling, not the paint"
        );
        assert_eq!(colour_channels("rgb(15 23 42 / 1)"), dark);
        assert_ne!(
            colour_channels("rgb(248, 250, 252)"),
            dark,
            "the light surface is a different colour, not a different spelling"
        );
        assert_eq!(
            colour_channels("transparent"),
            None,
            "a keyword carries no channels to compare"
        );
    }

    /// A command that answers is handed back untouched — the bound adds no
    /// behaviour of its own to the ordinary path.
    #[tokio::test(start_paused = true)]
    async fn a_command_that_answers_passes_through_unchanged() {
        let started = tokio::time::Instant::now();
        let answer = bounded("current_url()", async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, WebDriverError>("http://127.0.0.1:3000/login")
        })
        .await;

        assert_eq!(
            answer.expect("the command answered"),
            "http://127.0.0.1:3000/login"
        );
        assert!(
            started.elapsed() < COMMAND_BUDGET,
            "an answered command returns as soon as it answers"
        );
    }
}
