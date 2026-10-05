// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! In-process SSR render tests for the routed SCREENS.
//!
//! The sibling `ssr_components` module renders the shared kit; this one renders
//! whole pages the same way — `RenderHtml::to_html`, the exact call the axum
//! integration makes for the server pass — with no browser, no composed stack
//! and no CDR.
//!
//! What makes that possible is the viewer's own discipline. A screen's data
//! lives in `Resource`s read under `<Suspense>`/`<Transition>`, and everything
//! else — the page header, the URL-driven forms, the tab strip, the skeletons —
//! is built in component SETUP, outside any `Suspend` closure (rules §2/§4/§9).
//! The server pass therefore renders the whole static skeleton of a screen
//! synchronously and each suspended region as its fallback. That is exactly
//! what these tests assert: the heading and subtitle a screen declares, the
//! form fields and ids the E2E journeys drive, and the loading state a reader
//! sees before any data arrives.
//!
//! The harness provides the contexts a page reads at setup, mirroring what the
//! running viewer provides:
//!
//! * `leptos_meta` — every routed screen sets a `<Title/>`.
//! * `thaw::ConfigProvider` + `thaw::ToasterProvider` — the widget kit's
//!   injections; screens resolve `ToasterInjection` in setup, and the shell is
//!   where the running viewer mounts the toaster.
//! * `<Router>` + a `RequestUrl` — filter/search/paging state is URL state
//!   (rules §9), read in setup through `use_query_map`.
//! * The matched route's params (an `ArcMemo<ParamsMap>`, what a matched
//!   `<Route>` provides) — the detail screens read `:ehr_id`/`:uid`/`:kind`
//!   from it.
//! * [`ferroehr_viewer::state::AppState`] — the BFF state
//!   `leptos_routes_with_context` provides to every server function. A
//!   `Resource` polls its future once when it is created, so a server fn body
//!   runs as far as its first `await`; without the state that first step
//!   panics on `expect_context`. It is pointed at an unroutable loopback port,
//!   and the runtime never polls the future again, so no request is ever made.
//!
//! `ssr`-gated: `to_html` is the server-pass renderer, and the `ssr` feature is
//! what puts `leptos-use`/`thaw`/`leptos-chartistry` on their non-`WASM` code
//! paths (`Cargo.toml` §features).

#![cfg(feature = "ssr")]

use leptos::prelude::*;
use leptos_router::params::ParamsMap;

/// Render one screen's server pass and return the HTML.
///
/// `url` is the request URL the router resolves against; `params` are the route
/// parameters a matched `<Route>` would have captured. The body is a CLOSURE:
/// the context providers above it provide their injections inside a child owner
/// and then call `children()`, so a view built before the call would look its
/// context up in the wrong scope and panic.
fn render_page(
    url: &str,
    params: &[(&str, &str)],
    build: impl FnOnce() -> AnyView + Send + 'static,
) -> String {
    // The one public `leptos_axum` entry point that initializes the reactive
    // executor (`Executor::init_tokio`) the way the viewer's own server does;
    // an app fn with no `<Router>` yields no routes and does no work. Without
    // it, creating the first `Resource` panics.
    drop(leptos_axum::generate_route_list(|| ()));
    let owner = Owner::new();
    let url = url.to_owned();
    let mut map = ParamsMap::new();
    for (key, value) in params {
        map.replace((*key).to_owned(), (*value).to_owned());
    }
    owner.with(move || {
        leptos_meta::provide_meta_context();
        provide_context(leptos_router::location::RequestUrl::new(&url));
        provide_context(ArcMemo::new(move |_| map.clone()));
        provide_context(app_state());
        view! {
            <thaw::ConfigProvider>
                <thaw::ToasterProvider>
                    <leptos_router::components::Router>{build()}</leptos_router::components::Router>
                </thaw::ToasterProvider>
            </thaw::ConfigProvider>
        }
        .to_html()
    })
}

/// The BFF state every `#[server]` fn resolves, aimed at a port nothing serves.
///
/// A server fn body runs as far as its first `await` when its resource is
/// created; nothing polls it after that, so the CDR client never completes a
/// request.
#[expect(
    clippy::expect_used,
    reason = "CdrClient::new fails only if reqwest cannot build a client from a fixed timeout and \
              redirect policy; a fixture that cannot be built is a broken harness, not a test \
              outcome"
)]
fn app_state() -> ferroehr_viewer::state::AppState {
    let config = ferroehr_viewer::config::ViewerConfig {
        cdr: ferroehr_viewer::config::CdrConfig {
            base_url: "http://127.0.0.1:9".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    };
    ferroehr_viewer::state::AppState {
        cdr: ferroehr_viewer::cdr::CdrClient::new(&config.cdr)
            .expect("the CDR client should build from a well-formed base URL"),
        config: std::sync::Arc::new(config),
        oidc: None,
        session_keys: ferroehr_viewer::session::SessionKeys::from_secret("")
            .expect("an empty secret always yields an ephemeral key"),
    }
}

// -------------------------------------------------------------- dashboard

#[tokio::test]
async fn the_dashboard_renders_its_header_over_skeletons_for_every_data_section() {
    let html = render_page("/", &[], || {
        view! { <ferroehr_viewer::pages::dashboard::DashboardPage /> }.into_any()
    });
    assert!(html.contains(">Dashboard</h1>"), "{html}");
    assert!(
        html.contains("Repository overview, query cohorts, and recent commit activity."),
        "{html}"
    );
    // The stat tiles are a four-column grid; both cards below them are titled.
    assert!(html.contains("grid-cols-2 md:grid-cols-4"), "{html}");
    assert!(html.contains(">Query namespaces</h2>"), "{html}");
    assert!(html.contains(">Commit activity</h2>"), "{html}");
    // Every one of those sections is a Resource, so the server pass is all
    // skeleton: four tiles, the namespace card, the activity chart.
    assert_eq!(html.matches("aria-busy=\"true\"").count(), 6, "{html}");
}

// ------------------------------------------------------------------ login

#[tokio::test]
async fn the_login_screen_renders_the_wordmark_over_a_spinner_until_the_modes_resolve() {
    let html = render_page("/login", &[], || {
        view! { <ferroehr_viewer::pages::login::LoginPage /> }.into_any()
    });
    assert!(html.contains("FerroEHR"), "{html}");
    assert!(html.contains("Viewer"), "{html}");
    assert!(html.contains("thaw-spinner"), "{html}");
    // Which sign-in paths exist is the CDR's answer, awaited inside the
    // Suspense — so neither form is in the server pass.
    assert!(!html.contains("id=\"login-username\""), "{html}");
    assert!(!html.contains("Sign in with OIDC"), "{html}");
}

// -------------------------------------------------------------- templates

#[tokio::test]
async fn the_templates_screen_renders_its_header_and_the_opt_upload_control() {
    let html = render_page("/templates", &[], || {
        view! { <ferroehr_viewer::pages::templates::TemplatesPage /> }.into_any()
    });
    assert!(html.contains(">Templates</h1>"), "{html}");
    // ONE upload trigger, in the page-header action slot, labelled for the
    // family the URL names (#2955). Its dialog is teleported by thaw and so
    // contributes nothing to either pass; the real `<input type=file>` inside
    // it — the no-JS mandate — is driven in the browser by every journey that
    // seeds a template (`common::upload_via_dialog`).
    assert!(html.contains("id=\"template-upload-open\""), "{html}");
    assert!(html.contains("Upload OPT"), "{html}");
    assert!(!html.contains("type=\"file\""), "{html}");
}

#[tokio::test]
async fn the_template_detail_screen_renders_its_crumb_trail_panes_and_format_tabs() {
    let html = render_page(
        "/templates/vitals.v1",
        &[("template_id", "vitals.v1")],
        || view! { <ferroehr_viewer::pages::template_detail::TemplateDetailPage /> }.into_any(),
    );
    assert!(html.contains("href=\"/templates\""), "{html}");
    assert!(html.contains(">vitals.v1</h1>"), "{html}");
    assert!(html.contains(">Path catalog (WT tree)</h2>"), "{html}");
    assert!(html.contains(">Node inspector</h2>"), "{html}");
    assert!(
        html.contains("Select a node to inspect its path and metadata."),
        "{html}"
    );
    // The example pane offers all four serializations the CDR can produce.
    for format in ["JSON", "XML", "FLAT", "STRUCTURED"] {
        assert!(html.contains(format), "{format} missing: {html}");
    }
}

#[tokio::test]
async fn the_adl2_template_detail_screen_renders_its_four_panes_and_the_version_form() {
    let html = render_page(
        "/templates/adl2/org.example.t.v1",
        &[("template_id", "org.example.t.v1")],
        || view! { <ferroehr_viewer::pages::template_adl2::Adl2TemplateDetailPage /> }.into_any(),
    );
    assert!(html.contains(">org.example.t.v1</h1>"), "{html}");
    // All four panes are always mounted and toggled with `class:hidden`, so the
    // server and client view structure stay identical (rules §8).
    for pane in [
        "id=\"adl2-source-pane\"",
        "id=\"adl2-json-pane\"",
        "id=\"adl2-catalog-pane\"",
        "id=\"adl2-example-pane\"",
    ] {
        assert!(html.contains(pane), "{pane} missing: {html}");
    }
    // The version selector is URL state: a GET form carrying the active tab.
    assert!(html.contains("id=\"adl2-version-input\""), "{html}");
    assert!(html.contains("name=\"version\""), "{html}");
    assert!(html.contains("name=\"tab\""), "{html}");
}

// ---------------------------------------------------------------- queries

#[tokio::test]
async fn the_queries_screen_renders_both_panes_with_their_own_loading_copy() {
    let html = render_page("/queries", &[], || {
        view! { <ferroehr_viewer::pages::queries::QueriesPage /> }.into_any()
    });
    assert!(html.contains(">Queries</h1>"), "{html}");
    assert!(html.contains("href=\"/queries/builder\""), "{html}");
    assert!(html.contains("href=\"/queries/aql\""), "{html}");
    assert!(html.contains(">Stored queries</h2>"), "{html}");
    assert!(html.contains("Loading query…"), "{html}");
    assert!(html.contains(">Namespaces</h2>"), "{html}");
    assert!(html.contains("Loading namespaces…"), "{html}");
}

#[tokio::test]
async fn the_query_builder_renders_its_empty_criteria_state_and_the_default_aql() {
    let html = render_page("/queries/builder", &[], || {
        view! { <ferroehr_viewer::pages::query_builder::QueryBuilderPage /> }.into_any()
    });
    assert!(html.contains(">Query builder</h1>"), "{html}");
    // The template picker is the one suspended control; everything else is
    // built from the builder's own state in setup.
    assert!(html.contains("for=\"qb-template\""), "{html}");
    assert!(html.contains("Loading templates…"), "{html}");
    assert!(html.contains(">Path catalog</h2>"), "{html}");
    assert!(html.contains(">Criteria</h2>"), "{html}");
    assert!(html.contains(">No conditions yet</p>"), "{html}");
    for field in [
        "id=\"qb-limit\"",
        "id=\"qb-save-namespace\"",
        "id=\"qb-save-name\"",
        "id=\"qb-save-version\"",
    ] {
        assert!(html.contains(field), "{field} missing: {html}");
    }
    // The AQL preview is the pure lowering of the empty model, so the server
    // pass already carries the statement an empty builder produces — and the
    // hand-off link to the raw editor carries it percent-encoded.
    assert!(html.contains(">AQL preview</h2>"), "{html}");
    assert!(
        html.contains(">SELECT c FROM EHR e CONTAINS COMPOSITION c LIMIT 50</pre>"),
        "{html}"
    );
    assert!(
        html.contains(
            "href=\"/queries/aql?aql=SELECT%20c%20FROM%20EHR%20e%20CONTAINS%20COMPOSITION%20c%20LIMIT%2050\""
        ),
        "{html}"
    );
}

#[tokio::test]
async fn the_raw_aql_screen_renders_the_editor_the_parameter_field_and_the_save_fields() {
    let html = render_page("/queries/aql", &[], || {
        view! { <ferroehr_viewer::pages::query_aql::QueryAqlPage /> }.into_any()
    });
    assert!(html.contains("href=\"/queries\""), "{html}");
    assert!(html.contains(">Raw AQL</h1>"), "{html}");
    assert!(html.contains("id=\"aql-editor\""), "{html}");
    assert!(html.contains("id=\"aql-params\""), "{html}");
    assert!(html.contains("id=\"aql-save-name\""), "{html}");
    assert!(html.contains(">Validate</"), "{html}");
}

#[tokio::test]
async fn the_stored_query_runner_renders_the_three_version_resolutions_and_its_run_control() {
    let html = render_page("/queries/stored?load=org.example::demo/1.0.0", &[], || {
        view! { <ferroehr_viewer::pages::query_stored::QueryStoredPage /> }.into_any()
    });
    assert!(html.contains(">Run stored query</h1>"), "{html}");
    assert!(html.contains("id=\"stored-run-mode\""), "{html}");
    assert!(html.contains("id=\"stored-run-version\""), "{html}");
    assert!(html.contains("id=\"stored-run\""), "{html}");
    // The three ways a stored query's version resolves on the wire.
    assert!(html.contains("Latest version (no version sent)"), "{html}");
    assert!(html.contains("Version prefix (latest match)"), "{html}");
    assert!(html.contains("Exact version"), "{html}");
    // The parameter bindings come from the stored definition, so they are the
    // one suspended region of the form.
    assert!(html.contains(">Parameters</h2>"), "{html}");
    assert!(html.contains("Reading parameters…"), "{html}");
}

// ------------------------------------------------------------------- EHRs

#[tokio::test]
async fn the_ehrs_screen_renders_the_create_form_and_both_lookup_forms() {
    let html = render_page("/ehrs", &[], || {
        view! { <ferroehr_viewer::pages::ehrs::EhrsPage /> }.into_any()
    });
    assert!(html.contains(">EHRs</h1>"), "{html}");
    for field in [
        "id=\"ehr-create-id\"",
        "id=\"ehr-create-subject-id\"",
        "id=\"ehr-create-subject-namespace\"",
        "id=\"ehr-create-submit\"",
        "id=\"ehr-lookup\"",
        "id=\"ehr-subject-find\"",
    ] {
        assert!(html.contains(field), "{field} missing: {html}");
    }
}

/// One EHR-detail render exercises the whole screen: the seven tab bodies are
/// always mounted and toggled with `class:hidden` (rules §8), so each tab's
/// setup and its non-suspended chrome reach the server pass together.
#[tokio::test]
async fn the_ehr_detail_screen_renders_every_tab_body_in_one_server_pass() {
    let html = render_page("/ehrs/e1?tab=status", &[("ehr_id", "e1")], || {
        view! { <ferroehr_viewer::pages::ehr_detail::EhrDetailPage /> }.into_any()
    });
    assert!(html.contains("href=\"/ehrs\""), "{html}");
    assert!(html.contains(">EHR e1…</h1>"), "{html}");
    assert!(html.contains("id=\"ehr-summary\""), "{html}");
    // Every tab is reachable as a plain URL.
    for tab in [
        "status-history",
        "directory",
        "compositions",
        "contributions",
        "commit",
        "tags",
    ] {
        assert!(
            html.contains(&format!("href=\"/ehrs/e1?tab={tab}\"")),
            "{tab} link missing: {html}"
        );
    }
    // Status tab: the edit form is inert until the current document seeds it,
    // and the STATIC `disabled` attribute is what makes it inert on the server
    // HTML (rules §2).
    assert!(html.contains("id=\"status-edit\""), "{html}");
    assert!(
        html.contains("<button id=\"status-save\" type=\"button\" disabled"),
        "{html}"
    );
    // Status-history tab, then the directory tab's time-travel panel.
    assert!(html.contains(">Version history</h2>"), "{html}");
    assert!(html.contains("id=\"directory-at-time\""), "{html}");
    assert!(html.contains("id=\"directory-path\""), "{html}");
    // Compositions tab: the filter bar is a GET form whose fields ARE the URL
    // state, and it carries the active tab across the submit (rules §9).
    assert!(
        html.contains("<form method=\"GET\" action=\"/ehrs/e1\" id=\"compositions-filter\""),
        "{html}"
    );
    for field in ["template", "from", "to", "composer"] {
        assert!(
            html.contains(&format!("name=\"{field}\"")),
            "filter field {field} missing: {html}"
        );
    }
    assert!(
        html.contains("href=\"/ehrs/e1?tab=compositions\""),
        "{html}"
    );
    // Contributions, commit staging, and the EHR-wide tag browser.
    assert!(html.contains("id=\"contribution-uid\""), "{html}");
    assert!(html.contains("id=\"stage-add\""), "{html}");
    assert!(html.contains("id=\"stage-list\""), "{html}");
    assert!(html.contains(">Nothing staged yet</p>"), "{html}");
    assert!(html.contains("id=\"ehr-tag-browser\""), "{html}");
}

#[tokio::test]
async fn the_composition_viewer_renders_its_time_travel_editor_and_tag_panel() {
    let html = render_page(
        "/ehrs/e1/compositions/u1",
        &[("ehr_id", "e1"), ("uid", "u1")],
        || view! { <ferroehr_viewer::pages::composition::CompositionPage /> }.into_any(),
    );
    assert!(html.contains(">Composition u1…</h1>"), "{html}");
    assert!(html.contains("id=\"version-at-time\""), "{html}");
    assert!(html.contains("id=\"edit-new-version\""), "{html}");
    assert!(html.contains("id=\"edit-body\""), "{html}");
    assert!(html.contains("id=\"composition-tag-set\""), "{html}");
    assert!(html.contains("id=\"composition-delete\""), "{html}");
}

// ----------------------------------------------------------- demographics

/// `/demographics` is a redirect, not a screen: it renders the section title
/// and a `<Redirect/>` to the first party kind, and nothing else.
#[tokio::test]
async fn the_demographics_landing_route_renders_no_chrome_of_its_own() {
    let html = render_page("/demographics", &[], || {
        view! { <ferroehr_viewer::pages::demographics::browse::DemographicsPage /> }.into_any()
    });
    assert!(!html.contains("<h1"), "{html}");
    assert!(!html.contains("<section"), "{html}");
}

#[tokio::test]
async fn an_unknown_party_kind_renders_the_five_kinds_it_could_have_been() {
    let html = render_page("/demographics/wombat", &[("kind", "wombat")], || {
        ferroehr_viewer::pages::demographics::browse::unknown_kind_view("wombat")
    });
    assert!(html.contains(">Unknown party kind</h1>"), "{html}");
    assert!(html.contains("id=\"demographics-unknown-kind\""), "{html}");
    // The segment is echoed verbatim: an unreadable value is shown, never
    // swallowed.
    assert!(html.contains(">wombat<"), "{html}");
    for kind in ["People", "Organisations", "Groups", "Agents", "Roles"] {
        assert!(html.contains(kind), "{kind} missing: {html}");
    }
}

#[tokio::test]
async fn the_party_browser_renders_the_kind_strip_the_lookup_the_create_form_and_the_tag_index() {
    let html = render_page("/demographics/person", &[("kind", "person")], || {
        view! { <ferroehr_viewer::pages::demographics::browse::PartyBrowserPage /> }.into_any()
    });
    assert!(html.contains(">People</h1>"), "{html}");
    // The five kinds are a closed set, and every one is a link on every browser.
    assert_eq!(html.matches("data-kind=\"").count(), 5, "{html}");
    assert!(html.contains("id=\"party-lookup\""), "{html}");
    assert!(html.contains("id=\"party-create-body\""), "{html}");
    assert!(html.contains("id=\"party-create-submit\""), "{html}");
    assert!(html.contains("id=\"demographic-tag-index\""), "{html}");
    assert!(html.contains("name=\"tag_key\""), "{html}");
}

#[tokio::test]
async fn the_party_detail_screen_renders_its_edit_history_and_tag_panels() {
    let html = render_page(
        "/demographics/person/u1",
        &[("kind", "person"), ("uid", "u1")],
        || view! { <ferroehr_viewer::pages::demographics::party::PartyDetailPage /> }.into_any(),
    );
    assert!(html.contains(">PERSON u1…</h1>"), "{html}");
    assert!(html.contains("id=\"party-edit\""), "{html}");
    // Inert until the served document seeds it — the static attribute, not a
    // binding, is what the server HTML carries (rules §2).
    assert!(
        html.contains("<button id=\"party-save\" type=\"button\" disabled"),
        "{html}"
    );
    assert!(html.contains(">Revision history</h2>"), "{html}");
    assert!(html.contains("id=\"demographic-at-time\""), "{html}");
    assert!(html.contains(">Version document</h2>"), "{html}");
    assert!(html.contains("id=\"party-tag-set\""), "{html}");
    assert!(html.contains("id=\"party-delete\""), "{html}");
}

#[tokio::test]
async fn the_relationships_screen_renders_the_lookup_and_the_two_ended_create_form() {
    let html = render_page("/demographics/relationship", &[], || {
        view! { <ferroehr_viewer::pages::demographics::relationship::RelationshipsPage /> }
            .into_any()
    });
    assert!(html.contains(">Party relationships</h1>"), "{html}");
    assert!(html.contains("id=\"relationship-lookup\""), "{html}");
    for field in [
        "id=\"relationship-type\"",
        "id=\"relationship-archetype\"",
        "id=\"relationship-source\"",
        "id=\"relationship-source-kind\"",
        "id=\"relationship-target\"",
        "id=\"relationship-target-kind\"",
        "id=\"relationship-create-submit\"",
    ] {
        assert!(html.contains(field), "{field} missing: {html}");
    }
}

#[tokio::test]
async fn the_relationship_detail_screen_renders_its_edit_form_and_history_panels() {
    let html = render_page("/demographics/relationship/u1", &[("uid", "u1")], || {
        view! { <ferroehr_viewer::pages::demographics::relationship::RelationshipDetailPage /> }
            .into_any()
    });
    assert!(html.contains(">Relationship u1…</h1>"), "{html}");
    assert!(html.contains("id=\"relationship-edit\""), "{html}");
    assert!(html.contains("id=\"relationship-edit-type\""), "{html}");
    assert!(html.contains("id=\"relationship-save\""), "{html}");
    assert!(html.contains("id=\"relationship-delete\""), "{html}");
    assert!(html.contains(">Revision history</h2>"), "{html}");
}

#[tokio::test]
async fn the_demographic_contribution_screen_renders_its_crumb_trail_and_heading() {
    let html = render_page("/demographics/contribution/u1", &[("uid", "u1")], || {
        view! { <ferroehr_viewer::pages::demographics::contribution::DemographicContributionPage /> }
            .into_any()
    });
    assert!(html.contains(">Contribution u1…</h1>"), "{html}");
    assert!(html.contains("Demographics"), "{html}");
    assert!(html.contains("aria-current=\"page\""), "{html}");
}

// ------------------------------------------------------------ terminology

/// The terminology surface is config-gated on the CDR side, so the whole screen
/// waits on the probe: the server pass is the header over one skeleton.
#[tokio::test]
async fn the_terminology_screen_renders_its_header_over_the_probe_skeleton() {
    let html = render_page("/terminology", &[], || {
        view! { <ferroehr_viewer::pages::terminology::TerminologyPage /> }.into_any()
    });
    assert!(html.contains("id=\"terminology-screen\""), "{html}");
    assert!(html.contains(">Terminology</h1>"), "{html}");
    assert!(html.contains("thaw-skeleton"), "{html}");
    // Nothing that needs the surface to exist is rendered before the probe.
    assert!(!html.contains("id=\"terminology-code\""), "{html}");
    assert!(!html.contains("id=\"terminology-disabled\""), "{html}");
}

// ------------------------------------------------------------------ audit

#[tokio::test]
async fn the_audit_screen_renders_a_get_filter_form_over_every_outcome_and_action() {
    let html = render_page("/audit", &[], || {
        view! { <ferroehr_viewer::pages::audit::AuditPage /> }.into_any()
    });
    assert!(html.contains(">Audit log</h1>"), "{html}");
    // Filter state is URL state, so the form is a plain GET back to the screen.
    assert!(
        html.contains("<form method=\"GET\" action=\"/audit\""),
        "{html}"
    );
    for field in ["from", "to", "patient", "agent", "outcome", "action"] {
        assert!(
            html.contains(&format!("name=\"{field}\"")),
            "filter field {field} missing: {html}"
        );
    }
    // The two closed vocabularies are offered in full.
    for outcome in [
        "success",
        "minor failure",
        "serious failure",
        "major failure",
    ] {
        assert!(html.contains(outcome), "{outcome} missing: {html}");
    }
    for action in ["create", "read", "update", "delete", "execute"] {
        assert!(html.contains(action), "{action} missing: {html}");
    }
}

// ----------------------------------------------------------------- system

#[tokio::test]
async fn the_system_screen_renders_its_cards_and_the_openapi_family_picker() {
    let html = render_page("/system", &[], || {
        view! { <ferroehr_viewer::pages::system::SystemPage /> }.into_any()
    });
    assert!(html.contains(">System</h1>"), "{html}");
    for card in [
        ">Status</h2>",
        ">Conformance manifest</h2>",
        ">SMART</h2>",
        ">Repository usage</h2>",
        ">Served OpenAPI</h2>",
        ">Runtime configuration (redacted)</h2>",
    ] {
        assert!(html.contains(card), "{card} missing: {html}");
    }
    assert!(html.contains("id=\"openapi-family\""), "{html}");
    assert!(html.contains("name=\"openapi\""), "{html}");
    assert!(html.contains("id=\"openapi-family-show\""), "{html}");
    // The audit trail lives on its own screen; this one cross-links to it
    // rather than reading it a second time.
    assert!(html.contains("Open audit browser"), "{html}");
}

// ---------------------------------------------------------- subscriptions

#[tokio::test]
async fn the_subscriptions_screen_renders_the_create_form_with_its_filter_fields() {
    let html = render_page("/subscriptions", &[], || {
        view! { <ferroehr_viewer::pages::subscriptions::SubscriptionsPage /> }.into_any()
    });
    assert!(html.contains("id=\"subscriptions-screen\""), "{html}");
    assert!(html.contains(">Subscriptions</h1>"), "{html}");
    for field in [
        "id=\"subscription-create-name\"",
        "id=\"subscription-create-kind\"",
        "id=\"subscription-create-change-type\"",
        "id=\"subscription-create-template\"",
        "id=\"subscription-create-enabled\"",
        "id=\"subscription-create-submit\"",
    ] {
        assert!(html.contains(field), "{field} missing: {html}");
    }
}

// ------------------------------------------------------------------- FHIR

/// The FHIR connector is probe-and-hide: nothing that needs the surface renders
/// before the probe answers, so the server pass is the header over a skeleton.
#[tokio::test]
async fn the_fhir_screen_renders_its_header_over_the_connector_probe_skeleton() {
    let html = render_page("/fhir", &[], || {
        view! { <ferroehr_viewer::pages::fhir::FhirPage /> }.into_any()
    });
    assert!(html.contains("id=\"fhir-screen\""), "{html}");
    assert!(html.contains(">FHIR</h1>"), "{html}");
    assert!(html.contains("thaw-skeleton"), "{html}");
    assert!(!html.contains("id=\"fhir-create\""), "{html}");
    assert!(!html.contains("id=\"fhir-disabled\""), "{html}");
}

// ------------------------------------------------------------- operations

#[tokio::test]
async fn the_operations_screen_renders_its_four_operational_cards() {
    let html = render_page("/operations", &[], || {
        view! { <ferroehr_viewer::pages::operations::OperationsPage /> }.into_any()
    });
    assert!(html.contains(">Operations</h1>"), "{html}");
    for card in [
        ">Dependency health</h2>",
        ">Build &amp; spec provenance</h2>",
        ">Metrics</h2>",
        ">Log level</h2>",
    ] {
        assert!(html.contains(card), "{card} missing: {html}");
    }
    // The redacted configuration has ONE viewer, on /system; this screen only
    // points at it.
    assert!(html.contains(">Runtime configuration</h2>"), "{html}");
    assert!(html.contains("the System screen"), "{html}");
    assert!(html.contains("href=\"/system\""), "{html}");
    assert!(html.contains("Open runtime configuration"), "{html}");
}
