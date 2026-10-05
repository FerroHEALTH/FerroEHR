// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The **History** tab of a demographic object — the versioned family, shared
//! by parties and relationships.
//!
//! Four surfaces over one versioned object, in the order an operator reads
//! them:
//!
//! 1. the **versioned-object card** — the container's own facts plus the
//!    selected VERSION's envelope facts (`GET /demographic/{family}/{uid}` +
//!    `…/version[/{version_uid}]`), the contribution linked through to its
//!    own viewer;
//! 2. the **revision history** table (`…/revision_history`), newest-first,
//!    each row opening that version;
//! 3. the **at-time lookup** (`…/version?version_at_time=…`), which resolves an
//!    instant to a version and opens it;
//! 4. the **pinned document** — that version of the object itself, read by its
//!    `OBJECT_VERSION_ID` through the RESOURCE route.
//!
//! Three of the four are the shared
//! [`version_history`](crate::components::version_history) kit, parameterized by
//! this family's copy and DOM hooks; only the facts card is local.
//!
//! One reader per claim: this tab never reads the current object. Within the
//! tab, document CONTENT comes from the resource at that version, commit history
//! from the revision history, and the VERSION's envelope facts from the direct
//! VERSION read. Every resource is created ONCE in `history_section` — never
//! inside a `Suspend` — and gated on the tab being active.

use leptos::prelude::*;

use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::version_history::{
    DocumentResource, HistoryResource, VersionHistoryLabels, at_time_lookup_section,
    pinned_document_section, revision_history_section, versioned_facts_section,
};
use crate::components::wire::VersionedObjectFacts;
use crate::error::ViewerError;
use crate::pages::demographics::party::fact_row;
use crate::pages::demographics::{
    DemographicResource, contribution_href, fetch_demographic_revision_history,
    fetch_demographic_version_document, fetch_versioned_object,
    resolve_demographic_version_at_time,
};

/// The container + VERSION-envelope resource.
type VersionedResource = Resource<Result<Option<VersionedObjectFacts>, ViewerError>>;

/// This family's copy and DOM hooks for the shared History-tab kit.
const LABELS: VersionHistoryLabels = VersionHistoryLabels {
    row_hook: "data-demographic-version",
    empty_message: "No versions",
    empty_hint: "Every demographic object is created with a first version; if none is listed, the CDR reported no revision history for this id.",
    history_lead: "Newest first. Open a version to read the document exactly as it stood at that commit.",
    at_time_title: "Version at a point in time",
    at_time_field_id: "demographic-at-time",
    at_time_button_id: "demographic-at-time-go",
    at_time_absent: "No version existed at that time.",
    document_id: "demographic-version-document",
};

/// The History tab: the versioned-object card, the revision-history table, the
/// at-time lookup, and the pinned version's document.
///
/// `tab` is the `?tab=` value that activates this section on its screen, so the
/// same code serves the party detail and the relationship detail.
pub(super) fn history_section(
    object: DemographicResource,
    uid: Signal<String>,
    selected: Memo<String>,
    tab: &'static str,
) -> AnyView {
    // Which version the tab is showing. Empty = none pinned yet: the card then
    // reports the LATEST version (the version read's own default) and the
    // document pane invites the operator to open one from the table.
    let pinned = RwSignal::new(String::new());
    let at_time_input = RwSignal::new(String::new());
    let active = Memo::new(move |_| selected.get() == tab);
    let family = object.family().segment();

    // The at-time lookup is an Action, not a resource: it is a one-shot the
    // operator triggers, and its answer (an OBJECT_VERSION_ID) feeds the shared
    // selection rather than being rendered on its own. A failure leaves the
    // selection untouched — the lookup's own note renders it.
    let at_time: Action<(String, String), Result<String, ViewerError>> =
        Action::new(move |(uid, at): &(String, String)| {
            let (uid, at) = (uid.clone(), at.clone());
            async move {
                let resolved =
                    resolve_demographic_version_at_time(family.to_owned(), uid, at).await;
                // NOTE: the write rides the dispatched event's own continuation,
                // so it is an event write rather than an Effect write.
                if let Ok(version) = &resolved
                    && !version.is_empty()
                {
                    pinned.set(version.clone());
                }
                resolved
            }
        });

    let history: HistoryResource = Resource::new(
        move || active.get().then(|| uid.get()),
        move |target| async move {
            match target {
                Some(id) => fetch_demographic_revision_history(family.to_owned(), id).await,
                None => Ok(Vec::new()),
            }
        },
    );
    let versioned: VersionedResource = Resource::new(
        move || active.get().then(|| (uid.get(), pinned.get())),
        move |target| async move {
            match target {
                Some((id, version)) => fetch_versioned_object(family.to_owned(), id, version)
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
    );
    let segment = object.segment();
    let document: DocumentResource = Resource::new(
        move || {
            let version = pinned.get();
            (active.get() && !version.is_empty()).then_some(version)
        },
        move |target| async move {
            match target {
                Some(version) => fetch_demographic_version_document(segment.to_owned(), version)
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
    );

    let card = versioned_facts_section(versioned, versioned_card);
    let table = revision_history_section(history, pinned, LABELS);
    let lookup = at_time_lookup_section(uid, at_time, at_time_input, LABELS);
    let pane = pinned_document_section(document, pinned, LABELS);
    view! { <div class="flex flex-col gap-4">{card} {table} {lookup} {pane}</div> }.into_any()
}

/// Render the container + selected-VERSION facts as a card, with the
/// contribution linked to its own viewer.
fn versioned_card(facts: &VersionedObjectFacts) -> AnyView {
    let signature = if facts.signed {
        "present".to_owned()
    } else {
        "none".to_owned()
    };
    let contribution = (!facts.contribution_uid.is_empty()).then(|| {
        let href = contribution_href(&facts.contribution_uid);
        let shown = facts.contribution_uid.clone();
        view! {
            <div>
                <span class="font-medium text-ink-muted mr-1">"contribution:"</span>
                <leptos_router::components::A
                    href=href
                    attr:class="font-mono break-all text-accent hover:underline"
                    attr:data-demographic-fact="contribution"
                >
                    {shown}
                </leptos_router::components::A>
            </div>
        }
        .into_any()
    });
    view! {
        <section class=CARD_PAD id="demographic-versioned">
            <h2 class=CARD_TITLE>"Versioned object"</h2>
            <div class="grid grid-cols-1 sm:grid-cols-2 items-start gap-x-4 gap-y-2 text-sm">
                {fact_row("versioned object", "object-uid", facts.object_uid.clone())}
                {fact_row("created", "created", facts.time_created.clone())}
                {fact_row("version", "version-id", facts.version_id.clone())}
                {fact_row("lifecycle", "lifecycle", facts.lifecycle_state.clone())}
                {fact_row("preceding version", "preceding", facts.preceding_version_uid.clone())}
                {contribution} {fact_row("signature", "signature", signature)}
            </div>
        </section>
    }
    .into_any()
}
