// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One EHR's access log over ITS-REST (#5130), **our own extension**: the
//! subject of care's view of who accessed their record (EHDS Art. 9(2),
//! `docs/law/eu/ehds/text.html`) and the reviewer's full view.
//!
//! No openEHR ITS-REST operation governs these routes: the access log is a
//! draft openEHR package FerroEHR proposes, and its ITS-REST resources are
//! part of that proposal (`docs/specifications/access-log`, the ITS-REST page).
//! The log has its own access control apart from any clinical grant (the
//! draft's recording rule 8), so the SMART requirement of the routes is an
//! authenticated caller and this module decides the view:
//!
//! The subject of care, or a representative bound to them, reads the subject
//! view of the EHR they are confined to, and any other EHR answers `404`; a
//! caller the admin class admits reads the full view; anyone else gets `403`.
//! The subject view withholds what `ACCESS_POLICY.log_restrictions` restricts
//! (EHDS Art. 9(3)). Every read writes its own access-log entry.

use axum::response::Response;
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use ferroehr::access_log::view::{LOG_REFUSED, LogReader, LogStanding, Restrictions};
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

use crate::api::access_log_query::{
    EHR_PARAMETERS, log_query, page, record_view, recorded, standing,
};
use crate::api::{RequestParts, guarded_handler};
use crate::extensions::access::authn::current_principal;
use crate::extensions::access::delay;
use crate::overview::error::RestError;
use crate::overview::version_id::parse_ehr_id;
use crate::state::AppState;

/// The operation ids of the routes, which the clinical `EHR_ACCESS` gate
/// leaves to this module's own access control.
pub(crate) const OPERATIONS: [&str; 2] = ["ehr_access_log_list", "ehr_access_log_get"];

/// The EHR access-log routes, mounted beside the openEHR base path.
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ehr_access_log_list))
        .routes(routes!(ehr_access_log_get))
}

guarded_handler! {
    /// The entries of one EHR's access log, newest first
    /// (`GET /ehr/{ehr_id}/access_log`). **Our own extension** (module docs).
    #[utoipa::path(
        get, path = "/ehr/{ehr_id}/access_log", tag = "ACCESS_LOG",
        params(
            ("ehr_id" = String, Path, description = "The EHR whose log is read."),
            ("from" = Option<String>, Query,
             description = "The first `time_recorded` selected, an ISO 8601 date-time."),
            ("to" = Option<String>, Query,
             description = "The last `time_recorded` selected, an ISO 8601 date-time."),
            ("action" = Option<String>, Query,
             description = "A term of the `access action` group by its coded content, the token \
                            `terminology_id|code`, such as `HL7_AuditEventAction|R` or \
                            `openehr_access|query`."),
            ("outcome" = Option<String>, Query,
             description = "A term of the `access outcome` group by its coded content, the token \
                            `terminology_id|code`, such as `openehr_access|refused`."),
            ("accessor_kind" = Option<String>, Query,
             description = "A term of the `accessor kind` group by its coded content, the token \
                            `terminology_id|code`, such as \
                            `HL7_SecurityRoleType|datasubject`."),
            ("justification" = Option<String>, Query,
             description = "`justification` or `emergency`: only the entries an access \
                            justification of that kind admitted, with the reason the \
                            accessor stated."),
            ("include_self" = Option<bool>, Query,
             description = "Whether the subject's own reads of their log are included; \
                            default false."),
            ("offset" = Option<i64>, Query, description = "Entries skipped; default 0."),
            ("fetch" = Option<i64>, Query,
             description = "The most entries answered, 1 to 1000; default 100.")
        ),
        responses(
            (status = 200, description = "`offset`, `fetch` and `entries`: each entry in \
                                          its ITS-JSON form for a reviewer, or in the \
                                          subject view for the subject of care and their \
                                          representative. No total count.",
             body = serde_json::Value),
            (status = 400, description = "A parameter the route does not take, or a value \
                                          outside its form.", body = serde_json::Value),
            (status = 401, description = "No valid credential.", body = serde_json::Value),
            (status = 403, description = "The caller is neither the subject, their \
                                          representative nor a reviewer.",
             body = serde_json::Value),
            (status = 404, description = "No such EHR, or one the caller may not see.",
             body = serde_json::Value),
            (status = 409, description = "The local audit store, which holds the access \
                                          log, is off.", body = serde_json::Value)
        )
    )]
    ehr_access_log_list => |state, parts| async move { Ok(list(&state, &parts).await) }
}

guarded_handler! {
    /// One entry of an EHR's access log
    /// (`GET /ehr/{ehr_id}/access_log/{uid}`). **Our own extension** (module
    /// docs).
    #[utoipa::path(
        get, path = "/ehr/{ehr_id}/access_log/{uid}", tag = "ACCESS_LOG",
        params(
            ("ehr_id" = String, Path, description = "The EHR whose log is read."),
            ("uid" = String, Path, description = "The entry's `uid`.")
        ),
        responses(
            (status = 200, description = "The entry, in its ITS-JSON form for a reviewer or \
                                          in the subject view for the subject of care and \
                                          their representative.",
             body = serde_json::Value),
            (status = 400, description = "The EHR id is not a UUID.", body = serde_json::Value),
            (status = 401, description = "No valid credential.", body = serde_json::Value),
            (status = 403, description = "The caller is neither the subject, their \
                                          representative nor a reviewer.",
             body = serde_json::Value),
            (status = 404, description = "No such EHR, one the caller may not see, or no \
                                          entry of it with this uid. A tombstone keeps no \
                                          EHR, so it is read through the review.",
             body = serde_json::Value),
            (status = 409, description = "The local audit store, which holds the access \
                                          log, is off.", body = serde_json::Value)
        )
    )]
    ehr_access_log_get => |state, parts| async move { Ok(get(&state, &parts).await) }
}

/// The `ehr_access_log_list` operation.
async fn list(state: &AppState, parts: &RequestParts) -> Response {
    let ehr = parts.path.get("ehr_id").map_or("", String::as_str);
    let answered = async {
        let ehr_id = parse_ehr_id(ehr)?;
        let filter = log_query(parts.query.as_deref(), EHR_PARAMETERS)?;
        let reader = reader(state, ehr)?;
        let records = state
            .backend()
            .ehr_access_log(ehr_id, &filter, reader)
            .await?;
        let restrictions = restrictions(state);
        Ok((
            page(&records, &filter, reader, &restrictions)?,
            Some(records.len()),
        ))
    }
    .await;
    recorded(
        &parts.headers,
        answered,
        Some(ehr),
        None,
        parts.query.as_deref(),
    )
}

/// The `ehr_access_log_get` operation.
async fn get(state: &AppState, parts: &RequestParts) -> Response {
    let ehr = parts.path.get("ehr_id").map_or("", String::as_str);
    let uid = parts.path.get("uid").map_or("", String::as_str);
    let answered = async {
        let ehr_id = parse_ehr_id(ehr)?;
        let reader = reader(state, ehr)?;
        let entry = state
            .backend()
            .ehr_access_log_entry(ehr_id, uid, reader)
            .await?;
        let body = record_view(
            &AccessLogRecord::AccessLogEntry(entry),
            reader,
            &restrictions(state),
        )?
        .ok_or_else(|| unknown_entry(uid))?;
        Ok((body, Some(1)))
    }
    .await;
    recorded(&parts.headers, answered, Some(ehr), Some(uid), None)
}

/// The EHDS Art. 9(3) restrictions of the subject view in the controller's
/// access policy in force, which the request brought up to date before it ran.
fn restrictions(state: &AppState) -> Restrictions {
    Restrictions::of(state.backend().access_policy().in_force().policy.as_deref())
}

/// Who the current caller reads the log of `ehr` as: their [`standing`]
/// held to `ehr`.
///
/// # Errors
/// A `404` for the subject or a representative addressing an EHR other than
/// the one they are confined to, and a `403` for a caller the admin class
/// does not admit.
fn reader(state: &AppState, ehr: &str) -> Result<LogReader, RestError> {
    let unknown = || RestError::from(ApiError::NotFound(format!("no EHR has the id {ehr}")));
    match standing(state) {
        LogStanding::Reviewer => Ok(LogReader::Reviewer),
        // NOTE: EHDS Art. 9(2), the person reads the log of their own EHR only, and another
        // EHR answers as an unknown one does, so its existence is not disclosed.
        LogStanding::Person(confined)
            if confined
                .iter()
                .any(|own| own.to_string().eq_ignore_ascii_case(ehr)) =>
        {
            Ok(LogReader::Person)
        }
        LogStanding::Person(_) => Err(unknown()),
        LogStanding::Refused
            if current_principal()
                .is_some_and(|principal| delay::acts_for_the_person(&principal)) =>
        {
            Err(unknown())
        }
        LogStanding::Refused => Err(RestError::from(ApiError::Forbidden(LOG_REFUSED.to_owned()))),
    }
}

/// The `404` of an entry the EHR's log does not hold.
fn unknown_entry(uid: &str) -> RestError {
    RestError::from(ApiError::NotFound(format!(
        "the access log of this EHR holds no entry {uid}"
    )))
}
