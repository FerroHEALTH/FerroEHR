// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN operations on the access log's integrity (#5129).
//!
//! They seal a checkpoint on demand, verify the log against a checkpoint, and
//! export one EHR's log for its transfer to another system. **Our own
//! extension.**
//!
//! No openEHR ITS-REST operation governs these routes: the access log is a
//! draft openEHR package FerroEHR proposes, and its ITS-REST resources are
//! part of that proposal (`docs/plans/access-log.md` §ITS-REST resources).
//! The import of an exported log is the `ferroehr access-log import`
//! command, run by the operator of the receiving system.
//!
//! Gating: mounted under `/admin/`, so the coarse RBAC `OperationClass::Admin`
//! classifier and the `AppConfig::api.admin` gate apply, as for every admin
//! extension ([`super::integrity`]).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): the verification report and the \
              export are operational documents, not RM resources"
)]

use axum::response::Response;
use http::StatusCode;
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use ferroehr::access_log::checkpoint::{Checkpoints, Verification};
use ferroehr::access_log::store::AccessLogError;
use ferroehr::access_log::transfer::TransferError;
use ferroehr::versioning::signature::signer::AttestError;

use super::body::{AdminRefusal, required_query};
use crate::api::{RequestParts, guarded_handler};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The access-log integrity routes as a native `utoipa-axum` router.
pub(crate) fn access_log_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_seal_access_log_checkpoint))
        .routes(routes!(admin_verify_access_log))
        .routes(routes!(admin_export_access_log))
}

guarded_handler! {
    /// Seal the head of the access log in a signed checkpoint
    /// (`POST /admin/access-log/checkpoint`). **Our own extension.**
    #[utoipa::path(
        post, path = "/admin/access-log/checkpoint", tag = "admin-access-log",
        responses(
            (status = 200, description = "The checkpoint of the head of the chain, as its \
                                          ITS-JSON `ACCESS_LOG_CHECKPOINT`. A head already \
                                          sealed answers its stored checkpoint.",
             body = serde_json::Value),
            (status = 404, description = "The chain is empty.", body = serde_json::Value),
            (status = 409, description = "The keyring holds no OpenPGP key to sign with.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_seal_access_log_checkpoint
        => |state, parts| super::body::gated(state, parts, seal)
}

guarded_handler! {
    /// Verify the access log against its newest checkpoint
    /// (`POST /admin/access-log/verify`). **Our own extension.**
    ///
    /// The whole chain is recomputed, tombstones taken as recorded; the
    /// checkpoint's proof, the link it seals and every signed link are
    /// checked. A finding is not a request failure.
    #[utoipa::path(
        post, path = "/admin/access-log/verify", tag = "admin-access-log",
        responses(
            (status = 200, description = "The verification ran. `intact` is true when a \
                                          checkpoint was there to verify against and nothing \
                                          failed; `findings` names each failure of the \
                                          checkpoint and the signed links, `chain` each \
                                          damaged record or boundary of the chain.",
             body = serde_json::Value, example = json!({
                 "intact": false,
                 "checkpoint": { "sequence": 48213, "digest": "<64 hex>" },
                 "findings": ["the checkpoint at 48213 is not signed by a trusted key"],
                 "chain": [{ "chain_seq": 48190,
                             "finding": "record content was modified after it was written" }]
             })),
            AdminRefusal
        )
    )]
    admin_verify_access_log
        => |state, parts| super::body::gated(state, parts, verify)
}

guarded_handler! {
    /// Export one EHR's access log for its transfer to another system
    /// (`GET /admin/access-log/export?ehr_id=`). **Our own extension.**
    #[utoipa::path(
        get, path = "/admin/access-log/export", tag = "admin-access-log",
        params(
            ("ehr_id" = String, Query, description = "The EHR whose log is exported."),
        ),
        responses(
            (status = 200, description = "The EHR's records with their links, every link \
                                          from the first of them to the checkpoint they \
                                          verify against, and that checkpoint, verified \
                                          before it is answered. `ferroehr access-log import` \
                                          reads this document.",
             body = serde_json::Value),
            (status = 400, description = "`ehr_id` is missing.", body = serde_json::Value),
            (status = 404, description = "The log holds no record of the EHR.",
             body = serde_json::Value),
            (status = 409, description = "No checkpoint covers the records and the keyring \
                                          holds no OpenPGP key to seal one.",
             body = serde_json::Value),
            (status = 422, description = "The span holds `audit_event` records, which cannot \
                                          be verified apart from this database.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_export_access_log
        => |state, parts| super::body::gated(state, parts, export)
}

/// The wired access log, or the refusal of a server that wired none.
fn wired(state: &AppState) -> Result<Checkpoints, RestError> {
    state.backend().access_log().cloned().ok_or_else(|| {
        RestError::from(ApiError::ServiceUnavailable(
            "the access log is not wired on this server".to_owned(),
        ))
    })
}

/// The `admin_seal_access_log_checkpoint` operation.
async fn seal(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let checkpoint = wired(&state)?
        .seal()
        .await
        .map_err(log_error)?
        .ok_or_else(|| RestError::from(ApiError::NotFound("the access log is empty".to_owned())))?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &checkpoint,
    ))
}

/// The `admin_verify_access_log` operation.
async fn verify(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let verification = wired(&state)?.verify(None).await.map_err(log_error)?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &verification_report(&verification),
    ))
}

/// The `admin_export_access_log` operation.
async fn export(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let ehr_id = required_query(&parts, "ehr_id")?;
    let export = wired(&state)?
        .export(&ehr_id)
        .await
        .map_err(|error| match error {
            TransferError::Log(error) => log_error(error),
            TransferError::NoRecords { .. } => {
                RestError::from(ApiError::NotFound(error.to_string()))
            }
            other => {
                tracing::error!(error = %other, "the access-log export does not verify");
                RestError::from(ApiError::Internal(other.to_string()))
            }
        })?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &export,
    ))
}

/// The answer to a store or signing failure.
fn log_error(error: AccessLogError) -> RestError {
    match error {
        AccessLogError::Sign(AttestError::NoKey) => {
            RestError::from(ApiError::Conflict(AttestError::NoKey.to_string()))
        }
        AccessLogError::Interleaved { .. } => {
            RestError::from(ApiError::Unprocessable(error.to_string()))
        }
        other => {
            tracing::error!(error = %other, "the access-log operation failed");
            RestError::from(ApiError::Internal(
                "the access-log operation failed; see the server log".to_owned(),
            ))
        }
    }
}

/// The verification report as the response body.
///
/// Written out explicitly: the wire contract is this function, not a serde
/// attribute on a library type.
fn verification_report(verification: &Verification) -> Value {
    json!({
        "intact": verification.intact(),
        "checkpoint": verification.checkpoint.as_ref().map(|checkpoint| json!({
            "sequence": checkpoint.sequence,
            "digest": checkpoint.digest,
            "time_sealed": checkpoint.time_sealed.value,
        })),
        "findings": verification
            .findings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "chain": verification
            .chain
            .iter()
            .map(|finding| json!({
                "chain_seq": finding.chain_seq,
                "finding": finding.finding,
            }))
            .collect::<Vec<_>>(),
    })
}
