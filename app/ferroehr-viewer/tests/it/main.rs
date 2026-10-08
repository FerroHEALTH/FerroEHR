// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one viewer integration binary (`testing.md` §Where tests live):
//! every journey battery is a module here; the shared browser/router/DB
//! fixture lives in `common`.
//!
//! Two kinds of module live here. The `e2e_*` journeys drive a real browser
//! against a composed stack and skip-with-reason when it is absent;
//! `ssr_components` (the shared kit), `ssr_pages` (whole routed screens) and
//! `ssr_shell` (the persistent chrome, rendered through the whole route tree)
//! render in-process with no external process at all, so the component/page
//! code is exercised by the ordinary `--features ssr` test lane (and by the
//! coverage lane over it).

mod common;

mod ssr_components;
mod ssr_pages;
mod ssr_shell;

mod e2e_adl2;
mod e2e_admin_ops;
mod e2e_audit;
mod e2e_browse;
mod e2e_chart;
mod e2e_composition;
mod e2e_compositions_filters;
mod e2e_contribution_authoring;
mod e2e_demographics;
mod e2e_directory;
mod e2e_docs_shots;
mod e2e_ehr_ops;
mod e2e_ehr_status;
mod e2e_event_subscriptions;
mod e2e_export;
mod e2e_login;
mod e2e_operations;
mod e2e_paging;
mod e2e_scopes;
mod e2e_session_expiry;
mod e2e_stored_query_runner;
mod e2e_system;
mod e2e_tags;
mod e2e_terminology;
