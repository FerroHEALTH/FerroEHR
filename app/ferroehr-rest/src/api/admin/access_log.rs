// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN operations on the access log: its review (#5130) and its
//! integrity (#5129).
//!
//! They list the log's records across EHRs for review (EHDS Annex II 3.3),
//! answer the links of a span of its chain and a stored checkpoint, seal a
//! checkpoint on demand, verify the log against a checkpoint, and export one
//! EHR's log for its transfer to another system. Each read of the records or
//! the chain writes its own access-log entry. **Our own extension.**
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
use ferroehr::service::admin::access_log_read::LogReader;
use ferroehr::versioning::signature::signer::AttestError;

use super::body::{AdminRefusal, required_query};
use crate::api::access_log_query::{REVIEW_PARAMETERS, log_query, page, recorded};
use crate::api::{RequestParts, guarded_handler};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The access-log integrity routes as a native `utoipa-axum` router.
pub(crate) fn access_log_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(admin_list_access_log))
        .routes(routes!(admin_access_log_chain))
        .routes(routes!(
            admin_get_access_log_checkpoint,
            admin_seal_access_log_checkpoint
        ))
        .routes(routes!(admin_verify_access_log))
        .routes(routes!(admin_export_access_log))
}

guarded_handler! {
    /// The records of the access log across EHRs, newest first, for review
    /// (`GET /admin/access-log`). **Our own extension.**
    #[utoipa::path(
        get, path = "/admin/access-log", tag = "admin-access-log",
        params(
            ("ehr_id" = Option<String>, Query, description = "The EHR the entries are about."),
            ("party_namespace" = Option<String>, Query,
             description = "The `PARTY_REF.namespace` of the accessing party."),
            ("party_type" = Option<String>, Query,
             description = "The `PARTY_REF.type` of the accessing party."),
            ("party_id" = Option<String>, Query,
             description = "The `PARTY_REF.id` of the accessing party."),
            ("issuer" = Option<String>, Query,
             description = "The issuer of the accessor's credential."),
            ("subject" = Option<String>, Query,
             description = "The subject of the accessor's credential."),
            ("request_id" = Option<String>, Query,
             description = "The request the entries were written in."),
            ("from" = Option<String>, Query,
             description = "The first `time_recorded` selected, an ISO 8601 date-time."),
            ("to" = Option<String>, Query,
             description = "The last `time_recorded` selected, an ISO 8601 date-time."),
            ("outcome" = Option<String>, Query, description = "An `access outcome` rubric."),
            ("emergency" = Option<bool>, Query,
             description = "Only the entries whose `is_emergency` is this."),
            ("offset" = Option<i64>, Query, description = "Records skipped; default 0."),
            ("fetch" = Option<i64>, Query,
             description = "The most records answered, 1 to 1000; default 100.")
        ),
        responses(
            (status = 200, description = "`offset`, `fetch` and `entries`, each record, entry \
                                          or tombstone, in its ITS-JSON form with its link. \
                                          No total count.",
             body = serde_json::Value),
            (status = 400, description = "A parameter the route does not take, or a value \
                                          outside its form.", body = serde_json::Value),
            (status = 409, description = "The local audit store, which holds the access \
                                          log, is off.", body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_list_access_log
        => |state, parts| super::body::gated(state, parts, review)
}

guarded_handler! {
    /// The uid and link of each record of a span of the access log's chain,
    /// without their content (`GET /admin/access-log/chain`). **Our own
    /// extension.**
    #[utoipa::path(
        get, path = "/admin/access-log/chain", tag = "admin-access-log",
        params(
            ("from_sequence" = i64, Query, description = "The first position of the span."),
            ("to_sequence" = i64, Query,
             description = "The last position of the span, at most 10000 after the first."),
        ),
        responses(
            (status = 200, description = "`links`: the `uid` and `link` of each record from \
                                          `from_sequence` to `to_sequence`, in chain order.",
             body = serde_json::Value),
            (status = 400, description = "A bound is missing or outside its form, the span \
                                          is longer than 10000, or it holds `audit_event` \
                                          records, which only this database verifies.",
             body = serde_json::Value),
            (status = 409, description = "The local audit store, which holds the access \
                                          log, is off.", body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_access_log_chain
        => |state, parts| super::body::gated(state, parts, chain)
}

guarded_handler! {
    /// The newest stored checkpoint of the access log, or the one at
    /// `sequence` (`GET /admin/access-log/checkpoint`). **Our own extension.**
    #[utoipa::path(
        get, path = "/admin/access-log/checkpoint", tag = "admin-access-log",
        params(
            ("sequence" = Option<i64>, Query,
             description = "The sequence of the checkpoint; the newest when absent."),
        ),
        responses(
            (status = 200, description = "The checkpoint, as its ITS-JSON \
                                          `ACCESS_LOG_CHECKPOINT`.",
             body = serde_json::Value),
            (status = 400, description = "`sequence` is not an integer.",
             body = serde_json::Value),
            (status = 404, description = "No checkpoint is stored, or none at `sequence`.",
             body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_get_access_log_checkpoint
        => |state, parts| super::body::gated(state, parts, checkpoint)
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

/// The longest span of the chain one read answers: our own design.
const MAX_SPAN: i64 = 10_000;

/// The `admin_list_access_log` operation.
async fn review(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let answered = async {
        let filter = log_query(parts.query.as_deref(), REVIEW_PARAMETERS)?;
        let records = state.backend().access_log_search(&filter).await?;
        Ok((
            page(&records, &filter, LogReader::Reviewer)?,
            Some(records.len()),
        ))
    }
    .await;
    let query = parts.query.as_deref();
    Ok(recorded(&parts.headers, answered, None, None, query))
}

/// The `admin_access_log_chain` operation.
async fn chain(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let answered = async {
        let from = sequence(&required_query(&parts, "from_sequence")?, "from_sequence")?;
        let to = sequence(&required_query(&parts, "to_sequence")?, "to_sequence")?;
        if to < from || to.checked_sub(from).is_none_or(|span| span >= MAX_SPAN) {
            return Err(RestError::from(ApiError::BadRequest(format!(
                "`to_sequence` is at least `from_sequence` and less than {MAX_SPAN} after it"
            ))));
        }
        let links = state.backend().access_log_chain(from, to).await?;
        let served = links.len();
        let links = links
            .into_iter()
            .map(|(uid, link)| json!({ "uid": uid, "link": link }))
            .collect::<Vec<_>>();
        Ok((json!({ "links": links }), Some(served)))
    }
    .await;
    let query = parts.query.as_deref();
    Ok(recorded(&parts.headers, answered, None, None, query))
}

/// The `admin_get_access_log_checkpoint` operation.
async fn checkpoint(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let checkpoints = wired(&state)?;
    let stored = match crate::params::query_param(parts.query.as_deref(), "sequence") {
        Some(raw) => checkpoints.at(sequence(&raw, "sequence")?).await,
        None => checkpoints.latest(None).await,
    }
    .map_err(log_error)?
    .ok_or_else(|| {
        RestError::from(ApiError::NotFound(
            "no access-log checkpoint is stored there".to_owned(),
        ))
    })?;
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &stored,
    ))
}

/// `raw`, the chain position `name` names.
fn sequence(raw: &str, name: &str) -> Result<i64, RestError> {
    raw.parse::<i64>()
        .ok()
        .filter(|seq| *seq >= 1)
        .ok_or_else(|| {
            RestError::from(ApiError::BadRequest(format!(
                "`{name}` is a chain position, a positive integer"
            )))
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
