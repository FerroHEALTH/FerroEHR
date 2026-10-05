// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail **Status history** tab: the `VERSIONED_EHR_STATUS` family.
//!
//! Four surfaces over the one versioned object, in the order an operator reads
//! them:
//!
//! 1. the **versioned-object card** — the container's own facts plus the
//!    selected VERSION's envelope facts (`GET …/versioned_ehr_status` +
//!    `GET …/versioned_ehr_status/version[/{uid}]`);
//! 2. the **revision history** table (`GET …/versioned_ehr_status/revision_history`),
//!    newest-first, each row opening that version;
//! 3. the **at-time lookup** (`GET …/versioned_ehr_status/version?version_at_time=…`),
//!    which resolves an instant to a version and opens it;
//! 4. the **pinned document** — that version's `EHR_STATUS`, read by its
//!    `OBJECT_VERSION_ID` (`GET /ehr/{ehr_id}/ehr_status/{version_uid}`).
//!
//! Three of the four are the shared
//! [`version_history`](crate::components::version_history) kit, parameterized by
//! this family's copy and DOM hooks; only the facts card is local.
//!
//! One reader per claim: this tab never reads `GET /ehr/{ehr_id}/ehr_status` —
//! the current-status document belongs to the Status tab. Within the tab,
//! document CONTENT comes from the `EHR_STATUS` resource at that version, commit
//! history from the revision history, and the VERSION's envelope facts from the
//! direct VERSION read. Every resource is created ONCE in
//! `status_history_section` — never inside a `Suspend` — and gated on the tab
//! being active.

use leptos::prelude::*;

use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::version_history::{
    DocumentResource, HistoryResource, VersionHistoryLabels, at_time_lookup_section,
    pinned_document_section, revision_history_section, versioned_facts_section,
};
use crate::components::wire::VersionedObjectFacts;
use crate::error::ViewerError;
use crate::pages::ehr_detail::status::{
    fetch_ehr_status_version, fetch_status_revision_history, fetch_status_version_at_time,
    fetch_versioned_status,
};

/// The versioned-object + VERSION-envelope resource.
type VersionedResource = Resource<Result<Option<VersionedObjectFacts>, ViewerError>>;

/// This family's copy and DOM hooks for the shared History-tab kit.
const LABELS: VersionHistoryLabels = VersionHistoryLabels {
    row_hook: "data-status-version",
    empty_message: "No EHR_STATUS versions",
    empty_hint: "Every EHR is created with a first EHR_STATUS version; if none is listed, the CDR did not report a revision history for this EHR.",
    history_lead: "Newest first. Open a version to read the EHR_STATUS document exactly as it stood at that commit.",
    at_time_title: "EHR_STATUS at a point in time",
    at_time_field_id: "status-at-time",
    at_time_button_id: "status-at-time-go",
    at_time_absent: "No EHR_STATUS version existed at that time.",
    document_id: "status-version-document",
};

/// Status history tab: the versioned-object card, the revision-history table,
/// the at-time lookup, and the pinned version's document.
pub(in crate::pages::ehr_detail) fn status_history_section(
    ehr_id: Signal<String>,
    selected: Memo<String>,
) -> AnyView {
    // Which version the tab is showing. Empty = none pinned yet: the card then
    // reports the LATEST version (the versioned-object read's default) and the
    // document pane invites the operator to open one from the table.
    let pinned = RwSignal::new(String::new());
    let at_time_input = RwSignal::new(String::new());
    let active = Memo::new(move |_| selected.get() == "status-history");

    // The at-time lookup is an Action, not a resource: it is a one-shot the
    // operator triggers, and its answer (an OBJECT_VERSION_ID) feeds the shared
    // selection rather than being rendered on its own. A failure leaves the
    // selection untouched — the lookup's own note renders it.
    let at_time = Action::new(move |(ehr_id, at_time): &(String, String)| {
        let (ehr_id, at_time) = (ehr_id.clone(), at_time.clone());
        async move {
            let resolved = fetch_status_version_at_time(ehr_id, at_time).await;
            // NOTE: the write rides the dispatched event's own continuation, so
            // it is an event write rather than an Effect write.
            if let Ok(version) = &resolved
                && !version.is_empty()
            {
                pinned.set(version.clone());
            }
            resolved
        }
    });

    let history: HistoryResource = Resource::new(
        move || active.get().then(|| ehr_id.get()),
        |target| async move {
            match target {
                Some(id) => fetch_status_revision_history(id).await,
                None => Ok(Vec::new()),
            }
        },
    );
    let versioned: VersionedResource = Resource::new(
        move || active.get().then(|| (ehr_id.get(), pinned.get())),
        |target| async move {
            match target {
                Some((id, version)) => fetch_versioned_status(id, version).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let document: DocumentResource = Resource::new(
        move || {
            let version = pinned.get();
            (active.get() && !version.is_empty()).then(|| (ehr_id.get(), version))
        },
        |target| async move {
            match target {
                Some((id, version)) => fetch_ehr_status_version(id, version).await.map(Some),
                None => Ok(None),
            }
        },
    );

    let card = versioned_facts_section(versioned, versioned_card);
    let table = revision_history_section(history, pinned, LABELS);
    let lookup = at_time_lookup_section(ehr_id, at_time, at_time_input, LABELS);
    let pane = pinned_document_section(document, pinned, LABELS);
    view! { <div class="flex flex-col gap-4">{card} {table} {lookup} {pane}</div> }.into_any()
}

/// Render the container + selected-VERSION facts as a card.
fn versioned_card(details: &VersionedObjectFacts) -> AnyView {
    let signature = if details.signed {
        "present".to_owned()
    } else {
        "none".to_owned()
    };
    view! {
        <section class=CARD_PAD id="versioned-ehr-status">
            <h2 class=CARD_TITLE>"Versioned EHR_STATUS"</h2>
            <div class="grid grid-cols-1 sm:grid-cols-2 items-start gap-x-4 gap-y-2 text-sm">
                {fact_row("versioned object", "object-uid", details.object_uid.clone())}
                {fact_row("owner EHR", "owner", details.owner_id.clone())}
                {fact_row("created", "created", details.time_created.clone())}
                {fact_row("version", "version", details.version_id.clone())}
                {fact_row("lifecycle", "lifecycle", details.lifecycle_state.clone())}
                {fact_row("preceding version", "preceding", details.preceding_version_uid.clone())}
                {fact_row("contribution", "contribution", details.contribution_uid.clone())}
                {fact_row("signature", "signature", signature)}
            </div>
        </section>
    }
    .into_any()
}

/// One label/value line of the versioned-object card, carrying this family's
/// `data-versioned-fact` hook.
fn fact_row(label: &'static str, hook: &'static str, value: String) -> AnyView {
    crate::components::facts::fact_row(label, "data-versioned-fact", hook, value)
}
