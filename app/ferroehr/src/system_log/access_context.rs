// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The request-scoped purpose of use, published by the protocol adapter and
//! read by platform-layer emitters.
//!
//! The audit middleware stamps the declared purpose onto the records IT
//! builds, but a record the service layer emits by itself — a pseudonymisation
//! boundary crossing, which no HTTP operation id describes — is built where
//! the request headers are already gone. This carries that one fact across, by
//! the same means [`crate::service::committer`] carries the authenticated
//! committer: a task-local the adapter publishes for the request's scope.
//!
//! The value is the code the deployment's own `[audit] purpose_codes`
//! vocabulary accepted, never a caller's free text, because the adapter has
//! already filtered it ("an unagreed string in the trail reads at review time
//! as though a purpose was established when none was").
//!
//! **No openEHR spec governs this — our own design/extension.** The purpose of
//! use is required of an access record by NEN 7513 and EHDS Art. 9, not by
//! openEHR.

use tokio::task_local;

task_local! {
    static DECLARED_PURPOSE: Option<String>;
    static REQUEST_ROLES: Vec<String>;
}

/// The roles the authenticated caller holds for the current request (#3239).
///
/// Published by the authentication layer beside the committer identity; empty
/// outside a request scope or for an unauthenticated caller, and the record
/// says so rather than guessing.
#[must_use]
pub fn current_roles() -> Vec<String> {
    REQUEST_ROLES.try_with(Clone::clone).unwrap_or_default()
}

/// Run `fut` with `roles` published as the request's caller roles.
pub async fn with_roles<F>(roles: Vec<String>, fut: F) -> F::Output
where
    F: Future,
{
    REQUEST_ROLES.scope(roles, fut).await
}

/// The purpose of use declared for the current request, if any.
///
/// Outside a request scope — a background task, a test, or a deployment that
/// collects no purpose — this is `None`, and the record says truthfully that
/// no purpose was declared rather than guessing one.
#[must_use]
pub fn current_purpose() -> Option<String> {
    DECLARED_PURPOSE.try_with(Clone::clone).ok().flatten()
}

/// Run `fut` with `purpose` published as the request's declared purpose of use.
pub async fn with_purpose<F>(purpose: Option<String>, fut: F) -> F::Output
where
    F: Future,
{
    DECLARED_PURPOSE.scope(purpose, fut).await
}
