// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Toast feedback helpers over the FerroUI toast region: every mutation
//! (upload, save, delete, commit, create) reports its outcome as a transient
//! toast instead of scattered inline text.
//!
//! The shell provides and draws the region
//! (`ferrohealth_ui::widget::toast::{provide_toasts, toast_region}`); screens
//! take its handle with [`toaster`]. A toast says what happened and to which
//! object, never what was in it.

use ferrohealth_ui::tone::Tone;
use ferrohealth_ui::widget::toast::Toasts;
use leptos::prelude::*;

/// The toast region the shell provides.
///
/// # Panics
///
/// Panics when called outside the shell, which provides the region for every
/// guarded screen (the structural guarantee `expect_context` documents).
#[must_use]
pub fn toaster() -> Toasts {
    expect_context::<Toasts>()
}

/// One toast's words: the short outcome, then the sentence that names the
/// object and the next action.
fn words(title: &str, body: &str) -> String {
    if body.is_empty() {
        title.to_owned()
    } else {
        format!("{title}: {body}")
    }
}

/// Dispatch a success toast.
pub fn toast_success(toaster: Toasts, title: &str, body: &str) {
    toaster.push(Tone::Ok, words(title, body));
}

/// Dispatch an error toast.
pub fn toast_error(toaster: Toasts, title: &str, body: &str) {
    toaster.push(Tone::Bad, words(title, body));
}

/// Report one mutation's outcome as the viewer's two toasts.
///
/// `outcome` is an [`Action`]'s value carrying the name the mutation was
/// dispatched under beside the CDR's answer, so both toasts can name the exact
/// object (the action's value IS the mutation report). `titles` is the success
/// title and the failure title; the two closures build each toast's body from
/// that name.
///
/// Dispatching a toast is a side effect on the outside world, so an `Effect` is
/// its correct home: it never runs on the server pass.
pub fn toast_outcome<T, S, F>(
    toaster: Toasts,
    outcome: Signal<Option<(String, Result<T, crate::error::ViewerError>)>>,
    titles: (&'static str, &'static str),
    success_body: S,
    failure_body: F,
) where
    T: Clone + Send + Sync + 'static,
    S: Fn(&str, &T) -> String + Send + Sync + 'static,
    F: Fn(&str, &crate::error::ViewerError) -> String + Send + Sync + 'static,
{
    Effect::new(move |_| match outcome.get() {
        Some((name, Ok(value))) => toast_success(toaster, titles.0, &success_body(&name, &value)),
        Some((name, Err(error))) => toast_error(toaster, titles.1, &failure_body(&name, &error)),
        None => {}
    });
}

#[cfg(test)]
mod tests {
    use super::words;

    #[test]
    fn a_toast_reads_the_outcome_then_the_sentence() {
        assert_eq!(
            words("Upload failed", "The CDR refused it."),
            "Upload failed: The CDR refused it."
        );
        assert_eq!(words("Saved", ""), "Saved");
    }
}
