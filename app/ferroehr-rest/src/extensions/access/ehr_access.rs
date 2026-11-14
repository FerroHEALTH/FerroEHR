// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR_ACCESS` gate: the access evaluator at the request seam.
//!
//! `EHR_ACCESS` is a mandatory, versioned RM object and the openEHR
//! access-decision authority: "All access decisions to data in the EHR must be
//! made in accordance with the policies and rules in this object" (RM
//! `org.openehr.rm.ehr.ehr_access.adoc` §`EHR_ACCESS` Class). This gate is
//! therefore the foundational, always-on layer: every EHR carries an
//! `EHR_ACCESS`, and the `open` default keeps every flow working (BASE
//! `architecture_overview/master07-security.adoc` §Access Control).
//!
//! Before dispatch it judges, by the fixed decision procedure
//! (`ferroehr::access_eval`), the EHR an operation's path names, every EHR a
//! manifest export names (`enforce_each`), and each object the path names
//! when the verdict depends on the object. A restriction or an opt-out
//! answers `404`, as an unknown target does, with nothing in the body that
//! names it (EHDS Art. 8, third paragraph); every other refusal answers `403`
//! with its `refused-*` code. A query carries the same verdicts as a
//! condition of its one SQL statement (`query_condition`). The procedure and
//! its codes are our own design (#5487, #5513).

use axum::response::{IntoResponse, Response};
use ferroehr::access_eval::condition::{AccessCondition, RequestAccess, current};
use ferroehr::access_eval::procedure::{Outcome, Verdict};
use ferroehr::access_eval::scope::Item;
use ferroehr::error::ServiceError;
use ferroehr::error::refusal::Missing;
use ferroehr::ids::{EhrId, VoId};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::draft::access_control::terminology::{Refusal, Term as _};
use uuid::Uuid;

use crate::api::RequestParts;
use crate::extensions::access::authn::{Principal, current_principal};
use crate::overview::error::RestError;
use crate::overview::problem::Detail;
use crate::state::AppState;

/// The path parameters that name a versioned object, alone or as the head of
/// an `OBJECT_VERSION_ID`.
const OBJECT_PARAMS: [&str; 3] = ["versioned_object_uid", "uid_based_id", "version_uid"];

/// Enforce the access evaluator for an EHR-scoped request of `op`, after
/// authentication and before dispatch. `Err(response)` short-circuits with
/// the `404` of an absent target, the `403` of a refusal, the `503` of a
/// settings store that cannot answer, or the fail-closed `500` of settings
/// that do not read; `Ok(())` lets the request proceed. A request outside
/// the evaluator (the controller's own operations) and a route naming no EHR
/// proceed.
pub(crate) async fn enforce(
    state: &AppState,
    op: &str,
    parts: &RequestParts,
) -> Result<(), Box<Response>> {
    let Some(access) = current() else {
        return Ok(());
    };
    let principal = current_principal();
    let Some(ehr_id) = parts
        .path
        .get("ehr_id")
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .map(EhrId)
    else {
        // A malformed ehr_id is not ours to reject: the dispatch returns 400/404.
        return Ok(());
    };
    let verdict = state
        .backend()
        .ehr_verdict(&access, ehr_id)
        .await
        .map_err(|e| Box::new(unavailable(principal.as_ref(), e)))?;
    if let Some(refusal) = refusal_of(&access, op, principal.as_ref(), ehr_id, &verdict) {
        return Err(Box::new(refusal));
    }
    record_overrides(state, &[ehr_id]).await?;
    let objects = named_objects(parts);
    if !objects.is_empty() && verdict.item_dependent() {
        return enforce_objects(state, op, principal.as_ref(), &objects).await;
    }
    Ok(())
}

/// Judge each object `objects` names by the verdict of its own EHR: the
/// `404` of an object that reads as absent, the `403` of one refused. An
/// object this store does not hold is the dispatch's to answer.
async fn enforce_objects(
    state: &AppState,
    op: &str,
    principal: Option<&Principal>,
    objects: &[(&str, VoId)],
) -> Result<(), Box<Response>> {
    let Some(access) = current() else {
        return Ok(());
    };
    let backend = state.backend();
    let ids: Vec<VoId> = objects.iter().map(|(_, vo_id)| *vo_id).collect();
    let fault = |e: ServiceError| Box::new(unavailable(principal, e));
    let facts = backend.object_access_facts(&ids).await.map_err(fault)?;
    for held in &facts {
        let Some(ehr_id) = held.ehr_id else {
            continue;
        };
        let verdict = backend.ehr_verdict(&access, ehr_id).await.map_err(fault)?;
        let raw = objects
            .iter()
            .find(|(_, vo_id)| *vo_id == held.vo_id)
            .map_or("", |(raw, _)| raw);
        match access.note(ehr_id, &verdict, &held.item()) {
            Outcome::Admit => {}
            Outcome::Absent(_) => return Err(Box::new(object_not_found(op, raw))),
            Outcome::Refused(code) => return Err(Box::new(forbidden(principal, code))),
        }
    }
    Ok(())
}

/// Enforce the evaluator on every EHR an operation names in its body, such as
/// the manifest of `POST /message/export`, before any of their content is
/// read: the first EHR the caller may not reach refuses the whole operation
/// with the `404`, `403`, `503` or fail-closed `500` its path route would
/// answer, and an opt-out an emergency lifts is recorded for each EHR.
pub(crate) async fn enforce_each(
    state: &AppState,
    op: &str,
    ehr_ids: &[EhrId],
) -> Result<(), Box<Response>> {
    let Some(access) = current() else {
        return Ok(());
    };
    let principal = current_principal();
    for ehr_id in ehr_ids {
        let verdict = state
            .backend()
            .ehr_verdict(&access, *ehr_id)
            .await
            .map_err(|e| Box::new(unavailable(principal.as_ref(), e)))?;
        if let Some(refusal) = refusal_of(&access, op, principal.as_ref(), *ehr_id, &verdict) {
            return Err(Box::new(refusal));
        }
    }
    record_overrides(state, ehr_ids).await
}

/// The refusal an EHR-wide `verdict` answers for `ehr_id`, noted for the
/// access record of the request; `None` when it admits some data, which the
/// reads then judge one by one.
fn refusal_of(
    access: &RequestAccess,
    op: &str,
    principal: Option<&Principal>,
    ehr_id: EhrId,
    verdict: &Verdict,
) -> Option<Response> {
    if verdict.item_dependent() && verdict.admits_any() {
        return None;
    }
    match access.note(ehr_id, verdict, &Item::Ehr) {
        Outcome::Admit => None,
        Outcome::Absent(_) => Some(ehr_not_found(op, ehr_id)),
        Outcome::Refused(code) => Some(forbidden(principal, code)),
    }
}

/// Record the EHDS Art. 10(2) override of each opt-out an emergency lifted
/// among `ehr_ids`.
async fn record_overrides(state: &AppState, ehr_ids: &[EhrId]) -> Result<(), Box<Response>> {
    state
        .backend()
        .record_access_opt_out_overrides(ehr_ids)
        .await
        .map_err(|error| Box::new(RestError::from(error).into_response()))
}

/// The condition of a query of the running request: the evaluator's verdict
/// on every EHR, as the SQL generator applies it. `Err(response)` is the
/// `503` of a settings store that cannot answer.
pub(crate) async fn query_condition(state: &AppState) -> Result<AccessCondition, Box<Response>> {
    state
        .backend()
        .request_access_condition()
        .await
        .map_err(|e| Box::new(unavailable(current_principal().as_ref(), e)))
}

/// The versioned objects the request path names, each with the raw parameter
/// that named it; a parameter that is no UUID names none.
pub(crate) fn named_objects(parts: &RequestParts) -> Vec<(&str, VoId)> {
    OBJECT_PARAMS
        .iter()
        .filter_map(|param| parts.path.get(*param))
        .filter_map(|raw| {
            VoId::from_uid_head(raw)
                .ok()
                .map(|vo_id| (raw.as_str(), vo_id))
        })
        .collect()
}

/// Whether `op` reads the `EHR_ACCESS` extension routes
/// (`crate::api::ehr::ehr_access`).
fn reads_ehr_access(op: &str) -> bool {
    op.starts_with("ehr_access_get") || op.starts_with("versioned_ehr_access")
}

/// The `404` an unknown EHR answers for `op`, worded as the dispatch words it.
fn ehr_not_found(op: &str, ehr_id: EhrId) -> Response {
    let message = if op.contains("ehr_status") {
        format!("EHR_STATUS for EHR {ehr_id}")
    } else if reads_ehr_access(op) {
        format!("EHR_ACCESS for EHR {ehr_id}")
    } else {
        format!("EHR {ehr_id}")
    };
    not_found(Missing::Ehr, message)
}

/// The `404` an unknown versioned object of `op`'s kind answers.
pub(crate) fn object_not_found(op: &str, raw: &str) -> Response {
    if op.contains("composition") {
        not_found(Missing::Composition, format!("COMPOSITION {raw}"))
    } else if op.contains("ehr_status") {
        not_found(Missing::VersionedObject, format!("EHR_STATUS {raw}"))
    } else if reads_ehr_access(op) {
        not_found(Missing::VersionedObject, format!("EHR_ACCESS {raw}"))
    } else if op.contains("directory") || op.contains("folder") {
        not_found(Missing::VersionedObject, format!("DIRECTORY {raw}"))
    } else {
        not_found(Missing::VersionedObject, format!("versioned object {raw}"))
    }
}

/// A `404` carrying `missing` and `message`, built the way the dispatchers
/// build theirs.
fn not_found(missing: Missing, message: String) -> Response {
    RestError::from(ServiceError::not_found(missing, message)).into_response()
}

/// The `503` of a settings store that cannot answer, and the fail-closed
/// `500` of settings that cannot be read.
fn unavailable(principal: Option<&Principal>, error: ServiceError) -> Response {
    let mut resp = match error {
        overloaded @ ServiceError::Overloaded(_) => RestError::from(overloaded).into_response(),
        other => {
            return server_error(
                principal,
                &format!("EHR_ACCESS settings unavailable: {other}"),
            );
        }
    };
    if let Some(principal) = principal {
        resp.extensions_mut().insert(principal.clone());
    }
    resp
}

/// The `403` of a refusal under `code`, naming the code in `detail` and in the
/// `refusal` member, carrying the principal so the ATNA layer records the
/// deny (ITS-REST `401`/`403` discipline).
fn forbidden(principal: Option<&Principal>, code: Refusal) -> Response {
    let mut resp = RestError::from(ApiError::Forbidden(format!(
        "access denied: {}: {}",
        code.code(),
        reason(code)
    )))
    .with_detail(Detail::Refusal(code.code()))
    .into_response();
    if let Some(principal) = principal {
        resp.extensions_mut().insert(principal.clone());
    }
    resp
}

/// What a refusal under `code` means, in the words of the decision procedure.
const fn reason(code: Refusal) -> &'static str {
    match code {
        Refusal::ConsentDenied => "a directive of the subject denies this access",
        Refusal::Sensitivity => "the data carry a sensitivity the caller is not cleared for",
        Refusal::NoCareRelationship => {
            "the access policy requires a care relationship to the subject and the caller has none"
        }
        Refusal::PurposeMissing => "the access policy requires a declared purpose of use",
        Refusal::PurposeNotPermitted => {
            "the caller's capabilities do not permit the declared purpose of use"
        }
        Refusal::ConsentRequired => "the EHR needs consent and no permit directive applies",
        Refusal::OptOut | Refusal::Restricted => "the data cannot be read",
    }
}

/// A `500` (fail-closed) when the settings cannot be read: an access decision
/// must never proceed on unknown policy.
fn server_error(principal: Option<&Principal>, detail: &str) -> Response {
    let mut resp = RestError::from(crate::overview::error::internal_fault(
        "read the EHR access-control settings",
        &detail,
    ))
    .into_response();
    if let Some(principal) = principal {
        resp.extensions_mut().insert(principal.clone());
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings store that cannot answer is `503`, and settings that do
    /// not read are `500`.
    #[test]
    fn an_unanswering_store_is_503_and_unreadable_settings_500() {
        let overloaded = ServiceError::overloaded(
            "the EHR access settings could not be read",
            std::io::Error::other("no connection"),
        );
        assert_eq!(
            unavailable(None, overloaded).status(),
            http::StatusCode::SERVICE_UNAVAILABLE
        );
        let unreadable = ServiceError::internal("settings", std::io::Error::other("bogus"));
        assert_eq!(
            unavailable(None, unreadable).status(),
            http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// Every visible refusal answers `403`.
    #[test]
    fn a_visible_refusal_is_403() {
        for code in [
            Refusal::ConsentDenied,
            Refusal::Sensitivity,
            Refusal::NoCareRelationship,
            Refusal::PurposeMissing,
            Refusal::PurposeNotPermitted,
            Refusal::ConsentRequired,
        ] {
            assert!(!code.is_invisible());
            assert_eq!(forbidden(None, code).status(), http::StatusCode::FORBIDDEN);
        }
    }
}
