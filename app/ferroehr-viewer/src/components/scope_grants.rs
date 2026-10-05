// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scope-grant kit: ONE rendering of a SMART scope string, used by the
//! session drawer for the scopes this session actually carries and by the
//! previewer field for any scope a reader pastes in.
//!
//! The reading itself is [`crate::scopes`] over the shared master08 grammar
//! (`openehr_sdt::smart_scopes`) — a pure function, so this module is a
//! thin view: no server round-trip for a parse, and identical markup on the
//! server pass and at hydration.

use leptos::prelude::*;

use crate::components::field::INPUT;
use crate::scopes::{CAPABILITY_NOTE, Grant, GrantDetail, ResourceGrant, grants};

/// One grant card.
const CARD: &str = "flex flex-col gap-1.5 rounded-control border border-edge bg-sunken p-2.5";

/// The scope string itself, always shown verbatim above its reading.
const RAW: &str = "font-mono text-xs break-all text-ink";

/// An accent chip (compartment, context/identity label).
const CHIP: &str = "rounded-full bg-accent-subtle px-2 py-0.5 text-[10px] font-medium tracking-wide text-accent-ink";

/// A permission chip.
const CHIP_PERMISSION: &str =
    "rounded-full border border-edge-strong px-2 py-0.5 text-[10px] font-medium text-ink";

/// The wildcard-breadth chip (master08's cautionary NOTE) and the
/// not-recognised chip: both warn without claiming an error.
const CHIP_WARN: &str =
    "rounded-full bg-warn-subtle px-2 py-0.5 text-[10px] font-medium tracking-wide text-warn";

/// Explanatory copy under a card.
const NOTE: &str = "text-[11px] leading-snug text-ink-muted";

/// The capability-vs-authorization sentence the drawer states once: scopes
/// narrow, the CDR enforces (master08 §Scopes).
#[must_use]
pub fn capability_note() -> AnyView {
    view! {
        <p class="rounded-control border border-edge bg-raised px-3 py-2 text-[11px] leading-snug text-ink-muted">
            {CAPABILITY_NOTE}
        </p>
    }
    .into_any()
}

/// One label/value fact row (the drawer's identity block).
#[must_use]
pub fn fact_row(label: &'static str, value: String) -> AnyView {
    view! {
        <div class="grid grid-cols-[7rem_1fr] gap-x-3 text-sm">
            <span class="text-xs text-ink-muted">{label}</span>
            <span class="break-all text-ink">{value}</span>
        </div>
    }
    .into_any()
}

/// A list of scope strings rendered as their grants.
///
/// A plain collected `Vec` rather than `<For>`: this is a derived projection
/// replaced wholesale whenever the input changes, and a scope string is not a
/// stable unique identity (a claim may legitimately repeat one), so there is no
/// data-derived key to give `<For>` — and an index key is forbidden — the same
/// choice the document pane makes for its token stream.
#[must_use]
pub fn grant_cards(rendered: Vec<Grant>) -> AnyView {
    let cards = rendered.into_iter().map(grant_card).collect::<Vec<_>>();
    view! { <ul class="flex flex-col gap-2">{cards}</ul> }.into_any()
}

/// One scope as a card: the string verbatim, then its reading.
fn grant_card(grant: Grant) -> AnyView {
    let Grant { raw, detail } = grant;
    let (kind, body) = match detail {
        GrantDetail::Resource(resource) => ("resource", resource_body(resource)),
        GrantDetail::Context { label, note } => ("context", labelled_body(&label, note)),
        GrantDetail::Identity { label, note } => ("identity", labelled_body(&label, note)),
        GrantDetail::Unrecognized { expected } => ("unrecognized", unrecognized_body(expected)),
    };
    view! {
        // Stable hook for the E2E journeys: one attribute carrying the reading.
        <li class=CARD data-scope-grant=kind>
            <code class=RAW>{raw}</code>
            {body}
        </li>
    }
    .into_any()
}

/// A resource scope: compartment + family + pattern, the permission chips, and
/// the two sentences that say what it narrows to.
fn resource_body(resource: ResourceGrant) -> AnyView {
    let ResourceGrant {
        compartment,
        compartment_note,
        family,
        pattern,
        pattern_note,
        permissions,
        broad,
    } = resource;
    let chips = permissions
        .into_iter()
        .map(|permission| view! { <span class=CHIP_PERMISSION>{permission}</span> })
        .collect::<Vec<_>>();
    let broad_chip = broad.then(|| view! { <span class=CHIP_WARN>"broad access"</span> });
    view! {
        <div class="flex flex-col gap-1.5">
            <div class="flex flex-wrap items-center gap-1.5">
                <span class=CHIP>{compartment}</span>
                <span class="text-xs font-medium text-ink">{family}</span>
                <span class="font-mono text-[11px] break-all text-ink-muted">{pattern}</span>
                {broad_chip}
            </div>
            <div class="flex flex-wrap items-center gap-1">
                <span class="text-[11px] text-ink-faint">"may"</span>
                {chips}
            </div>
            <p class=NOTE>{compartment_note} " " {pattern_note}</p>
        </div>
    }
    .into_any()
}

/// A launch-context or identity scope: its label and what it means.
fn labelled_body(label: &str, note: &'static str) -> AnyView {
    let label = label.to_owned();
    view! {
        <div class="flex flex-col gap-1.5">
            <div class="flex flex-wrap items-center gap-1.5">
                <span class=CHIP>{label}</span>
            </div>
            <p class=NOTE>{note}</p>
        </div>
    }
    .into_any()
}

/// A scope the grammar does not recognise: kept verbatim and inert, with the
/// master08 expectation when the string looks like a resource scope.
fn unrecognized_body(expected: Option<String>) -> AnyView {
    let explanation = expected.unwrap_or_else(|| {
        "Not a scope openEHR defines. It is carried alongside the openEHR scopes and grants nothing here.".to_owned()
    });
    view! {
        <div class="flex flex-col gap-1.5">
            <div class="flex flex-wrap items-center gap-1.5">
                <span class=CHIP_WARN>"not recognised"</span>
            </div>
            <p class=NOTE>{explanation}</p>
        </div>
    }
    .into_any()
}

/// The free-input previewer: type or paste any scope string (or a whole
/// space-delimited claim) and read the grants it would describe.
///
/// State is a local signal, deliberately not the URL: this is an ephemeral
/// scratchpad inside an overlay drawer, not a listing filter — a `GET` form
/// navigation would close the drawer it lives in (the URL-state rule governs
/// filter/search/pagination). The parse is client-side, so the field is inert
/// until hydration; the session's own scopes above it are server-rendered and
/// need no `WASM`.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn ScopePreviewer() -> impl IntoView {
    let input = RwSignal::new(String::new());
    // A derived memo, never an effect writing a signal.
    let previewed = Memo::new(move |_| input.with(|text| grants(text.as_str())));
    let results = move || {
        if previewed.with(Vec::is_empty) {
            ().into_any()
        } else {
            grant_cards(previewed.get())
        }
    };
    view! {
        <section class="flex flex-col gap-2">
            <label class="text-xs font-medium text-ink" for="scope-previewer-input">
                "Preview a scope"
            </label>
            <input
                id="scope-previewer-input"
                type="text"
                class=format!("{INPUT} w-full font-mono text-xs")
                placeholder="patient/composition-*.rs user/template-MyHospital::Template.v0.crud"
                autocomplete="off"
                spellcheck="false"
                prop:value=move || input.get()
                on:input:target=move |ev| input.set(ev.target().value())
            />
            <p class=NOTE>
                "One scope or a whole space-separated claim. The form is <compartment>/<resource>.<permission> — nothing is sent to the CDR, and nothing here changes what you may do."
            </p>
            <div id="scope-preview-results">{results}</div>
        </section>
    }
}
