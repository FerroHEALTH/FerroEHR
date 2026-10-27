// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `FerroEHR` viewer — Leptos SSR web app + BFF over the ITS-REST API.
//!
//! No openEHR spec governs an
//! viewer — our own design / product extension. The three binding
//! mandates: Rust only (zero authored JavaScript), the CDR is reached ONLY
//! over ITS-REST (never `app/ferroehr` / `app/ferroehr-rest` in-process), and
//! every `#[server]` fn is a publicly reachable endpoint that enforces the
//! viewer's own session auth itself.

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
// NOTE: FerroUI's sign-in view nests deep enough that computing its layout inside an async page passes the default limit of 128 (rustc reference, `recursion_limit`).
#![recursion_limit = "256"]
// Every `#[component]` in this crate expands `#[derive(TypedBuilder)]` on its
// generated props struct, and that derive emits an inherent `builder()` whose
// name matches a trait method already in scope — so `same_name_method` fires
// once per component with the macro invocation as its only span, never on
// hand-written code. Crate-level because there is no smaller item to scope it
// to: the finding does not exist in this crate's source
// (https://docs.rs/leptos/0.8/leptos/attr.component.html).
#![allow(
    clippy::same_name_method,
    reason = "emitted only by leptos's TypedBuilder derive inside #[component]; no hand-written method in this crate shadows a trait method"
)]
// The wasm half of every `#[server]` fn is a network call, so the `ServerFn`
// impl the macro writes for the client target awaits nothing. The span is the
// `#[server]` attribute in all 126 cases and there is no hand-written async fn
// behind any of them
// (https://docs.rs/server_fn/0.8/server_fn/attr.server.html).
#![allow(
    clippy::unused_async_trait_impl,
    reason = "emitted only by the server_fn #[server] macro on the wasm target; this crate writes no such impl"
)]
// The server half (`not(target_arch = "wasm32")`) and the browser half
// (`target_arch = "wasm32"`) are chosen by compilation target, never by Cargo
// feature, so `--all-features` over the workspace can never turn both on.

pub mod activity;
pub mod adl2;
#[cfg(not(target_arch = "wasm32"))]
pub mod admin;
#[cfg(not(target_arch = "wasm32"))]
pub mod aql_text;
pub mod auth;
pub mod builder;
#[cfg(not(target_arch = "wasm32"))]
pub mod chart_model;
#[cfg(not(target_arch = "wasm32"))]
pub mod cli;
pub mod clinical;
pub mod components;
#[cfg(not(target_arch = "wasm32"))]
pub mod deployment;
pub mod error;
pub mod example_options;
#[cfg(not(target_arch = "wasm32"))]
pub mod feedback;
pub mod format;
pub mod locale;
pub mod management;
// NOTE: no openEHR spec governs this — our own design; the viewer links no
// application crate, so it compiles the manufacturer's one source file itself.
#[path = "../../ferroehr/src/manufacturer.rs"]
#[cfg(not(target_arch = "wasm32"))]
pub mod manufacturer;
#[cfg(not(target_arch = "wasm32"))]
pub mod queries_api;
pub mod query_namespace;
pub mod scopes;
// NOTE: no openEHR spec governs this — our own design; the gap codes and the
// OpenAPI family slugs come from the one file the server holds itself to.
#[path = "../../ferroehr/src/served_vocabulary.rs"]
#[cfg(not(target_arch = "wasm32"))]
pub mod served_vocabulary;
pub mod session_client;
pub mod subscriptions;
#[cfg(not(target_arch = "wasm32"))]
pub mod system_api;
pub mod terminology;
pub mod uid;

pub mod app;
pub mod bff;
#[cfg(not(target_arch = "wasm32"))]
pub mod cdr;
#[cfg(not(target_arch = "wasm32"))]
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
pub mod download;
#[cfg(not(target_arch = "wasm32"))]
pub mod export;
pub mod flash;
#[cfg(not(target_arch = "wasm32"))]
pub mod oidc;
pub mod pages;
pub mod paging;
pub mod posted;
pub mod prefs;
pub mod request;
pub mod routes;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(not(target_arch = "wasm32"))]
pub mod session;
pub mod shell;
#[cfg(not(target_arch = "wasm32"))]
pub mod state;

/// Room in the browser half's read-only data that nothing reads.
///
/// The code splitter (`cargo leptos build --split`, `wasm_split_cli_support`)
/// lays the read-only data out again for the entry and each chunk, aligned,
/// and when that layout outgrows the input's data segment it leaves the whole
/// segment in the entry WebAssembly, about 63000 brotli bytes against the
/// viewer's budget. These bytes give the layout room; no module reads them,
/// so the splitter writes them into none.
// NOTE: wasm-split defect #4225; without these bytes the entry measured 474943 of 450000.
// TODO(#4225): drop the room once the splitter splits an overlong segment.
#[cfg(target_arch = "wasm32")]
#[used]
static SPLIT_ROOM: [u8; 4096] = [0xA5; 4096];

/// The WebAssembly entry: loads the code of the page's lazy group
/// ([`routes::preload`]), then hydrates the page the server drew, whole,
/// with FerroUI's `widget::hydration::hydrate`.
///
/// The browser then draws every page after the first itself (FerroUI
/// `docs/consoles.md`, "How a console builds"). Until the page has hydrated
/// a click or a submit is the browser's own, and the root element then
/// carries `data-ferroui-hydrated`, which the browser journeys wait for
/// before they act.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    ferrohealth_ui::widget::hydration::explain();
    wasm_bindgen_futures::spawn_local(async {
        let path = leptos::prelude::window()
            .location()
            .pathname()
            .unwrap_or_default();
        routes::preload(&path).await;
        ferrohealth_ui::widget::hydration::hydrate(app::App);
    });
}
