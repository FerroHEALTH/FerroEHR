# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the sign-in group: the reference catalog and the
# one every lookup falls back to. Each message is one `SignInMsg` of the
# crate's `locale::signin` module; a catalog of another language holds the
# same messages in a directory named by its BCP 47 tag beside this one.

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

sign-in-title = Sign in
sign-in-at = Sign in at { $host }
sign-in-redirecting = Taking you to { $host }…
sign-in-identity-provider = the identity provider
sign-in-basic-submit = Sign in with a CDR account
sign-in-caption = The viewer of the FerroEHR clinical data repository.
sign-in-expired = Your session ended. Sign in again to return to the page you were on.
sign-in-signed-out = You are signed out.
sign-in-refused-provider = The identity provider did not sign you in.
sign-in-refused = Not signed in: the CDR refused that username or password.
sign-in-failed = Not signed in: { $error }
sign-in-cdr-unreachable = The CDR did not answer. You can still try to sign in.
