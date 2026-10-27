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

/// The words [`Harness::mark_document`] leaves on the window.
const DOCUMENT_MARK: &str = "drawn-in-this-document";

/// The site bundle's files a browser without WebAssembly is refused: the
/// entry's WebAssembly and glue and the lazy routes' chunks.
const WITHHELD: [&str; 3] = [
    "*/pkg/ferroehr-viewer*",
    "*/pkg/__wasm_split*",
    "*/pkg/*.wasm",
];

/// The width of a phone screen, in CSS pixels.
pub(crate) const PHONE_WIDTH: u32 = 390;

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
        // Transitions finish at once, so a capture taken right after a click
        // shows the settled state rather than a cross-fade.
        caps.add_arg("--force-prefers-reduced-motion")
            .expect("caps");
        // The system theme reads light whatever the host prefers, so a journey
        // and a light screenshot see the same page on every machine.
        caps.add_arg("--blink-settings=preferredColorScheme=1")
            .expect("caps");
        // Image-mode OIDC: the composed viewer advertises the in-network
        // issuer host (`keycloak`); the browser resolves it to the host-
        // mapped port. A no-op in host mode (nothing references the name).
        caps.add_arg("--host-resolver-rules=MAP keycloak 127.0.0.1")
            .expect("caps");
        // The dev Keycloak serves its issuer over TLS with the development CA
        // the stack generates (docker-compose.dev.yml `keycloak-tls`), which no
        // browser trust store holds.
        caps.accept_insecure_certs(true).expect("caps");
        caps.set_logging_prefs("browser", thirtyfour::LoggingPrefsLogLevel::All)
            .expect("logging prefs");
        // NOTE: session creation keeps thirtyfour's own 120 s request timeout —
        // a budget short enough for a steady-state command is exceeded here
        // under nextest parallelism; those commands are bounded by `bounded`.
        let driver = WebDriver::builder(&webdriver_url, caps)
            .await
            .expect("webdriver session (is chromedriver up?)");
        let harness = Self {
            driver,
            base,
            shots_dir,
            journey,
            js: true,
        };
        // The phone lane runs every journey at 390 px (#3910).
        if env("UI_E2E_VIEWPORT").as_deref() == Some("phone") {
            harness.emulate(PHONE_WIDTH, false).await;
        }
        Some(harness)
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
        // Transitions finish at once, so a capture taken right after a click
        // shows the settled state rather than a cross-fade.
        caps.add_arg("--force-prefers-reduced-motion")
            .expect("caps");
        // The system theme reads light whatever the host prefers, so a journey
        // and a light screenshot see the same page on every machine.
        caps.add_arg("--blink-settings=preferredColorScheme=1")
            .expect("caps");
        // Image-mode OIDC: the composed viewer advertises the in-network
        // issuer host (`keycloak`); the browser resolves it to the host-
        // mapped port. A no-op in host mode (nothing references the name).
        caps.add_arg("--host-resolver-rules=MAP keycloak 127.0.0.1")
            .expect("caps");
        // The dev Keycloak serves its issuer over TLS with the development CA
        // the stack generates (docker-compose.dev.yml `keycloak-tls`), which no
        // browser trust store holds.
        caps.accept_insecure_certs(true).expect("caps");
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

    /// Wait until no toast card is on screen (a visible toast overlays
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

    /// Wait until client hydration has completed on the current page
    /// (FerroUI writes `data-ferroui-hydrated` on the root element once the
    /// page and its lazy route have hydrated).
    /// Required before driving any control whose handler exists only
    /// hydrated — a click or file selection landing earlier is silently
    /// lost, and a same-value re-send fires no later event (#2285).
    ///
    /// This one wait gets [`HYDRATION_WAIT`] rather than [`WAIT`]: it is the
    /// only condition whose latency is dominated by the WASM bundle's size,
    /// which differs by an order of magnitude between the two lanes (module
    /// docs).
    pub(crate) async fn wait_hydrated(&self) {
        self.wait_css_for("html[data-ferroui-hydrated]", HYDRATION_WAIT)
            .await;
    }

    /// Start a journey whose browser is refused the site bundle's
    /// WebAssembly, its glue and its chunks, so no page hydrates, as on a
    /// page whose WebAssembly has not loaded or will not; the stylesheets
    /// still load.
    ///
    /// # Panics
    /// When the stack env is set but the browser session cannot start, or
    /// the browser refuses the blocked addresses.
    pub(crate) async fn start_without_webassembly(journey: &'static str) -> Option<Self> {
        let mut h = Self::start(journey).await?;
        let cdp = h.driver.cdp();
        bounded(
            "cdp(Network.enable)",
            cdp.send_raw("Network.enable", serde_json::json!({})),
        )
        .await
        .expect("enable the network domain");
        bounded(
            "cdp(Network.setBlockedURLs)",
            cdp.send_raw(
                "Network.setBlockedURLs",
                serde_json::json!({ "urls": WITHHELD }),
            ),
        )
        .await
        .expect("block the bundle");
        h.js = false;
        Some(h)
    }

    /// Marks the window, which a document the browser loads anew does not
    /// carry, so [`Self::document_kept`] tells a page the router drew from a
    /// page the browser loaded.
    ///
    /// # Panics
    /// When the browser refuses the script.
    pub(crate) async fn mark_document(&self) {
        bounded(
            "execute(mark)",
            self.driver.execute(
                &format!("window.ferroehrJourney = '{DOCUMENT_MARK}';"),
                Vec::new(),
            ),
        )
        .await
        .expect("mark the window");
    }

    /// Whether the window still carries the mark of [`Self::mark_document`].
    ///
    /// # Panics
    /// When the browser refuses the script.
    pub(crate) async fn document_kept(&self) -> bool {
        let answer = bounded(
            "execute(read the mark)",
            self.driver
                .execute("return window.ferroehrJourney || null;", Vec::new()),
        )
        .await
        .expect("read the window's mark");
        answer.json().as_str() == Some(DOCUMENT_MARK)
    }

    /// Goes back one page in the browser's history.
    ///
    /// # Panics
    /// When the browser does not answer.
    pub(crate) async fn back(&self) {
        bounded("back()", self.driver.back()).await.expect("back");
    }

    /// Goes forward one page in the browser's history.
    ///
    /// # Panics
    /// When the browser does not answer.
    pub(crate) async fn forward(&self) {
        bounded("forward()", self.driver.forward())
            .await
            .expect("forward");
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

    /// Replace the value of the cookie named `name`, keeping it on the
    /// viewer's whole origin.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer.
    pub(crate) async fn set_cookie(&self, name: &str, value: &str) {
        let mut cookie = Cookie::new(name.to_owned(), value.to_owned());
        cookie.set_path("/");
        bounded(
            &format!("add_cookie({name})"),
            self.driver.add_cookie(cookie),
        )
        .await
        .expect("set the cookie");
    }

    /// Press `key` where the focus is, as a person at a keyboard does.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer.
    pub(crate) async fn press(&self, key: impl Into<TypingData>) {
        bounded(
            "action_chain(send_keys).perform()",
            self.driver.action_chain().send_keys(key).perform(),
        )
        .await
        .expect("press the key");
    }

    /// Whether the focused element matches `css`.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer.
    pub(crate) async fn focus_matches(&self, css: &str) -> bool {
        let answer = bounded(
            "execute(activeElement.matches)",
            self.driver.execute(
                "return !!document.activeElement && document.activeElement.matches(arguments[0]);",
                vec![serde_json::Value::String(css.to_owned())],
            ),
        )
        .await
        .expect("read the focused element");
        answer.json().as_bool().unwrap_or(false)
    }

    /// Press Tab until the focus reaches an element matching `css`, at most
    /// `limit` times; whether it got there.
    ///
    /// # Panics
    /// When the `WebDriver` does not answer.
    pub(crate) async fn tab_to(&self, css: &str, limit: usize) -> bool {
        for _ in 0..limit {
            if self.focus_matches(css).await {
                return true;
            }
            self.press(Key::Tab).await;
        }
        self.focus_matches(css).await
    }

    /// Lay the page out at `width` CSS pixels (a phone at 390) in the colour
    /// scheme `dark` asks for, by Chrome `DevTools` Protocol emulation: the
    /// headless window will not size itself below 500 px.
    ///
    /// # Panics
    /// When the browser refuses the emulation.
    pub(crate) async fn emulate(&self, width: u32, dark: bool) {
        let cdp = self.driver.cdp();
        let metrics = if width >= 1440 {
            bounded(
                "Emulation.clearDeviceMetricsOverride",
                cdp.send_raw(
                    "Emulation.clearDeviceMetricsOverride",
                    serde_json::json!({}),
                ),
            )
            .await
        } else {
            bounded(
                "Emulation.setDeviceMetricsOverride",
                cdp.send_raw(
                    "Emulation.setDeviceMetricsOverride",
                    serde_json::json!({
                        "width": width,
                        "height": 844,
                        "deviceScaleFactor": 1,
                        "mobile": true
                    }),
                ),
            )
            .await
        };
        metrics.expect("the browser takes the viewport emulation");
        bounded(
            "Emulation.setEmulatedMedia",
            cdp.send_raw(
                "Emulation.setEmulatedMedia",
                serde_json::json!({
                    "features": [{
                        "name": "prefers-color-scheme",
                        "value": if dark { "dark" } else { "light" }
                    }]
                }),
            ),
        )
        .await
        .expect("the browser takes the colour-scheme emulation");
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

    /// Scroll it to the middle of the viewport, clear of the sticky top bar
    /// and the docked decision bar.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn scroll_to_centre(&self) {
        let arg = self
            .inner
            .to_json()
            .expect("an element handle serializes as a script argument");
        element_command(&format!("scroll_to_centre({})", self.what), async {
            self.inner
                .handle()
                .execute("arguments[0].scrollIntoView({block: 'center'});", vec![arg])
                .await
                .map(|_| ())
        })
        .await;
    }

    /// Replace its value with `text` as a paste does: one `input` event of
    /// type `insertFromPaste` carrying the whole text.
    ///
    /// [`Self::send_keys`] fires a key and an `input` event per character,
    /// and the code editor reworks its whole text on each, so a document of
    /// a few kilobytes costs time quadratic in its length on the debug build.
    ///
    /// # Panics
    /// When the `WebDriver` answers with anything but success.
    pub(crate) async fn paste(&self, text: &str) {
        let arg = self
            .inner
            .to_json()
            .expect("an element handle serializes as a script argument");
        element_command(&format!("paste({})", self.what), async {
            self.inner
                .handle()
                .execute(
                    "arguments[0].value = arguments[1]; \
                     arguments[0].dispatchEvent(new InputEvent('input', \
                       {bubbles: true, inputType: 'insertFromPaste', data: arguments[1]}));",
                    vec![arg, serde_json::Value::String(text.to_owned())],
                )
                .await
                .map(|_| ())
        })
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
/// FerroUI's dialog is never removed from the DOM: a closed `<dialog>` is
/// still findable. Openness is therefore visibility, never mere presence.
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
/// about a dialog: a page holds one `<dialog>` per dialog, closed ones ahead
/// of an open one in document order. "Is a modal up" has to look at all of
/// them or it answers about the wrong one.
pub(crate) async fn is_any_visible(h: &Harness, css: &str) -> bool {
    for element in find_all(h, css).await {
        if element.read_displayed().await == Some(true) {
            return true;
        }
    }
    false
}

/// The footer of the signed-in shell, the authenticated-chrome marker: the
/// shell's footer follows its `.fui-body`, while the centred sign-in page and
/// the not-found page carry a footer with no body beside it.
pub(crate) const SHELL_FOOTER: &str = ".fui-body ~ footer";

/// FerroUI's modal dialog while it is open (the `show-modal` command sets
/// `open`); its `::backdrop` covers the page under it.
const DIALOG_SURFACE: &str = "dialog.fui-dialog[open]";

/// Establish the precondition every page-level click has: nothing is covering
/// the page.
///
/// A modal's backdrop spans the viewport, so a click issued while one is up
/// lands on it instead of the control and `WebDriver` reports
/// `ElementClickIntercepted`. A dialog can be deliberately still open because
/// the CDR refused what it sent (the upload dialog keeps its diagnostic beside
/// the input that caused it), so an open dialog is dismissed through its own
/// Escape path — the same exit a person takes — and the wait covers the close.
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
    wait_hidden(h, DIALOG_SURFACE).await;
}

/// Wait until the dialog holding `control_css` is open: that control visible.
///
/// Takes a control INSIDE the target dialog rather than matching the dialog
/// element, because a page holds one `<dialog>` per dialog, and a bare match
/// answers for whichever comes first — which can be a CLOSED one.
///
/// # Panics
/// When the dialog is not open and settled within 15 s.
pub(crate) async fn wait_dialog_settled(h: &Harness, control_css: &str) {
    for _ in 0..75 {
        if is_visible(h, control_css).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let url = bounded("current_url()", h.driver.current_url())
        .await
        .expect("current url");
    panic!("the dialog holding `{control_css}` never became open and settled (at {url})");
}

/// Every toast card currently on screen.
const TOAST_CARD: &str = ".fui-toast-text";

/// Upload the file at `path` as a template of `family` (`adl14` or `adl2`)
/// through Upload a template: the hydrated file field reads the file into
/// the source editor, and the dock's primary button posts it.
///
/// Leaves the browser on the page that answers the post: the template's own
/// page when the CDR took it, the upload page with the CDR's diagnostic when
/// it refused.
///
/// # Panics
/// When the file never reaches the editor or no page answers the post.
pub(crate) async fn upload_template(h: &Harness, family: &str, path: &str) {
    h.goto(&format!("/templates/upload?family={family}")).await;
    h.wait_css("#source-file").await.send_keys(path).await;
    let read = poll_until_for(Duration::from_secs(15), async || {
        read_prop(h, "#source-text", "value")
            .await
            .is_some_and(|value| !value.is_empty())
    })
    .await;
    assert!(read, "the chosen file never reached the source editor");
    submit(h, "#editor-dock button[value='save']").await;
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

/// Poll until the control at `css` is present, ENABLED and not read-only.
///
/// The viewer keeps an edit form inert until the document it edits has been
/// seeded into it, so this is the condition that makes typing (or saving) safe:
/// input accepted earlier would be replaced by the seed and the save would then
/// commit the pre-seed draft. FerroUI's code editor keeps its text area
/// read-only (not disabled) until the page hydrates and while its form is
/// unseeded, so both properties are read.
///
/// # Panics
/// When it never becomes enabled within 15 s, or when the `WebDriver` stops
/// answering — a stalled driver used to look exactly like an unseeded form.
pub(crate) async fn wait_enabled(h: &Harness, css: &str) {
    for _ in 0..75 {
        if read_enabled(h, css).await == Some(true)
            && read_prop(h, css, "readOnly").await.as_deref() != Some("true")
        {
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

/// Type `text` into the field at `css`, clearing whatever is there first.
///
/// # Panics
/// On any interaction failure.
pub(crate) async fn retype(h: &Harness, css: &str, text: &str) {
    wait_enabled(h, css).await;
    let field = h.wait_css(css).await;
    field.clear().await;
    field.send_keys(text).await;
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
/// A closed `<dialog>` stays in the DOM, so "the dialog opened" is a
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

/// One attempt at reading `attribute` off the first element matching `css`,
/// or `None`.
///
/// `None` covers both "nothing matches yet" and a STALE element handle: a
/// re-rendering table detaches the handle between the find and the read, and
/// `attr` then answers `stale element reference` — a retry, never a failure.
pub(crate) async fn read_attr(h: &Harness, css: &str, attribute: &str) -> Option<String> {
    find_css(h, css).await?.read_attr(attribute).await
}

/// Drive one action through its confirmation dialog: click the trigger, which
/// opens the `<dialog>` by the browser's own `command="show-modal"`, then
/// click the confirm control `confirm_css` inside it. The confirm control is a
/// submit button, so the click posts the page's form and the browser leaves
/// the page; the wait covers the dialog going with it.
///
/// # Panics
/// When the dialog never opens or never closes.
pub(crate) async fn confirm_in_dialog(h: &Harness, trigger_css: &str, confirm_css: &str) {
    // A visible toast overlays the bottom-right corner and intercepts clicks.
    h.wait_toasts_cleared().await;
    clear_dialog_overlay(h).await;
    h.wait_css(trigger_css).await.click().await;
    wait_dialog_settled(h, confirm_css).await;
    submit(h, confirm_css).await;
}

/// Follow the sidebar's link to `href`, opening the drawer first where the
/// sidebar is one (below 48 rem).
///
/// # Panics
/// When the link is not in the sidebar.
pub(crate) async fn follow_nav(h: &Harness, href: &str) {
    let link = format!("nav a[href='{href}']");
    if !is_visible(h, &link).await {
        h.wait_css("button[command='show-modal'][commandfor='sidebar']")
            .await
            .click()
            .await;
        wait_visible(h, &link).await;
    }
    h.wait_css(&link).await.click().await;
}

/// Wait until the text of `main` contains `fragment`.
///
/// # Panics
/// When it never does within 15 s.
pub(crate) async fn wait_main_contains(h: &Harness, fragment: &str) {
    wait_text_contains(h, "main", fragment).await;
}

/// The text of the page's `main`.
///
/// # Panics
/// When the page has no `main`.
pub(crate) async fn main_text(h: &Harness) -> String {
    h.wait_css("main").await.text().await
}

/// Type `text` into the field at `css`, replacing what it holds.
///
/// # Panics
/// When the field is not on the page.
pub(crate) async fn fill(h: &Harness, css: &str, text: &str) {
    let field = h.wait_css(css).await;
    field.clear().await;
    field.send_keys(text).await;
}

/// Click the submit control at `css` and wait for the page that answers the
/// post ([`leave_page`]).
///
/// # Panics
/// When the click fails or no page answers within 15 s.
pub(crate) async fn submit(h: &Harness, css: &str) {
    leave_page(h, &format!("the click on `{css}`"), async {
        let control = h.wait_css(css).await;
        control.scroll_to_centre().await;
        control.click().await;
    })
    .await;
}

/// [`submit`] for a control found some other way than by CSS, such as by
/// an `XPath`: click `control` and wait for the page that answers the post
/// ([`leave_page`]).
///
/// # Panics
/// When the click fails or no page answers within 15 s.
pub(crate) async fn submit_control(h: &Harness, control: Element) {
    let what = format!("the click on `{}`", control.what);
    leave_page(h, &what, async {
        control.scroll_to_centre().await;
        control.click().await;
    })
    .await;
}

/// Press `key` on the focused submit control and wait for the page that
/// answers the post ([`leave_page`]): [`submit`] for a keyboard journey.
///
/// # Panics
/// When the key press fails or no page answers within 15 s.
pub(crate) async fn submit_by_key(h: &Harness, key: impl Into<TypingData>) {
    let key = key.into();
    leave_page(h, "the key press", h.press(key)).await;
}

/// Mark the document, run `action`, and wait for the page that answers it: a
/// new document without the mark, with its `main` drawn.
///
/// A probe that runs before the new document arrives can find an element in
/// the old one and read it after the browser replaced it, which Chrome
/// answers with "Node with given id does not belong to the document", an
/// error [`is_absence`] rightly refuses.
///
/// # Panics
/// When the mark cannot be set or no page answers within 15 s.
async fn leave_page(h: &Harness, what: &str, action: impl Future<Output = ()>) {
    bounded(
        "execute(mark the document)",
        h.driver.execute(
            "document.documentElement.dataset.e2eLeft = '1';",
            Vec::new(),
        ),
    )
    .await
    .expect("mark the document before the post");
    action.await;
    let moved = poll_until_for(Duration::from_secs(15), async || {
        !is_present(h, "html[data-e2e-left]").await
    })
    .await;
    assert!(moved, "{what} never brought a new page");
    h.wait_css("main").await;
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
    h.wait_css("#sign-in-username").await.send_keys(&user).await;
    h.wait_css("#sign-in-password").await.send_keys(&pass).await;
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
    h.wait_css(SHELL_FOOTER).await;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use thirtyfour::error::{WebDriverError, WebDriverErrorInfo, WebDriverErrorInner};

    use super::{COMMAND_BUDGET, WAIT, bounded, element_command, element_probe, is_absence};

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
