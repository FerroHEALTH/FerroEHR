// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The control strip above a template's CDR-generated example composition.
//!
//! ONE kit for both template families (ADL 1.4 and ADL 2): the representation
//! the example is negotiated in, plus the two query options the Definition
//! API's example resources take — `detail_level` and `type`
//! ([`crate::example_options`]). Each control drives a signal the pane's
//! resource reads as its source, so a change refetches the example rather than
//! re-rendering the loaded one.

use leptos::prelude::*;

use crate::components::field::LABEL;
use crate::components::format_view::{FormatSelector, SEGMENTED, segment_button};
use crate::example_options::{ExampleDetail, ExampleType};
use crate::format::ReprFormat;

/// The example pane's control strip: representation, detail level, and the
/// form the example is shaped for.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn ExampleControls(
    /// The representation the example is negotiated in.
    format: RwSignal<ReprFormat>,
    /// How much of the template the CDR fills in.
    detail: RwSignal<ExampleDetail>,
    /// Whether the example is the submitted or the retrieved form.
    kind: RwSignal<ExampleType>,
) -> impl IntoView {
    let offered = vec![
        ReprFormat::CanonicalJson,
        ReprFormat::CanonicalXml,
        ReprFormat::Flat,
        ReprFormat::Structured,
    ];
    let detail_buttons = [
        ExampleDetail::Required,
        ExampleDetail::Medium,
        ExampleDetail::Complete,
    ]
    .into_iter()
    .map(|level| {
        segment_button(
            level.label(),
            Signal::derive(move || detail.get() == level),
            move || detail.set(level),
        )
    })
    .collect::<Vec<_>>();
    let type_buttons = [ExampleType::Input, ExampleType::Output]
        .into_iter()
        .map(|value| {
            segment_button(
                value.label(),
                Signal::derive(move || kind.get() == value),
                move || kind.set(value),
            )
        })
        .collect::<Vec<_>>();
    view! {
        <div class="flex flex-wrap items-center gap-x-6 gap-y-3">
            <div class="flex items-center gap-2">
                <span class=LABEL>"Format"</span>
                <FormatSelector offered=offered selected=format />
            </div>
            <div class="flex items-center gap-2">
                <span class=LABEL>"Detail level"</span>
                <div class=SEGMENTED role="group" aria-label="Detail level">
                    {detail_buttons}
                </div>
            </div>
            <div class="flex items-center gap-2">
                <span class=LABEL>"Form"</span>
                <div class=SEGMENTED role="group" aria-label="Example form">
                    {type_buttons}
                </div>
            </div>
        </div>
    }
}
