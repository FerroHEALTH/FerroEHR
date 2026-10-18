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
use crate::components::format_view::{FormatSelector, choice_strip};
use crate::example_options::{ExampleDetail, ExampleType};
use crate::format::ReprFormat;

/// The detail levels, each its key and its words.
const DETAILS: &[(&str, &str)] = &[
    ("required", "Required"),
    ("medium", "Medium"),
    ("complete", "Complete"),
];

/// The example forms, each its key and its words.
const FORMS: &[(&str, &str)] = &[("input", "Input"), ("output", "Output")];

/// The key a detail level carries in [`DETAILS`].
fn detail_key(detail: ExampleDetail) -> &'static str {
    match detail {
        ExampleDetail::Required => "required",
        ExampleDetail::Medium => "medium",
        ExampleDetail::Complete => "complete",
    }
}

/// The detail level a key of [`DETAILS`] names; anything else is the default.
fn detail_of(key: &str) -> ExampleDetail {
    match key {
        "medium" => ExampleDetail::Medium,
        "complete" => ExampleDetail::Complete,
        _ => ExampleDetail::Required,
    }
}

/// The key an example form carries in [`FORMS`].
fn form_key(kind: ExampleType) -> &'static str {
    match kind {
        ExampleType::Input => "input",
        ExampleType::Output => "output",
    }
}

/// The example form a key of [`FORMS`] names; anything else is the default.
fn form_of(key: &str) -> ExampleType {
    if key == "output" {
        ExampleType::Output
    } else {
        ExampleType::Input
    }
}

/// The example pane's control strip: representation, detail level, and the
/// form the example is shaped for.
#[component]
pub fn ExampleControls(
    /// The representation the example is negotiated in.
    format: RwSignal<ReprFormat>,
    /// How much of the template the CDR fills in.
    detail: RwSignal<ExampleDetail>,
    /// Whether the example is the submitted or the retrieved form.
    kind: RwSignal<ExampleType>,
) -> impl IntoView {
    view! {
        <div class="flex flex-wrap items-center gap-x-6 gap-y-3">
            <div class="flex items-center gap-2">
                <span class=LABEL>"Format"</span>
                <FormatSelector selected=format />
            </div>
            <div class="flex items-center gap-2">
                <span class=LABEL>"Detail level"</span>
                {choice_strip("Detail level", DETAILS, detail, detail_key, detail_of)}
            </div>
            <div class="flex items-center gap-2">
                <span class=LABEL>"Form"</span>
                {choice_strip("Example form", FORMS, kind, form_key, form_of)}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{DETAILS, FORMS, detail_key, detail_of, form_key, form_of};

    #[test]
    fn every_choice_round_trips_through_its_key_and_shows_its_label() {
        for (key, words) in DETAILS {
            assert_eq!(detail_key(detail_of(key)), *key);
            assert_eq!(detail_of(key).label(), *words);
        }
        for (key, words) in FORMS {
            assert_eq!(form_key(form_of(key)), *key);
            assert_eq!(form_of(key).label(), *words);
        }
    }
}
