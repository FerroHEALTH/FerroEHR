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
// `hydrate` (wasm client) and `ssr` (server) are mutually exclusive build
// modes — cargo-leptos always builds them separately. Guarded per the Cargo
// book's prescription for genuinely exclusive features
// (https://doc.rust-lang.org/cargo/reference/features.html#mutually-exclusive-features);
// CI's workspace lanes therefore run `--exclude ferroehr-viewer` under
// `--all-features` and lint/test this crate per-feature instead.
#[cfg(all(feature = "hydrate", feature = "ssr"))]
compile_error!("features \"hydrate\" and \"ssr\" cannot be enabled at the same time");

pub mod activity;
pub mod adl2;
pub mod admin;
pub mod app;
pub mod aql_text;
pub mod auth;
pub mod builder;
pub mod chart_model;
pub mod clinical;
pub mod components;
pub mod deployment;
pub mod error;
pub mod example_options;
pub mod feedback;
pub mod format;
pub mod highlight;
pub mod management;
// NOTE: no openEHR spec governs this — our own design; the viewer links no
// application crate, so it compiles the manufacturer's one source file itself.
#[path = "../../ferroehr/src/manufacturer.rs"]
pub mod manufacturer;
pub mod pages;
pub mod queries_api;
pub mod query_namespace;
pub mod scopes;
pub mod session_client;
pub mod subscriptions;
pub mod system_api;
pub mod terminology;
pub mod theme;
pub mod uid;

#[cfg(feature = "ssr")]
pub mod cdr;
#[cfg(feature = "ssr")]
pub mod config;
#[cfg(feature = "ssr")]
pub mod export;
#[cfg(feature = "ssr")]
pub mod oidc;
#[cfg(feature = "ssr")]
pub mod server;
#[cfg(feature = "ssr")]
pub mod session;
#[cfg(feature = "ssr")]
pub mod state;

/// WASM entry point: attaches interactivity to the server-rendered DOM.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
