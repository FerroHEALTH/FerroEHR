// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `VERSIONED_PARTY_RELATIONSHIP` reads — **our own extension**, the
//! versioned half of the [`relationship`](super::relationship) wire.
//!
//! No openEHR ITS-REST operation governs these: the vendored Demographic API
//! (`demographic.openapi.yaml`) defines no `versioned_party_relationship`
//! path. The reads mirror the `VERSIONED_PARTY` reads, reuse the generated
//! party `*Params` structs by analogy, and are **excluded from any
//! conformance-profile claim**.

use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::generated::demographic::{
    VersionedPartyGetParams, VersionedPartyVersionGetAtTimeParams,
    VersionedPartyVersionGetByIdParams,
};

use crate::api::{RequestParts, guarded_handler};
use crate::overview::error::RestError;
use crate::state::AppState;
use ferroehr::versioning::object_version_id::parse_version_uid;

// The handlers' docs, which the served OpenAPI descriptions carry, link the create route.
#[cfg(doc)]
use super::relationship::party_relationship_create;

/// The `VERSIONED_PARTY_RELATIONSHIP` read routes as a native `utoipa-axum`
/// router, one `routes!` per path (see the module docs).
pub(crate) fn versioned_relationship_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(versioned_party_relationship_get))
        .routes(routes!(party_relationship_revision_history))
        .routes(routes!(party_relationship_version_get_at_time))
        .routes(routes!(party_relationship_version_get_by_id))
}

guarded_handler! {
    /// Retrieve the `VERSIONED_PARTY_RELATIONSHIP` container
    /// (`GET /demographic/versioned_party_relationship/{versioned_object_uid}`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (see
    /// [`party_relationship_create`] and the module docs) and is excluded from any
    /// conformance-profile claim.
    #[utoipa::path(
        get, path = "/demographic/versioned_party_relationship/{versioned_object_uid}", tag = "demographic-relationship",
        params(
            ("versioned_object_uid" = String, Path,
             description = "The version-container id of the relationship (a \
                            HIER_OBJECT_ID / `versioned_object_uid`, `Resources.md` \
                            §\"Identifier types\").",
             example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8"),
            ("Accept" = Option<String>, Header,
             description = "This container is served as canonical `application/json`; \
                            an `Accept` that excludes JSON is `406`.",
             example = "application/json")
        ),
        responses(
            (status = 200, description = "The VERSIONED_OBJECT container of the \
                                          relationship, as canonical JSON. No \
                                          `Location`: §Location restricts the header \
                                          to creation/redirect responses \
                                          (`Requests_and_responses.md`).",
             body = serde_json::Value,
             headers(
                 ("ETag" = String,
                  description = "The weak entity tag `W/\"<versioned_object_uid>\"` — \
                                 the `ETag` value \"is usually taken from e.g. \
                                 VERSIONED_OBJECT.uid.value, VERSION.uid.value\" \
                                 (§\"ETag and Last-Modified\"). A container body \
                                 exposes no commit audit, so no `Last-Modified` \
                                 accompanies it.")
             )),
            (status = 400, description = "The `versioned_object_uid` is not a \
                                          well-formed id — the overview table's `400` \
                                          row, \"syntactically invalid content\" \
                                          (`Requests_and_responses.md` §\"HTTP status \
                                          codes\").",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                          principal) — refused before the request \
                                          reaches the resource.",
             body = serde_json::Value),
            (status = 404, description = "No such relationship container — the \
                                          overview table's `404` row.",
             body = serde_json::Value),
            (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                          container is served as canonical JSON only, \
                                          so an `Accept` excluding `application/json` \
                                          is refused (`Resources.md` §\"JSON \
                                          Format\").",
             body = serde_json::Value),
            (status = 415, description = "The request DECLARES a Simplified \
                                          `Content-Type`, refused before the read \
                                          because an untemplated resource has no \
                                          template to expand one against \
                                          (`Resources.md` §\"Simplified Formats\").",
             body = serde_json::Value)
        )
    )]
    versioned_party_relationship_get => get
}

guarded_handler! {
    /// Retrieve the relationship's `REVISION_HISTORY`
    /// (`GET /demographic/versioned_party_relationship/{versioned_object_uid}/revision_history`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (see
    /// [`party_relationship_create`] and the module docs) and is excluded from any
    /// conformance-profile claim.
    #[utoipa::path(
        get, path = "/demographic/versioned_party_relationship/{versioned_object_uid}/revision_history", tag = "demographic-relationship",
        params(
            ("versioned_object_uid" = String, Path,
             description = "The version-container id of the relationship (a \
                            HIER_OBJECT_ID / `versioned_object_uid`).",
             example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8"),
            ("Accept" = Option<String>, Header,
             description = "The history is served as canonical `application/json`; an \
                            `Accept` that excludes JSON is `406`.",
             example = "application/json")
        ),
        responses(
            (status = 200, description = "The relationship's REVISION_HISTORY as \
                                          canonical JSON; `items` runs oldest-first \
                                          (`REVISION_HISTORY.most_recent_version` is \
                                          the last item, RM `common/master04` \
                                          §REVISION_HISTORY).",
             body = serde_json::Value,
             headers(
                 ("ETag" = String,
                  description = "The weak entity tag `W/\"<versioned_object_uid>\"` — \
                                 a REVISION_HISTORY carries no `uid` of its own, so \
                                 the addressed container's id is the `ETag` source \
                                 (§\"ETag and Last-Modified\" names \
                                 VERSIONED_OBJECT.uid.value as one)."),
                 ("Last-Modified" = String,
                  description = "The most recent revision's commit instant as an \
                                 HTTP-date, \"derived from \
                                 VERSION.commit_audit.time_committed.value\" \
                                 (§\"ETag and Last-Modified\").")
             )),
            (status = 400, description = "The `versioned_object_uid` is not a \
                                          well-formed id — the overview table's `400` \
                                          row (`Requests_and_responses.md` §\"HTTP \
                                          status codes\").",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                          principal) — refused before the request \
                                          reaches the resource.",
             body = serde_json::Value),
            (status = 404, description = "No such relationship container — the \
                                          overview table's `404` row.",
             body = serde_json::Value),
            (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                          history is served as canonical JSON only \
                                          (`Resources.md` §\"JSON Format\").",
             body = serde_json::Value),
            (status = 415, description = "The request DECLARES a Simplified \
                                          `Content-Type`, refused before the read \
                                          (`Resources.md` §\"Simplified Formats\").",
             body = serde_json::Value)
        )
    )]
    party_relationship_revision_history => revision_history
}

guarded_handler! {
    /// Retrieve the relationship VERSION at a point in time
    /// (`GET /demographic/versioned_party_relationship/{versioned_object_uid}/version`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (see
    /// [`party_relationship_create`] and the module docs) and is excluded from any
    /// conformance-profile claim.
    #[utoipa::path(
        get, path = "/demographic/versioned_party_relationship/{versioned_object_uid}/version", tag = "demographic-relationship",
        params(
            ("versioned_object_uid" = String, Path,
             description = "The version-container id of the relationship (a \
                            HIER_OBJECT_ID / `versioned_object_uid`).",
             example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8"),
            ("version_at_time" = Option<String>, Query,
             description = "A given time in the extended ISO 8601 format; selects the \
                            VERSION extant at that instant (the latest when omitted). \
                            The timezone is optional — server-local when absent.",
             example = "2015-01-20T19:30:22.765+01:00"),
            ("Accept" = Option<String>, Header,
             description = "The VERSION is served as canonical `application/json`; an \
                            `Accept` that excludes JSON is `406`.",
             example = "application/json")
        ),
        responses(
            (status = 200, description = "The ORIGINAL_VERSION wrapper as canonical \
                                          JSON, `data` carrying the relationship. No \
                                          `Location` — §Location restricts the header \
                                          to creation/redirect responses \
                                          (`Requests_and_responses.md`).",
             body = serde_json::Value,
             headers(
                 ("ETag" = String,
                  description = "The weak entity tag `W/\"<version_uid>\"` of the \
                                 served VERSION (§\"ETag and Last-Modified\")."),
                 ("Last-Modified" = String,
                  description = "The served VERSION's commit instant as an HTTP-date, \
                                 \"derived from \
                                 VERSION.commit_audit.time_committed.value\" \
                                 (§\"ETag and Last-Modified\").")
             )),
            (status = 400, description = "The `versioned_object_uid` is malformed, or \
                                          `version_at_time` is not an extended ISO \
                                          8601 instant — the overview table's `400` \
                                          row (`Requests_and_responses.md` §\"HTTP \
                                          status codes\").",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                          principal) — refused before the request \
                                          reaches the resource.",
             body = serde_json::Value),
            (status = 404, description = "No such relationship container, or no \
                                          version at the requested `version_at_time` \
                                          — the overview table's `404` row.",
             body = serde_json::Value),
            (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                          VERSION is served as canonical JSON only \
                                          (`Resources.md` §\"JSON Format\").",
             body = serde_json::Value),
            (status = 415, description = "The request DECLARES a Simplified \
                                          `Content-Type`, refused before the read \
                                          (`Resources.md` §\"Simplified Formats\").",
             body = serde_json::Value)
        )
    )]
    party_relationship_version_get_at_time => version_get_at_time
}

guarded_handler! {
    /// Retrieve a specific relationship VERSION by version uid
    /// (`GET /demographic/versioned_party_relationship/{versioned_object_uid}/version/{version_uid}`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (see
    /// [`party_relationship_create`] and the module docs) and is excluded from any
    /// conformance-profile claim.
    #[utoipa::path(
        get, path = "/demographic/versioned_party_relationship/{versioned_object_uid}/version/{version_uid}", tag = "demographic-relationship",
        params(
            ("versioned_object_uid" = String, Path,
             description = "The version-container id of the relationship (a \
                            HIER_OBJECT_ID / `versioned_object_uid`).",
             example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8"),
            ("version_uid" = String, Path,
             description = "The VERSION identifier (an OBJECT_VERSION_ID) whose \
                            `object_id` segment is the `versioned_object_uid` above \
                            (`Resources.md` §\"Identifier types\").",
             example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8::openEHRSys.example.com::1"),
            ("Accept" = Option<String>, Header,
             description = "The VERSION is served as canonical `application/json`; an \
                            `Accept` that excludes JSON is `406`.",
             example = "application/json")
        ),
        responses(
            (status = 200, description = "The ORIGINAL_VERSION wrapper as canonical \
                                          JSON, `data` carrying the relationship. No \
                                          `Location` — §Location restricts the header \
                                          to creation/redirect responses \
                                          (`Requests_and_responses.md`).",
             body = serde_json::Value,
             headers(
                 ("ETag" = String,
                  description = "The weak entity tag `W/\"<version_uid>\"` of the \
                                 served VERSION (§\"ETag and Last-Modified\")."),
                 ("Last-Modified" = String,
                  description = "The served VERSION's commit instant as an HTTP-date, \
                                 \"derived from \
                                 VERSION.commit_audit.time_committed.value\" \
                                 (§\"ETag and Last-Modified\").")
             )),
            (status = 400, description = "The `versioned_object_uid` or `version_uid` \
                                          is not well-formed — the overview table's \
                                          `400` row (`Requests_and_responses.md` \
                                          §\"HTTP status codes\").",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                          principal) — refused before the request \
                                          reaches the resource.",
             body = serde_json::Value),
            (status = 404, description = "No such relationship container, or a \
                                          `version_uid` that names no version of it — \
                                          the overview table's `404` row.",
             body = serde_json::Value),
            (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                          VERSION is served as canonical JSON only \
                                          (`Resources.md` §\"JSON Format\").",
             body = serde_json::Value),
            (status = 415, description = "The request DECLARES a Simplified \
                                          `Content-Type`, refused before the read \
                                          (`Resources.md` §\"Simplified Formats\").",
             body = serde_json::Value)
        )
    )]
    party_relationship_version_get_by_id => version_get_by_id
}

/// The `versioned_party_relationship_get` operation.
///
/// The versioned reads carry the weak `ETag` (+ `Last-Modified` where the
/// served body exposes a commit audit) the overview §"`ETag` and
/// Last-Modified" asks of `VERSION` / `VERSIONED_OBJECT` responses, and no
/// `Location` (§Location — creation/redirect only).
pub(crate) async fn get(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedPartyGetParams::from_request,
        VersionedPartyGetParams::PARAMS,
    )?;
    let vo = p.versioned_object_uid.clone();
    let resp = state
        .backend()
        .versioned_party_relationship_get(super::object_of(&vo)?)
        .await?;
    Ok(super::read_versioned(h, &vo, &resp.body))
}

/// The `party_relationship_revision_history` operation.
pub(crate) async fn revision_history(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedPartyGetParams::from_request,
        VersionedPartyGetParams::PARAMS,
    )?;
    let vo = p.versioned_object_uid.clone();
    let resp = state
        .backend()
        .party_relationship_revision_history(super::object_of(&vo)?)
        .await?;
    Ok(super::read_versioned(h, &vo, &resp.body))
}

/// The `party_relationship_version_get_at_time` operation.
pub(crate) async fn version_get_at_time(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedPartyVersionGetAtTimeParams::from_request,
        VersionedPartyVersionGetAtTimeParams::PARAMS,
    )?;
    let resp = state
        .backend()
        .party_relationship_version_get_at_time(
            super::object_of(&p.versioned_object_uid)?,
            p.version_at_time,
        )
        .await?;
    Ok(super::respond_versioned(h, &resp))
}

/// The `party_relationship_version_get_by_id` operation.
pub(crate) async fn version_get_by_id(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedPartyVersionGetByIdParams::from_request,
        VersionedPartyVersionGetByIdParams::PARAMS,
    )?;
    let vo = p.versioned_object_uid.clone();
    let resp = state
        .backend()
        .party_relationship_version_get_by_id(
            super::object_of(&vo)?,
            parse_version_uid(&p.version_uid)?.1,
        )
        .await?;
    Ok(super::read_versioned(h, &vo, &resp.body))
}
