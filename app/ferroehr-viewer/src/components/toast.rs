// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Toast feedback helpers over `thaw`'s toaster: every mutation (upload, save,
//! delete, commit, create) reports its outcome as a transient toast instead of
//! scattered inline text.
//!
//! The shell mounts the `thaw::ToasterProvider`; screens call these with
//! `thaw::ToasterInjection::expect_context()`.

use leptos::prelude::*;

/// Dispatch a success toast.
pub fn toast_success(toaster: thaw::ToasterInjection, title: &str, body: &str) {
    dispatch(toaster, thaw::ToastIntent::Success, title, body);
}

/// Dispatch an error toast.
pub fn toast_error(toaster: thaw::ToasterInjection, title: &str, body: &str) {
    dispatch(toaster, thaw::ToastIntent::Error, title, body);
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
/// its correct home: it never writes a signal, and it never runs on the server
/// pass.
pub fn toast_outcome<T, S, F>(
    toaster: thaw::ToasterInjection,
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

fn dispatch(toaster: thaw::ToasterInjection, intent: thaw::ToastIntent, title: &str, body: &str) {
    let title = title.to_owned();
    let body = body.to_owned();
    toaster.dispatch_toast(
        move || {
            view! {
                <thaw::Toast>
                    <thaw::ToastTitle>{title}</thaw::ToastTitle>
                    <thaw::ToastBody>{body}</thaw::ToastBody>
                </thaw::Toast>
            }
        },
        thaw::ToastOptions::default().with_intent(intent),
    );
}
