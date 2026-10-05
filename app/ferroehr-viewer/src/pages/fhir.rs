// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/fhir` screen — the CDR's FHIR connector: its mapping store, and the
//! two ways to verify a mapping without writing anything.
//!
//! Three panels over one endpoint family ([`crate::fhir`], which carries the
//! wire facts): the mapping store (list, create, edit, two-step delete), the
//! read-path viewer (what a stored mapping produces for one patient), and the
//! validate-only dry run (the verdict the ingest door would reach).
//!
//! **No viewer path commits a FHIR resource, and that is the design.** The
//! connector's inbound door (`POST /fhir/r4/{type}`) maps, validates and
//! commits a COMPOSITION; sending a real resource is an integration act, not an
//! operator affordance. Everything here either reads or dry-runs.
//!
//! **The mapping definition is edited as a JSON document, deliberately.** It is
//! a deep, open-ended structure — subject binding, context, per-entry paths and
//! transforms — whose shape the CDR owns; a bespoke nested form here would be a
//! second model of it, drifting the moment the connector grows a field. The
//! textarea sends the document verbatim and the CDR's own rejection comes back
//! verbatim.
//!
//! **No Effect on this screen reads a resource**: the edit form is seeded by
//! the row's own Edit click, so there is no seed to race hydration with, and
//! every mutation's answer is handled in the action's own async continuation.

use leptos::component;
use leptos::prelude::*;
use leptos_meta::Title;

use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::data_table::{
    CELL, CELL_MONO, ROW, TablePaging, page_rows, page_window, paging_from_url, row_total,
    table_footer, table_shell, table_skeleton,
};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_DANGER, BTN_PRIMARY, BTN_SECONDARY, INPUT, LABEL, TEXTAREA};
use crate::components::format_view::DocumentPane;
use crate::components::notice::inline_error;
use crate::components::page_header::PageHeader;
use crate::components::surface::{CARD_PAD, CARD_TITLE, WELL};
use crate::components::toast::{toast_error, toast_success};
use crate::error::ViewerError;
use crate::fhir::{
    DryRunVerdict, FhirAnswer, FhirMappingRow, create_fhir_mapping, definition_complaint,
    delete_fhir_mapping, dry_run_fhir_resource, list_fhir_mappings, mapping_draft_complaint,
    mapping_failure_copy, read_fhir_resources, read_request_is_complete, update_fhir_mapping,
    verdict_of,
};

/// The mapping listing: `None` = the CDR does not serve the FHIR connector.
type Store = Resource<Result<Option<Vec<FhirMappingRow>>, ViewerError>>;

/// The read facade's answer: `None` = no scope has been asked for yet.
type ReadFacade = Resource<Result<Option<FhirAnswer>, ViewerError>>;

/// The create action: the name it was dispatched with, paired with the CDR's
/// answer, so both toasts name the exact mapping (the action's value IS the
/// mutation report).
type CreateAction = Action<MappingDraft, (String, Result<FhirMappingRow, ViewerError>)>;

/// The update action, reporting the same way.
type UpdateAction = Action<MappingEdit, (String, Result<FhirMappingRow, ViewerError>)>;

/// The delete action, reporting the same way.
type DeleteAction = Action<FhirMappingRow, (String, Result<(), ViewerError>)>;

/// The dry run: a resource type and a resource body in, the CDR's verdict out.
/// It is not a mutation — it commits nothing — so it reports inline only.
type DryRunAction = Action<(String, String), Result<FhirAnswer, ViewerError>>;

/// A new mapping as the create card assembled it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MappingDraft {
    /// The deployable name.
    name: String,
    /// The mapping definition document, as JSON text.
    definition: String,
    /// Whether the connector should resolve it.
    enabled: bool,
}

/// One in-flight edit: which stored mapping, and what to store on it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MappingEdit {
    /// The store id of the mapping being edited.
    id: String,
    /// The mapping's name — never sent (it is immutable), only reported.
    name: String,
    /// The definition document to store.
    definition: String,
    /// The enabled flag to store.
    enabled: bool,
}

/// The editor's signals: which mapping is open, and its two draft fields.
///
/// Seeded from the row's OWN values when its Edit button is clicked — a user
/// event, so the write happens where it arrives and no Effect reads a resource
/// to fill a form.
#[derive(Debug, Clone, Copy)]
struct Editor {
    /// The mapping being edited (`None` = the editor is closed).
    target: RwSignal<Option<FhirMappingRow>>,
    /// The definition draft.
    definition: RwSignal<String>,
    /// The enabled draft.
    enabled: RwSignal<bool>,
    /// The client-side complaint about the draft; `None` while it is sendable.
    validation: RwSignal<Option<String>>,
}

impl Editor {
    /// Open the editor on `row`, seeded with the document it currently holds.
    fn open(self, row: FhirMappingRow) {
        self.definition.set(row.definition.clone());
        self.enabled.set(row.enabled);
        self.validation.set(None);
        self.target.set(Some(row));
    }

    /// Close the editor and drop the draft.
    fn close(self) {
        self.target.set(None);
        self.definition.set(String::new());
        self.validation.set(None);
    }
}

/// The create card's signals.
#[derive(Debug, Clone, Copy)]
struct Draft {
    /// The name draft.
    name: RwSignal<String>,
    /// The definition-document draft.
    definition: RwSignal<String>,
    /// The enabled draft.
    enabled: RwSignal<bool>,
    /// The client-side complaint about the draft; `None` while it is sendable.
    validation: RwSignal<Option<String>>,
}

/// The dry-run panel's signals.
#[derive(Debug, Clone, Copy)]
struct DryRunForm {
    /// The FHIR resource type to validate against.
    resource_type: RwSignal<String>,
    /// The FHIR resource, as JSON text.
    resource: RwSignal<String>,
}

/// Everything the screen's sections are built from, created ONCE in setup.
///
/// One carrier rather than a dozen parameters, and `Copy` throughout, because
/// the sections are built inside a `Suspend` closure that re-runs on every
/// notification of the listing it awaits: the closure must not consume its
/// environment, and nothing it holds may be a resource it CREATES.
#[derive(Clone, Copy)]
struct Screen {
    /// The mapping listing — the screen's availability signal.
    store: Store,
    /// The read facade's answer for the URL scope.
    facade: ReadFacade,
    /// The mapping table's page window, from the URL.
    paging: TablePaging,
    /// The read scope's resource type, from the URL.
    read_type: Signal<String>,
    /// The read scope's patient, from the URL.
    read_patient: Signal<String>,
    /// The create card's state.
    draft: Draft,
    /// The edit card's state.
    editor: Editor,
    /// The dry-run panel's state.
    dry_run_form: DryRunForm,
    /// The row awaiting delete confirmation.
    pending_delete: RwSignal<Option<FhirMappingRow>>,
    /// The create mutation.
    create: CreateAction,
    /// The update mutation.
    update: UpdateAction,
    /// The delete mutation.
    delete: DeleteAction,
    /// The dry run (not a mutation — it commits nothing).
    dry_run: DryRunAction,
}

impl std::fmt::Debug for Screen {
    /// Signal, resource and action handles carry no readable content outside a
    /// reactive owner, so the `Debug` impl names the type only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Screen")
    }
}

/// The FHIR-connector screen.
///
/// The whole screen is probe-and-hide at the NAV level ([`crate::fhir`]);
/// reached directly, it renders the connector when the CDR serves it and one
/// naming-the-switch card when it does not — the terminology browser's
/// disabled-surface precedent, never a page of controls that cannot work.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one screen assembled from `.into_any()`-erased section locals (rules §1) — the \
              setup that creates every resource and action outside a Suspend is deliberately one \
              function"
)]
#[component]
pub fn FhirPage() -> impl IntoView {
    let toaster = thaw::ToasterInjection::expect_context();
    // The table's page window and the read scope, both read from the URL in
    // SETUP — never inside the suspense that fetches the rows.
    let paging = paging_from_url();
    let query = leptos_router::hooks::use_query_map();
    let read_type = Signal::derive(move || {
        query
            .with(|q| q.get("resource_type").unwrap_or_default())
            .trim()
            .to_owned()
    });
    let read_patient = Signal::derive(move || {
        query
            .with(|q| q.get("patient").unwrap_or_default())
            .trim()
            .to_owned()
    });

    let draft = Draft {
        name: RwSignal::new(String::new()),
        definition: RwSignal::new(String::new()),
        enabled: RwSignal::new(true),
        validation: RwSignal::new(None),
    };
    let editor = Editor {
        target: RwSignal::new(None),
        definition: RwSignal::new(String::new()),
        enabled: RwSignal::new(true),
        validation: RwSignal::new(None),
    };
    let dry_run_form = DryRunForm {
        resource_type: RwSignal::new(String::new()),
        resource: RwSignal::new(String::new()),
    };
    // The row awaiting confirmation in the modal (`None` = no dialog). ONE
    // dialog serves every row — the signal is both "which row" and "open".
    let pending_delete = RwSignal::new(Option::<FhirMappingRow>::None);

    // Each mutation clears its own form in its OWN async continuation, never
    // from an Effect reading the action's value: a dispatch is the user event,
    // so the answer is handled where it arrives.
    let create: CreateAction = Action::new(move |new: &MappingDraft| {
        let new = new.clone();
        async move {
            let outcome = create_fhir_mapping(new.name.clone(), new.definition, new.enabled).await;
            if outcome.is_ok() {
                draft.name.set(String::new());
                draft.definition.set(String::new());
                draft.enabled.set(true);
            }
            (new.name, outcome)
        }
    });
    let update: UpdateAction = Action::new(move |edit: &MappingEdit| {
        let edit = edit.clone();
        async move {
            let outcome = update_fhir_mapping(edit.id, edit.definition, edit.enabled).await;
            if outcome.is_ok() {
                editor.close();
            }
            (edit.name, outcome)
        }
    });
    let delete: DeleteAction = Action::new(|row: &FhirMappingRow| {
        let row = row.clone();
        async move {
            let outcome = delete_fhir_mapping(row.id).await;
            (row.name, outcome)
        }
    });
    let dry_run: DryRunAction = Action::new(|input: &(String, String)| {
        let (resource_type, resource) = input.clone();
        async move { dry_run_fhir_resource(resource_type, resource).await }
    });

    let store: Store = Resource::new(
        move || {
            (
                create.version().get(),
                update.version().get(),
                delete.version().get(),
            )
        },
        |_| async move { list_fhir_mappings().await },
    );
    // The read facade follows the URL scope AND every mapping change: editing a
    // mapping changes what a read produces, which is the whole point of the
    // viewer sitting beside the editor.
    let facade: ReadFacade = Resource::new(
        move || {
            (
                read_type.get(),
                read_patient.get(),
                create.version().get(),
                update.version().get(),
                delete.version().get(),
            )
        },
        |(resource_type, patient, _, _, _)| async move {
            if read_request_is_complete(&resource_type, &patient) {
                read_fhir_resources(resource_type, patient).await.map(Some)
            } else {
                Ok(None)
            }
        },
    );

    mutation_toasts(toaster, create, update, delete);

    let screen = Screen {
        store,
        facade,
        paging,
        read_type,
        read_patient,
        draft,
        editor,
        dry_run_form,
        pending_delete,
        create,
        update,
        delete,
        dry_run,
    };
    let body = connector_body(&screen);
    let confirm = delete_dialog(pending_delete, delete);

    view! {
        <Title text="FHIR" />
        <div id="fhir-screen" class="p-6">
            <PageHeader
                title="FHIR"
                subtitle="The FHIR connector's mapping store, and the two ways to verify a mapping without writing anything."
            />
            {body}
            {confirm}
        </div>
    }
}

/// Wire the screen's three mutations to their success/failure toasts.
///
/// Every mutation toasts on BOTH outcomes (the viewer's mutation-feedback
/// rule); the CDR's diagnostic ALSO stays inline beside each form, because a
/// rejected mapping document is worth reading line by line. Dispatching a toast
/// is a side effect on the outside world, so an Effect is its correct home — it
/// never writes a signal, and it never runs on the server pass. The dry run has
/// no toast here on purpose: it writes nothing, so it is a read, and reads
/// report inline only.
fn mutation_toasts(
    toaster: thaw::ToasterInjection,
    create: CreateAction,
    update: UpdateAction,
    delete: DeleteAction,
) {
    Effect::new(move |_| match create.value().get() {
        Some((name, Ok(row))) => toast_success(
            toaster,
            "Mapping stored",
            &format!(
                "{name} maps {} onto template {}.",
                row.resource_type, row.template_id
            ),
        ),
        Some((name, Err(error))) => toast_error(
            toaster,
            "Mapping not stored",
            &mapping_failure_copy(&format!("mapping `{name}`"), &error),
        ),
        None => {}
    });

    Effect::new(move |_| match update.value().get() {
        Some((name, Ok(row))) => toast_success(
            toaster,
            "Mapping updated",
            &format!(
                "{name} now maps {} onto template {}.",
                row.resource_type, row.template_id
            ),
        ),
        Some((name, Err(error))) => toast_error(
            toaster,
            "Update failed",
            &mapping_failure_copy(&format!("mapping `{name}`"), &error),
        ),
        None => {}
    });

    Effect::new(move |_| match delete.value().get() {
        Some((name, Ok(()))) => toast_success(
            toaster,
            "Mapping deleted",
            &format!("{name} was removed from the mapping store."),
        ),
        Some((name, Err(error))) => toast_error(
            toaster,
            "Delete failed",
            &mapping_failure_copy(&format!("mapping `{name}`"), &error),
        ),
        None => {}
    });
}

/// The whole screen body under ONE `<Transition>` over the mapping listing.
///
/// The listing decides whether the surface exists at all, so a disabled
/// connector renders one card naming the switch instead of a store, a viewer and
/// a dry run that cannot work. `<Transition>` (not `<Suspense>`): the listing
/// refetches after every mutation and the previous rows must stay visible.
/// Nothing inside the closure CREATES a resource, so its re-runs are safe.
///
/// A REFUSED listing is not an absent connector: the store sits under `/admin`
/// while the read facade and the dry run are ordinary clinical calls, so a
/// session the store turns down still gets both verification panels — the
/// refusal replaces the store, not the screen.
fn connector_body(screen: &Screen) -> AnyView {
    // One owned copy for the closure: the `Suspend` body must own what it
    // reads, and every field is a `Copy` handle.
    let screen = *screen;
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match screen.store.await {
                    Ok(None) => disabled_card(),
                    Ok(Some(rows)) => served_body(store_section(rows, &screen), &screen),
                    Err(error) => served_body(read_error(&error, "the FHIR mapping store"), &screen),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The screen as a served connector renders it: whatever the mapping store had
/// to say, then the two verification panels.
fn served_body(store: AnyView, screen: &Screen) -> AnyView {
    let viewer = read_viewer(screen.facade, screen.read_type, screen.read_patient);
    let panel = dry_run_panel(screen.dry_run_form, screen.dry_run);
    view! {
        {store}
        {viewer}
        {panel}
    }
    .into_any()
}

/// The mapping store: the create card, the edit card, and the paged table (or
/// its empty state).
fn store_section(rows: Vec<FhirMappingRow>, screen: &Screen) -> AnyView {
    let create_card = create_card(screen.draft, screen.create);
    let editor_card = edit_card(screen.editor, screen.update);
    let table = if rows.is_empty() {
        empty_store()
    } else {
        rows_view(
            rows,
            screen.paging,
            screen.editor,
            screen.pending_delete,
            screen.delete,
        )
    };
    view! {
        {create_card}
        {editor_card}
        {table}
    }
    .into_any()
}

/// The whole screen when the CDR does not serve the FHIR connector.
///
/// A `404` on `GET admin/fhir_mapping` means the routes answer as if unmounted
/// — what `[fhir] api_enabled = false` does — so the honest screen names the
/// switch instead of showing a store, a viewer and a dry run that cannot work.
fn disabled_card() -> AnyView {
    view! {
        <section id="fhir-disabled" class=CARD_PAD>
            <EmptyState
                icon=icondata_lu::LuPlug
                message="The FHIR connector is disabled on this server"
                hint="The CDR answers its FHIR routes as if unmounted. Set api_enabled = true under [fhir] in the CDR's configuration to administer mappings here."
            />
        </section>
    }
    .into_any()
}

/// The empty store: no mapping is registered yet, so no resource type maps.
fn empty_store() -> AnyView {
    view! {
        <EmptyState
            icon=icondata_lu::LuPlug
            message="No FHIR mappings stored"
            hint="Register the first mapping with the card above. Until one exists, the connector answers every resource type with 'no enabled FHIR mapping'."
        />
    }
    .into_any()
}

/// A failed READ, rendered inline (never a toast — the viewer's one feedback
/// rule) and, for a refusal, as ACTIONABLE copy rather than the bare wire error.
///
/// The mapping store sits under `/admin`, so the CDR's coarse RBAC classes every
/// call to it as admin work: a session without the role is answered `403`, and
/// "forbidden" alone tells the reader nothing about what to do next. A `401` —
/// the credential itself no longer accepted — lands here too, with its own next
/// action. Capability
/// is not authorization — the screen renders because the surface EXISTS, and
/// this is where the per-request refusal lands.
fn read_error(error: &ViewerError, object: &str) -> AnyView {
    match error {
        ViewerError::CdrUnauthorized(_) | ViewerError::Forbidden(_) => view! {
            <p
                id="fhir-refused"
                role="alert"
                class="rounded-control border border-danger/40 bg-danger-subtle px-3 py-2 text-sm text-danger"
            >
                {mapping_failure_copy(object, error)}
            </p>
        }
        .into_any(),
        other => inline_error(other),
    }
}

/// The create card: the name, the definition document, the enabled flag, and a
/// register button that is inert until the draft can actually be sent.
///
/// The button carries a STATIC `disabled` attribute for the server HTML — the
/// form is empty on the first paint, so the control is inert before hydration —
/// with the live state on `prop:disabled`: an attribute sets the INITIAL state
/// and only a property carries the live one, so an attribute binding alone can
/// leave a control stuck at whatever was serialized.
fn create_card(draft: Draft, create: CreateAction) -> AnyView {
    let unsendable = Signal::derive(move || {
        mapping_draft_complaint(&draft.name.read(), &draft.definition.read()).is_err()
    });
    let on_create = move |_| {
        let name = draft.name.get_untracked();
        let definition = draft.definition.get_untracked();
        // Client-side validation first, before any round trip; the server fn
        // re-checks — it is a public endpoint.
        if let Err(message) = mapping_draft_complaint(&name, &definition) {
            draft.validation.set(Some(message));
        } else {
            draft.validation.set(None);
            drop(create.dispatch(MappingDraft {
                name,
                definition,
                enabled: draft.enabled.get_untracked(),
            }));
        }
    };
    let validation = complaint_bar("fhir-create-validation", draft.validation);
    let diagnostic = diagnostic_pane(
        "fhir-create-diagnostic",
        Signal::derive(move || create.value().get().and_then(|(_, outcome)| outcome.err())),
    );
    view! {
        <section id="fhir-create" class=format!("{CARD_PAD} mb-4")>
            <h2 class=CARD_TITLE>"Store a mapping"</h2>
            <p class="mb-3 text-xs text-ink-muted">
                "The definition is the mapping document itself — the resource type it binds, the \
                 profile URL it matches, the openEHR template it builds under, the subject binding \
                 and the field entries. It is sent verbatim; the CDR validates it and its rejection \
                 is shown here in full. The name is the deployable identity and cannot be changed \
                 afterwards."
            </p>
            <div class="flex flex-col gap-3">
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="fhir-create-name">
                            "Name"
                        </label>
                        <input
                            id="fhir-create-name"
                            type="text"
                            class=INPUT
                            placeholder="observation-bp"
                            prop:value=move || draft.name.get()
                            on:input:target=move |ev| draft.name.set(ev.target().value())
                        />
                    </div>
                    <label class="flex items-center gap-2 text-sm text-ink">
                        <input
                            id="fhir-create-enabled"
                            type="checkbox"
                            checked=true
                            prop:checked=move || draft.enabled.get()
                            on:change:target=move |ev| draft.enabled.set(ev.target().checked())
                        />
                        "Enabled"
                    </label>
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="fhir-create-definition">
                        "Definition (JSON document)"
                    </label>
                    <textarea
                        id="fhir-create-definition"
                        class=format!("{TEXTAREA} min-h-[12rem]")
                        placeholder="{ \"resource_type\": \"Observation\", \"template_id\": \"…\", \"subject\": { … }, \"entries\": [ … ] }"
                        prop:value=move || draft.definition.get()
                        on:input:target=move |ev| draft.definition.set(ev.target().value())
                    >
                        {draft.definition.get_untracked()}
                    </textarea>
                </div>
                <div>
                    <button
                        id="fhir-create-submit"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=true
                        prop:disabled=move || unsendable.get() || create.pending().get()
                        on:click=on_create
                    >
                        "Store mapping"
                    </button>
                </div>
                {validation}
                {diagnostic}
            </div>
        </section>
    }
    .into_any()
}

/// The edit card: nothing at all until a row's Edit button opens it, then that
/// mapping's document in a textarea plus save/cancel.
///
/// Rendered as part of the store section, so it is created with the rows and a
/// refetch never leaves a stale editor behind; it starts closed on both passes
/// of the first render. The name is not editable: the CDR treats it as the
/// mapping's immutable deployable identity. The save button carries no STATIC
/// `disabled` twin, unlike the create card's — this card exists only after a
/// click, so it is never server-rendered.
fn edit_card(editor: Editor, update: UpdateAction) -> AnyView {
    let unsendable =
        Signal::derive(move || definition_complaint(&editor.definition.read()).is_err());
    let on_save = move |_| {
        let Some(row) = editor.target.get_untracked() else {
            return;
        };
        let definition = editor.definition.get_untracked();
        if let Err(message) = definition_complaint(&definition) {
            editor.validation.set(Some(message));
        } else {
            editor.validation.set(None);
            drop(update.dispatch(MappingEdit {
                id: row.id,
                name: row.name,
                definition,
                enabled: editor.enabled.get_untracked(),
            }));
        }
    };
    let validation = complaint_bar("fhir-edit-validation", editor.validation);
    // The CDR's rejection belongs to the editor that caused it: a save fails
    // with the editor still open, so the bar disappears with the card rather
    // than outliving it after a Cancel.
    let diagnostic = diagnostic_pane(
        "fhir-edit-diagnostic",
        Signal::derive(move || {
            if editor.target.with(Option::is_none) {
                return None;
            }
            update.value().get().and_then(|(_, outcome)| outcome.err())
        }),
    );
    let fields = move || {
        let Some(row) = editor.target.get() else {
            return ().into_any();
        };
        let seeded_enabled = row.enabled;
        view! {
            <section id="fhir-edit" class=format!("{CARD_PAD} mb-4")>
                <h2 class=CARD_TITLE>{format!("Edit mapping {}", row.name)}</h2>
                <p class="mb-3 text-xs text-ink-muted">
                    <span class="font-medium">"Store id: "</span>
                    <span class="font-mono break-all text-ink">{row.id}</span>
                </p>
                <div class="flex flex-col gap-3">
                    <label class="flex items-center gap-2 text-sm text-ink">
                        <input
                            id="fhir-edit-enabled"
                            type="checkbox"
                            checked=seeded_enabled
                            prop:checked=move || editor.enabled.get()
                            on:change:target=move |ev| editor.enabled.set(ev.target().checked())
                        />
                        "Enabled"
                    </label>
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="fhir-edit-definition">
                            "Definition (JSON document)"
                        </label>
                        <textarea
                            id="fhir-edit-definition"
                            class=format!("{TEXTAREA} min-h-[14rem]")
                            prop:value=move || editor.definition.get()
                            on:input:target=move |ev| editor.definition.set(ev.target().value())
                        >
                            {editor.definition.get_untracked()}
                        </textarea>
                    </div>
                    <div class="flex items-center gap-3">
                        <button
                            id="fhir-edit-save"
                            type="button"
                            class=BTN_PRIMARY
                            prop:disabled=move || unsendable.get() || update.pending().get()
                            on:click=on_save
                        >
                            "Save mapping"
                        </button>
                        <button
                            id="fhir-edit-cancel"
                            type="button"
                            class=BTN_SECONDARY
                            on:click=move |_| editor.close()
                        >
                            "Cancel"
                        </button>
                    </div>
                </div>
            </section>
        }
        .into_any()
    };
    view! {
        {fields}
        {validation}
        {diagnostic}
    }
    .into_any()
}

/// The viewer's own complaint about a draft it refused to send.
fn complaint_bar(id: &'static str, complaint: RwSignal<Option<String>>) -> AnyView {
    view! {
        {move || {
            complaint
                .get()
                .map(|message| {
                    view! {
                        <div
                            role="alert"
                            id=id
                            class="rounded-control border border-danger/40 bg-danger-subtle px-3 py-2 text-sm text-danger"
                        >
                            {message}
                        </div>
                    }
                })
        }}
    }
    .into_any()
}

/// The CDR's own diagnostic for a refused write, verbatim, inline BESIDE the
/// failure toast (the viewer's feedback rule: a rejected mapping document
/// names the field or the conflict, and that is worth reading line by line).
fn diagnostic_pane(id: &'static str, error: Signal<Option<ViewerError>>) -> AnyView {
    view! {
        {move || {
            error
                .get()
                .map(|error| {
                    let detail = error.to_string();
                    view! {
                        <div class=WELL id=id role="alert">
                            <pre class="overflow-auto max-h-[40vh] whitespace-pre-wrap font-mono text-xs text-danger">
                                {detail}
                            </pre>
                        </div>
                    }
                })
        }}
    }
    .into_any()
}

/// Render the loaded mappings: the paged table plus its footer.
///
/// The window comes from the URL, so turning the page re-renders the rows
/// without re-running the suspense that fetched them, and the `<For>` key
/// is the store id — stable and data-derived, never an index.
fn rows_view(
    rows: Vec<FhirMappingRow>,
    paging: TablePaging,
    editor: Editor,
    pending_delete: RwSignal<Option<FhirMappingRow>>,
    delete: DeleteAction,
) -> AnyView {
    let count = row_total(rows.len());
    let total = Signal::derive(move || count);
    let body = view! {
        <For
            each=move || {
                let window = page_window(total.get(), paging.page.get(), paging.size.get());
                page_rows(&rows, window)
            }
            key=|row: &FhirMappingRow| row.id.clone()
            children=move |row| row_view(row, editor, pending_delete, delete)
        />
    }
    .into_any();
    let footer = table_footer("/fhir", "mappings", paging, total);
    view! {
        <section id="fhir-mappings">
            {table_shell(
                &["Name", "Resource type", "Profile", "Template", "Enabled", "Mapping id", ""],
                body,
            )}
        </section>
        {footer}
    }
    .into_any()
}

/// One store row: its facts, plus the edit trigger and the delete trigger
/// (which only opens the screen's confirmation modal).
fn row_view(
    row: FhirMappingRow,
    editor: Editor,
    pending_delete: RwSignal<Option<FhirMappingRow>>,
    delete: DeleteAction,
) -> impl IntoView {
    let hook = row.name.clone();
    let edit_hook = row.name.clone();
    let delete_hook = row.name.clone();
    let for_edit = row.clone();
    let for_delete = row.clone();
    // A mapping with no profile URL is the resource type's profile-less
    // default; an em dash says so without inventing a word for it.
    let profile = if row.profile_url.is_empty() {
        "—".to_owned()
    } else {
        row.profile_url
    };
    let enabled = if row.enabled { "enabled" } else { "disabled" };
    view! {
        <tr class=ROW data-fhir-mapping=hook>
            <td class=CELL>{row.name}</td>
            <td class=CELL data-fhir-cell="resource-type">
                {row.resource_type}
            </td>
            <td class=CELL_MONO data-fhir-cell="profile">
                {profile}
            </td>
            <td class=CELL_MONO data-fhir-cell="template">
                {row.template_id}
            </td>
            <td class=CELL data-fhir-cell="enabled">
                {enabled}
            </td>
            <td class=CELL_MONO data-fhir-cell="id">
                {row.id}
            </td>
            <td class=format!("{CELL} text-right")>
                <div class="flex justify-end gap-2">
                    <button
                        type="button"
                        class=BTN_SECONDARY
                        data-fhir-edit=edit_hook
                        on:click=move |_| editor.open(for_edit.clone())
                    >
                        "Edit"
                    </button>
                    <button
                        type="button"
                        class=BTN_DANGER
                        data-fhir-delete=delete_hook
                        disabled=Signal::derive(move || delete.pending().get())
                        on:click=move |_| pending_delete.set(Some(for_delete.clone()))
                    >
                        "Delete"
                    </button>
                </div>
            </td>
        </tr>
    }
}

/// The screen's ONE delete-confirmation modal, driven by `pending_delete`
/// (which row triggered it). Rendered once outside the store section, so a
/// listing refetch never re-creates it; it is inert while no row is pending.
fn delete_dialog(
    pending_delete: RwSignal<Option<FhirMappingRow>>,
    delete: DeleteAction,
) -> AnyView {
    let message = Signal::derive(move || {
        pending_delete.get().map_or_else(String::new, |row| {
            format!(
                "Permanently remove the mapping “{}” from the store? The connector then answers \
                 every {} resource with 'no enabled FHIR mapping', and data already committed \
                 through it is untouched.",
                row.name, row.resource_type
            )
        })
    });
    view! {
        <ConfirmDialog
            open=Signal::derive(move || pending_delete.get().is_some())
            title="Delete mapping"
            message=message
            confirm_label="Delete mapping"
            confirm_id="fhir-delete-confirm"
            on_cancel=Callback::new(move |()| pending_delete.set(None))
            on_confirm=Callback::new(move |()| {
                if let Some(row) = pending_delete.get_untracked() {
                    drop(delete.dispatch(row));
                }
                pending_delete.set(None);
            })
        />
    }
    .into_any()
}

/// The read-path viewer: what a stored mapping produces for one patient.
///
/// The scope lives in the URL behind a `<Form method="GET">`, so a read is
/// shareable, survives a reload, and works before the WASM bundle loads. This
/// is a pure READ: a refusal renders inline — VERBATIM, because the CDR
/// authored it as a FHIR `OperationOutcome` — and never as a toast.
fn read_viewer(
    facade: ReadFacade,
    read_type: Signal<String>,
    read_patient: Signal<String>,
) -> AnyView {
    let initial_type = read_type.get_untracked();
    let initial_patient = read_patient.get_untracked();
    view! {
        <section id="fhir-read" class=format!("{CARD_PAD} mb-4")>
            <h2 class=CARD_TITLE>"Read path"</h2>
            <p class="mb-3 text-xs text-ink-muted">
                "Ask the connector what a stored mapping produces on READ for one patient. The \
                 facade serves only this explicit scope — a resource type and a patient — never a \
                 general FHIR search, and it reads committed data without changing anything."
            </p>
            <leptos_router::components::Form method="GET" action="/fhir" attr:class="mb-3">
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="fhir-read-type">
                            "Resource type"
                        </label>
                        <input
                            id="fhir-read-type"
                            type="text"
                            name="resource_type"
                            class=INPUT
                            placeholder="Observation"
                            value=initial_type
                        />
                    </div>
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="fhir-read-patient">
                            "Patient"
                        </label>
                        <input
                            id="fhir-read-patient"
                            type="text"
                            name="patient"
                            class=INPUT
                            placeholder="p-42"
                            value=initial_patient
                        />
                    </div>
                    <button id="fhir-read-submit" type="submit" class=BTN_PRIMARY>
                        "Read"
                    </button>
                </div>
            </leptos_router::components::Form>
            <Transition fallback=table_skeleton>
                {move || Suspend::new(async move {
                    match facade.await {
                        Ok(None) => idle_hint("Enter a resource type and a patient, then Read."),
                        Ok(Some(answer)) => answered_document("fhir-read", &answer),
                        Err(error) => read_error(&error, "the FHIR read facade"),
                    }
                })}
            </Transition>
        </section>
    }
    .into_any()
}

/// The dry-run panel: validate a resource against its mapping, commit nothing.
///
/// Not a mutation, so it reports inline only — the viewer's toast rule covers
/// writes to the CDR, and this operation is the ingest door's dry twin
/// precisely because it writes nothing.
fn dry_run_panel(form: DryRunForm, dry_run: DryRunAction) -> AnyView {
    let incomplete = Signal::derive(move || {
        form.resource_type.read().trim().is_empty() || form.resource.read().trim().is_empty()
    });
    let on_run = move |_| {
        drop(dry_run.dispatch((
            form.resource_type.get_untracked(),
            form.resource.get_untracked(),
        )));
    };
    let result = move || match dry_run.value().get() {
        None => idle_hint(
            "Paste a FHIR resource and validate it against its stored mapping. Nothing is committed.",
        ),
        Some(Ok(answer)) => verdict_view(&answer),
        Some(Err(error)) => inline_error(&error),
    };
    view! {
        <section id="fhir-dry-run" class=CARD_PAD>
            <h2 class=CARD_TITLE>"Dry run (validate only)"</h2>
            <p class="mb-3 text-xs text-ink-muted">
                "Runs the resource through its stored mapping and the full commit validation, then \
                 reports the verdict. "
                <span class="font-medium text-ink">
                    "Nothing is committed — no EHR, no COMPOSITION, no version."
                </span>
                " Sending a resource for real is the connector's integration door, and the viewer \
                 deliberately offers no path to it."
            </p>
            <div class="flex flex-col gap-3">
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="fhir-dry-run-type">
                            "Resource type"
                        </label>
                        <input
                            id="fhir-dry-run-type"
                            type="text"
                            class=INPUT
                            placeholder="Observation"
                            prop:value=move || form.resource_type.get()
                            on:input:target=move |ev| form.resource_type.set(ev.target().value())
                        />
                    </div>
                    <button
                        id="fhir-dry-run-submit"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=true
                        prop:disabled=move || incomplete.get() || dry_run.pending().get()
                        on:click=on_run
                    >
                        "Validate"
                    </button>
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="fhir-dry-run-resource">
                        "FHIR resource (JSON)"
                    </label>
                    <textarea
                        id="fhir-dry-run-resource"
                        class=format!("{TEXTAREA} min-h-[12rem]")
                        placeholder="{ \"resourceType\": \"Observation\", \"subject\": { \"reference\": \"Patient/p-42\" }, … }"
                        prop:value=move || form.resource.get()
                        on:input:target=move |ev| form.resource.set(ev.target().value())
                    >
                        {form.resource.get_untracked()}
                    </textarea>
                </div>
                {result}
            </div>
        </section>
    }
    .into_any()
}

/// The verdict chip plus the CDR's `OperationOutcome`, verbatim.
///
/// The three verdicts are visually distinct because they mean different things:
/// a validation that ran and passed, one that ran and refused, and an operation
/// that never reached a verdict at all (no mapping for the type, a type the
/// connector does not carry, a resource it could not map).
fn verdict_view(answer: &FhirAnswer) -> AnyView {
    let verdict = verdict_of(answer);
    let chip = match verdict {
        DryRunVerdict::Valid => "rounded-full bg-ok-subtle px-3 py-1 text-sm font-semibold text-ok",
        DryRunVerdict::Invalid => {
            "rounded-full bg-danger-subtle px-3 py-1 text-sm font-semibold text-danger"
        }
        DryRunVerdict::NotRun => {
            "rounded-full bg-warn-subtle px-3 py-1 text-sm font-semibold text-warn"
        }
    };
    let note = match verdict {
        DryRunVerdict::Valid => "The resource maps to a COMPOSITION the CDR would accept.",
        DryRunVerdict::Invalid => "The CDR would refuse the mapped COMPOSITION.",
        DryRunVerdict::NotRun => "The validation did not run — the outcome below says why.",
    };
    let document = answered_document("fhir-dry-run", answer);
    view! {
        <div class="flex flex-col gap-2">
            <div class="flex flex-wrap items-center gap-3">
                <span id="fhir-dry-run-verdict" class=chip data-fhir-verdict=verdict.hook()>
                    {verdict.label()}
                </span>
                <span class="text-xs text-ink-muted">{note}</span>
                <span class="text-xs text-ink-muted">{format!("HTTP {}", answer.status)}</span>
            </div>
            {document}
        </div>
    }
    .into_any()
}

/// One FHIR document the CDR answered with, rendered in the shared document
/// pane, with an `OperationOutcome` marked as such.
///
/// `id_prefix` gives the two panels their own stable E2E hooks
/// (`{prefix}-outcome` for a refusal, `{prefix}-result` for a resource).
fn answered_document(id_prefix: &str, answer: &FhirAnswer) -> AnyView {
    let body = RwSignal::new(answer.body.clone());
    let id = if answer.is_outcome() {
        format!("{id_prefix}-outcome")
    } else {
        format!("{id_prefix}-result")
    };
    view! {
        <div id=id data-fhir-status=answer.status.to_string()>
            <DocumentPane body=body />
        </div>
    }
    .into_any()
}

/// A panel that has not been asked for anything yet: say so, rather than
/// rendering an empty pane that reads as a failure.
fn idle_hint(message: &'static str) -> AnyView {
    view! { <p class="text-sm text-ink-muted">{message}</p> }.into_any()
}
