// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR_STATUS` edit form: the two capability toggles, the `other_details`
//! JSON editor, and the pure merge that turns them into the body
//! `PUT /ehr/{ehr_id}/ehr_status` sends.
//!
//! The form edits EXACTLY three attributes of the loaded document
//! (`is_queryable`, `is_modifiable`, `other_details`) and re-sends everything
//! else — the `subject`, the `name`, the `archetype_node_id`, the `uid`, any
//! attribute a future spec release adds — byte-for-byte as the CDR served it.
//! The viewer never rebuilds an `EHR_STATUS` from its own model, so an edit
//! cannot drop what the screen does not render (`EHR_STATUS` requires
//! `subject`, `is_queryable` and `is_modifiable` — RM
//! `docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc` §`EHR_STATUS`).
//!
//! The merge and the `other_details` check are pure functions with unit tests;
//! the view stays thin. `other_details` is optional and typed `UItemStructure`
//! in that schema, so a blank editor REMOVES the attribute and a non-object
//! value is refused before any round trip — the CDR's own validation is still
//! the authority and its diagnostic is rendered verbatim.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::prelude::*;
use serde_json::Value;

use crate::components::field::{BTN_PRIMARY, LABEL, TEXTAREA};
use crate::components::surface::{CARD_PAD, CARD_TITLE, WELL};
use crate::error::ViewerError;
use crate::pages::ehr_detail::status::EhrStatusState;

/// One dispatched status edit: the target EHR, the `If-Match` version, the
/// verbatim document the edits apply to, and the three edited values.
///
/// A named struct rather than a tuple because the action carries six values and
/// two of them are booleans — a tuple would make the dispatch site unreadable
/// and a swapped pair invisible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatusEdit {
    /// The EHR whose status is being updated.
    pub ehr_id: String,
    /// The loaded version's `OBJECT_VERSION_ID` — the `If-Match` value.
    pub version_uid: String,
    /// The loaded `EHR_STATUS` document, verbatim — the merge base.
    pub base_body: String,
    /// The new `is_queryable`.
    pub is_queryable: bool,
    /// The new `is_modifiable`.
    pub is_modifiable: bool,
    /// The new `other_details` as JSON text; blank removes the attribute.
    pub other_details: String,
}

/// The edit form's long-lived reactive state, created ONCE in
/// [`status_section`](super::status_section) — ABOVE the `<Transition>` — and
/// re-seeded (idempotent per loaded version) by [`seed`] on each Suspend re-run.
///
/// This is the disposal contract in signal form: a `Suspend` closure re-runs on
/// every notification of the resource it awaits (the status resource re-notifies
/// after every successful save) and each re-run disposes the previous run's
/// reactive owner, so signals created inside it would die while the
/// already-mounted form's event handlers still reference them. Held here, at the
/// tab's owner, every signal outlives every re-run.
#[derive(Clone, Copy)]
pub(super) struct StatusForm {
    /// The edited `is_queryable`.
    queryable: RwSignal<bool>,
    /// The edited `is_modifiable`.
    modifiable: RwSignal<bool>,
    /// The `other_details` JSON draft (blank = remove the attribute).
    other_details: RwSignal<String>,
    /// The loaded version's `OBJECT_VERSION_ID` — the `If-Match` a save sends.
    version_uid: RwSignal<String>,
    /// The loaded document, verbatim — the merge base a save sends.
    base_body: RwSignal<String>,
    /// The client-side `other_details` complaint, when the draft is not a JSON
    /// object; `None` while the draft is acceptable.
    validation: RwSignal<Option<String>>,
    /// The version this state was last seeded from; [`seed`] is a no-op while it
    /// already equals the loaded version, so a Suspend re-run for the SAME
    /// version never overwrites the operator's in-progress edits.
    seeded_uid: RwSignal<Option<String>>,
}

impl std::fmt::Debug for StatusForm {
    /// Signal handles carry no readable content outside a reactive owner, so the
    /// `Debug` impl names the type only — and deliberately never a clinical
    /// value (the PHI caveat in `.claude/rules/reliability.md`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StatusForm")
    }
}

impl StatusForm {
    /// Create the form's long-lived state, empty until the first [`seed`].
    #[must_use]
    pub(super) fn new() -> Self {
        Self {
            queryable: RwSignal::new(false),
            modifiable: RwSignal::new(false),
            other_details: RwSignal::new(String::new()),
            version_uid: RwSignal::new(String::new()),
            base_body: RwSignal::new(String::new()),
            validation: RwSignal::new(None),
            seeded_uid: RwSignal::new(None),
        }
    }
}

impl Default for StatusForm {
    fn default() -> Self {
        Self::new()
    }
}

/// Seed [`StatusForm`] from the freshly loaded status, ONCE per loaded version.
///
/// A Suspend re-run for the SAME version is a no-op (the state lives above the
/// Suspend, so re-seeding would overwrite edits in progress); a NEW version
/// resets the toggles, the `other_details` draft, the merge base, the
/// `If-Match` value and the validation note. Every write runs during a render
/// pass, so plain `.set()` is correct.
pub(super) fn seed(form: StatusForm, state: &EhrStatusState) {
    if form.seeded_uid.get_untracked().as_deref() == Some(state.version_uid.as_str()) {
        return;
    }
    form.queryable.set(state.is_queryable);
    form.modifiable.set(state.is_modifiable);
    form.other_details.set(state.other_details.clone());
    form.version_uid.set(state.version_uid.clone());
    form.base_body.set(state.body.clone());
    form.validation.set(None);
    form.seeded_uid.set(Some(state.version_uid.clone()));
}

/// Read an `other_details` draft: `None` for a blank draft (the attribute is
/// removed), `Some(value)` for a JSON object.
///
/// `EHR_STATUS.other_details` is an `ITEM_STRUCTURE` `0..1` (RM
/// `docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc` §`EHR_STATUS`), so
/// a non-object can never be valid and is refused here, before anything is
/// sent.
///
/// # Errors
/// The operator-facing complaint when the draft is not parseable JSON or not a
/// JSON object.
pub(super) fn parse_other_details(draft: &str) -> Result<Option<Value>, String> {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|e| format!("other_details is not valid JSON: {e}"))?;
    if value.is_object() {
        Ok(Some(value))
    } else {
        Err(
            "other_details must be a JSON object — an ITEM_STRUCTURE such as \
             {\"_type\": \"ITEM_TREE\", …}"
                .to_owned(),
        )
    }
}

#[cfg(feature = "ssr")]
/// Apply the form's three edits to the loaded `EHR_STATUS` document and return
/// the body to PUT.
///
/// `base` is re-sent verbatim apart from `is_queryable`, `is_modifiable` and
/// `other_details` — the merge is a key replacement on the parsed object, never
/// a rebuild, so every other attribute (`subject`, `name`,
/// `archetype_node_id`, `uid`, anything a newer spec release adds) survives
/// unchanged. A blank `other_details` REMOVES the key, which is the only way to
/// express "no other details" for an optional attribute.
///
/// # Errors
/// [`ViewerError::Invalid`] when `base` is not a JSON object (the CDR would
/// answer `422` anyway — `EHR_STATUS` must be a JSON object per
/// RM `docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc` §`EHR_STATUS`), when the `other_details` draft
/// is not a JSON object, or when the merged document cannot be re-serialized.
pub(super) fn apply_status_edits(
    base: &str,
    is_queryable: bool,
    is_modifiable: bool,
    other_details: &str,
) -> Result<String, ViewerError> {
    let mut doc: Value = serde_json::from_str(base).map_err(|e| {
        ViewerError::Invalid(format!(
            "the loaded EHR_STATUS document is not valid JSON ({e}) — reload this tab and retry"
        ))
    })?;
    let details = parse_other_details(other_details).map_err(ViewerError::Invalid)?;
    let object = doc.as_object_mut().ok_or_else(|| {
        ViewerError::Invalid(
            "the loaded EHR_STATUS document is not a JSON object — reload this tab and retry"
                .to_owned(),
        )
    })?;
    drop(object.insert("is_queryable".to_owned(), Value::Bool(is_queryable)));
    drop(object.insert("is_modifiable".to_owned(), Value::Bool(is_modifiable)));
    match details {
        Some(details) => drop(object.insert("other_details".to_owned(), details)),
        None => drop(object.remove("other_details")),
    }
    serde_json::to_string(&doc).map_err(|e| {
        ViewerError::Invalid(format!(
            "the edited EHR_STATUS could not be serialized: {e}"
        ))
    })
}

/// The edit card: the two capability toggles, the `other_details` editor, the
/// save button, and the two inline feedback places (the client-side complaint and
/// the CDR's verbatim diagnostic).
///
/// Always mounted with a constant structure, so the server HTML and the client
/// view match; every value comes from the long-lived [`StatusForm`], so the card
/// survives the facts section's Suspend re-runs. The save is an `Action`, and
/// its outcome toasts in [`status_section`](super::status_section); the inline
/// pane keeps the diagnostic beside the input that caused it.
///
/// Every control is DISABLED until [`seed`] has loaded a document into the
/// form: the card is mounted before the read resolves, and an edit made in it
/// then would be silently replaced by the seed.
#[expect(
    clippy::too_many_lines,
    reason = "one erased section: the edit card's two toggles + draft + validation + action wiring (rules §1)"
)]
pub(super) fn edit_form(
    ehr_id: Signal<String>,
    form: StatusForm,
    save: Action<StatusEdit, Result<String, ViewerError>>,
) -> AnyView {
    let unseeded = Signal::derive(move || form.seeded_uid.with(Option::is_none));
    let on_save = move |_| {
        let draft = form.other_details.get();
        // Client-side validation first: a malformed draft is refused inline,
        // before any round trip. The server
        // fn re-checks — it is a public endpoint.
        if let Err(message) = parse_other_details(&draft) {
            form.validation.set(Some(message));
        } else {
            form.validation.set(None);
            save.dispatch(StatusEdit {
                ehr_id: ehr_id.get(),
                version_uid: form.version_uid.get(),
                base_body: form.base_body.get(),
                is_queryable: form.queryable.get(),
                is_modifiable: form.modifiable.get(),
                other_details: draft,
            });
        }
    };
    // The client-side complaint about the `other_details` draft, in the place
    // the draft is typed.
    let validation = move || {
        match form.validation.get() {
        Some(message) => {
            view! {
                <div
                    role="alert"
                    id="status-validation"
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
    // The CDR's own diagnostic, kept beside the form it refused: the toast is
    // the notification, this is the detail worth reading line by line.
    let diagnostic = move || match save.value().get() {
        Some(Err(error)) => {
            let detail = error.to_string();
            view! {
                <div class=WELL id="status-diagnostic" role="alert">
                    <pre class="overflow-auto max-h-[40vh] whitespace-pre-wrap font-mono text-xs text-danger">
                        {detail}
                    </pre>
                </div>
            }
            .into_any()
        }
        _ => ().into_any(),
    };
    view! {
        <section class=CARD_PAD id="status-edit">
            <h2 class=CARD_TITLE>"Edit status"</h2>
            <p class="mb-3 text-xs text-ink-muted">
                "Commits a new EHR_STATUS version on top of the one loaded above (If-Match), so a concurrent change is refused rather than overwritten. Every other attribute — the subject included — is sent back exactly as the CDR served it."
            </p>
            <div class="flex flex-col gap-3">
                {toggle_row(
                    "status-queryable",
                    "is_queryable",
                    "Include this EHR in population queries (AQL).",
                    form.queryable,
                    unseeded,
                )}
                {toggle_row(
                    "status-modifiable",
                    "is_modifiable",
                    "Allow new content to be committed to this EHR.",
                    form.modifiable,
                    unseeded,
                )} <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="status-other-details">
                        "other_details (canonical JSON ITEM_STRUCTURE — leave blank to remove)"
                    </label>
                    <textarea
                        id="status-other-details"
                        class=format!("{TEXTAREA} min-h-[10rem]")
                        placeholder="{ \"_type\": \"ITEM_TREE\", \"archetype_node_id\": \"at0001\", … }"
                        disabled=true
                        prop:disabled=move || unseeded.get()
                        prop:value=move || form.other_details.get()
                        on:input:target=move |ev| form.other_details.set(ev.target().value())
                    >
                        {form.other_details.get_untracked()}
                    </textarea>
                </div> <div class="flex items-center gap-3">
                    <button
                        id="status-save"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=true
                        prop:disabled=move || unseeded.get() || save.pending().get()
                        on:click=on_save
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuSave width="14" height="14" />
                        "Save status"
                    </button>
                    <Show when=move || save.pending().get()>
                        <span class="text-sm text-ink-muted">"Saving…"</span>
                    </Show>
                </div> {validation} {diagnostic}
            </div>
        </section>
    }
    .into_any()
}

/// One boolean attribute as a labelled checkbox with a one-line explanation.
///
/// `prop:checked` + an `on:change` listener (the `checked` attribute would only
/// set the initial state, and an `onchange="…"` JS attribute is forbidden
/// outright). Inert-until-seeded is the same split: a STATIC `disabled`
/// attribute for the server HTML (inert from first paint) and `prop:disabled`
/// for the live state — the seed can land during hydration replay, before this
/// binding exists, so only a property applied at hydration enables reliably.
fn toggle_row(
    id: &'static str,
    label: &'static str,
    hint: &'static str,
    value: RwSignal<bool>,
    disabled: Signal<bool>,
) -> AnyView {
    view! {
        <div class="flex flex-col gap-0.5">
            <label class="flex items-center gap-2 text-sm font-medium text-ink" r#for=id>
                <input
                    id=id
                    type="checkbox"
                    class="accent-accent disabled:opacity-50"
                    disabled=true
                    prop:disabled=move || disabled.get()
                    prop:checked=move || value.get()
                    on:change=move |ev| value.set(event_target_checked(&ev))
                />
                {label}
            </label>
            <span class="ml-6 text-xs text-ink-muted">{hint}</span>
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::parse_other_details;

    #[test]
    fn a_blank_other_details_draft_removes_the_attribute() {
        assert_eq!(parse_other_details(""), Ok(None));
        assert_eq!(parse_other_details("   \n  "), Ok(None));
    }

    #[test]
    fn an_item_structure_object_is_accepted() {
        let draft = r#"{"_type":"ITEM_TREE","archetype_node_id":"at0001","items":[]}"#;
        let parsed = parse_other_details(draft).expect("an object draft is accepted");
        let value = parsed.expect("some value");
        assert_eq!(value["_type"], "ITEM_TREE");
    }

    #[test]
    fn a_non_object_or_malformed_draft_is_refused_before_any_round_trip() {
        // ITEM_STRUCTURE is an object; an array or scalar can never be valid.
        for draft in ["[]", "\"ITEM_TREE\"", "42", "true", "null"] {
            let message = parse_other_details(draft).expect_err("a non-object draft is refused");
            assert!(message.contains("must be a JSON object"), "{message}");
        }
        let message = parse_other_details("{\"_type\":").expect_err("malformed JSON is refused");
        assert!(message.contains("not valid JSON"), "{message}");
    }
}

#[cfg(all(test, feature = "ssr"))]
mod merge_tests {
    use super::apply_status_edits;
    use serde_json::Value;

    /// The loaded document, carrying attributes the form does not edit.
    const BASE: &str = r#"{
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "name": {"_type": "DV_TEXT", "value": "EHR status"},
        "uid": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.org::1"},
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": "demographic",
                "type": "PERSON",
                "id": {"_type": "GENERIC_ID", "value": "p-42", "scheme": "local"}
            }
        },
        "is_queryable": true,
        "is_modifiable": true,
        "other_details": {"_type": "ITEM_TREE", "items": []}
    }"#;

    #[test]
    fn the_three_edited_fields_change_and_everything_else_survives() {
        let merged = apply_status_edits(
            BASE,
            false,
            true,
            r#"{"_type":"ITEM_TREE","archetype_node_id":"at0002","items":[]}"#,
        )
        .expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&merged).expect("merged JSON");
        assert_eq!(doc["is_queryable"], Value::Bool(false));
        assert_eq!(doc["is_modifiable"], Value::Bool(true));
        assert_eq!(doc["other_details"]["archetype_node_id"], "at0002");
        // Untouched attributes travel back verbatim — the whole point of
        // merging into the served document instead of rebuilding it.
        assert_eq!(doc["_type"], "EHR_STATUS");
        assert_eq!(doc["name"]["value"], "EHR status");
        assert_eq!(
            doc["archetype_node_id"],
            "openEHR-EHR-EHR_STATUS.generic.v1"
        );
        assert_eq!(doc["uid"]["value"], "8849182c::example.org::1");
        assert_eq!(doc["subject"]["external_ref"]["id"]["value"], "p-42");
        assert_eq!(doc["subject"]["external_ref"]["namespace"], "demographic");
    }

    #[test]
    fn a_blank_draft_removes_other_details_and_keeps_the_rest() {
        let merged = apply_status_edits(BASE, true, false, "").expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&merged).expect("merged JSON");
        assert!(doc.get("other_details").is_none());
        assert_eq!(doc["is_queryable"], Value::Bool(true));
        assert_eq!(doc["is_modifiable"], Value::Bool(false));
        assert_eq!(doc["subject"]["_type"], "PARTY_SELF");
    }

    #[test]
    fn a_status_without_other_details_gains_it_when_the_draft_is_filled() {
        let base = r#"{"_type":"EHR_STATUS","is_queryable":true,"is_modifiable":true}"#;
        let merged = apply_status_edits(base, true, true, r#"{"_type":"ITEM_TREE"}"#)
            .expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&merged).expect("merged JSON");
        assert_eq!(doc["other_details"]["_type"], "ITEM_TREE");
    }

    #[test]
    fn a_non_object_base_or_draft_is_refused() {
        assert!(apply_status_edits("not json", true, true, "").is_err());
        assert!(apply_status_edits("[]", true, true, "").is_err());
        assert!(apply_status_edits(BASE, true, true, "[]").is_err());
    }
}
