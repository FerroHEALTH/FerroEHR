// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `FerroEHR` viewer: a server-rendered web console + BFF over the ITS-REST
//! API.
//!
//! No openEHR spec governs the viewer: our own design / product extension.
//! The three binding mandates: no script of its own (every page is drawn on
//! the server from FerroUI's parts, and every behaviour is a FerroUI
//! module), the CDR is reached ONLY over ITS-REST (never `app/ferroehr` /
//! `app/ferroehr-rest` in-process), and every route is a publicly reachable
//! endpoint that enforces the viewer's own session auth itself.

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]

pub mod activity;
pub mod adl2;
pub mod admin;
pub mod aql_text;
pub mod auth;
pub mod builder;
pub mod chart_model;
pub mod cli;
pub mod clinical;
pub mod components;
pub mod deployment;
pub mod error;
pub mod example_options;
pub mod feedback;
pub mod format;
pub mod locale;
pub mod management;
// NOTE: no openEHR spec governs this — our own design; the viewer links no
// application crate, so it compiles the manufacturer's one source file itself.
#[path = "../../ferroehr/src/manufacturer.rs"]
pub mod manufacturer;
pub mod markup;
pub mod queries_api;
pub mod query_namespace;
pub mod scopes;
// NOTE: no openEHR spec governs this — our own design; the gap codes and the
// OpenAPI family slugs come from the one file the server holds itself to.
#[path = "../../ferroehr/src/served_vocabulary.rs"]
pub mod served_vocabulary;
pub mod session_client;
pub mod subscriptions;
pub mod system_api;
pub mod terminology;
pub mod uid;

pub mod app;
pub mod bff;
pub mod cdr;
pub mod config;
pub mod download;
pub mod export;
pub mod flash;
pub mod oidc;
pub mod pages;
pub mod paging;
pub mod posted;
pub mod prefs;
pub mod request;
pub mod routes;
pub mod scope;
pub mod server;
pub mod session;
pub mod shell;
pub mod state;
pub mod telemetry;
