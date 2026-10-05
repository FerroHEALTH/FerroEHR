// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/templates` screen — the Template Manager list + template upload.
//!
//! One screen, two template FAMILIES, switched by `?family=` (the switch is
//! deep-linkable URL state, not a private signal): ADL 1.4 operational
//! templates (`definition/template/adl1.4`, OPT/XML upload) and ADL2 ones
//! (`definition/template/adl2`, `text/plain` ADL2 source upload). Each family
//! has its own list, its own upload affordance, and its own detail route; the
//! client-side text filter and the URL page window serve both. No openEHR spec
//! governs the viewer — our own design / product extension; the wire it
//! reads/writes is the ITS-REST Definition API.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;

use crate::adl2::TemplateFamily;
use crate::admin::AdminAvailability;
use crate::components::data_table::{
    CELL, CELL_MONO, ROW, TablePaging, page_rows, page_window, paging_from_url, row_total,
    table_footer, table_shell, table_skeleton,
};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_DANGER, BTN_PRIMARY, INPUT};
use crate::components::page_header::PageHeader;
use crate::components::toast::{toast_error, toast_success};
use crate::error::ViewerError;

/// A template-upload action: the source text it was dispatched with is the
/// input, the CDR's answer (the accepted `template_id`, or the diagnostic) the
/// value both the toast and the inline bar read.
type TemplateUploadAction = Action<String, Result<String, ViewerError>>;

/// The template-delete action: the target it was dispatched with, paired with
/// the CDR's answer, so both the success and the failure toast can name the
/// exact template (the action's value IS the mutation report).
type TemplateDeleteAction = Action<DeleteTarget, (String, Result<(), ViewerError>)>;

/// Which template a delete addresses — the id, and the family that decides
/// WHICH wire route removes it.
///
/// The family travels with the id rather than being read off the URL at
/// confirm time: the row that opened the dialog is the one that knows its own
/// family, so the confirmation copy and the dispatched route can never
/// disagree with the row the reader clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    /// The archetype-model family the template belongs to.
    pub family: TemplateFamily,
    /// The `template_id` (ADL 1.4) or artefact HRID (ADL2) to remove.
    pub template_id: String,
}

/// One row of the template list, distilled from the ITS-REST Definition list
/// shape (`template_id` / `concept` / `created_timestamp`).
///
/// Shared across both compilation targets, so it carries only fixed-size,
/// client-safe fields (no `usize` in serialized types).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TemplateRow {
    /// The operational-template id (the detail route segment).
    pub template_id: String,
    /// The template concept / display name.
    pub concept: String,
    /// The root archetype id the template constrains.
    pub archetype_id: String,
    /// The creation timestamp as the CDR reported it (raw string).
    pub created: String,
}

/// List the CDR's ADL 1.4 operational templates.
///
/// GET `definition/template/adl1.4?version=*` with
/// `Accept: application/json`; the body is an array whose elements are parsed
/// defensively (missing fields become empty strings) into [`TemplateRow`]s.
/// `version=*` pins the FULL inventory: an absent `version` collapses the
/// CDR's listing to the latest version of each template (the released OAS
/// `parameters/query/filter_version.yaml`), and a management client must
/// show every stored version, not just the latest.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the CDR body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_templates() -> Result<Vec<TemplateRow>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("definition/template/adl1.4?version=*");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("template list JSON: {e}")))?;
    let rows = value
        .as_array()
        .map(|items| items.iter().map(template_row).collect())
        .unwrap_or_default();
    Ok(rows)
}

/// Distil one Definition-list element into a [`TemplateRow`], reading each
/// field defensively so a missing/renamed field yields an empty string rather
/// than dropping the row.
#[cfg(feature = "ssr")]
fn template_row(item: &serde_json::Value) -> TemplateRow {
    let text = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    TemplateRow {
        template_id: text("template_id"),
        concept: text("concept"),
        archetype_id: text("archetype_id"),
        created: text("created_timestamp"),
    }
}

/// Upload an operational template (OPT/XML) to the CDR.
///
/// POST `definition/template/adl1.4` with `Content-Type: application/xml`,
/// `Accept: application/json`, `Prefer: return=minimal`. The CDR's 400/409/422
/// validation diagnostics surface verbatim through
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success). On
/// success the (possibly empty) response body is returned; the UI ignores it
/// and refetches the list.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for empty content;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR (the diagnostic verbatim).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn upload_template(
    /// The operational-template XML to upload, as read in the browser.
    opt_xml: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    if opt_xml.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "the selected file was empty".to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("definition/template/adl1.4");
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/xml",
            "application/json",
            &[("Prefer", "return=minimal")],
            opt_xml,
        )
        .await?;
    Ok(crate::cdr::CdrClient::expect_success(response)?.body)
}

/// List the CDR's ADL2 operational templates.
///
/// GET `definition/template/adl2?version=*` with
/// `Accept: application/json`. The row shape is the same four fields the ADL
/// 1.4 listing carries (`template_id` / `concept` / `archetype_id` /
/// `created_timestamp`), so both families share one [`TemplateRow`] and one
/// parser. `version=*` pins the FULL inventory: without it the CDR collapses
/// the listing to the latest version of each HRID family (the released OAS
/// `parameters/query/filter_version.yaml`), which would hide every superseded
/// ADL2 artefact from a management client.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the CDR body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_adl2_templates() -> Result<Vec<TemplateRow>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("definition/template/adl2?version=*");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("ADL2 template list JSON: {e}")))?;
    let rows = value
        .as_array()
        .map(|items| items.iter().map(template_row).collect())
        .unwrap_or_default();
    Ok(rows)
}

/// Upload an ADL2 operational template (the artefact SOURCE) to the CDR.
///
/// POST `definition/template/adl2` with `Content-Type: text/plain` — the
/// operation's single declared body type — `Accept: application/json` and
/// `Prefer: return=identifier`, so the `201` echoes
/// `{"template_id": "<resolved HRID>"}` and the success toast can name the
/// artefact the CDR actually stored. The `openehr-adl` engine's refusals
/// (`400` unparseable, `422` AOM2-invalid with the rule codes, `409` duplicate
/// HRID) surface verbatim through
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for empty content;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR (the diagnostic verbatim).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn upload_adl2_template(
    /// The ADL2 operational-template source, pasted or read from a file.
    adl2_source: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    if adl2_source.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "there is no ADL2 source to upload".to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("definition/template/adl2");
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "text/plain",
            "application/json",
            &[("Prefer", "return=identifier")],
            adl2_source,
        )
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    Ok(accepted_template_id(&body))
}

/// The `template_id` a `Prefer: return=identifier` upload answer echoes
/// (`{"template_id": …}`), or the trimmed body when the CDR echoed something
/// else — the upload SUCCEEDED either way, so an unexpected body shape is
/// reported as-is rather than turned into a failure.
#[cfg(feature = "ssr")]
fn accepted_template_id(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(id) = value.get("template_id").and_then(serde_json::Value::as_str)
    {
        return id.to_owned();
    }
    body.trim().to_owned()
}

/// Delete one stored ADL2 artefact
/// (`DELETE definition/artefact/adl2/{artefact_id}`).
///
/// The ADL2 store's templates are removed through the ARTEFACT resource, not
/// through the Admin API's `admin/template/{template_id}` (which addresses the
/// ADL 1.4 store alone): the route is the CDR's own extension realizing SM
/// `I_DEFINITION_ADL2.delete_artefact`
/// (`docs/specs/openehr/SM/docs/UML/classes/i_definition_adl2.adoc`), whose
/// `artefact_does_not_exist` error is the `404` — which is also the answer for
/// a malformed HRID, since the store key is opaque. A template-kind artefact
/// that committed versions still reference is refused `409` with the
/// referencing count in the diagnostic, and that text surfaces verbatim
/// through [`delete_failure_copy`](crate::admin::delete_failure_copy).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty id;
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_adl2_artefact(
    /// The AOM2 artefact HRID to remove.
    artefact_id: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    if artefact_id.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "no ADL2 artefact id to delete".to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&crate::adl2::artefact_path(&artefact_id));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

/// The Template Manager screen: the family switch, an upload affordance, a
/// client-side filter, and the filterable template table.
///
/// The FAMILY (`?family=`) is URL state, so a link lands on the family it names
/// and the listing, the upload card and the row links all follow it. The filter
/// is a private client-side `contains` over already-loaded rows; the PAGE the
/// filtered rows are windowed at does live in the URL (`?page=`/`?size=`). A
/// successful upload or delete bumps its action's version, all of which are the
/// list resource's source, refetching it.
///
/// The per-row delete is admin-gated and serves BOTH families, each over the
/// route that actually removes its store: `DELETE admin/template/{template_id}`
/// for ADL 1.4, and `DELETE definition/artefact/adl2/{artefact_id}` for ADL2
/// ([`delete_adl2_artefact`]). The buttons render when the
/// [`admin_gate`](crate::admin::admin_gate) probe finds the CDR advertising its
/// Admin API, which is the right gate for both: the ADL2 route is Admin-classed
/// by the CDR's coarse RBAC even though it is not mounted under `/admin`.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn TemplatesPage() -> impl IntoView {
    let toaster = thaw::ToasterInjection::expect_context();
    let filter = RwSignal::new(String::new());
    // The table's page window, read from the URL in SETUP (never inside the
    // suspense that fetches the list).
    let paging = paging_from_url();
    // The family the screen is showing, derived from the URL in setup — the
    // same value on the server pass and at hydration.
    let query = leptos_router::hooks::use_query_map();
    let family = Memo::new(move |_| {
        TemplateFamily::from_query(&query.with(|q| q.get("family").unwrap_or_default()))
    });
    // The upload dialog's state, shared by both families: the source awaiting
    // upload (the picker fills it, the paste area edits it) and whether the
    // dialog is open. Only one family is on screen at a time, so ONE dialog and
    // one source signal serve both.
    let upload_open = RwSignal::new(false);
    let upload_source = RwSignal::new(String::new());
    // The accepted source is cleared, and the dialog closed, in each action's
    // OWN async continuation — never from an Effect reading the action's
    // value: a dispatch is the user event, so the answer is written where it
    // arrives.
    let upload: TemplateUploadAction = Action::new(move |opt_xml: &String| {
        let opt_xml = opt_xml.clone();
        async move {
            let outcome = upload_template(opt_xml).await;
            if outcome.is_ok() {
                upload_source.set(String::new());
                upload_open.set(false);
            }
            outcome
        }
    });
    let adl2_upload: TemplateUploadAction = Action::new(move |source: &String| {
        let source = source.clone();
        async move {
            let outcome = upload_adl2_template(source).await;
            if outcome.is_ok() {
                upload_source.set(String::new());
                upload_open.set(false);
            }
            outcome
        }
    });
    // The admin probe, and the delete action it gates. Both live in setup so
    // the gated view can re-render without re-creating them.
    let gate = crate::admin::admin_gate();
    let delete: TemplateDeleteAction = Action::new(|target: &DeleteTarget| {
        let target = target.clone();
        async move {
            let outcome = match target.family {
                TemplateFamily::Adl14 => {
                    crate::admin::admin_delete_template(target.template_id.clone()).await
                }
                TemplateFamily::Adl2 => delete_adl2_artefact(target.template_id.clone()).await,
            };
            (target.template_id, outcome)
        }
    });
    // The template awaiting confirmation in the modal (`None` = no dialog).
    // ONE dialog serves every row — the signal is both "which row" and "open".
    let pending_delete = RwSignal::new(Option::<DeleteTarget>::None);
    // The resource carries the family it fetched BESIDE the rows: reading the
    // family signal inside the table's `Suspend` makes the suspense re-run the
    // instant the URL changes — disposing the mounted table's owner while the
    // same URL change is still notifying the footer's page signals, which then
    // read a disposed value and panic in the browser.
    let list: Resource<(TemplateFamily, Result<Vec<TemplateRow>, ViewerError>)> = Resource::new(
        move || {
            (
                family.get(),
                upload.version().get(),
                adl2_upload.version().get(),
                delete.version().get(),
            )
        },
        |(family, ..)| async move {
            let rows = match family {
                TemplateFamily::Adl14 => list_templates().await,
                TemplateFamily::Adl2 => list_adl2_templates().await,
            };
            (family, rows)
        },
    );

    mutation_toasts(toaster, upload, adl2_upload, delete);

    // ONE upload affordance for both families: the same header-slot trigger in
    // the same position, opening the same dialog, with only the copy and the
    // accepted input differing. The family comes from the URL, so both branches
    // render identically on the server pass and at hydration.
    // Opening the dialog clears BOTH halves of the previous attempt: the
    // source, and the action value the diagnostic MessageBar renders. Leaving
    // either behind shows the last attempt inside a dialog the reader just
    // opened fresh.
    let reset_upload = Callback::new(move |()| {
        upload_source.set(String::new());
        match family.get_untracked() {
            TemplateFamily::Adl14 => upload.value().set(None),
            TemplateFamily::Adl2 => adl2_upload.value().set(None),
        }
    });
    let action_slot = upload_trigger(upload_open, reset_upload, family);
    let upload_dialog = upload_dialog_view(upload_open, upload_source, family, upload, adl2_upload);
    let switch = family_switch(family);
    let table_section = templates_table(filter, paging, list, gate, delete, pending_delete);
    let confirm = delete_dialog(pending_delete, delete);

    view! {
        <Title text="Templates" />
        <div class="p-6">
            <PageHeader
                title="Templates"
                subtitle="Operational templates registered in the CDR, by archetype-model family."
            >
                {action_slot}
            </PageHeader>
            {switch}
            <div class="mb-3">
                <input
                    type="text"
                    class=format!("w-full max-w-sm {INPUT}")
                    placeholder="filter by id or concept…"
                    prop:value=move || filter.get()
                    on:input:target=move |ev| filter.set(ev.target().value())
                />
            </div>
            {table_section}
            {confirm}
            {upload_dialog}
        </div>
    }
}

/// Wire the screen's three mutations to their success/failure toasts.
///
/// Every mutation toasts on BOTH outcomes; an upload rejection also keeps its
/// inline `MessageBar`, because a validation diagnostic is worth reading line by
/// line. Dispatching a toast is a side-effect on the outside world, so an
/// `Effect` is its correct home: it writes no signal and never runs on the
/// server pass.
fn mutation_toasts(
    toaster: thaw::ToasterInjection,
    upload: TemplateUploadAction,
    adl2_upload: TemplateUploadAction,
    delete: TemplateDeleteAction,
) {
    Effect::new(move |_| match upload.value().get() {
        Some(Ok(_)) => toast_success(
            toaster,
            "Template uploaded",
            "The operational template was accepted by the CDR.",
        ),
        Some(Err(error)) => crate::feedback::toast_write_failure(
            toaster,
            "Upload failed",
            "the operational template",
            &error,
        ),
        None => {}
    });

    // The ADL2 upload names the HRID the CDR resolved the source to (the
    // `Prefer: return=identifier` echo) so the reader can find the new row.
    Effect::new(move |_| match adl2_upload.value().get() {
        Some(Ok(template_id)) => toast_success(
            toaster,
            "ADL2 template uploaded",
            &format!("{template_id} was accepted by the CDR."),
        ),
        Some(Err(error)) => crate::feedback::toast_write_failure(
            toaster,
            "Upload failed",
            "the ADL2 operational template",
            &error,
        ),
        None => {}
    });

    // The delete outcome names the template: success plainly, failure with the
    // actionable copy (the CDR's in-use `409` diagnostic included).
    Effect::new(move |_| match delete.value().get() {
        Some((template_id, Ok(()))) => toast_success(
            toaster,
            "Template deleted",
            &format!("{template_id} was removed from the CDR."),
        ),
        Some((template_id, Err(error))) => toast_error(
            toaster,
            "Delete failed",
            &crate::admin::delete_failure_copy(&format!("Template {template_id}"), &error),
        ),
        None => {}
    });
}

/// The family switch: one URL-driven pill link per template family (the
/// selected family is a shareable query parameter, not private widget state).
/// Each href carries the family alone, so switching never inherits the other
/// listing's page window.
fn family_switch(family: Memo<TemplateFamily>) -> AnyView {
    let link = move |target: TemplateFamily| {
        let class = move || {
            let base = "rounded-control px-3 py-1.5 text-sm font-medium transition-colors";
            if family.get() == target {
                format!("{base} bg-accent-subtle text-accent-ink")
            } else {
                format!("{base} text-ink-muted hover:bg-sunken")
            }
        };
        view! {
            <leptos_router::components::A
                href=target.href()
                attr:class=class
                attr:data-template-family=target.as_query()
            >
                {target.label()}
            </leptos_router::components::A>
        }
        .into_any()
    };
    view! {
        <nav aria-label="Template families" class="flex gap-1 mb-3">
            {link(TemplateFamily::Adl14)}
            {link(TemplateFamily::Adl2)}
        </nav>
    }
    .into_any()
}

/// The screen's ONE upload dialog, serving both families: the copy, the
/// accepted file types and the dispatched action follow the URL family, and
/// nothing else about the flow differs.
///
/// It is rendered once outside the table (a list refetch never re-creates it)
/// and is inert while closed. Success is reported by each action's own
/// continuation, which clears the source and closes the dialog; a refusal keeps
/// it open with the CDR's diagnostic beside the input that caused it.
fn upload_dialog_view(
    open: RwSignal<bool>,
    source: RwSignal<String>,
    family: Memo<TemplateFamily>,
    upload: TemplateUploadAction,
    adl2_upload: TemplateUploadAction,
) -> AnyView {
    // The active family's action drives the busy state and the diagnostic;
    // both are read reactively, so switching families while the dialog is
    // closed carries no stale state onto the next open.
    let active = move || match family.get() {
        TemplateFamily::Adl14 => upload,
        TemplateFamily::Adl2 => adl2_upload,
    };
    let pending = Signal::derive(move || active().pending().get());
    let error = Signal::derive(move || match active().value().get() {
        Some(Err(error)) => Some(error.to_string()),
        _ => None,
    });
    view! {
        <crate::components::upload_dialog::UploadDialog
            open=Signal::derive(move || open.get())
            on_dismiss=Callback::new(move |()| open.set(false))
            title=Signal::derive(move || family.get().upload_title().to_owned())
            help=Signal::derive(move || family.get().upload_help().to_owned())
            accept=Signal::derive(move || family.get().upload_accept().to_owned())
            placeholder=Signal::derive(move || family.get().upload_placeholder().to_owned())
            choose_label=Signal::derive(move || family.get().upload_choose_label().to_owned())
            submit_label=Signal::derive(|| "Upload template".to_owned())
            source=source
            pending=pending
            error=error
            on_submit=Callback::new(move |text: String| {
                drop(active().dispatch(text));
            })
            picker_id="template-upload-picker"
            source_id="template-upload-source"
            submit_id="template-upload-submit"
        />
    }
    .into_any()
}

/// The screen's ONE delete-confirmation modal, driven by `pending_delete`
/// (which row triggered it). Rendered once outside the table, so a list
/// refetch never re-creates it; it is inert (nothing in the DOM) while no row
/// is pending, which is also why it needs no admin gate of its own — only an
/// admin-gated trigger can set the signal.
fn delete_dialog(
    pending_delete: RwSignal<Option<DeleteTarget>>,
    delete: TemplateDeleteAction,
) -> AnyView {
    let message = Signal::derive(move || {
        pending_delete
            .get()
            .map_or_else(String::new, |target| delete_prompt(&target))
    });
    view! {
        <crate::components::confirm_dialog::ConfirmDialog
            open=Signal::derive(move || pending_delete.get().is_some())
            title="Delete template"
            message=message
            confirm_label="Delete template"
            confirm_id="template-delete-confirm"
            on_cancel=Callback::new(move |()| pending_delete.set(None))
            on_confirm=Callback::new(move |()| {
                if let Some(target) = pending_delete.get_untracked() {
                    drop(delete.dispatch(target));
                }
                pending_delete.set(None);
            })
        />
    }
    .into_any()
}

/// The confirmation copy for one delete target: what goes, and the one refusal
/// the CDR can still answer with.
///
/// Each family names the object it actually removes — an ADL 1.4 delete takes
/// the operational template out of the Admin API's template store, an ADL2
/// delete takes the whole AOM2 artefact out of the definition store — and both
/// state the never-orphan guard the CDR enforces over committed versions.
fn delete_prompt(target: &DeleteTarget) -> String {
    let id = &target.template_id;
    match target.family {
        TemplateFamily::Adl14 => format!(
            "Permanently delete the operational template “{id}” from the CDR? This cannot be \
             undone. The CDR refuses the delete while a committed version still references the \
             template."
        ),
        TemplateFamily::Adl2 => format!(
            "Permanently delete the ADL 2 artefact “{id}” from the CDR's definition store? This \
             cannot be undone, and the store keeps no version history of it. The CDR refuses the \
             delete while a committed version still references the template."
        ),
    }
}

/// The upload trigger for the page-header action slot: the SAME button in the
/// same place for both families, opening the screen's one upload dialog
/// ([`upload_dialog_view`]) on an EMPTY source. Only its label follows the
/// family.
///
/// Opening clears the source because an open is a new attempt: the previous
/// one is cleared on success only, so without this a lost or refused upload
/// leaves its source in the editor and the reader is offered a dialog that
/// already looks ready to send something they did not just choose.
fn upload_trigger(
    open: RwSignal<bool>,
    on_open: Callback<()>,
    family: Memo<TemplateFamily>,
) -> AnyView {
    view! {
        <button
            id="template-upload-open"
            type="button"
            class=BTN_PRIMARY
            on:click=move |_| {
                on_open.run(());
                open.set(true);
            }
        >
            <leptos_icons::Icon icon=icondata_lu::LuUpload width="14" height="14" />
            {move || family.get().upload_label()}
        </button>
    }
    .into_any()
}

/// The template table: the list resource read under `<Transition>` (keep the
/// current rows visible while a refetch runs), resolving its `Result` inside
/// the transition (an SSR'd `ErrorBoundary` fallback mismatches at hydration in
/// leptos 0.8), then the filtered rows.
///
/// The admin probe is awaited in the SAME `Suspend` as the list, so the header
/// row and the rows agree on whether the delete column exists. The family comes
/// off the list resource, never from a signal read here: a signal read inside
/// the suspense re-runs it, disposes the mounted table and panics the browser.
fn templates_table(
    filter: RwSignal<String>,
    paging: TablePaging,
    list: Resource<(TemplateFamily, Result<Vec<TemplateRow>, ViewerError>)>,
    gate: Resource<Result<AdminAvailability, ViewerError>>,
    delete: TemplateDeleteAction,
    pending_delete: RwSignal<Option<DeleteTarget>>,
) -> AnyView {
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                let admin = crate::admin::renders_admin_ops(&gate.await);
                let (family, rows) = list.await;
                match rows {
                    Ok(rows) => {
                        rows_view(rows, filter, paging, admin, delete, pending_delete, family)
                    }
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// Render the loaded rows: the empty-catalogue state, or a filterable, paged
/// table (with the admin delete column when `admin`).
///
/// The filtered listing is a [`Memo`], so the rendered window and the footer's
/// total are computed from ONE derivation of the filter and can never disagree
/// (a derived value, not an effect writing a signal). The window itself comes
/// from the URL, so paging never re-runs the enclosing suspense.
fn rows_view(
    rows: Vec<TemplateRow>,
    filter: RwSignal<String>,
    paging: TablePaging,
    admin: bool,
    delete: TemplateDeleteAction,
    pending_delete: RwSignal<Option<DeleteTarget>>,
    family: TemplateFamily,
) -> AnyView {
    if rows.is_empty() {
        return empty_family_state(family);
    }

    let matched = Memo::new(move |_| {
        let needle = filter.read().to_lowercase();
        rows.iter()
            .filter(|&row| matches_filter(row, &needle))
            .cloned()
            .collect::<Vec<_>>()
    });
    // `.read()` guards, never `.get()`: the filtered listing is a collection
    // and cloning it per read would be wasted work.
    let total = Signal::derive(move || row_total(matched.read().len()));
    let none_match = move || {
        // Two statements on purpose: the filter guard must be released before
        // `matched` is read, because recomputing the memo reads the filter
        // again (never hold one signal's guard across another read that
        // depends on it).
        let filtering = !filter.read().is_empty();
        filtering && matched.read().is_empty()
    };

    let body = view! {
        <For
            each=move || {
                let window = page_window(total.get(), paging.page.get(), paging.size.get());
                matched.with(|rows| page_rows(rows, window))
            }
            key=|row| row.template_id.clone()
            children=move |row| row_view(row, family, admin, delete, pending_delete)
        />
    }
    .into_any();

    let headers: &[&str] = if admin {
        &["Template ID", "Concept", "Archetype ID", "Created", ""]
    } else {
        &["Template ID", "Concept", "Archetype ID", "Created"]
    };
    // `base` stays the screen's own path: the paging href carries every OTHER
    // query parameter across, `?family=` included.
    let footer = table_footer("/templates", family_noun(family), paging, total);

    view! {
        {table_shell(headers, body)}
        {footer}
        <Show when=none_match>
            <div class="mt-3">
                <EmptyState
                    icon=icondata_lu::LuSearchX
                    message="No templates match the filter"
                    hint="Clear or shorten the filter to see the whole catalogue again."
                />
            </div>
        </Show>
    }
    .into_any()
}

/// Whether one row matches the screen's client-side filter: a case-insensitive
/// `contains` over the id, the concept, and the archetype id. `needle` is
/// already lowercased.
fn matches_filter(row: &TemplateRow, needle: &str) -> bool {
    row.template_id.to_lowercase().contains(needle)
        || row.concept.to_lowercase().contains(needle)
        || row.archetype_id.to_lowercase().contains(needle)
}

/// The `/templates/{template_id}` detail-route link for one template id.
///
/// The id is a CDR-supplied string, so the path segment is percent-encoded with
/// the `urlencoding` crate: reserved characters (`/`, `#`, `?`, `%`) and
/// non-ASCII bytes would otherwise split the segment or truncate the URL.
/// `leptos_router` percent-DEcodes route params on both targets, so
/// [`crate::pages::template_detail`] reads the original id back.
///
/// NOTE: no openEHR spec governs the viewer's internal links — our own
/// design/extension.
fn detail_href(template_id: &str) -> String {
    format!("/templates/{}", urlencoding::encode(template_id))
}

/// The detail-route link for one row of `family` — the two families have
/// separate detail screens because they serve different representations.
fn family_detail_href(family: TemplateFamily, template_id: &str) -> String {
    match family {
        TemplateFamily::Adl14 => detail_href(template_id),
        TemplateFamily::Adl2 => crate::adl2::detail_href(template_id),
    }
}

/// How the pagination footer names this family's rows
/// (`26–50 of 137 ADL 2 templates`).
fn family_noun(family: TemplateFamily) -> &'static str {
    match family {
        TemplateFamily::Adl14 => "ADL 1.4 templates",
        TemplateFamily::Adl2 => "ADL 2 templates",
    }
}

/// The empty-catalogue state for one family, naming the upload that fills it.
fn empty_family_state(family: TemplateFamily) -> AnyView {
    let hint = match family {
        TemplateFamily::Adl14 => {
            "Upload your first operational template (OPT/XML) with the Upload OPT button above."
        }
        TemplateFamily::Adl2 => {
            "Upload your first ADL2 operational template with the Upload ADL2 button above."
        }
    };
    view! { <EmptyState icon=icondata_lu::LuFileCode2 message="No templates yet" hint=hint /> }
        .into_any()
}

/// One table row: the template id links to its family's detail route; concept
/// and created are plain cells; the admin delete cell renders only when the
/// probe said so.
fn row_view(
    row: TemplateRow,
    family: TemplateFamily,
    admin: bool,
    delete: TemplateDeleteAction,
    pending_delete: RwSignal<Option<DeleteTarget>>,
) -> impl IntoView {
    let href = family_detail_href(family, &row.template_id);
    let action = if admin {
        delete_cell(family, &row.template_id, delete, pending_delete)
    } else {
        ().into_any()
    };
    view! {
        <tr class=ROW>
            <td class=CELL_MONO>
                <leptos_router::components::A href=href attr:class="text-accent hover:underline">
                    {row.template_id}
                </leptos_router::components::A>
            </td>
            <td class=CELL>{row.concept}</td>
            <td class=CELL_MONO>{row.archetype_id}</td>
            <td class=CELL_MONO>{row.created}</td>
            {action}
        </tr>
    }
}

/// The admin delete cell for one row: the danger button that opens the screen's
/// confirmation modal for THIS template (it only sets `pending_delete`; the
/// dialog owns the confirm). `data-template-delete` is the stable hook the E2E
/// journeys select on.
fn delete_cell(
    family: TemplateFamily,
    template_id: &str,
    delete: TemplateDeleteAction,
    pending_delete: RwSignal<Option<DeleteTarget>>,
) -> AnyView {
    let target = DeleteTarget {
        family,
        template_id: template_id.to_owned(),
    };
    let on_click = move |_| pending_delete.set(Some(target.clone()));
    view! {
        <td class=format!("{CELL} text-right")>
            <button
                type="button"
                class=BTN_DANGER
                data-template-delete=template_id.to_owned()
                disabled=Signal::derive(move || delete.pending().get())
                on:click=on_click
            >
                "Delete"
            </button>
        </td>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use crate::adl2::TemplateFamily;
    use crate::pages::templates::{detail_href, family_detail_href, family_noun};

    #[test]
    fn each_family_links_rows_at_its_own_detail_route() {
        assert_eq!(
            family_detail_href(TemplateFamily::Adl14, "minimal_evaluation.en.v1"),
            "/templates/minimal_evaluation.en.v1"
        );
        assert_eq!(
            family_detail_href(
                TemplateFamily::Adl2,
                "openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0"
            ),
            "/templates/adl2/openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0"
        );
    }

    #[test]
    fn each_family_confirms_the_delete_in_its_own_words() {
        let opt = crate::pages::templates::delete_prompt(&crate::pages::templates::DeleteTarget {
            family: TemplateFamily::Adl14,
            template_id: "minimal_evaluation.en.v1".to_owned(),
        });
        assert!(opt.contains("operational template"), "{opt}");
        assert!(opt.contains("minimal_evaluation.en.v1"), "{opt}");

        let artefact =
            crate::pages::templates::delete_prompt(&crate::pages::templates::DeleteTarget {
                family: TemplateFamily::Adl2,
                template_id: "openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0".to_owned(),
            });
        assert!(artefact.contains("ADL 2 artefact"), "{artefact}");
        assert!(
            artefact.contains("openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0"),
            "{artefact}"
        );
        // Both name the never-orphan guard the CDR enforces on either store.
        for copy in [&opt, &artefact] {
            assert!(copy.contains("still references the template"), "{copy}");
        }
    }

    #[test]
    fn the_footer_names_the_family_whose_rows_it_pages() {
        assert_eq!(family_noun(TemplateFamily::Adl14), "ADL 1.4 templates");
        assert_eq!(family_noun(TemplateFamily::Adl2), "ADL 2 templates");
    }

    #[test]
    fn detail_href_leaves_a_url_safe_template_id_alone() {
        assert_eq!(
            detail_href("Vital signs-v2.0_TEST~x"),
            "/templates/Vital%20signs-v2.0_TEST~x"
        );
    }

    #[test]
    fn detail_href_percent_encodes_reserved_and_non_ascii_bytes() {
        // A slash would otherwise split the segment and route elsewhere.
        assert_eq!(detail_href("a/b"), "/templates/a%2Fb");
        // A hash would otherwise truncate the URL into a fragment.
        assert_eq!(detail_href("a#b"), "/templates/a%23b");
        // A literal percent must survive as `%25`, not be read as an escape.
        assert_eq!(detail_href("a%2Fb"), "/templates/a%252Fb");
        // Non-ASCII is escaped per UTF-8 byte, uppercase hex.
        assert_eq!(
            detail_href("temperatur-°C"),
            "/templates/temperatur-%C2%B0C"
        );
        // Every reserved character at once.
        assert_eq!(
            detail_href("a b/c?d#e%f&g=h+i"),
            "/templates/a%20b%2Fc%3Fd%23e%25f%26g%3Dh%2Bi"
        );
    }

    /// Both families' listings are read through this one parser, so it is
    /// pinned against the four fields the Definition API returns.
    #[cfg(feature = "ssr")]
    #[test]
    fn a_list_element_distils_into_a_row_and_missing_fields_stay_empty() {
        let item = serde_json::json!({
            "template_id": "openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0",
            "concept": "cnf_vitals",
            "archetype_id": "openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0",
            "created_timestamp": "2026-08-22T14:51:32.596215Z",
        });
        let row = crate::pages::templates::template_row(&item);
        assert_eq!(row.template_id, "openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0");
        assert_eq!(row.concept, "cnf_vitals");
        assert_eq!(row.created, "2026-08-22T14:51:32.596215Z");
        // A renamed or absent field empties that cell rather than dropping the
        // whole row from the listing.
        let sparse = serde_json::json!({ "template_id": "only-the-id" });
        let row = crate::pages::templates::template_row(&sparse);
        assert_eq!(row.template_id, "only-the-id");
        assert!(row.concept.is_empty());
        assert!(row.archetype_id.is_empty());
        assert!(row.created.is_empty());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn the_upload_answer_yields_the_echoed_template_id() {
        let body = r#"{"template_id":"openEHR-EHR-COMPOSITION.cnf_minimal.v1.0.0"}"#;
        assert_eq!(
            crate::pages::templates::accepted_template_id(body),
            "openEHR-EHR-COMPOSITION.cnf_minimal.v1.0.0"
        );
        // A body that is not the identifier object is reported as-is: the
        // upload still succeeded.
        assert_eq!(
            crate::pages::templates::accepted_template_id("  not json  "),
            "not json"
        );
        assert_eq!(crate::pages::templates::accepted_template_id("{}"), "{}");
    }
}
