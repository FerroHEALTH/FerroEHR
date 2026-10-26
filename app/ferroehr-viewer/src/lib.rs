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

#[cfg(not(target_arch = "wasm32"))]
pub mod activity;
#[cfg(not(target_arch = "wasm32"))]
pub mod adl2;
#[cfg(not(target_arch = "wasm32"))]
pub mod admin;
#[cfg(not(target_arch = "wasm32"))]
pub mod aql_text;
#[cfg(not(target_arch = "wasm32"))]
pub mod auth;
#[cfg(not(target_arch = "wasm32"))]
pub mod builder;
#[cfg(not(target_arch = "wasm32"))]
pub mod chart_model;
#[cfg(not(target_arch = "wasm32"))]
pub mod clinical;
#[cfg(not(target_arch = "wasm32"))]
pub mod deployment;
pub mod error;
#[cfg(not(target_arch = "wasm32"))]
pub mod example_options;
#[cfg(not(target_arch = "wasm32"))]
pub mod feedback;
#[cfg(not(target_arch = "wasm32"))]
pub mod format;
pub mod islands;
pub mod locale;
#[cfg(not(target_arch = "wasm32"))]
pub mod management;
// NOTE: no openEHR spec governs this — our own design; the viewer links no
// application crate, so it compiles the manufacturer's one source file itself.
#[path = "../../ferroehr/src/manufacturer.rs"]
#[cfg(not(target_arch = "wasm32"))]
pub mod manufacturer;
#[cfg(not(target_arch = "wasm32"))]
pub mod queries_api;
#[cfg(not(target_arch = "wasm32"))]
pub mod query_namespace;
#[cfg(not(target_arch = "wasm32"))]
pub mod scopes;
// NOTE: no openEHR spec governs this — our own design; the gap codes and the
// OpenAPI family slugs come from the one file the server holds itself to.
#[path = "../../ferroehr/src/served_vocabulary.rs"]
#[cfg(not(target_arch = "wasm32"))]
pub mod served_vocabulary;
pub mod session_client;
#[cfg(not(target_arch = "wasm32"))]
pub mod subscriptions;
#[cfg(not(target_arch = "wasm32"))]
pub mod system_api;
#[cfg(not(target_arch = "wasm32"))]
pub mod terminology;
#[cfg(not(target_arch = "wasm32"))]
pub mod uid;

#[cfg(not(target_arch = "wasm32"))]
pub mod app;
#[cfg(not(target_arch = "wasm32"))]
pub mod bff;
#[cfg(not(target_arch = "wasm32"))]
pub mod cdr;
#[cfg(not(target_arch = "wasm32"))]
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
pub mod download;
#[cfg(not(target_arch = "wasm32"))]
pub mod export;
#[cfg(not(target_arch = "wasm32"))]
pub mod flash;
#[cfg(not(target_arch = "wasm32"))]
pub mod oidc;
#[cfg(not(target_arch = "wasm32"))]
pub mod pages;
#[cfg(not(target_arch = "wasm32"))]
pub mod paging;
#[cfg(not(target_arch = "wasm32"))]
pub mod posted;
#[cfg(not(target_arch = "wasm32"))]
pub mod prefs;
#[cfg(not(target_arch = "wasm32"))]
pub mod request;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(not(target_arch = "wasm32"))]
pub mod session;
#[cfg(not(target_arch = "wasm32"))]
pub mod shell;
#[cfg(not(target_arch = "wasm32"))]
pub mod state;

/// The WebAssembly entry: hydrates the islands of the page the server drew.
///
/// Every page renders whole on the server; only the islands ship code to the
/// browser, and each takes its words as props, so the browser builds no locale
/// (FerroUI `crates/ferrohealth-ui/CLAUDE.md`, the Leptos book "Guide:
/// Islands"). The library is named so its own islands link into this bundle.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use ferrohealth_ui as _;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_islands();
    // NOTE: the browser journeys wait for this mark before their first interaction (tests/it/common).
    if let Some(body) = leptos::prelude::document().body() {
        drop(body.set_attribute("data-hydrated", ""));
    }
}
