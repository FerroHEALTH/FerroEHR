// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One guarded operation's run: the policy confirmed, the access decision,
//! the handler and the PEP checks (`super::guarded_dispatch`).
//!
//! Every operation's decision rests on policy copies a read of the request
//! confirmed (#6048, the exact-reads rule of #5891). An operation outside
//! [`confirms_after`] reads the policy revision before it is decided
//! ([`confirmed`]). An overlapped read confirms after it ran ([`after`]): its
//! own statements read the policy and definition revisions beside the rows
//! they judge, at one snapshot, and its answer leaves only once the copies it
//! was decided and rendered on held them; otherwise that run is dropped
//! whole, with nothing recorded or counted, and the request runs once more
//! behind the up-front confirm (#6251). An `EHR_STATUS` or directory read, a
//! party read and an AQL query confirm after they ran the same way (#6320). A
//! COMPOSITION or CONTRIBUTION commit confirms after it was decided the same
//! way: its gate read returns the revisions, and a copy found behind them
//! refuses the write before anything is written, so the dropped run leaves
//! nothing; a run that reached its write is never run again (#6256). No
//! openEHR spec governs this: our own design.

use std::future::Future;
use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use ferroehr::access_eval::condition::RequestAccess;
use ferroehr::system_log::access_context::{self, AccessDelay};

use super::RequestParts;
use crate::extensions::access::{admission, caller, delay, ehr_access, pep};
use crate::overview::error::RestError;
use crate::state::AppState;

/// `op` run behind the up-front confirm of the policy it reads: the
/// decision's and the handler's answer, its decisions counted.
pub(super) async fn confirmed<H, F>(
    state: &AppState,
    op: &'static str,
    parts: RequestParts,
    handler: &H,
) -> Response
where
    H: Fn(AppState, RequestParts) -> F + Send + Sync,
    F: Future<Output = Result<Response, RestError>> + Send,
{
    // NOTE: no specification governs operational policy: our own design; the request reads
    // the policy revision first in one indexed read, and the newest versions when it moved,
    // so a change decides this request whatever its notification's delay (#6251).
    if let Err(error) = refresh_policy(state, op).await {
        return RestError::from(error).into_response();
    }
    let (resp, access) = run(state, op, parts, handler).await;
    if let Some(access) = access {
        ferroehr::party_binding::request::defer(move || access.count_decisions());
    }
    resp
}

/// The commits that confirm their policy copies after they were decided:
/// each one's gate read returns the revisions before anything is written
/// (#6256, #6257).
const COMMITS_AFTER: [&str; 8] = [
    "composition_create",
    "composition_update",
    "composition_delete",
    "contribution_create",
    "ehr_status_update",
    "directory_create",
    "directory_update",
    "directory_delete",
];

/// The reads that confirm their policy copies after they ran: each one's
/// clinical statement selects the policy and definition revisions beside its
/// rows, the version read of an EHR object and the AQL statement of a query
/// a request judges, and a party read's party statement selects them through
/// the clinical door where the two domains share a database (#6320).
const READS_AFTER: [&str; 15] = [
    "person_get",
    "agent_get",
    "group_get",
    "organisation_get",
    "role_get",
    "ehr_status_get_at_time",
    "ehr_status_get_by_version_id",
    "directory_get_at_time",
    "directory_get_by_version_id",
    "query_execute_adhoc_query",
    "query_execute_adhoc_query_body",
    "query_execute_stored_query",
    "query_execute_stored_query_body",
    "query_execute_stored_query_version",
    "query_execute_stored_query_version_body",
];

/// Whether `op` confirms its policy copies after it was decided ([`after`]).
pub(super) fn confirms_after(op: &str) -> bool {
    admission::overlaps(op) || COMMITS_AFTER.contains(&op) || READS_AFTER.contains(&op)
}

/// The overlapped read or commit `op` run on the policy copies as they stand,
/// served only once they held the revisions its statements read or once it
/// reached its write; otherwise dropped unrecorded and run once more by
/// [`confirmed`], which never runs it again.
pub(super) async fn after<H, F>(
    state: &AppState,
    op: &'static str,
    parts: RequestParts,
    handler: &H,
) -> Response
where
    H: Fn(AppState, RequestParts) -> F + Send + Sync,
    F: Future<Output = Result<Response, RestError>> + Send,
{
    let ((resp, access), seen) =
        ferroehr::policy_register::observed::scope(run(state, op, parts.clone(), handler)).await;
    // NOTE: our own design (#6256), a run that reached its write is the request's answer.
    let held = if seen.wrote {
        Ok(true)
    } else {
        state.backend().policy_held(seen.observed).await
    };
    match held {
        Ok(true) => {
            if let Some(access) = access {
                ferroehr::party_binding::request::defer(move || access.count_decisions());
            }
            resp
        }
        Ok(false) => {
            // NOTE: our own design (#6251), the run decided on a copy behind the store leaves
            // nothing: its answer, its evaluator and its uncounted decisions go unused.
            drop((resp, access));
            ferroehr::party_binding::request::defer(move || {
                ferroehr::telemetry::access_path::count_policy_rerun(op);
            });
            confirmed(state, op, parts, handler).await
        }
        Err(error) => RestError::from(error).into_response(),
    }
}

/// The decision and the handler of `op` on the policy copies as they stand,
/// with the request's evaluator, whose decisions are left uncounted.
async fn run<H, F>(
    state: &AppState,
    op: &'static str,
    parts: RequestParts,
    handler: &H,
) -> (Response, Option<Arc<RequestAccess>>)
where
    H: Fn(AppState, RequestParts) -> F + Send + Sync,
    F: Future<Output = Result<Response, RestError>> + Send,
{
    // NOTE: EHDS Art. 3(3) (`docs/law/eu/ehds/text.html`), data a professional has not
    // released reads as absent to a patient-context request on a deployment that delays.
    let delay = delay::of(state, op);
    // NOTE: our own design (#5513), every operation but the controller's own is judged by
    // the access evaluator, which holds the EHDS Art. 8 restriction and Art. 10 opt-out.
    let access = caller::request_access(state, op);
    let guarded = Box::pin(async move {
        // NOTE: our own design (#5959), the handler of an overlapped read starts its store read
        // while the decision is made, and the decision's refusal is the answer whenever it refuses.
        if admission::overlaps(op) {
            let decided_on = parts.clone();
            // Boxed: the handler and the decision are each large, and a debug build's stack
            // overflows on the copies made while the overlap is built.
            let overlapped = admission::overlap(
                Box::pin(admit(state, op, &decided_on, delay)),
                Box::pin(handler(state.clone(), parts)),
            );
            return match overlapped.await {
                Ok(resp) => {
                    let resp = resp.unwrap_or_else(IntoResponse::into_response);
                    pep::post_check(state, op, resp).await
                }
                Err(deny) => *deny,
            };
        }
        if let Err(deny) = admit(state, op, &parts, delay).await {
            return *deny;
        }
        let resp = handler(state.clone(), parts)
            .await
            .unwrap_or_else(IntoResponse::into_response);
        pep::post_check(state, op, resp).await
    });
    let delayed_guard = access_context::with_access_delay(delay, guarded);
    match access {
        Some(access) => {
            let mut resp = ferroehr::access_eval::condition::with_request_access_uncounted(
                Arc::clone(&access),
                delayed_guard,
            )
            .await;
            // NOTE: EHDS Annex II 3.2 (`docs/law/eu/ehds/text.html`), the access record names
            // the grounds of the decision, which the audit layer reads off the response.
            resp.extensions_mut().insert(Arc::clone(&access));
            (resp, Some(access))
        }
        None => (delayed_guard.await, None),
    }
}

/// The decision on `op` before its handler answers: the access evaluator,
/// the EHDS Art. 3(3) delay where `delay` holds, and the PEP pre-check, each
/// answering its refusal.
async fn admit(
    state: &AppState,
    op: &'static str,
    parts: &RequestParts,
    delay: AccessDelay,
) -> Result<(), Box<Response>> {
    ehr_access::enforce(state, op, parts).await?;
    // NOTE: EHDS Art. 3(3), a caller reading its own record by the party binding has the
    // delay held on that record, which the decision above found (#6108).
    let withheld = match delay {
        AccessDelay::Off => false,
        AccessDelay::Person => true,
        AccessDelay::Records => {
            access_context::access_delay_enforced()
                || parts
                    .path
                    .get("ehr_id")
                    .and_then(|raw| uuid::Uuid::parse_str(raw).ok())
                    .is_some_and(|ehr| {
                        access_context::access_delay_enforced_on(ferroehr::ids::EhrId(ehr))
                    })
        }
    };
    if withheld {
        match delay::refuse_withheld(state, op, parts).await {
            Ok(Some(absent)) => return Err(Box::new(absent)),
            Ok(None) => {}
            Err(fault) => return Err(fault),
        }
    }
    pep::pre_check(op, parts).map_err(Box::new)
}

/// The operations of the admin group that consult a policy register: the hold
/// of the access delay, the retention period keyed on a category, and the dump
/// and load, which classify and commit content. The register routes read and
/// write the register store themselves.
const ADMIN_REGISTER_READERS: [&str; 4] = [
    "admin_delay_access",
    "admin_dump",
    "admin_load",
    "admin_put_retention_policy",
];

/// Bring the policy `op` reads up to date before it runs: the policy
/// registers ([`reads_registers`]) and, for every operation outside the admin
/// group, the controller's access policy.
async fn refresh_policy(state: &AppState, op: &str) -> Result<(), ferroehr::error::ServiceError> {
    state
        .backend()
        .confirm_policy(reads_registers(state, op), !op.starts_with("admin_"))
        .await
}

/// Whether `op` reads the policy registers before it runs: every operation but
/// those of the admin group that consult none, and none while the admin group
/// is off, which answers `405` before it touches the backend.
fn reads_registers(state: &AppState, op: &str) -> bool {
    if !op.starts_with("admin_") {
        return true;
    }
    state.config().api.admin && ADMIN_REGISTER_READERS.contains(&op)
}
