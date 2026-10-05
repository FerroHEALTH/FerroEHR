// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! In-process SSR render tests for the shared component kit.
//!
//! These are NOT browser journeys: every test builds a component with
//! representative props inside a bare reactive [`Owner`] and renders it with
//! `RenderHtml::to_html` — the exact call the axum integration makes for the
//! server pass — then asserts on the HTML that comes out. No `WebDriver`, no
//! composed stack, no external process, so they run in the ordinary
//! `cargo nextest run -p ferroehr-viewer --features ssr` lane.
//!
//! What they pin, beyond "it renders": the text a screen passes in actually
//! reaches the document, the structural classes/attributes the E2E journeys
//! and the stylesheet key off are present, and the two client-only kits
//! (`thaw`'s teleported dialog, the toaster) contribute NOTHING to the server
//! pass — the property `.claude/rules/leptos-ui.md` §8 requires, since markup
//! the server emits and the client does not is a hydration mismatch.
//!
//! `ssr`-gated: `to_html` is the server-pass renderer, and the viewer's
//! `ssr` feature is what puts `leptos-use`/`thaw`/`leptos-chartistry` on their
//! non-`WASM` code paths (`Cargo.toml` §features).

#![cfg(feature = "ssr")]

use ferroehr_viewer::activity::ActivityPoint;
use ferroehr_viewer::components::activity_chart::activity_chart;
use ferroehr_viewer::components::brand::Wordmark;
use ferroehr_viewer::components::confirm_dialog::ConfirmDialog;
use ferroehr_viewer::components::empty_state::EmptyState;
use ferroehr_viewer::components::page_header::{Crumb, PageHeader};
use ferroehr_viewer::components::scope_grants::{
    ScopePreviewer, capability_note, fact_row, grant_cards,
};
use ferroehr_viewer::components::stat_card::StatCard;
use ferroehr_viewer::components::surface::{CARD_TITLE, titled_card};
use ferroehr_viewer::components::toast::{toast_error, toast_success};
use ferroehr_viewer::components::upload_dialog::UploadDialog;
use ferroehr_viewer::scopes::{CAPABILITY_NOTE, grants};
use leptos::prelude::*;

/// Build a view under a fresh reactive owner and render the server pass.
///
/// The owner is what `StoredValue`/`RwSignal`/`Memo` and `provide_context`
/// need; every test gets its own so contexts cannot leak between them.
fn render(build: impl FnOnce() -> AnyView) -> String {
    let owner = Owner::new();
    owner.with(|| build().to_html())
}

/// The `thaw` kit's chrome: every thaw widget resolves `ConfigInjection`, so a
/// test that renders one wraps its body exactly like the viewer's own shell.
///
/// The body is a CLOSURE, not a built view: `thaw::ConfigProvider` provides its
/// injection inside a child owner and then calls `children()`, so a body built
/// before the call would look the context up in the wrong scope and panic.
fn thaw_shell(body: impl FnOnce() -> AnyView + Send + 'static) -> AnyView {
    view! { <thaw::ConfigProvider>{body()}</thaw::ConfigProvider> }.into_any()
}

// ---------------------------------------------------------------- brand

#[test]
fn the_wordmark_renders_the_product_name_over_an_inline_svg_mark() {
    let html = render(|| view! { <Wordmark /> }.into_any());
    assert!(html.contains("FerroEHR"), "{html}");
    assert!(html.contains("Viewer"), "{html}");
    // Pure SVG, no asset fetch: the mark is three nodes and the edges between
    // them, inline in the document.
    assert!(html.contains("<svg"), "{html}");
    assert!(html.contains("viewBox=\"0 0 24 24\""), "{html}");
    assert_eq!(html.matches("<circle").count(), 3, "{html}");
    assert!(html.contains("aria-hidden=\"true\""), "{html}");
}

// -------------------------------------------------------------- surface

#[test]
fn a_titled_card_renders_its_heading_over_the_body_it_was_given() {
    let html = render(|| {
        titled_card(
            "Repository usage",
            false,
            view! { <p id="body">"41 compositions"</p> }.into_any(),
        )
    });
    assert!(html.contains("<section"), "{html}");
    assert!(html.contains(CARD_TITLE), "{html}");
    assert!(html.contains(">Repository usage</h2>"), "{html}");
    assert!(html.contains("41 compositions"), "{html}");
    // A single-column card must not claim the second grid column.
    assert!(!html.contains("lg:col-span-2"), "{html}");
}

#[test]
fn a_full_width_titled_card_spans_both_grid_columns() {
    let html = render(|| titled_card("Effective configuration", true, ().into_any()));
    assert!(html.contains("lg:col-span-2"), "{html}");
    assert!(html.contains(">Effective configuration</h2>"), "{html}");
}

// ---------------------------------------------------------------- toast

#[test]
fn a_success_toast_is_dispatched_client_side_and_adds_nothing_to_the_server_pass() {
    let html = render(|| {
        let (toaster, _receiver) = thaw::ToasterInjection::channel();
        toast_success(toaster, "Template uploaded", "vitals.v1 is now available.");
        thaw_shell(|| {
            view! {
                <thaw::ToasterProvider>
                    <p id="screen">"Templates"</p>
                </thaw::ToasterProvider>
            }
            .into_any()
        })
    });
    // The screen under the toaster still renders …
    assert!(html.contains("Templates"), "{html}");
    // … and the toast itself does not: it is mounted by an effect into a
    // teleported container, which only exists in the browser. Server markup
    // for it would be a hydration mismatch (rules §8).
    assert!(!html.contains("thaw-toast"), "{html}");
    assert!(!html.contains("Template uploaded"), "{html}");
    assert!(!html.contains("vitals.v1 is now available."), "{html}");
}

#[test]
fn an_error_toast_is_dispatched_client_side_and_adds_nothing_to_the_server_pass() {
    let html = render(|| {
        let (toaster, _receiver) = thaw::ToasterInjection::channel();
        toast_error(
            toaster,
            "Upload failed",
            "The CDR rejected vitals.v1: duplicate template id.",
        );
        thaw_shell(|| {
            view! {
                <thaw::ToasterProvider>
                    <p id="screen">"Templates"</p>
                </thaw::ToasterProvider>
            }
            .into_any()
        })
    });
    assert!(html.contains("Templates"), "{html}");
    assert!(!html.contains("thaw-toast"), "{html}");
    assert!(!html.contains("Upload failed"), "{html}");
    assert!(!html.contains("duplicate template id"), "{html}");
}

// ---------------------------------------------------------- empty state

#[test]
fn an_empty_state_renders_its_message_hint_and_action() {
    let html = render(|| {
        view! {
            <EmptyState
                icon=icondata_lu::LuFileText
                message="No templates yet"
                hint="Upload your first OPT to get started"
            >
                <a href="/templates/upload">"Upload an OPT"</a>
            </EmptyState>
        }
        .into_any()
    });
    assert!(html.contains("No templates yet"), "{html}");
    assert!(
        html.contains("Upload your first OPT to get started"),
        "{html}"
    );
    assert!(html.contains("href=\"/templates/upload\""), "{html}");
    assert!(html.contains("Upload an OPT"), "{html}");
    // The dashed well is what distinguishes an empty state from muted text.
    assert!(html.contains("border-dashed"), "{html}");
    assert!(html.contains("<svg"), "{html}");
}

#[test]
fn an_empty_state_without_a_hint_or_action_renders_only_its_message() {
    let html = render(|| {
        view! { <EmptyState icon=icondata_lu::LuInbox message="No contributions in this window" /> }
            .into_any()
    });
    assert!(html.contains("No contributions in this window"), "{html}");
    assert!(html.contains("border-dashed"), "{html}");
    // Exactly one paragraph: the message. A hint paragraph would be a second.
    assert_eq!(html.matches("<p ").count(), 1, "{html}");
    assert!(!html.contains("<a "), "{html}");
}

// ------------------------------------------------------------ stat card

#[test]
fn a_stat_card_with_a_link_navigates_from_the_whole_tile() {
    let html = render(|| {
        view! {
            <StatCard
                label="EHRs"
                value=Signal::derive(|| "1,204".to_owned())
                icon=icondata_lu::LuUsers
                href="/ehrs"
            />
        }
        .into_any()
    });
    assert!(html.contains("<a"), "{html}");
    assert!(html.contains("href=\"/ehrs\""), "{html}");
    assert!(html.contains("1,204"), "{html}");
    assert!(html.contains("EHRs"), "{html}");
    // The value is the tabular-nums headline, not the label.
    assert!(html.contains("tabular-nums"), "{html}");
}

#[test]
fn a_stat_card_without_a_link_renders_an_inert_tile() {
    let html = render(|| {
        view! {
            <StatCard
                label="Templates"
                value=Signal::derive(|| "17".to_owned())
                icon=icondata_lu::LuFileText
            />
        }
        .into_any()
    });
    assert!(html.contains("Templates"), "{html}");
    assert!(html.contains("17"), "{html}");
    assert!(!html.contains("<a"), "{html}");
    assert!(html.contains("rounded-card"), "{html}");
}

// ------------------------------------------------------- activity chart

#[test]
fn an_activity_chart_with_points_renders_the_named_chart_container() {
    let points = vec![
        ActivityPoint {
            day: "2026-08-20".to_owned(),
            count: 3,
        },
        ActivityPoint {
            day: "2026-08-21".to_owned(),
            count: 7,
        },
        ActivityPoint {
            day: "2026-08-22".to_owned(),
            count: 1,
        },
    ];
    let html = render(move || {
        activity_chart(
            &points,
            "Commits",
            "No commits yet",
            "Commit a composition to see the trend",
        )
    });
    // The journey hook naming which chart this is.
    assert!(html.contains("data-activity-chart=\"Commits\""), "{html}");
    // chartistry's own root, and its server-pass placeholder: the SVG geometry
    // is drawn only once the browser has measured the container, so the server
    // emits the placeholder and the client swaps in the chart.
    assert!(html.contains("_chartistry"), "{html}");
    assert!(html.contains("Loading..."), "{html}");
    // With data there is no empty state.
    assert!(!html.contains("No commits yet"), "{html}");
}

#[test]
fn an_activity_chart_without_points_renders_the_shared_empty_state() {
    let html = render(|| {
        activity_chart(
            &[],
            "Commits",
            "No commits yet",
            "Commit a composition to see the trend",
        )
    });
    assert!(html.contains("No commits yet"), "{html}");
    assert!(
        html.contains("Commit a composition to see the trend"),
        "{html}"
    );
    assert!(html.contains("border-dashed"), "{html}");
    assert!(!html.contains("_chartistry"), "{html}");
    assert!(!html.contains("data-activity-chart"), "{html}");
}

// ------------------------------------------------------- confirm dialog

/// A closed confirm dialog contributes nothing to the server pass.
#[test]
fn a_closed_confirm_dialog_renders_no_server_markup() {
    let html = render(|| {
        let open = RwSignal::new(false);
        thaw_shell(move || {
            view! {
                <ConfirmDialog
                    open=Signal::derive(move || open.get())
                    title="Delete template"
                    message=Signal::derive(|| "vitals.v1 will be removed.".to_owned())
                    confirm_label="Delete"
                    confirm_id="confirm-delete-template"
                    on_cancel=Callback::new(move |()| open.set(false))
                    on_confirm=Callback::new(move |()| open.set(false))
                />
            }
            .into_any()
        })
    });
    // The shell around it DID render, so the absences below are real.
    assert!(html.contains("thaw-config-provider"), "{html}");
    assert!(!html.contains("Delete template"), "{html}");
    assert!(!html.contains("confirm-delete-template"), "{html}");
    assert!(!html.contains("thaw-dialog"), "{html}");
}

/// The upload dialog is the same shape as the confirm dialog and carries the
/// same hydration property, in BOTH states: nothing of it — not the copy, not
/// the file input, not the paste area — reaches the server pass, so the
/// client cannot hydrate against markup the server never produced (rules §8).
/// The controls themselves are driven in the browser by the template journeys
/// (`common::upload_via_dialog`).
#[test]
fn an_upload_dialog_renders_no_server_markup_open_or_closed() {
    for open_state in [false, true] {
        let html = render(move || {
            let open = RwSignal::new(open_state);
            let source = RwSignal::new(String::new());
            thaw_shell(move || {
                view! {
                    <UploadDialog
                        open=Signal::derive(move || open.get())
                        on_dismiss=Callback::new(move |()| open.set(false))
                        title=Signal::derive(|| "Upload an operational template".to_owned())
                        help=Signal::derive(|| "The CDR ingests OPT/XML.".to_owned())
                        accept=Signal::derive(|| ".opt,.xml".to_owned())
                        placeholder=Signal::derive(|| "<template …".to_owned())
                        choose_label=Signal::derive(|| " Choose an OPT/XML file".to_owned())
                        submit_label=Signal::derive(|| "Upload template".to_owned())
                        source=source
                        pending=Signal::derive(|| false)
                        error=Signal::derive(|| Option::<String>::None)
                        on_submit=Callback::new(move |_: String| {})
                        picker_id="template-upload-picker"
                        source_id="template-upload-source"
                        submit_id="template-upload-submit"
                    />
                }
                .into_any()
            })
        });
        // The shell around it DID render, so the absences below are real.
        assert!(html.contains("thaw-config-provider"), "{html}");
        assert!(!html.contains("Upload an operational template"), "{html}");
        assert!(!html.contains("template-upload-picker"), "{html}");
        assert!(!html.contains("template-upload-submit"), "{html}");
        assert!(!html.contains("type=\"file\""), "{html}");
        assert!(!html.contains("thaw-dialog"), "{html}");
    }
}

/// And so does an OPEN one — which is the point. `thaw::Dialog` teleports its
/// surface into the document body from an effect, so the dialog exists only in
/// the browser; if the `open=true` server pass emitted the surface, the client
/// would hydrate against markup it never produces (rules §8). Both states
/// therefore render the same (empty) server HTML.
#[test]
fn an_open_confirm_dialog_also_renders_no_server_markup() {
    let html = render(|| {
        let open = RwSignal::new(true);
        thaw_shell(move || {
            view! {
                <ConfirmDialog
                    open=Signal::derive(move || open.get())
                    title="Delete template"
                    message=Signal::derive(|| "vitals.v1 will be removed.".to_owned())
                    confirm_label="Delete"
                    confirm_icon=icondata_lu::LuTrash
                    confirm_id="confirm-delete-template"
                    on_cancel=Callback::new(move |()| open.set(false))
                    on_confirm=Callback::new(move |()| open.set(false))
                />
            }
            .into_any()
        })
    });
    assert!(html.contains("thaw-config-provider"), "{html}");
    assert!(!html.contains("Delete template"), "{html}");
    assert!(!html.contains("vitals.v1 will be removed."), "{html}");
    assert!(!html.contains("confirm-delete-template"), "{html}");
    assert!(!html.contains("thaw-dialog"), "{html}");
}

// ----------------------------------------------------------- page header

#[test]
fn a_page_header_renders_its_title_subtitle_and_actions() {
    let html = render(|| {
        view! {
            <PageHeader
                title=Signal::derive(|| "Templates".to_owned())
                subtitle="Operational templates known to the CDR"
            >
                <button type="button">"Upload OPT"</button>
            </PageHeader>
        }
        .into_any()
    });
    assert!(html.contains("<h1"), "{html}");
    assert!(html.contains(">Templates</h1>"), "{html}");
    assert!(
        html.contains("Operational templates known to the CDR"),
        "{html}"
    );
    assert!(html.contains("Upload OPT"), "{html}");
    // No ancestors were given, so there is no breadcrumb trail.
    assert!(!html.contains("<nav"), "{html}");
    // The default face is the proportional one.
    assert!(!html.contains("font-mono"), "{html}");
}

#[test]
fn a_mono_page_header_renders_its_title_in_the_monospace_face() {
    let html = render(|| {
        view! { <PageHeader title=Signal::derive(|| "vitals.v1".to_owned()) mono=true /> }
            .into_any()
    });
    assert!(html.contains("font-mono"), "{html}");
    assert!(html.contains(">vitals.v1</h1>"), "{html}");
}

#[test]
fn a_page_header_renders_its_ancestors_as_links_and_the_title_as_the_current_crumb() {
    let html = render(|| {
        // `<A>` resolves against the router, so the trail needs a `<Router>`;
        // on the server pass the router's location IS the request URL.
        provide_context(leptos_router::location::RequestUrl::new(
            "/templates/vitals.v1",
        ));
        view! {
            <leptos_router::components::Router>
                <PageHeader
                    title=Signal::derive(|| "vitals.v1".to_owned())
                    crumbs=vec![Crumb::new("Templates", "/templates")]
                />
            </leptos_router::components::Router>
        }
        .into_any()
    });
    assert!(html.contains("aria-label=\"Breadcrumb\""), "{html}");
    assert!(html.contains("href=\"/templates\""), "{html}");
    assert!(html.contains("Templates"), "{html}");
    // The terminal crumb is the title as plain text, marked as the page.
    assert!(html.contains("aria-current=\"page\""), "{html}");
    assert!(html.contains(">vitals.v1<"), "{html}");
}

// ---------------------------------------------------------- scope grants

#[test]
fn the_capability_note_states_that_scopes_narrow_rather_than_grant() {
    let html = render(capability_note);
    assert!(html.contains(CAPABILITY_NOTE), "{html}");
}

#[test]
fn a_fact_row_renders_its_label_beside_its_value() {
    let html = render(|| fact_row("Identity", "alice@example.org".to_owned()));
    assert!(html.contains("Identity"), "{html}");
    assert!(html.contains("alice@example.org"), "{html}");
}

#[test]
fn a_granted_resource_scope_renders_its_compartment_family_and_permissions() {
    let html = render(|| grant_cards(grants("patient/composition-vitals.v1.rs")));
    assert!(html.contains("data-scope-grant=\"resource\""), "{html}");
    // The string is always shown verbatim above its reading.
    assert!(html.contains("patient/composition-vitals.v1.rs"), "{html}");
    assert!(html.contains(">patient<"), "{html}");
    assert!(
        html.contains("Compositions of the matching template"),
        "{html}"
    );
    assert!(html.contains(">read<"), "{html}");
    assert!(html.contains(">search<"), "{html}");
    // An exact pattern is not broad access.
    assert!(!html.contains("broad access"), "{html}");
    assert!(html.contains("Exact match only."), "{html}");
}

#[test]
fn a_wildcard_resource_scope_is_flagged_as_broad_access() {
    let html = render(|| grant_cards(grants("system/aql-*.rs")));
    assert!(html.contains("data-scope-grant=\"resource\""), "{html}");
    assert!(html.contains(">system<"), "{html}");
    assert!(html.contains("AQL queries"), "{html}");
    assert!(html.contains("broad access"), "{html}");
    assert!(
        html.contains("All available templates or queries"),
        "{html}"
    );
}

#[test]
fn an_identity_scope_renders_as_an_identity_grant_that_reaches_no_clinical_data() {
    let html = render(|| grant_cards(grants("openid")));
    assert!(html.contains("data-scope-grant=\"identity\""), "{html}");
    assert!(html.contains("Identity claim"), "{html}");
    assert!(html.contains("reaches no clinical data"), "{html}");
}

#[test]
fn a_launch_scope_renders_as_a_context_grant() {
    let html = render(|| grant_cards(grants("launch")));
    assert!(html.contains("data-scope-grant=\"context\""), "{html}");
    assert!(html.contains("Launch marker"), "{html}");
}

#[test]
fn a_scope_the_grammar_refuses_is_kept_verbatim_and_marked_not_recognised() {
    let html = render(|| grant_cards(grants("patient/composition-vitals.v1.zz")));
    assert!(html.contains("data-scope-grant=\"unrecognized\""), "{html}");
    // Kept verbatim: an unreadable scope is visible, never swallowed.
    assert!(html.contains("patient/composition-vitals.v1.zz"), "{html}");
    assert!(html.contains("not recognised"), "{html}");
}

#[test]
fn a_whole_claim_renders_one_card_per_scope() {
    let html = render(|| grant_cards(grants("openid launch patient/composition-vitals.v1.rs")));
    assert_eq!(html.matches("data-scope-grant=").count(), 3, "{html}");
}

#[test]
fn the_scope_previewer_renders_an_empty_field_and_an_empty_result_region() {
    let html = render(|| view! { <ScopePreviewer /> }.into_any());
    assert!(html.contains("id=\"scope-previewer-input\""), "{html}");
    assert!(html.contains("Preview a scope"), "{html}");
    assert!(html.contains("id=\"scope-preview-results\""), "{html}");
    // Nothing typed yet, so no grant cards on the server pass.
    assert!(!html.contains("data-scope-grant"), "{html}");
}

// ------------------------------------------------------------ 404 screen

#[test]
fn the_not_found_screen_offers_a_way_back_to_the_dashboard() {
    let html = render(|| {
        leptos_meta::provide_meta_context();
        provide_context(leptos_router::location::RequestUrl::new("/no-such-page"));
        thaw_shell(|| {
            view! {
                <leptos_router::components::Router>
                    <ferroehr_viewer::app::NotFound />
                </leptos_router::components::Router>
            }
            .into_any()
        })
    });
    assert!(html.contains("Page not found"), "{html}");
    assert!(
        html.contains("The page you requested does not exist."),
        "{html}"
    );
    assert!(html.contains("href=\"/\""), "{html}");
    assert!(html.contains("Return to the dashboard."), "{html}");
}
