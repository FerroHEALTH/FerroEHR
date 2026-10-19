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
//! and the stylesheet key off are present, and the client-only parts (a
//! dialog's open state, a toast an action raises) contribute NOTHING to the
//! server pass — the property `.claude/rules/leptos-ui.md` §8 requires, since markup
//! the server emits and the client does not is a hydration mismatch.
//!
//! Server half only: `to_html` is the server-pass renderer.

#![cfg(not(target_arch = "wasm32"))]

use ferroehr_viewer::activity::ActivityPoint;
use ferroehr_viewer::components::activity_chart::activity_chart;
use ferroehr_viewer::components::brand::viewer_lockup;
use ferroehr_viewer::components::confirm_dialog::ConfirmDialog;
use ferroehr_viewer::components::empty_state::EmptyState;
use ferroehr_viewer::components::page_header::{Crumb, PageHeader};
use ferroehr_viewer::components::scope_grants::{
    ScopePreviewer, capability_note, fact_row, grant_cards,
};
use ferroehr_viewer::components::stat_card::StatCard;
use ferroehr_viewer::components::surface::{CARD_TITLE, titled_card};
use ferroehr_viewer::components::toast::{toast_error, toast_outcome, toast_success};
use ferroehr_viewer::components::upload_dialog::UploadDialog;
use ferroehr_viewer::error::ViewerError;
use ferroehr_viewer::scopes::{CAPABILITY_NOTE, grants};
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::widget::toast::{provide_toasts, toast_region};
use leptos::prelude::*;

/// Build a view under a fresh reactive owner and render the server pass.
///
/// The owner is what `StoredValue`/`RwSignal`/`Memo` and `provide_context`
/// need; every test gets its own so contexts cannot leak between them.
fn render(build: impl FnOnce() -> AnyView) -> String {
    let owner = Owner::new();
    owner.with(|| build().to_html())
}

// ---------------------------------------------------------------- brand

#[test]
fn the_lockup_renders_the_product_name_beside_an_inline_svg_mark() {
    let html = render(|| viewer_lockup().into_any());
    assert!(html.contains("FerroEHR"), "{html}");
    assert!(html.contains("Viewer"), "{html}");
    // Pure SVG, no asset fetch: the mark is three nodes and the edges between
    // them, inline in the document.
    assert!(html.contains("<svg"), "{html}");
    assert!(html.contains("viewBox=\"0 0 24 24\""), "{html}");
    assert_eq!(html.matches("<circle").count(), 3, "{html}");
    assert!(html.contains("aria-hidden=\"true\""), "{html}");
    // The lockup leads to the dashboard.
    assert!(html.contains("href=\"/\""), "{html}");
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
fn a_success_toast_shows_its_outcome_and_sentence_in_the_ok_tone() {
    let html = render(|| {
        let toasts = provide_toasts();
        toast_success(toasts, "Template uploaded", "vitals.v1 is now available.");
        view! {
            <p id="screen">"Templates"</p>
            {toast_region(toasts)}
        }
        .into_any()
    });
    assert!(html.contains("Templates"), "{html}");
    // The region is a polite live region, so a screen reader reads a toast
    // as it arrives.
    assert!(html.contains("aria-live=\"polite\""), "{html}");
    assert!(
        html.contains("Template uploaded: vitals.v1 is now available."),
        "{html}"
    );
    assert!(html.contains("fui-tone-ok"), "{html}");
}

#[test]
fn an_error_toast_shows_its_outcome_and_sentence_in_the_bad_tone() {
    let html = render(|| {
        let toasts = provide_toasts();
        toast_error(
            toasts,
            "Upload failed",
            "The CDR rejected vitals.v1: duplicate template id.",
        );
        toast_region(toasts).into_any()
    });
    assert!(
        html.contains("Upload failed: The CDR rejected vitals.v1"),
        "{html}"
    );
    assert!(html.contains("fui-tone-bad"), "{html}");
}

/// A mutation's outcome is toasted from an effect, which never runs on the
/// server pass: the server emits an empty region, so the client hydrates
/// against the same markup it starts from (rules §8).
#[test]
fn a_mutation_outcome_adds_no_toast_to_the_server_pass() {
    let html = render(|| {
        let toasts = provide_toasts();
        let outcome: Signal<Option<(String, Result<(), ViewerError>)>> =
            Signal::derive(|| Some(("vitals.v1".to_owned(), Ok(()))));
        toast_outcome(
            toasts,
            outcome,
            ("Template uploaded", "Upload failed"),
            |name, ()| format!("{name} is now available."),
            |name, error| format!("{name}: {error}"),
        );
        view! {
            <p id="screen">"Templates"</p>
            {toast_region(toasts)}
        }
        .into_any()
    });
    assert!(html.contains("Templates"), "{html}");
    assert!(html.contains("aria-live=\"polite\""), "{html}");
    assert!(!html.contains("Template uploaded"), "{html}");
    assert!(!html.contains("fui-toast "), "{html}");
}

// ---------------------------------------------------------- empty state

#[test]
fn an_empty_state_renders_its_message_hint_and_action() {
    let html = render(|| {
        view! {
            <EmptyState
                icon=Glyph::FILES
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
    // FerroUI's empty state is what distinguishes it from muted text.
    assert!(html.contains("fui-empty"), "{html}");
    assert!(html.contains("<svg"), "{html}");
}

#[test]
fn an_empty_state_without_a_hint_or_action_renders_only_its_message() {
    let html = render(|| {
        view! { <EmptyState icon=Glyph::INBOX message="No contributions in this window" /> }
            .into_any()
    });
    assert!(html.contains("No contributions in this window"), "{html}");
    assert!(html.contains("fui-empty"), "{html}");
    // The message is the heading, and the hint line under it says nothing.
    assert!(
        html.contains(">No contributions in this window</h2>"),
        "{html}"
    );
    let hint = html
        .split_once("class=\"fui-empty-body\">")
        .and_then(|(_, rest)| rest.split_once("</p>"))
        .map(|(body, _)| body.replace("<!>", "").trim().to_owned());
    assert_eq!(hint.as_deref(), Some(""), "{html}");
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
                icon=Glyph::USERS
                href="/ehrs"
            />
        }
        .into_any()
    });
    assert!(html.contains("<a"), "{html}");
    assert!(html.contains("href=\"/ehrs\""), "{html}");
    assert!(html.contains("1,204"), "{html}");
    assert!(html.contains("EHRs"), "{html}");
    // The value is FerroUI's stat headline, not the label.
    assert!(html.contains("fui-stat-value"), "{html}");
}

#[test]
fn a_stat_card_without_a_link_renders_an_inert_tile() {
    let html = render(|| {
        view! {
            <StatCard
                label="Templates"
                value=Signal::derive(|| "17".to_owned())
                icon=Glyph::FILES
            />
        }
        .into_any()
    });
    assert!(html.contains("Templates"), "{html}");
    assert!(html.contains("17"), "{html}");
    assert!(!html.contains("<a"), "{html}");
    assert!(html.contains("fui-stat"), "{html}");
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
    // FerroUI draws the SVG on the server, with its accessible name and the
    // summary sentence, so hydration finds the same drawing.
    assert!(html.contains("<svg"), "{html}");
    assert!(html.contains("Commits per day"), "{html}");
    assert!(
        html.contains("11 commits over 3 days, most on 2026-08-21."),
        "{html}"
    );
    assert!(
        html.contains("2026-08-20") && html.contains("2026-08-22"),
        "{html}"
    );
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
    assert!(html.contains("fui-empty"), "{html}");
    assert!(!html.contains("<svg class=\"fui-chart"), "{html}");
    assert!(!html.contains("data-activity-chart"), "{html}");
}

// ------------------------------------------------------- confirm dialog

/// A confirmation renders FerroUI's modal dialog CLOSED on the server pass,
/// whatever its signal says: the browser shows it from an effect, so the server
/// HTML and the hydrated view agree (rules §8).
fn confirm_dialog_html(open_state: bool) -> String {
    render(move || {
        let open = RwSignal::new(open_state);
        view! {
            <ConfirmDialog
                open=Signal::derive(move || open.get())
                title="Delete template"
                message=Signal::derive(|| "vitals.v1 will be removed.".to_owned())
                confirm_label="Delete"
                confirm_icon=Glyph::TRASH
                dialog_id="confirm-delete-template-dialog"
                confirm_id="confirm-delete-template"
                on_cancel=Callback::new(move |()| open.set(false))
                on_confirm=Callback::new(move |()| open.set(false))
            />
        }
        .into_any()
    })
}

#[test]
fn a_confirm_dialog_renders_closed_with_its_copy_and_its_destructive_button() {
    let html = confirm_dialog_html(false);
    assert!(html.contains("<dialog"), "{html}");
    assert!(
        html.contains("id=\"confirm-delete-template-dialog\""),
        "{html}"
    );
    // Closed: no `open` attribute, so the browser shows nothing until the
    // effect calls `showModal()`.
    assert!(!html.contains(" open"), "{html}");
    assert!(html.contains("Delete template"), "{html}");
    assert!(html.contains("vitals.v1 will be removed."), "{html}");
    assert!(html.contains("id=\"confirm-delete-template\""), "{html}");
    // The confirmation is FerroUI's solid destructive button, the one a danger
    // dialog's last step carries.
    assert!(html.contains("fui-button-destructive"), "{html}");
}

/// An OPEN confirmation renders exactly the server HTML a closed one does: the
/// open state is the browser's, shown by an effect after hydration.
#[test]
fn an_open_confirm_dialog_renders_the_same_server_markup_as_a_closed_one() {
    assert_eq!(confirm_dialog_html(true), confirm_dialog_html(false));
}

/// The upload dialog carries the same property, in BOTH states, and holds
/// FerroUI's upload field: the drop zone, the file input and the text area the
/// file is read into. The controls themselves are driven in the browser by the
/// template journeys (`common::upload_via_dialog`).
#[test]
fn an_upload_dialog_renders_closed_with_its_upload_field_open_or_closed() {
    let html_of = |open_state: bool| {
        render(move || {
            let open = RwSignal::new(open_state);
            let source = RwSignal::new(String::new());
            view! {
                <UploadDialog
                    open=Signal::derive(move || open.get())
                    on_dismiss=Callback::new(move |()| open.set(false))
                    title=Signal::derive(|| "Upload an operational template".to_owned())
                    help=Signal::derive(|| "The CDR ingests OPT/XML.".to_owned())
                    upload=Signal::derive(|| ferrohealth_ui::widget::upload::Upload {
                        id: "template-upload",
                        label: "Template source",
                        choose: "Choose an OPT/XML file",
                        accept: &[".opt", ".xml"],
                        max_bytes: 1024,
                        placeholder: "<template …",
                    })
                    submit_label="Upload template"
                    source=source
                    pending=Signal::derive(|| false)
                    error=Signal::derive(|| Option::<String>::None)
                    on_submit=Callback::new(move |_: String| {})
                    dialog_id="template-upload-dialog"
                    submit_id="template-upload-submit"
                />
            }
            .into_any()
        })
    };
    let closed = html_of(false);
    assert_eq!(html_of(true), closed, "the open state is the browser's");
    assert!(closed.contains("<dialog"), "{closed}");
    assert!(!closed.contains(" open"), "{closed}");
    assert!(
        closed.contains("Upload an operational template"),
        "{closed}"
    );
    assert!(closed.contains("type=\"file\""), "{closed}");
    assert!(closed.contains("id=\"template-upload-text\""), "{closed}");
    assert!(closed.contains("id=\"template-upload-submit\""), "{closed}");
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
    assert!(!html.contains("fui-page-title-mono"), "{html}");
}

#[test]
fn a_mono_page_header_renders_its_title_in_the_monospace_face() {
    let html = render(|| {
        view! { <PageHeader title=Signal::derive(|| "vitals.v1".to_owned()) mono=true /> }
            .into_any()
    });
    assert!(html.contains("fui-page-title-mono"), "{html}");
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
        view! {
            <leptos_router::components::Router>
                <ferroehr_viewer::app::NotFound />
            </leptos_router::components::Router>
        }
        .into_any()
    });
    assert!(html.contains("Page not found"), "{html}");
    assert!(
        html.contains("The page you requested does not exist."),
        "{html}"
    );
    assert!(html.contains("href=\"/\""), "{html}");
    assert!(html.contains("Return to the dashboard"), "{html}");
}
