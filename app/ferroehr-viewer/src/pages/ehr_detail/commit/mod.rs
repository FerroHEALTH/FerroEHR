// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail Commit tab: a staging area that commits several correlated
//! changes as ONE atomic CONTRIBUTION.
//!
//! `POST /ehr/{ehr_id}/contribution` is "the 'native' way of committing"
//! (ITS-REST
//! `specifications/docs/overview/Requests_and_responses.md` §"openehr-version
//! and openehr-audit-details"): one change set, several `VERSION`s, all of them
//! committed or none. The per-resource routes the other tabs use are
//! convenience wrappers over exactly this operation, so a correlated change —
//! a new COMPOSITION *and* the EHR status it belongs with — is one commit here
//! instead of two independent ones.
//!
//! The staging list is **viewer-session state only**: it lives in this tab's
//! component state, so navigating away discards it. The viewer stores nothing
//! of its own (§No viewer-local domain state), and the screen says so.
//!
//! Reads are reused, never re-implemented: the template list, the composition
//! list, an amend's seed and the status seed all come from the tabs that
//! already own them. The one WRITE is [`commit_contribution`]; the body it
//! posts is assembled by the component-free, unit-tested [`staged`] module.
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! The wire it writes IS spec-bound. The `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches client-visible state.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

pub mod staged;

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssr")]
use serde_json::Value;

use crate::components::data_table::{CELL, CELL_MONO, ROW, table_shell};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, INPUT, LABEL, SELECT, TEXTAREA};
use crate::components::format_view::pretty_body;
use crate::components::surface::{CARD_PAD, CARD_TITLE, WELL};
use crate::components::toast::toast_success;
use crate::error::ViewerError;
use crate::format::ReprFormat;
use crate::pages::ehr_detail::commit::staged::{
    ChangeType, StagedChange, StagedKind, check, create_target_label,
};
use crate::pages::ehr_detail::status::EhrStatusState;
use crate::pages::ehrs::{ResultPage, cell_text};

/// The noun phrase every commit-failure toast is built around
/// ([`crate::feedback::write_failure_copy`]).
const COMMIT_OBJECT: &str = "this contribution";

/// What a successful commit reports back.
///
/// The CONTRIBUTION's own uid plus the `OBJECT_VERSION_ID` of every version the
/// commit MINTED — the `201` representation's `versions` are the `OBJECT_REF`s
/// of exactly those (ITS-REST
/// `specifications/responses/201_CONTRIBUTION.yaml`). All strings, so the type
/// is safe across the server-fn boundary on the 32-bit WASM target.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommitOutcome {
    /// `CONTRIBUTION.uid.value` — the id the contributions tab looks up.
    pub contribution_uid: String,
    /// The `OBJECT_VERSION_ID` of each version this commit created.
    pub version_uids: Vec<String>,
}

/// Commit the staged changes as ONE CONTRIBUTION
/// (`POST /ehr/{ehr_id}/contribution`).
///
/// The envelope is assembled by [`staged::contribution_body`] from the staged
/// rows; `committer` falls back to the viewer session's own identity when the
/// operator leaves it blank, because the wire REQUIRES one
/// (`specifications/schemas/common/UpdateAudit.yaml`: `required: [change_type,
/// committer]`) and the session identity is the honest answer to "who committed
/// this". `Prefer: return=representation` is sent so the answer names the
/// CONTRIBUTION and every version it minted.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when nothing is staged or a staged change is
/// structurally unusable; CDR transport errors pass through; a non-2xx CDR
/// answer (its validation diagnostics, which the UI renders verbatim,
/// included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(input = server_fn::codec::Json, client = crate::session_client::SessionAwareClient)]
pub async fn commit_contribution(
    /// The EHR to commit the change set into.
    ehr_id: String,
    /// The staged changes, in the order they were staged.
    changes: Vec<StagedChange>,
    /// The `AUDIT_DETAILS.committer` name; blank uses the session identity.
    committer: String,
    /// The optional `AUDIT_DETAILS.description`; blank omits the attribute.
    description: String,
) -> Result<CommitOutcome, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let committer = if committer.trim().is_empty() {
        session.identity.clone()
    } else {
        committer.trim().to_owned()
    };
    let body = staged::contribution_body(&changes, &committer, &description)
        .map_err(ViewerError::Invalid)?;
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/contribution",
        urlencoding::encode(&ehr_id)
    ));
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[("Prefer", "return=representation")],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    Ok(parse_commit_outcome(&response.body))
}

#[cfg(feature = "ssr")]
/// Flatten the committed CONTRIBUTION into a [`CommitOutcome`].
///
/// Defensive throughout — a body the CDR served without the representation
/// (an older server honouring `return=minimal`) reads as an empty outcome
/// rather than failing a commit that actually succeeded.
fn parse_commit_outcome(body: &str) -> CommitOutcome {
    let Ok(doc) = serde_json::from_str::<Value>(body) else {
        return CommitOutcome::default();
    };
    let version_uids = doc
        .get("versions")
        .and_then(Value::as_array)
        .map(|refs| {
            refs.iter()
                .map(|reference| {
                    reference
                        .get("id")
                        .and_then(|id| id.get("value"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                })
                .filter(|uid| !uid.is_empty())
                .collect()
        })
        .unwrap_or_default();
    CommitOutcome {
        contribution_uid: crate::uid::uid_value_of_document(&doc),
        version_uids,
    }
}

/// The staging area's long-lived reactive state, created ONCE in
/// [`commit_section`] — above every `<Transition>` on the tab.
///
/// Held at the tab's owner for the disposal contract: a `Suspend` closure
/// re-runs on every notification of the resources it awaits and each re-run
/// disposes the previous run's reactive owner, so signals created inside one
/// would die while the already-mounted form still references them.
#[derive(Clone, Copy)]
struct CommitForm {
    /// The staged changes, in staging order.
    staged: RwSignal<Vec<StagedChange>>,
    /// The sequence number the next staged row takes as its identity.
    next_seq: RwSignal<u32>,
    /// Which kind of change the draft builds.
    kind: RwSignal<StagedKind>,
    /// The draft's `commit_audit.change_type`.
    change_type: RwSignal<ChangeType>,
    /// The picked template id (a create's label).
    template_id: RwSignal<String>,
    /// The picked composition's `uid` as the list reported it (an amend).
    composition_uid: RwSignal<String>,
    /// The `OBJECT_VERSION_ID` the draft supersedes; empty for a create.
    preceding: RwSignal<String>,
    /// The draft document, as text.
    body: RwSignal<String>,
    /// The seed key already applied, so a resource notification never
    /// overwrites the same version twice.
    seeded: RwSignal<String>,
    /// The client-side complaint about the draft; `None` while it is usable.
    complaint: RwSignal<Option<String>>,
    /// The `AUDIT_DETAILS.committer` name; blank uses the session identity.
    committer: RwSignal<String>,
    /// The `AUDIT_DETAILS.description`.
    description: RwSignal<String>,
    /// Which staged row has its document expanded, by sequence number.
    preview: RwSignal<Option<u32>>,
}

impl std::fmt::Debug for CommitForm {
    /// Signal handles carry no readable content outside a reactive owner, so the
    /// `Debug` impl names the type only — and deliberately never a clinical
    /// value (the PHI caveat in `.claude/rules/reliability.md`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CommitForm")
    }
}

impl CommitForm {
    /// Create the empty staging state.
    fn new() -> Self {
        Self {
            staged: RwSignal::new(Vec::new()),
            next_seq: RwSignal::new(1),
            kind: RwSignal::new(StagedKind::CompositionCreate),
            change_type: RwSignal::new(ChangeType::Creation),
            template_id: RwSignal::new(String::new()),
            composition_uid: RwSignal::new(String::new()),
            preceding: RwSignal::new(String::new()),
            body: RwSignal::new(String::new()),
            seeded: RwSignal::new(String::new()),
            complaint: RwSignal::new(None),
            committer: RwSignal::new(String::new()),
            description: RwSignal::new(String::new()),
            preview: RwSignal::new(None),
        }
    }

    /// Drop the draft's target, document and seed guard, keeping the kind.
    fn clear_draft(self) {
        self.template_id.set(String::new());
        self.composition_uid.set(String::new());
        self.preceding.set(String::new());
        self.body.set(String::new());
        self.seeded.set(String::new());
        self.complaint.set(None);
    }

    /// Switch the draft to another kind: a fresh target, document, seed guard
    /// and the kind's own default change type.
    fn switch_kind(self, kind: StagedKind) {
        self.kind.set(kind);
        self.change_type.set(kind.default_change_type());
        self.clear_draft();
    }

    /// Move the draft into the staging list, or record why it cannot go.
    fn stage(self) {
        let kind = self.kind.get_untracked();
        let seq = self.next_seq.get_untracked();
        let template_id = self.template_id.get_untracked();
        let body = self.body.get_untracked();
        let target = match kind {
            StagedKind::CompositionCreate => create_target_label(&template_id, &body),
            StagedKind::CompositionAmend => self.composition_uid.get_untracked(),
            StagedKind::StatusModify => "EHR_STATUS".to_owned(),
        };
        let candidate = StagedChange {
            seq,
            kind,
            change_type: self.change_type.get_untracked(),
            preceding_version_uid: self.preceding.get_untracked(),
            target,
            body,
        };
        if let Err(complaint) = check(&candidate) {
            self.complaint.set(Some(complaint));
            return;
        }
        self.staged.update(|rows| rows.push(candidate));
        self.next_seq.set(seq.saturating_add(1));
        // Back to the neutral kind rather than an emptied superseding draft:
        // that draft's target picker is cleared too, so it would sit reading
        // "no preceding version" until the operator re-picked one.
        self.switch_kind(StagedKind::CompositionCreate);
    }
}

/// Commit tab: the staging notice, the add-a-change form, the staging list, and
/// the contribution audit that commits them all as one CONTRIBUTION.
///
/// Every resource is created here in setup and gated on the tab being active,
/// so only the visible tab fetches (never fetch-in-effect), and none is created
/// inside a `Suspend`.
pub(super) fn commit_section(ehr_id: Signal<String>, selected: Memo<String>) -> AnyView {
    let toaster = thaw::ToasterInjection::expect_context();
    let form = CommitForm::new();
    let active = Memo::new(move |_| selected.get() == "commit");

    let commit: Action<CommitRequest, Result<CommitOutcome, ViewerError>> =
        Action::new(move |request: &CommitRequest| {
            let request = request.clone();
            async move {
                let outcome = commit_contribution(
                    request.ehr_id,
                    request.changes,
                    request.committer,
                    request.description,
                )
                .await;
                // The staging survives a refusal and is cleared only by
                // success — written in the action's own async continuation,
                // never from an Effect reading its value.
                if outcome.is_ok() {
                    form.staged.set(Vec::new());
                    form.preview.set(None);
                    form.description.set(String::new());
                }
                outcome
            }
        });
    // A successful commit moves the EHR on, so the commit version is a source
    // of every read below: the composition list gains what was created and the
    // seeds resolve against the versions this commit minted (the action's
    // version is the refetch trigger, never an Effect).
    let templates = Resource::new(
        move || active.get().then_some(()),
        |on| async move {
            match on {
                Some(()) => crate::pages::templates::list_templates().await.map(Some),
                None => Ok(None),
            }
        },
    );
    let compositions = Resource::new(
        move || {
            let version = commit.version().get();
            active.get().then(|| (ehr_id.get(), version))
        },
        |on| async move {
            match on {
                Some((id, _)) => crate::pages::ehr_detail::compositions::all_compositions(id, 0)
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
    );
    let amend_seed = Resource::new(
        move || {
            let uid = form.composition_uid.get();
            let version = commit.version().get();
            (active.get() && form.kind.get() == StagedKind::CompositionAmend && !uid.is_empty())
                .then(|| (ehr_id.get(), crate::uid::container_uid_of(&uid), version))
        },
        |on| async move {
            match on {
                // The reader already answers `None` for a deleted version
                // (the operation's `204`), which is exactly "nothing to seed".
                Some((id, vo_id, _)) => {
                    crate::pages::composition::fetch_composition(
                        id,
                        vo_id,
                        ReprFormat::CanonicalJson,
                    )
                    .await
                }
                None => Ok(None),
            }
        },
    );
    let status_seed = Resource::new(
        move || {
            let version = commit.version().get();
            (active.get() && form.kind.get() == StagedKind::StatusModify)
                .then(|| (ehr_id.get(), version))
        },
        |on| async move {
            match on {
                Some((id, _)) => crate::pages::ehr_detail::status::fetch_ehr_status(id)
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
    );
    seed_amend(form, amend_seed);
    seed_status(form, status_seed);
    Effect::new(move |_| match commit.value().get() {
        Some(Ok(outcome)) => {
            toast_success(toaster, "Contribution committed", &committed_copy(&outcome));
        }
        Some(Err(error)) => {
            crate::feedback::toast_write_failure(toaster, "Commit failed", COMMIT_OBJECT, &error);
        }
        None => {}
    });

    let notice = staging_notice();
    let add = add_change_card(form, templates, compositions);
    let list = staging_list(form);
    let audit = audit_card(ehr_id, form, commit);
    view! { <div class="flex flex-col gap-4">{notice} {add} {list} {audit}</div> }.into_any()
}

/// One dispatched commit: everything the action hands the server fn.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitRequest {
    /// The EHR to commit into.
    ehr_id: String,
    /// The staged changes, in staging order.
    changes: Vec<StagedChange>,
    /// The audit committer, or blank for the session identity.
    committer: String,
    /// The audit description.
    description: String,
}

/// Seed the draft from the picked composition's CURRENT version.
///
/// A resource-reading `Effect`, kept deliberately: the textarea and the
/// preceding-version readout it writes are ALWAYS mounted, outside the picker's
/// own `<Transition>` — seeding from inside that `Suspend` would write them
/// during the server pass and again during hydration replay, the mid-walk
/// divergence that surfaces as tachys' unrecoverable-hydration panic. Two guards
/// keep it from fighting the operator: the seed key (one seed per loaded
/// version) and the empty-draft check (an in-progress document is never
/// overwritten).
fn seed_amend(form: CommitForm, resource: Resource<Result<Option<String>, ViewerError>>) {
    Effect::new(move |_| {
        let Some(Ok(Some(body))) = resource.get() else {
            return;
        };
        let version_uid = crate::uid::uid_value_of(&body);
        let key = format!("amend:{version_uid}");
        if form.seeded.with_untracked(|seeded| *seeded == key)
            || !form.body.with_untracked(|draft| draft.trim().is_empty())
        {
            return;
        }
        form.seeded.set(key);
        form.preceding.set(version_uid);
        form.body.set(body);
        form.complaint.set(None);
    });
}

/// Seed the draft from the EHR's current `EHR_STATUS` — same shape, same two
/// guards, and the same justification as [`seed_amend`].
fn seed_status(form: CommitForm, resource: Resource<Result<Option<EhrStatusState>, ViewerError>>) {
    Effect::new(move |_| {
        let Some(Ok(Some(state))) = resource.get() else {
            return;
        };
        let key = format!("status:{}", state.version_uid);
        if form.seeded.with_untracked(|seeded| *seeded == key)
            || !form.body.with_untracked(|draft| draft.trim().is_empty())
        {
            return;
        }
        form.seeded.set(key);
        form.preceding.set(state.version_uid.clone());
        form.body
            .set(pretty_body(&state.body, ReprFormat::CanonicalJson));
        form.complaint.set(None);
    });
}

/// The tab's opening note: what a change set is, and that the staging is held
/// in this browser tab alone.
fn staging_notice() -> AnyView {
    view! {
        <section class=CARD_PAD id="stage-notice">
            <h2 class=CARD_TITLE>"Commit several changes as one contribution"</h2>
            <p class="text-sm text-ink-muted">
                "A CONTRIBUTION is openEHR's atomic change set: every staged change is committed together, or none of them is. Stage the changes that belong together — a new composition and the EHR status it goes with — then commit once."
            </p>
            <p class="mt-2 text-xs text-ink-faint">
                "Staged changes live in this browser tab only. The viewer stores nothing of its own, so leaving this screen discards them; nothing reaches the CDR until you commit."
            </p>
        </section>
    }
    .into_any()
}

/// The add-a-change card: the kind picker, the per-kind target field, the
/// change-type picker, the document draft, and the Stage button.
///
/// Every field is mounted in the same structure on the server and the client
/// and toggled with `class:hidden`, so switching kinds never changes the view's
/// shape.
fn add_change_card(
    form: CommitForm,
    templates: Resource<Result<Option<Vec<crate::pages::templates::TemplateRow>>, ViewerError>>,
    compositions: Resource<Result<Option<ResultPage>, ViewerError>>,
) -> AnyView {
    let is_create = move || form.kind.get() == StagedKind::CompositionCreate;
    let is_amend = move || form.kind.get() == StagedKind::CompositionAmend;
    let is_status = move || form.kind.get() == StagedKind::StatusModify;
    // A superseding draft is inert until its preceding version is known: an
    // edit made before the seed lands would simply be replaced by it.
    let unseeded = Signal::derive(move || {
        form.kind.get().supersedes() && form.preceding.with(|uid| uid.trim().is_empty())
    });
    let kind_row = kind_picker(form);
    let template_row = template_picker(form, templates);
    let composition_row = composition_picker(form, compositions);
    let preceding_row = preceding_readout(form);
    let change_type_row = change_type_picker(form);
    let draft = draft_editor(form, unseeded);
    view! {
        <section class=CARD_PAD id="stage-add">
            <h2 class=CARD_TITLE>"Add a change"</h2>
            <div class="flex flex-col gap-3">
                <div class="flex flex-wrap items-end gap-3">
                    {kind_row} <div class="flex flex-col gap-1" class:hidden=move || !is_create()>
                        {template_row}
                    </div> <div class="flex flex-col gap-1" class:hidden=move || !is_amend()>
                        {composition_row}
                    </div> {change_type_row}
                </div>
                <div class:hidden=move || is_create()>{preceding_row}</div>
                {draft}
                <p class="text-xs text-ink-faint" class:hidden=move || !is_status()>
                    "The document is the EHR's current status, loaded for you — edit it and it commits as a new EHR_STATUS version."
                </p>
            </div>
        </section>
    }
    .into_any()
}

/// The kind `<select>`: which of the three changes the draft builds.
fn kind_picker(form: CommitForm) -> AnyView {
    view! {
        <div class="flex flex-col gap-1">
            <label class=LABEL r#for="stage-kind">
                "Change"
            </label>
            <select
                id="stage-kind"
                class=SELECT
                prop:value=move || form.kind.get().as_value()
                on:change:target=move |ev| {
                    form.switch_kind(StagedKind::from_value(&ev.target().value()));
                }
            >
                <option value="create">{StagedKind::CompositionCreate.label()}</option>
                <option value="amend">{StagedKind::CompositionAmend.label()}</option>
                <option value="status">{StagedKind::StatusModify.label()}</option>
            </select>
        </div>
    }
    .into_any()
}

/// The template `<select>` for a create — the CDR's own operational-template
/// list, used as the staged row's label.
fn template_picker(
    form: CommitForm,
    templates: Resource<Result<Option<Vec<crate::pages::templates::TemplateRow>>, ViewerError>>,
) -> AnyView {
    view! {
        <label class=LABEL r#for="stage-template">
            "Template"
        </label>
        <Transition fallback=|| {
            view! { <span class="text-sm text-ink-muted">"Loading templates…"</span> }
        }>
            {move || Suspend::new(async move {
                match templates.await {
                    Ok(Some(rows)) => {
                        let options = rows
                            .into_iter()
                            .map(|row| {
                                view! {
                                    <option value=row
                                        .template_id
                                        .clone()>{row.template_id.clone()}</option>
                                }
                            })
                            .collect::<Vec<_>>();
                        view! {
                            <select
                                id="stage-template"
                                class=SELECT
                                prop:value=move || form.template_id.get()
                                on:change:target=move |ev| form.template_id.set(ev.target().value())
                            >
                                <option value="">"— from the document —"</option>
                                {options}
                            </select>
                        }
                            .into_any()
                    }
                    Ok(None) => ().into_any(),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The composition `<select>` for an amend — the EHR's own compositions, the
/// same AQL-driven list the Compositions tab shows.
fn composition_picker(
    form: CommitForm,
    compositions: Resource<Result<Option<ResultPage>, ViewerError>>,
) -> AnyView {
    view! {
        <label class=LABEL r#for="stage-composition">
            "Composition"
        </label>
        <Transition fallback=|| {
            view! { <span class="text-sm text-ink-muted">"Loading compositions…"</span> }
        }>
            {move || Suspend::new(async move {
                match compositions.await {
                    Ok(Some(page)) => {
                        let options = page
                            .rows
                            .iter()
                            .map(|row| {
                                let uid = row.first().map(cell_text).unwrap_or_default();
                                let name = row.get(1).map(cell_text).unwrap_or_default();
                                let label = if name.is_empty() {
                                    uid.clone()
                                } else {
                                    format!("{name} — {uid}")
                                };
                                view! { <option value=uid.clone()>{label}</option> }
                            })
                            .collect::<Vec<_>>();
                        view! {
                            <select
                                id="stage-composition"
                                class=SELECT
                                prop:value=move || form.composition_uid.get()
                                on:change:target=move |ev| {
                                    form.seeded.set(String::new());
                                    form.preceding.set(String::new());
                                    form.body.set(String::new());
                                    form.composition_uid.set(ev.target().value());
                                }
                            >
                                <option value="">"— pick a composition —"</option>
                                {options}
                            </select>
                        }
                            .into_any()
                    }
                    Ok(None) => ().into_any(),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The preceding-version readout: the `OBJECT_VERSION_ID` this draft
/// supersedes, so the operator sees exactly what the member will name.
fn preceding_readout(form: CommitForm) -> AnyView {
    view! {
        <p class="text-xs text-ink-muted">
            <span class="font-medium">"Preceding version: "</span>
            <span id="stage-preceding" class="font-mono break-all">
                {move || {
                    let uid = form.preceding.get();
                    if uid.is_empty() { "not loaded yet".to_owned() } else { uid }
                }}
            </span>
        </p>
    }
    .into_any()
}

/// The change-type `<select>`, offering exactly the `audit_change_type` codes
/// the wire accepts for the picked kind ([`StagedKind::change_types`]).
fn change_type_picker(form: CommitForm) -> AnyView {
    view! {
        <div class="flex flex-col gap-1">
            <label class=LABEL r#for="stage-change-type">
                "Change type"
            </label>
            <select
                id="stage-change-type"
                class=SELECT
                prop:value=move || form.change_type.get().rubric()
                on:change:target=move |ev| {
                    form.change_type.set(ChangeType::from_rubric(&ev.target().value()));
                }
            >
                {move || {
                    form.kind
                        .get()
                        .change_types()
                        .iter()
                        .map(|change_type| {
                            view! {
                                <option value=change_type
                                    .rubric()>
                                    {format!("{} ({})", change_type.rubric(), change_type.code())}
                                </option>
                            }
                        })
                        .collect::<Vec<_>>()
                }}
            </select>
        </div>
    }
    .into_any()
}

/// The draft document editor plus the Stage button and the client-side
/// complaint about the draft.
///
/// Inert-until-seeded is a two-part split: a live
/// `prop:disabled` for the state after hydration, and a STATIC `disabled`
/// attribute on the Stage button, which starts inert on both the server pass
/// and the client because the draft starts empty.
fn draft_editor(form: CommitForm, unseeded: Signal<bool>) -> AnyView {
    let stageable =
        Signal::derive(move || !unseeded.get() && !form.body.with(|draft| draft.trim().is_empty()));
    let complaint = move || {
        match form.complaint.get() {
        Some(message) => {
            view! {
                <div
                    role="alert"
                    id="stage-complaint"
                    class="rounded-control border border-danger/40 bg-danger-subtle px-3 py-2 text-sm text-danger"
                >
                    {message}
                </div>
            }
            .into_any()
        }
        None => ().into_any(),
    }
    };
    view! {
        <div class="flex flex-col gap-1">
            <label class=LABEL r#for="stage-body">
                "Document (canonical JSON)"
            </label>
            <textarea
                id="stage-body"
                class=format!("{TEXTAREA} min-h-[14rem]")
                placeholder="paste the canonical JSON document, or pick a target above to load it…"
                prop:disabled=move || unseeded.get()
                prop:value=move || form.body.get()
                on:input:target=move |ev| form.body.set(ev.target().value())
            >
                {form.body.get_untracked()}
            </textarea>
        </div>
        <div class="flex items-center gap-3">
            <button
                id="stage-add-change"
                type="button"
                class=BTN_SECONDARY
                disabled=true
                prop:disabled=move || !stageable.get()
                on:click=move |_| form.stage()
            >
                <leptos_icons::Icon icon=icondata_lu::LuPlus width="14" height="14" />
                "Stage this change"
            </button>
        </div>
        {complaint}
    }
    .into_any()
}

/// The staging list: one row per pending change, each with a document preview
/// and a Remove.
///
/// `<Show>` memoizes the empty/non-empty branch and renders each once, so
/// the `<For>` inside it is created ONCE and updates row by row on a stable,
/// data-derived key — never an index, which would re-key every row after a
/// Remove.
fn staging_list(form: CommitForm) -> AnyView {
    let table = view! {
        <Show
            when=move || !form.staged.with(Vec::is_empty)
            fallback=|| {
                view! {
                    <EmptyState
                        icon=icondata_lu::LuLayers
                        message="Nothing staged yet"
                        hint="Add a change above; they commit together as one contribution."
                    />
                }
            }
        >
            {table_shell(
                &["Change", "Target", "Change type", ""],
                view! {
                    <For each=move || form.staged.get() key=|change| change.seq let:change>
                        {staged_row(form, &change)}
                    </For>
                }
                    .into_any(),
            )}
        </Show>
    }
    .into_any();
    view! {
        <section class=CARD_PAD id="stage-list">
            <h2 class=CARD_TITLE>
                "Staged changes" <span id="stage-count" class="ml-2 font-normal text-ink-muted">
                    {move || form.staged.with(Vec::len)}
                </span>
            </h2>
            {table}
        </section>
    }
    .into_any()
}

/// One staged row plus its (hidden until asked for) document preview row.
fn staged_row(form: CommitForm, change: &StagedChange) -> AnyView {
    let seq = change.seq;
    let kind = change.kind.label();
    let target = change.target.clone();
    let change_type = format!(
        "{} ({})",
        change.change_type.rubric(),
        change.change_type.code()
    );
    let document = change.body.clone();
    let toggle = move |_| {
        form.preview
            .update(|open| *open = if *open == Some(seq) { None } else { Some(seq) });
    };
    let remove = move |_| {
        form.staged.update(|rows| rows.retain(|row| row.seq != seq));
        form.preview.update(|open| {
            if *open == Some(seq) {
                *open = None;
            }
        });
    };
    view! {
        <tr class=ROW data-staged=seq.to_string()>
            <td class=CELL>{kind}</td>
            <td class=CELL_MONO>{target}</td>
            <td class=CELL>{change_type}</td>
            <td class=CELL>
                <div class="flex gap-2">
                    <button
                        id=format!("stage-preview-{seq}")
                        type="button"
                        class=BTN_SECONDARY
                        on:click=toggle
                    >
                        "Preview"
                    </button>
                    <button
                        id=format!("stage-remove-{seq}")
                        type="button"
                        class=BTN_SECONDARY
                        on:click=remove
                    >
                        "Remove"
                    </button>
                </div>
            </td>
        </tr>
        <tr class:hidden=move || form.preview.get() != Some(seq)>
            <td class=CELL colspan="4">
                <div class=WELL>
                    <pre class="overflow-auto max-h-[24rem] whitespace-pre-wrap font-mono text-xs text-ink">
                        {document}
                    </pre>
                </div>
            </td>
        </tr>
    }
    .into_any()
}

/// The contribution-audit card: the committer, the description, the commit
/// button, and the commit's own result pane.
fn audit_card(
    ehr_id: Signal<String>,
    form: CommitForm,
    commit: Action<CommitRequest, Result<CommitOutcome, ViewerError>>,
) -> AnyView {
    let count =
        Signal::derive(move || u32::try_from(form.staged.with(Vec::len)).unwrap_or(u32::MAX));
    let label = Signal::derive(move || commit_button_label(count.get()));
    let on_commit = move |_| {
        commit.dispatch(CommitRequest {
            ehr_id: ehr_id.get(),
            changes: form.staged.get(),
            committer: form.committer.get(),
            description: form.description.get(),
        });
    };
    let result = commit_result(ehr_id, commit);
    view! {
        <section class=CARD_PAD id="stage-audit">
            <h2 class=CARD_TITLE>"Contribution audit"</h2>
            <div class="flex flex-col gap-3">
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="stage-committer">
                            "Committer (optional)"
                        </label>
                        <input
                            id="stage-committer"
                            type="text"
                            class=INPUT
                            placeholder="defaults to your viewer identity"
                            prop:value=move || form.committer.get()
                            on:input:target=move |ev| form.committer.set(ev.target().value())
                        />
                    </div>
                    <div class="flex grow flex-col gap-1">
                        <label class=LABEL r#for="stage-description">
                            "Description"
                        </label>
                        <input
                            id="stage-description"
                            type="text"
                            class=format!("{INPUT} w-full")
                            placeholder="why these changes belong together"
                            prop:value=move || form.description.get()
                            on:input:target=move |ev| form.description.set(ev.target().value())
                        />
                    </div>
                </div>
                <div class="flex items-center gap-3">
                    <button
                        id="stage-commit"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=true
                        prop:disabled=move || count.get() == 0 || commit.pending().get()
                        on:click=on_commit
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuUpload width="14" height="14" />
                        {move || label.get()}
                    </button>
                    <Show when=move || commit.pending().get()>
                        <span class="text-sm text-ink-muted">"Committing…"</span>
                    </Show>
                </div>
                {result}
            </div>
        </section>
    }
    .into_any()
}

/// The commit's own result pane: the CDR's diagnostics verbatim on refusal (the
/// toast is the notification, this is the detail worth reading line by line),
/// the committed contribution on success.
fn commit_result(
    ehr_id: Signal<String>,
    commit: Action<CommitRequest, Result<CommitOutcome, ViewerError>>,
) -> AnyView {
    view! {
        {move || match commit.value().get() {
            Some(Err(error)) => {
                view! {
                    <div class=WELL id="stage-diagnostic" role="alert">
                        <p class="mb-2 text-xs text-ink-muted">
                            "Nothing was committed and your staged changes are untouched. The CDR refused the change set as a whole — its diagnostic follows verbatim. It always names the defect, and a shape defect also names the member index (counting from the top of the list above); a validation refusal does not, so read it against the staged documents."
                        </p>
                        <pre class="overflow-auto max-h-[40vh] whitespace-pre-wrap font-mono text-xs text-danger">
                            {error.to_string()}
                        </pre>
                    </div>
                }
                    .into_any()
            }
            Some(Ok(outcome)) => {
                let href = format!("/ehrs/{}?tab=contributions", ehr_id.get());
                let uids = outcome
                    .version_uids
                    .iter()
                    .map(|uid| {
                        view! { <li class="font-mono break-all">{uid.clone()}</li> }
                    })
                    .collect::<Vec<_>>();
                view! {
                    <div class=WELL id="stage-result">
                        <p class="text-sm text-ink">{committed_copy(&outcome)}</p>
                        <ul class="mt-2 text-xs text-ink-muted">{uids}</ul>
                        <a href=href class="mt-2 inline-block text-sm text-accent hover:underline">
                            "Open it in the Contributions tab →"
                        </a>
                    </div>
                }
                    .into_any()
            }
            None => ().into_any(),
        }}
    }
    .into_any()
}

/// The commit button's label, naming how many changes ride the one
/// contribution.
fn commit_button_label(count: u32) -> String {
    match count {
        0 => "Commit as one contribution".to_owned(),
        1 => "Commit 1 change as one contribution".to_owned(),
        many => format!("Commit {many} changes as one contribution"),
    }
}

/// The sentence a successful commit reports, in the toast and in the result
/// pane.
fn committed_copy(outcome: &CommitOutcome) -> String {
    let versions = outcome.version_uids.len();
    let noun = if versions == 1 { "version" } else { "versions" };
    if outcome.contribution_uid.is_empty() {
        format!("The change set was committed as one contribution ({versions} {noun}).")
    } else {
        format!(
            "Contribution {} committed with {versions} {noun}.",
            outcome.contribution_uid
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{CommitOutcome, commit_button_label, committed_copy};

    #[test]
    fn the_commit_button_states_the_staged_count() {
        assert_eq!(commit_button_label(0), "Commit as one contribution");
        assert_eq!(
            commit_button_label(1),
            "Commit 1 change as one contribution"
        );
        assert_eq!(
            commit_button_label(2),
            "Commit 2 changes as one contribution"
        );
    }

    #[test]
    fn the_success_copy_names_the_contribution_and_its_versions() {
        let outcome = CommitOutcome {
            contribution_uid: "0826851c-c4c2-4d61-92b9-410fb8275ff0".to_owned(),
            version_uids: vec!["a::sys::1".to_owned(), "b::sys::2".to_owned()],
        };
        assert_eq!(
            committed_copy(&outcome),
            "Contribution 0826851c-c4c2-4d61-92b9-410fb8275ff0 committed with 2 versions."
        );
        // A `return=minimal` answer still reports the commit honestly.
        assert_eq!(
            committed_copy(&CommitOutcome::default()),
            "The change set was committed as one contribution (0 versions)."
        );
    }
}

#[cfg(all(test, feature = "ssr"))]
mod outcome_tests {
    use super::parse_commit_outcome;

    #[test]
    fn the_201_representation_yields_the_contribution_and_every_minted_version() {
        // The body a composed CDR answered to a two-member change set.
        let body = r#"{
            "_type": "CONTRIBUTION",
            "uid": {"_type": "HIER_OBJECT_ID", "value": "01a02ba0-f378-7034-9628-cc373c56ece4"},
            "audit": {"_type": "AUDIT_DETAILS"},
            "versions": [
                {"_type": "OBJECT_REF", "namespace": "local", "type": "EHR_STATUS",
                 "id": {"_type": "OBJECT_VERSION_ID", "value": "01a0::ferroehr.local::2"}},
                {"_type": "OBJECT_REF", "namespace": "local", "type": "COMPOSITION",
                 "id": {"_type": "OBJECT_VERSION_ID", "value": "01a1::ferroehr.local::1"}}
            ]
        }"#;
        let outcome = parse_commit_outcome(body);
        assert_eq!(
            outcome.contribution_uid,
            "01a02ba0-f378-7034-9628-cc373c56ece4"
        );
        assert_eq!(
            outcome.version_uids,
            vec![
                "01a0::ferroehr.local::2".to_owned(),
                "01a1::ferroehr.local::1".to_owned()
            ]
        );
    }

    #[test]
    fn an_absent_representation_reads_as_an_empty_outcome_never_a_failure() {
        for body in ["", "{}", "not json", r#"{"versions":[{"id":{}}]}"#] {
            let outcome = parse_commit_outcome(body);
            assert!(outcome.contribution_uid.is_empty(), "{body}");
            assert!(outcome.version_uids.is_empty(), "{body}");
        }
    }
}
