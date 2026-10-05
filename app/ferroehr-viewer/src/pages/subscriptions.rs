// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/subscriptions` screen — the CDR's event subscriptions.
//!
//! One endpoint family ([`crate::subscriptions`], which carries the wire
//! facts): the table lists what the CDR stores, and the two cards administer it
//! — create, edit, and a two-step delete.
//!
//! NOTE: no openEHR spec governs eventing — our own enterprise extension; the
//! surface is config-gated on the CDR and probe-and-hide here.
//!
//! A subscription is a flat predicate record, so the editors are ordinary form
//! fields rather than a document editor. The name is immutable on the CDR (it
//! is the queue key), so the edit card shows it and never offers to change it,
//! and the update REPLACES the whole predicate set — which is why the form
//! always submits all four, seeded from the row it opened on.

use leptos::component;
use leptos::prelude::*;
use leptos_meta::Title;

use crate::components::confirm_dialog::delete_confirmation;
use crate::components::data_table::{
    CELL, CELL_MONO, ROW, TablePaging, paged_table, paging_from_url, table_skeleton,
};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_DANGER, BTN_PRIMARY, BTN_SECONDARY, text_field};
use crate::components::notice::inline_error;
use crate::components::notice::{alert_note, failure_bar};
use crate::components::page_header::PageHeader;
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::toast::toast_outcome;
use crate::error::ViewerError;
use crate::subscriptions::{
    SubscriptionPredicates, SubscriptionRow, create_event_subscription, delete_event_subscription,
    list_event_subscriptions, match_summary, name_is_valid, predicate_label,
    subscription_failure_copy, update_event_subscription,
};

/// The listing: `None` = the CDR does not serve the event-subscription admin
/// API.
type Listing = Resource<Result<Option<Vec<SubscriptionRow>>, ViewerError>>;

/// The create action: the name it was dispatched with, paired with the CDR's
/// answer, so both toasts name the exact subscription (the action's value IS
/// the mutation report).
type CreateAction =
    Action<(String, SubscriptionPredicates), (String, Result<SubscriptionRow, ViewerError>)>;

/// The update action, reporting the same way.
type UpdateAction = Action<SubscriptionEdit, (String, Result<SubscriptionRow, ViewerError>)>;

/// The delete action, reporting the same way.
type DeleteAction = Action<SubscriptionRow, (String, Result<(), ViewerError>)>;

/// One in-flight edit: which stored subscription, the name to report it under,
/// and the predicates to store on it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SubscriptionEdit {
    /// The CDR id of the subscription being edited.
    id: String,
    /// The subscription's name — immutable on the CDR, carried so the toasts
    /// can name it.
    name: String,
    /// The predicates and the enabled flag to store.
    predicates: SubscriptionPredicates,
}

/// The five draft signals a create or an edit form is made of.
///
/// One struct for both, because the CDR takes the same five values either way;
/// the create form additionally owns a name (an edit cannot change it).
#[derive(Debug, Clone, Copy)]
struct Draft {
    /// The kind predicate draft.
    kind: RwSignal<String>,
    /// The change-type predicate draft.
    change_type: RwSignal<String>,
    /// The template-id predicate draft.
    template_id: RwSignal<String>,
    /// The enabled flag draft.
    enabled: RwSignal<bool>,
}

impl Draft {
    /// An empty draft with the CDR's own default for `enabled` (`true`).
    fn new() -> Self {
        Self {
            kind: RwSignal::new(String::new()),
            change_type: RwSignal::new(String::new()),
            template_id: RwSignal::new(String::new()),
            enabled: RwSignal::new(true),
        }
    }

    /// Fill the draft from a stored row (the Edit click seeds from the row's
    /// OWN values — a user event, so the write happens where it arrives and no
    /// Effect reads a resource to fill a form).
    fn fill_from(self, row: &SubscriptionRow) {
        self.kind.set(row.kind.clone());
        self.change_type.set(row.change_type.clone());
        self.template_id.set(row.template_id.clone());
        self.enabled.set(row.enabled);
    }

    /// Reset to the empty draft.
    fn clear(self) {
        self.kind.set(String::new());
        self.change_type.set(String::new());
        self.template_id.set(String::new());
        self.enabled.set(true);
    }

    /// The values as the server function takes them, read untracked at
    /// dispatch.
    fn read(self) -> SubscriptionPredicates {
        SubscriptionPredicates {
            kind: self.kind.get_untracked(),
            change_type: self.change_type.get_untracked(),
            template_id: self.template_id.get_untracked(),
            enabled: self.enabled.get_untracked(),
        }
    }
}

/// The editor's state: which row is open, plus that row's draft.
#[derive(Debug, Clone, Copy)]
struct Editor {
    /// The row being edited (`None` = the editor is closed).
    target: RwSignal<Option<SubscriptionRow>>,
    /// The draft the save sends.
    draft: Draft,
}

impl Editor {
    /// Open the editor on `row`, seeded with the values it currently holds.
    fn open(self, row: SubscriptionRow) {
        self.draft.fill_from(&row);
        self.target.set(Some(row));
    }

    /// Close the editor and drop the draft.
    fn close(self) {
        self.target.set(None);
        self.draft.clear();
    }
}

/// The event-subscriptions screen.
///
/// The whole screen is probe-and-hide at the NAV level
/// ([`crate::subscriptions`]); reached directly, it renders the listing when
/// the CDR serves it and one naming-the-switch card when it does not — the
/// same gated shape as the other extension screens, never a page of controls
/// that cannot work.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn SubscriptionsPage() -> impl IntoView {
    let toaster = thaw::ToasterInjection::expect_context();
    // The table's page window, read from the URL in SETUP (never inside the
    // suspense that fetches the rows).
    let paging = paging_from_url();
    let editor = Editor {
        target: RwSignal::new(None),
        draft: Draft::new(),
    };
    let new_name = RwSignal::new(String::new());
    let new_draft = Draft::new();
    // The row awaiting confirmation in the modal (`None` = no dialog). ONE
    // dialog serves every row — the signal is both "which row" and "open".
    let pending_delete = RwSignal::new(Option::<SubscriptionRow>::None);

    // Each mutation clears its own form in its OWN async continuation, never
    // from an Effect reading the action's value: a dispatch is the user event,
    // so the answer is handled where it arrives.
    let create: CreateAction = Action::new(move |draft: &(String, SubscriptionPredicates)| {
        let (name, predicates) = draft.clone();
        async move {
            let outcome = create_event_subscription(name.clone(), predicates).await;
            if outcome.is_ok() {
                new_name.set(String::new());
                new_draft.clear();
            }
            (name, outcome)
        }
    });
    let update: UpdateAction = Action::new(move |edit: &SubscriptionEdit| {
        let edit = edit.clone();
        async move {
            let outcome = update_event_subscription(edit.id, edit.predicates).await;
            if outcome.is_ok() {
                editor.close();
            }
            (edit.name, outcome)
        }
    });
    let delete: DeleteAction = Action::new(|row: &SubscriptionRow| {
        let row = row.clone();
        async move {
            let outcome = delete_event_subscription(row.id).await;
            (row.name, outcome)
        }
    });

    let listing: Listing = Resource::new(
        move || {
            (
                create.version().get(),
                update.version().get(),
                delete.version().get(),
            )
        },
        |_| async move { list_event_subscriptions().await },
    );

    mutation_toasts(toaster, create, update, delete);

    let create_card = create_card(new_name, new_draft, create);
    let editor_card = edit_card(editor, update);
    let table = listing_table(listing, paging, editor, pending_delete, delete);
    let confirm = delete_dialog(pending_delete, delete);
    // The delete's own diagnostic, under the table it applies to: a refused
    // delete names its reason in the CDR's words, beside the failure toast.
    let delete_failure = failure_bar(Signal::derive(move || {
        delete.value().get().and_then(|(_, outcome)| outcome.err())
    }));

    view! {
        <Title text="Subscriptions" />
        <div id="subscriptions-screen" class="p-6">
            <PageHeader
                title="Subscriptions"
                subtitle="The CDR's event subscriptions: the stored filters over its committed-version event stream."
            />
            {create_card}
            {editor_card}
            {table}
            {delete_failure}
            {confirm}
        </div>
    }
}

/// Wire the screen's three mutations to their success/failure toasts.
///
/// Every mutation toasts on BOTH outcomes (the viewer's mutation-feedback
/// rule); the CDR's diagnostic ALSO stays inline beside each form, because
/// a `400`/`409` on a subscription write names the exact field or conflict.
fn mutation_toasts(
    toaster: thaw::ToasterInjection,
    create: CreateAction,
    update: UpdateAction,
    delete: DeleteAction,
) {
    toast_outcome(
        toaster,
        create.value().into(),
        ("Subscription created", "Create failed"),
        |name, row| format!("{name} — {}", match_summary(row)),
        write_failure,
    );
    toast_outcome(
        toaster,
        update.value().into(),
        ("Subscription updated", "Update failed"),
        |name, row| format!("{name} — {}", match_summary(row)),
        write_failure,
    );
    toast_outcome(
        toaster,
        delete.value().into(),
        ("Subscription deleted", "Delete failed"),
        |name, ()| format!("{name} was removed from the CDR."),
        write_failure,
    );
}

/// The actionable copy every refused subscription write is reported with,
/// naming the subscription the operator was working on.
fn write_failure(name: &str, error: &ViewerError) -> String {
    subscription_failure_copy(&format!("subscription `{name}`"), error)
}

/// The create card: the name, the four predicates, the enabled toggle, and the
/// create button — inert until the name can actually be accepted.
///
/// The button carries a STATIC `disabled` attribute for the server HTML — the
/// form is empty on the first paint, so the control is inert before hydration —
/// with the live state on `prop:disabled`: an attribute sets the INITIAL state
/// and only a property carries the live one, so an attribute binding alone can
/// leave a control stuck at whatever was serialized.
fn create_card(name: RwSignal<String>, draft: Draft, create: CreateAction) -> AnyView {
    let unusable = Signal::derive(move || !name_is_valid(&name.read()));
    let failure = failure_bar(Signal::derive(move || {
        create.value().get().and_then(|(_, outcome)| outcome.err())
    }));
    view! {
        <section id="subscription-create" class=format!("{CARD_PAD} mb-4")>
            <h2 class=CARD_TITLE>"Create a subscription"</h2>
            <div class="flex flex-wrap items-end gap-3">
                {text_field(
                    "subscription-create-name".to_owned(),
                    "Name",
                    Some("vitals-feed"),
                    name,
                )} {predicate_fields("create", draft)}
                {enabled_toggle("subscription-create-enabled", draft.enabled, true)}
                <button
                    id="subscription-create-submit"
                    type="button"
                    class=BTN_PRIMARY
                    disabled=true
                    prop:disabled=move || unusable.get() || create.pending().get()
                    on:click=move |_| {
                        drop(create.dispatch((name.get_untracked(), draft.read())));
                    }
                >
                    "Create subscription"
                </button>
            </div>
            <p class="mt-2 text-xs text-ink-muted">
                "The name is unique on the CDR and cannot be changed afterwards — it may hold only \
                 letters, digits, and “_”, “.” or “-”. Every predicate left empty matches anything, \
                 so a subscription with none of them set receives every committed version."
            </p>
            {failure}
        </section>
    }
    .into_any()
}

/// The edit card: nothing at all until a row's Edit button opens it, then that
/// row's predicates plus save/cancel.
///
/// Rendered ONCE outside the table, so a list refetch never re-creates it, and
/// closed on both passes of the first render (the editor signal starts empty on
/// server and client alike). The name is shown and never editable: the CDR
/// treats it as the queue key and ignores an echoed one.
fn edit_card(editor: Editor, update: UpdateAction) -> AnyView {
    let failure = failure_bar(Signal::derive(move || {
        update.value().get().and_then(|(_, outcome)| outcome.err())
    }));
    let on_save = move |_| {
        let Some(row) = editor.target.get_untracked() else {
            return;
        };
        drop(update.dispatch(SubscriptionEdit {
            id: row.id,
            name: row.name,
            predicates: editor.draft.read(),
        }));
    };
    let fields = move || {
        let Some(row) = editor.target.get() else {
            return ().into_any();
        };
        view! {
            <section id="subscription-edit" class=format!("{CARD_PAD} mb-4")>
                <h2 class=CARD_TITLE>{format!("Edit subscription {}", row.name)}</h2>
                <p class="mb-3 text-xs text-ink-muted">
                    <span class="font-medium">"Subscription id: "</span>
                    <span class="font-mono break-all text-ink">{row.id}</span>
                    <span class="ml-2">
                        "The name is the CDR's queue key and cannot be changed."
                    </span>
                </p>
                <div class="flex flex-wrap items-end gap-3">
                    {predicate_fields("edit", editor.draft)}
                    {enabled_toggle("subscription-edit-enabled", editor.draft.enabled, false)}
                    <button
                        id="subscription-edit-save"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=Signal::derive(move || update.pending().get())
                        on:click=on_save
                    >
                        "Save subscription"
                    </button>
                    <button
                        id="subscription-edit-cancel"
                        type="button"
                        class=BTN_SECONDARY
                        on:click=move |_| editor.close()
                    >
                        "Cancel"
                    </button>
                </div>
                <p class="mt-2 text-xs text-ink-muted">
                    "Saving replaces every predicate: one cleared here becomes “any” on the CDR."
                </p>
            </section>
        }
        .into_any()
    };
    view! {
        {fields}
        {failure}
    }
    .into_any()
}

/// The four predicate inputs, identical on the create and the edit card.
///
/// `form` distinguishes the two sets of DOM ids (`subscription-create-kind`,
/// `subscription-edit-kind`, …) — both cards can be on screen at once.
fn predicate_fields(form: &'static str, draft: Draft) -> AnyView {
    view! {
        {text_field(format!("subscription-{form}-kind"), "Kind", Some("COMPOSITION"), draft.kind)}
        {text_field(
            format!("subscription-{form}-change-type"),
            "Change type",
            Some("249"),
            draft.change_type,
        )}
        {text_field(
            format!("subscription-{form}-template"),
            "Template id",
            Some("blank = any"),
            draft.template_id,
        )}
    }
    .into_any()
}

/// The enabled toggle: a labelled checkbox bound to `value`.
///
/// `initial` is the value the server pass renders as the `checked` ATTRIBUTE,
/// which must match what the signal holds on the first render of both passes;
/// the live state rides `prop:checked` (an attribute sets the initial state,
/// a property carries the live one).
fn enabled_toggle(id: &'static str, value: RwSignal<bool>, initial: bool) -> AnyView {
    view! {
        <label class="flex items-center gap-2 py-1.5 text-sm font-medium text-ink" r#for=id>
            <input
                id=id
                type="checkbox"
                class="accent-accent"
                checked=initial
                prop:checked=move || value.get()
                on:change=move |ev| value.set(event_target_checked(&ev))
            />
            "Enabled"
        </label>
    }
    .into_any()
}

/// The listing table: the read under `<Transition>` (keep the current rows
/// visible while a refetch runs), resolving its `Result` inside the
/// transition, then the paged rows.
fn listing_table(
    listing: Listing,
    paging: TablePaging,
    editor: Editor,
    pending_delete: RwSignal<Option<SubscriptionRow>>,
    delete: DeleteAction,
) -> AnyView {
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match listing.await {
                    Ok(Some(rows)) if rows.is_empty() => empty_listing(),
                    Ok(Some(rows)) => rows_view(rows, paging, editor, pending_delete, delete),
                    Ok(None) => disabled_card(),
                    Err(error) => read_error(&error, "event subscriptions"),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// A failed READ, rendered inline (never a toast — the viewer's one feedback
/// rule) and, for a refusal, as ACTIONABLE copy rather than the bare wire
/// error.
///
/// The group sits under `/admin`, so the CDR's coarse RBAC classes every call
/// here as admin work: a session without the role is answered `403`, and
/// "forbidden" alone tells the reader nothing about what to do next. A `401` —
/// the credential itself not accepted — lands here too, with its own next
/// action. Capability is not authorization: the screen renders because the
/// surface EXISTS, and this is where the per-request refusal lands.
fn read_error(error: &ViewerError, object: &str) -> AnyView {
    match error {
        ViewerError::CdrUnauthorized(_) | ViewerError::Forbidden(_) => alert_note(
            "subscription-refused",
            subscription_failure_copy(object, error),
        ),
        other => inline_error(other),
    }
}

/// The whole listing when the CDR does not serve the event-subscription admin
/// API.
///
/// A `404` on `GET admin/event_subscription` means the routes answer as if
/// unmounted — what `[events] admin_api = false` does — so the honest screen
/// names the switch instead of showing a table that cannot be read or written.
fn disabled_card() -> AnyView {
    view! {
        <section id="subscriptions-disabled" class=CARD_PAD>
            <EmptyState
                icon=icondata_lu::LuRadioTower
                message="The event-subscription API is disabled on this server"
                hint="The CDR answers its subscription routes as if unmounted. Set admin_api = true under [events] in the CDR's configuration to administer subscriptions here."
            />
        </section>
    }
    .into_any()
}

/// The empty listing — a served surface with nothing stored yet.
fn empty_listing() -> AnyView {
    view! {
        <EmptyState
            icon=icondata_lu::LuRadioTower
            message="No event subscriptions"
            hint="Create the first subscription with the card above."
        />
    }
    .into_any()
}

/// Render the loaded rows: the paged table plus its footer.
///
/// The window comes from the URL, so turning the page re-renders the rows
/// without re-running the suspense that fetched them, and the `<For>` key
/// is the CDR id — stable and data-derived, never an index.
fn rows_view(
    rows: Vec<SubscriptionRow>,
    paging: TablePaging,
    editor: Editor,
    pending_delete: RwSignal<Option<SubscriptionRow>>,
    delete: DeleteAction,
) -> AnyView {
    paged_table(
        rows,
        paging,
        "/subscriptions",
        "subscriptions",
        &[
            "Subscription",
            "Kind",
            "Change type",
            "Template",
            "State",
            "Created",
            "",
        ],
        |row: &SubscriptionRow| row.id.clone(),
        move |row| row_view(row, editor, pending_delete, delete),
    )
}

/// One stored subscription: its name and what it matches in words, the four
/// predicates, its state, and the edit + delete triggers (the latter only opens
/// the screen's confirmation modal).
fn row_view(
    row: SubscriptionRow,
    editor: Editor,
    pending_delete: RwSignal<Option<SubscriptionRow>>,
    delete: DeleteAction,
) -> impl IntoView {
    let hook = row.name.clone();
    let edit_hook = row.name.clone();
    let delete_hook = row.name.clone();
    let summary = match_summary(&row);
    let state = if row.enabled { "enabled" } else { "disabled" };
    let state_class = if row.enabled {
        "inline-flex items-center rounded-control bg-ok-subtle px-2 py-0.5 text-xs font-medium text-ok"
    } else {
        "inline-flex items-center rounded-control bg-sunken px-2 py-0.5 text-xs font-medium text-ink-muted"
    };
    let state_value = if row.enabled { "true" } else { "false" };
    let for_edit = row.clone();
    let for_delete = row.clone();
    view! {
        <tr class=ROW data-subscription=hook>
            <td class=CELL>
                <div class="font-medium text-ink">{row.name}</div>
                <div class="text-xs text-ink-muted" data-subscription-cell="summary">
                    {summary}
                </div>
            </td>
            <td class=CELL_MONO data-subscription-cell="kind">
                {predicate_label(&row.kind)}
            </td>
            <td class=CELL_MONO data-subscription-cell="change-type">
                {predicate_label(&row.change_type)}
            </td>
            <td class=CELL_MONO data-subscription-cell="template">
                {predicate_label(&row.template_id)}
            </td>
            <td class=CELL>
                <span class=state_class data-subscription-state=state_value>
                    {state}
                </span>
            </td>
            <td class=CELL_MONO>{row.created_at}</td>
            <td class=format!("{CELL} text-right")>
                <div class="flex justify-end gap-2">
                    <button
                        type="button"
                        class=BTN_SECONDARY
                        data-subscription-edit=edit_hook
                        on:click=move |_| editor.open(for_edit.clone())
                    >
                        "Edit"
                    </button>
                    <button
                        type="button"
                        class=BTN_DANGER
                        data-subscription-delete=delete_hook
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
/// (which row triggered it). Rendered once outside the table, so a list
/// refetch never re-creates it; it is inert while no row is pending.
fn delete_dialog(
    pending_delete: RwSignal<Option<SubscriptionRow>>,
    delete: DeleteAction,
) -> AnyView {
    let message = Signal::derive(move || {
        pending_delete.get().map_or_else(String::new, |row| {
            format!(
                "Delete the subscription “{}” from the CDR? {} Consumers reading its stream stop \
                 receiving events, and the subscription cannot be restored — only created again \
                 under the same name.",
                row.name,
                match_summary(&row)
            )
        })
    });
    delete_confirmation(
        pending_delete,
        delete,
        "Delete subscription",
        "Delete subscription",
        "subscription-delete-confirm",
        message,
    )
}
