// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The backstop that turns "this session is over" into the sign-in page, and
//! the `/login` URL the signed-out transition lands on.
//!
//! Every page reaches the CDR through the viewer's own reads, and every one
//! of them refuses a dead session the same two ways:
//! [`ViewerError::Unauthenticated`] (the viewer's own sealed cookie is gone
//! or idle-expired) and [`ViewerError::CdrUnauthorized`] (the CDR answered
//! `401` to the credential the session carries). Each is noted in the
//! request's scope as it is met ([`note_if_over`]), and the page handler then
//! answers with the sign-in page ([`signed_out_url`]) in place of the page,
//! so a revoked or expired session cannot leave a shell over a dead
//! credential, whatever the reader was doing at the time (FerroUI
//! `docs/consoles.md`, "The session's end", part 4).
//!
//! NOTE: no openEHR spec governs the viewer session — our own design.

use crate::error::ViewerError;

/// The `/login` URL the signed-out transition lands on.
///
/// `expired=1` is what makes the login screen say the session ended instead of
/// presenting a bare sign-in card, and `next` carries the screen the user was
/// on so signing in again returns them to it. The destination is percent-encoded
/// with the `urlencoding` crate (the owner's never-hand-roll-a-codec rule);
/// `search` is the raw query string without its `?`.
#[must_use]
pub fn signed_out_url(path: &str, search: &str) -> String {
    let next = if search.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{search}")
    };
    format!("/login?expired=1&next={}", urlencoding::encode(&next))
}

/// Whether `error` says the session is over: the viewer's own session is
/// gone, or the CDR refused the credential it carries.
#[must_use]
pub const fn ends_session(error: &ViewerError) -> bool {
    matches!(
        error,
        ViewerError::Unauthenticated | ViewerError::CdrUnauthorized(_)
    )
}

/// Notes in the request's scope that its session is over when `error` says
/// so, so the page answers with the sign-in page.
pub fn note_if_over(error: &ViewerError) {
    if ends_session(error) {
        crate::scope::with(|scope| scope.answer(|answer| answer.session_over = true));
    }
}

#[cfg(test)]
mod tests {
    use super::{ends_session, signed_out_url};
    use crate::error::ViewerError;

    #[test]
    fn the_signed_out_url_carries_the_expiry_flag_and_the_encoded_destination() {
        assert_eq!(
            signed_out_url("/ehrs/abc", "tab=status&page=2"),
            "/login?expired=1&next=%2Fehrs%2Fabc%3Ftab%3Dstatus%26page%3D2"
        );
        assert_eq!(signed_out_url("/", ""), "/login?expired=1&next=%2F");
    }

    #[test]
    fn only_the_two_session_refusals_end_the_session() {
        assert!(!ends_session(&ViewerError::CdrUnreachable(
            "down".to_owned()
        )));
        assert!(!ends_session(&ViewerError::Forbidden("nope".to_owned())));
        assert!(!ends_session(&ViewerError::Cdr {
            status: 500,
            message: "boom".to_owned()
        }));
        assert!(!ends_session(&ViewerError::Invalid("bad".to_owned())));
        assert!(ends_session(&ViewerError::Unauthenticated));
        assert!(ends_session(&ViewerError::CdrUnauthorized(
            "gone".to_owned()
        )));
    }
}
