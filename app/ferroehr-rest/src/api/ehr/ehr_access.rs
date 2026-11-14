// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR_ACCESS` reads, **our own extension**: an EHR's access directives
//! over the wire, so that another product can read the directives a
//! CONTRIBUTION wrote and exchange them (#5517).
//!
//! An EHR holds exactly one `EHR_ACCESS`, versioned "via the normal
//! mechanism" (RM ehr `master04-ehr_package.adoc` §EHR Access; EHR class
//! `ehr_access` 1..1), and its `settings` carry the draft
//! `ACCESS_DIRECTIVE_SETTINGS`. ITS-REST 1.1.0 publishes no `EHR_ACCESS`
//! operation, so no openEHR specification governs these routes. They are
//! mounted under the extensions root and shaped on the ITS-REST `EHR_STATUS`
//! and `VERSIONED_EHR_STATUS` operations (`docs/specs/openehr/ITS-REST/`
//! `specifications/docs/ehr/`): the same paths below the EHR, the same
//! `version_at_time` and `version_uid` addressing, the weak `ETag` and
//! `Last-Modified`. Each route runs the `EHR_STATUS` handler of its shape for
//! [`EhrAccess`]. A read that carries the `EHR_ACCESS` itself is canonical
//! JSON only, since canonical XML maps only published classes and no
//! published class defines the settings; the versioned object and its
//! revision history are canonical JSON or XML by `Accept`.
//!
//! The access evaluator judges every route as it judges a clinical read of
//! the EHR, and each read writes its access-log entry, both through
//! `crate::api::guarded_dispatch`. The write path is the
//! CONTRIBUTION, which commits an `EHR_ACCESS` version.

use openehr_rm::prelude::EhrAccess;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::api::guarded_handler;
use crate::api::refusals::{ResponseMap, SpecProfileConflict, json_responses};
use crate::state::AppState;

/// The `EHR_ACCESS` read routes, mounted beside the openEHR base path.
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ehr_access_get_at_time))
        .routes(routes!(ehr_access_get_by_version_id))
        .routes(routes!(versioned_ehr_access_get))
        .routes(routes!(versioned_ehr_access_revision_history))
        .routes(routes!(versioned_ehr_access_version_get_at_time))
        .routes(routes!(versioned_ehr_access_version_get_by_id))
}

/// The answers every `EHR_ACCESS` read can give besides its `200`, as one
/// response map for the OpenAPI document.
struct EhrAccessRefusal;

impl utoipa::IntoResponses for EhrAccessRefusal {
    fn responses() -> ResponseMap {
        json_responses(&[
            (
                400,
                "`ehr_id` is not a UUID, `version_uid` is not an OBJECT_VERSION_ID, or \
                 `version_at_time` is not an extended ISO 8601 date-time.",
            ),
            (401, "No valid credential."),
            (
                403,
                "The access evaluator refuses the caller this EHR, naming its \
                 `refused-*` code, or the role, scope or ABAC layer refuses the read.",
            ),
            (
                404,
                "No such EHR, an EHR an EHDS restriction or opt-out hides from the \
                 caller, or no EHR_ACCESS version at the addressed version or instant.",
            ),
            (
                406,
                "The `Accept` header cannot be satisfied: EHR_ACCESS is not templated, \
                 so no Simplified Format serves it, and a read that carries the \
                 EHR_ACCESS is `application/json` only, since no canonical-XML class \
                 maps its settings.",
            ),
        ])
    }
}

guarded_handler! {
    /// Read the `EHR_ACCESS` of an EHR, current or at a point in time
    /// (`GET /ehr/{ehr_id}/ehr_access`). **Our own extension** (module docs).
    #[utoipa::path(
        get, path = "/ehr/{ehr_id}/ehr_access", tag = "EHR_ACCESS",
        params(
            ("ehr_id" = String, Path, description = "The EHR, by `EHR.ehr_id.value`."),
            ("version_at_time" = Option<String>, Query,
             description = "An extended ISO 8601 date-time: the version extant then. \
                            Absent reads the current version.")
        ),
        responses(
            (status = 200, description = "The bare EHR_ACCESS with its `uid`, its \
                                          `settings` the `ACCESS_DIRECTIVE_SETTINGS` of \
                                          the EHR's directives, as canonical JSON. \
                                          `ETag` is the weak `W/\"<version_uid>\"` of \
                                          the version served and `Last-Modified` its \
                                          commit instant.",
             body = serde_json::Value),
            EhrAccessRefusal,
            SpecProfileConflict
        )
    )]
    ehr_access_get_at_time => super::ehr_status::get_at_time::<EhrAccess>
}

guarded_handler! {
    /// Read the `EHR_ACCESS` of an EHR at a version
    /// (`GET /ehr/{ehr_id}/ehr_access/{version_uid}`). **Our own extension**
    /// (module docs).
    #[utoipa::path(
        get, path = "/ehr/{ehr_id}/ehr_access/{version_uid}", tag = "EHR_ACCESS",
        params(
            ("ehr_id" = String, Path, description = "The EHR, by `EHR.ehr_id.value`."),
            ("version_uid" = String, Path,
             description = "The version, its full `object_id::creating_system_id::\
                            version_tree_id`; one naming another system is `404`.")
        ),
        responses(
            (status = 200, description = "The bare EHR_ACCESS of that version, its \
                                          weak `ETag` and its `Last-Modified`.",
             body = serde_json::Value),
            EhrAccessRefusal,
            SpecProfileConflict
        )
    )]
    ehr_access_get_by_version_id => super::ehr_status::get_by_version_id::<EhrAccess>
}

guarded_handler! {
    /// Read the `VERSIONED_EHR_ACCESS` of an EHR
    /// (`GET /ehr/{ehr_id}/versioned_ehr_access`). **Our own extension**
    /// (module docs).
    #[utoipa::path(
        get, path = "/ehr/{ehr_id}/versioned_ehr_access", tag = "EHR_ACCESS",
        params(("ehr_id" = String, Path, description = "The EHR, by `EHR.ehr_id.value`.")),
        responses(
            (status = 200, description = "The VERSIONED_EHR_ACCESS: its uid, `owner_id` \
                                          and `time_created`. `ETag` is the weak \
                                          `W/\"<versioned_object_uid>\"` and \
                                          `Last-Modified` the commit instant of its \
                                          newest version.",
             body = serde_json::Value),
            EhrAccessRefusal,
            SpecProfileConflict
        )
    )]
    versioned_ehr_access_get => super::versioned_ehr_status::get::<EhrAccess>
}

guarded_handler! {
    /// Read the revision history of an EHR's `EHR_ACCESS`
    /// (`GET /ehr/{ehr_id}/versioned_ehr_access/revision_history`). **Our own
    /// extension** (module docs).
    #[utoipa::path(
        get, path = "/ehr/{ehr_id}/versioned_ehr_access/revision_history", tag = "EHR_ACCESS",
        params(("ehr_id" = String, Path, description = "The EHR, by `EHR.ehr_id.value`.")),
        responses(
            (status = 200, description = "The REVISION_HISTORY of the EHR_ACCESS, one \
                                          item per version with its commit audit.",
             body = serde_json::Value),
            EhrAccessRefusal,
            SpecProfileConflict
        )
    )]
    versioned_ehr_access_revision_history =>
        super::versioned_ehr_status::revision_history::<EhrAccess>
}

guarded_handler! {
    /// Read the VERSION of an EHR's `EHR_ACCESS` extant at a point in time
    /// (`GET /ehr/{ehr_id}/versioned_ehr_access/version`). **Our own
    /// extension** (module docs).
    #[utoipa::path(
        get, path = "/ehr/{ehr_id}/versioned_ehr_access/version", tag = "EHR_ACCESS",
        params(
            ("ehr_id" = String, Path, description = "The EHR, by `EHR.ehr_id.value`."),
            ("version_at_time" = Option<String>, Query,
             description = "An extended ISO 8601 date-time: the version extant then. \
                            Absent reads the newest version.")
        ),
        responses(
            (status = 200, description = "The ORIGINAL_VERSION or IMPORTED_VERSION \
                                          wrapping the EHR_ACCESS, with its commit \
                                          audit, its weak `ETag` and its \
                                          `Last-Modified`.",
             body = serde_json::Value),
            EhrAccessRefusal,
            SpecProfileConflict
        )
    )]
    versioned_ehr_access_version_get_at_time =>
        super::versioned_ehr_status::version_get_at_time::<EhrAccess>
}

guarded_handler! {
    /// Read one VERSION of an EHR's `EHR_ACCESS`
    /// (`GET /ehr/{ehr_id}/versioned_ehr_access/version/{version_uid}`).
    /// **Our own extension** (module docs).
    #[utoipa::path(
        get, path = "/ehr/{ehr_id}/versioned_ehr_access/version/{version_uid}",
        tag = "EHR_ACCESS",
        params(
            ("ehr_id" = String, Path, description = "The EHR, by `EHR.ehr_id.value`."),
            ("version_uid" = String, Path,
             description = "The version, its full `object_id::creating_system_id::\
                            version_tree_id`.")
        ),
        responses(
            (status = 200, description = "The ORIGINAL_VERSION or IMPORTED_VERSION of \
                                          that uid, with its weak `ETag` and its \
                                          `Last-Modified`.",
             body = serde_json::Value),
            EhrAccessRefusal,
            SpecProfileConflict
        )
    )]
    versioned_ehr_access_version_get_by_id =>
        super::versioned_ehr_status::version_get_by_id::<EhrAccess>
}
