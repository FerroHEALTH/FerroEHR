// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Mutation feedback: the viewer's one error-feedback rule and the
//! actionable copy behind it.
//!
//! The rule: **a mutation toasts on success AND on failure**, a detailed
//! inline `MessageBar` may stay beside the failure toast where the diagnostic
//! is worth reading, and **pure reads render inline errors only**. A screen
//! that quietly leaves a failed write's diagnostic below the fold looks like
//! nothing happened.
//!
//! No openEHR spec governs the viewer's feedback — our own design / product
//! extension. The copy shape follows
//! [`delete_failure_copy`](crate::admin::delete_failure_copy) (the
//! destructive-operation sibling): name the object, name what went wrong
//! (carrying the CDR's own diagnostic verbatim), name the next action.

use crate::error::ViewerError;

/// Actionable copy for a failed write: names `object`, what went wrong, and
/// the next action.
///
/// `object` is a noun phrase for the thing being written ("the operational
/// template", "the new EHR", "the stored query"), so every arm reads as one
/// sentence about that object.
///
/// The CDR's own diagnostic travels verbatim — a `422` body naming the
/// offending path is exactly what the user needs.
#[must_use]
pub fn write_failure_copy(object: &str, error: &ViewerError) -> String {
    match error {
        ViewerError::Invalid(message) => {
            format!("{object} was not sent: {message}. Correct the input and retry.")
        }
        // 412 and 409 are different failures on the wire, so they get
        // different next actions: `412` is "one or more conditions given in
        // the request header fields evaluated to false" (the stale `If-Match`
        // version), `409` is "the request could not be processed because it
        // might generate a duplicate or a conflict" — ITS-REST
        // `specifications/docs/overview/Requests_and_responses.md`
        // §status codes.
        ViewerError::Cdr { status, message } => match error.status_code() {
            Some(http::StatusCode::BAD_REQUEST | http::StatusCode::UNPROCESSABLE_ENTITY) => {
                format!("The CDR rejected {object}: {message}. Fix the reported problem and retry.")
            }
            Some(http::StatusCode::PRECONDITION_FAILED) => format!(
                "{object} changed in the CDR since this screen loaded ({message}). Reload the \
                 latest version, then reapply your change."
            ),
            Some(http::StatusCode::CONFLICT) => format!(
                "The CDR already holds a conflicting object, so {object} was not created \
                 ({message}). Open the existing one, or change the identifier and retry."
            ),
            Some(http::StatusCode::NOT_FOUND) => format!(
                "{object} is not in the CDR ({message}) — it may have been deleted meanwhile. \
                 Reload this screen and retry."
            ),
            _ => format!("The CDR answered {status} to {object}: {message}. Nothing was saved."),
        },
        // 401 and 403 are different refusals with different next actions: the
        // credential is not accepted any more versus it is accepted and not
        // authorized (ITS-REST
        // `specifications/docs/overview/Requests_and_responses.md`
        // §HTTP status codes).
        ViewerError::CdrUnauthorized(message) => format!(
            "The CDR no longer accepts this session, so {object} was not saved ({message}). Sign \
             in again and retry."
        ),
        ViewerError::Forbidden(message) => format!(
            "This session may not write {object} ({message}). Sign in with an account that \
             carries the required role and retry."
        ),
        ViewerError::Unauthenticated => format!(
            "The viewer session expired before {object} was saved — sign in again and retry; \
             your input is still on screen."
        ),
        ViewerError::CdrUnreachable(message) => format!(
            "The CDR is unreachable ({message}), so {object} was not saved. Check the CDR, then \
             retry."
        ),
        ViewerError::Internal(message) => format!(
            "{object} could not be saved: {message}. Retry; if it persists, check the viewer \
             logs."
        ),
    }
}

/// Actionable copy for a refused openEHR **logical delete**.
///
/// The versioned delete of a COMPOSITION or a directory, which is a normal
/// write on the versioned object (not the destructive admin delete
/// [`delete_failure_copy`](crate::admin::delete_failure_copy) covers).
///
/// The concurrency family gets its own next action: a `409` is returned "when
/// supplied `uid_based_id` doesn't match the latest version"
/// (`docs/specs/openehr/ITS-REST/specifications/responses/409_COMPOSITION_with_uid_based_id.yaml`)
/// and a `412` is the `If-Match` precondition on the preceding version
/// evaluating to false
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §"If-Match and accidental overwrites") — the same cause, so one next
/// action: reload the history and delete the version that is now latest.
/// Everything else falls through to [`write_failure_copy`].
#[must_use]
pub fn logical_delete_failure_copy(object: &str, error: &ViewerError) -> String {
    match error {
        ViewerError::Cdr { message, .. } => match error.status_code() {
            Some(http::StatusCode::CONFLICT | http::StatusCode::PRECONDITION_FAILED) => format!(
                "{object} moved on in the CDR since this screen loaded, so nothing was deleted \
                 ({message}). Reload the version history and delete the version that is latest \
                 now."
            ),
            // `400_already_deleted` on the COMPOSITION delete: the version is
            // already logically deleted, so there is nothing left to do.
            Some(http::StatusCode::BAD_REQUEST) => format!(
                "The CDR refused to delete {object}: {message}. It may already be deleted — \
                 reload this screen to see the current history."
            ),
            Some(http::StatusCode::NOT_FOUND) => format!(
                "{object} is not in the CDR ({message}) — it may have been deleted meanwhile. \
                 Reload this screen."
            ),
            _ => write_failure_copy(object, error),
        },
        ViewerError::Forbidden(message) => format!(
            "This session may not delete {object} ({message}). Sign in with an account that \
             carries the required role and retry."
        ),
        ViewerError::CdrUnauthorized(message) => format!(
            "The CDR no longer accepts this session, so nothing was deleted ({message}). Sign in \
             again and retry."
        ),
        other => write_failure_copy(object, other),
    }
}

/// Toast a failed write with [`write_failure_copy`] — the failure half of the
/// rule this module records.
///
/// `title` is the short outcome ("Upload failed", "Commit failed"); `object`
/// is the noun phrase the copy is built around.
pub fn toast_write_failure(
    toaster: thaw::ToasterInjection,
    title: &str,
    object: &str,
    error: &ViewerError,
) {
    crate::components::toast::toast_error(toaster, title, &write_failure_copy(object, error));
}

#[cfg(test)]
mod tests {
    use super::{logical_delete_failure_copy, write_failure_copy};
    use crate::error::ViewerError;

    #[test]
    fn the_two_refusals_ask_for_two_different_next_actions() {
        // 401: the credential is not accepted any more — sign in AGAIN.
        let stale = write_failure_copy(
            "the directory",
            &ViewerError::CdrUnauthorized("the bearer token has expired".to_owned()),
        );
        assert!(stale.contains("the bearer token has expired"), "{stale}");
        assert!(stale.contains("Sign in again"), "{stale}");
        assert!(!stale.contains("required role"), "{stale}");

        // 403: the credential IS accepted — sign in as someone ELSE. This copy
        // is unchanged by the split.
        let refused = write_failure_copy(
            "the directory",
            &ViewerError::Forbidden("operation requires the 'ADMIN' role".to_owned()),
        );
        assert_eq!(
            refused,
            "This session may not write the directory (operation requires the 'ADMIN' role). \
             Sign in with an account that carries the required role and retry."
        );

        // The delete vocabulary splits the same way.
        let deleted = logical_delete_failure_copy(
            "this composition version",
            &ViewerError::CdrUnauthorized("token expired".to_owned()),
        );
        assert!(
            deleted.contains("nothing was deleted") && deleted.contains("Sign in again"),
            "{deleted}"
        );
    }

    #[test]
    fn a_logical_delete_conflict_says_reload_the_history() {
        // 409 (path uid is not the latest version) and 412 (If-Match failed)
        // are the same cause and share one next action.
        for status in [409_u16, 412] {
            let copy = logical_delete_failure_copy(
                "this composition version",
                &ViewerError::Cdr {
                    status,
                    message: "latest is ::3".to_owned(),
                },
            );
            assert!(copy.contains("this composition version"), "{copy}");
            assert!(copy.contains("latest is ::3"), "{copy}");
            assert!(copy.contains("Reload the version history"), "{copy}");
            assert!(copy.contains("nothing was deleted"), "{copy}");
        }
    }

    #[test]
    fn an_already_deleted_version_and_a_refusal_read_as_deletes() {
        let already = logical_delete_failure_copy(
            "this composition version",
            &ViewerError::Cdr {
                status: 400,
                message: "already deleted".to_owned(),
            },
        );
        assert!(
            already.contains("already deleted") && already.contains("reload"),
            "{already}"
        );

        let refused = logical_delete_failure_copy(
            "this composition version",
            &ViewerError::Forbidden("insufficient scope".to_owned()),
        );
        assert!(
            refused.contains("may not delete") && refused.contains("insufficient scope"),
            "{refused}"
        );

        // Anything outside the delete-specific family keeps the shared write
        // copy rather than inventing a second vocabulary.
        let down = logical_delete_failure_copy(
            "this composition version",
            &ViewerError::CdrUnreachable("connection refused".to_owned()),
        );
        assert_eq!(
            down,
            write_failure_copy(
                "this composition version",
                &ViewerError::CdrUnreachable("connection refused".to_owned())
            )
        );
    }

    #[test]
    fn cdr_validation_rejection_names_the_object_the_diagnostic_and_the_next_action() {
        let copy = write_failure_copy(
            "the operational template",
            &ViewerError::Cdr {
                status: 422,
                message: "template id already registered".to_owned(),
            },
        );
        assert!(copy.contains("the operational template"), "{copy}");
        assert!(copy.contains("template id already registered"), "{copy}");
        assert!(copy.contains("retry"), "{copy}");
    }

    #[test]
    fn concurrency_duplication_and_absence_get_their_own_next_action() {
        let stale = write_failure_copy(
            "the directory",
            &ViewerError::Cdr {
                status: 412,
                message: "If-Match did not hold".to_owned(),
            },
        );
        assert!(stale.contains("Reload the latest version"), "{stale}");

        // A 409 is a duplicate/conflict, NOT a stale version: reloading a
        // version the user does not have is the wrong next action.
        let duplicate = write_failure_copy(
            "the new EHR",
            &ViewerError::Cdr {
                status: 409,
                message: "an EHR for that subject already exists".to_owned(),
            },
        );
        assert!(
            duplicate.contains("an EHR for that subject already exists")
                && duplicate.contains("Open the existing one"),
            "{duplicate}"
        );
        assert!(
            !duplicate.contains("Reload the latest version"),
            "{duplicate}"
        );

        let gone = write_failure_copy(
            "the composition",
            &ViewerError::Cdr {
                status: 404,
                message: "HTTP 404".to_owned(),
            },
        );
        assert!(
            gone.contains("the composition") && gone.contains("Reload this screen"),
            "{gone}"
        );
    }

    #[test]
    fn transport_auth_and_input_failures_all_name_an_action() {
        let down = write_failure_copy(
            "the new EHR",
            &ViewerError::CdrUnreachable("connection refused".to_owned()),
        );
        assert!(
            down.contains("the new EHR")
                && down.contains("connection refused")
                && down.contains("retry"),
            "{down}"
        );

        let refused = write_failure_copy(
            "the stored query",
            &ViewerError::Forbidden("insufficient scope".to_owned()),
        );
        assert!(
            refused.contains("insufficient scope") && refused.contains("required role"),
            "{refused}"
        );

        let expired = write_failure_copy("the directory", &ViewerError::Unauthenticated);
        assert!(expired.contains("sign in again"), "{expired}");

        let invalid = write_failure_copy(
            "the composition",
            &ViewerError::Invalid("the body is not valid JSON".to_owned()),
        );
        assert!(
            invalid.contains("the body is not valid JSON") && invalid.contains("Correct the input"),
            "{invalid}"
        );

        let other = write_failure_copy(
            "the stored query",
            &ViewerError::Cdr {
                status: 500,
                message: "internal".to_owned(),
            },
        );
        assert!(
            other.contains("500") && other.contains("Nothing was saved"),
            "{other}"
        );
    }
}
