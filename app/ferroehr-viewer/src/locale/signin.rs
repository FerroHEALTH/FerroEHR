// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the sign-in group, in the catalogs under `locales/signin/`.
//!
//! Only code of the sign-in group looks them up, so their catalog travels
//! with that code: the browser loads it with the group's chunks, never with
//! the entry WebAssembly (`crate::locale`).

crate::locale::messages! {
    /// A string the sign-in group draws, by its message in its catalog.
    SignInMsg in "./locales/signin" {
        /// `Sign in at { $host }`
        At = "sign-in-at",
        /// `Sign in with a CDR account`
        BasicSubmit = "sign-in-basic-submit",
        /// `The viewer of the FerroEHR clinical data repository.`
        Caption = "sign-in-caption",
        /// `The CDR did not answer. You can still try to sign in.`
        CdrUnreachable = "sign-in-cdr-unreachable",
        /// `Your session ended. Sign in again to return to the page you were on.`
        Expired = "sign-in-expired",
        /// `Not signed in: { $error }`
        Failed = "sign-in-failed",
        /// `the identity provider`
        IdentityProvider = "sign-in-identity-provider",
        /// `Taking you to { $host }…`
        Redirecting = "sign-in-redirecting",
        /// `Not signed in: the CDR refused that username or password.`
        Refused = "sign-in-refused",
        /// `The identity provider did not sign you in.`
        RefusedProvider = "sign-in-refused-provider",
        /// `You are signed out.`
        SignedOut = "sign-in-signed-out",
        /// `Sign in`
        Title = "sign-in-title",
    }
}
