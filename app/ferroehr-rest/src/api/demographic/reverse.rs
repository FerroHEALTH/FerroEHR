// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `PARTY.reverse_relationships()` evaluated — **our own extension**.
//!
//! RM 1.2.0 turns the RM 1.1.0 attribute into a function (SPECRM-124,
//! `RM/docs/UML/classes/org.openehr.rm.demographic.party.adoc`), so a
//! development-profile PARTY carries no such member and no ITS-REST resource
//! serves the function's result. ITS-JSON publishes no RM 1.2.0 schema; the
//! released schemas are generated from the BMM properties with
//! `additionalProperties: false` (`components/RM/Release-1.1.0/Demographic/PERSON.json`),
//! and the 1.2.0 BMM lists `reverse_relationships` among the functions, so a
//! member in the body would be one the development reader, FerroEHR's own
//! included, refuses on the way back. The result is served on its own route
//! instead, beside the openEHR base path, in both profiles: the same list a
//! stable-profile read carries in its body. No openEHR spec governs the route:
//! our own design, excluded from any conformance-profile claim.

use axum::response::Response;
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::api::{RequestParts, guarded_handler};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The `reverse_relationships()` extension route as a native `utoipa-axum`
/// router (see the module docs).
pub(crate) fn reverse_relationship_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(party_reverse_relationships_get))
}

guarded_handler! {
    /// Evaluate `PARTY.reverse_relationships()`
    /// (`GET /demographic/party/{uid_based_id}/reverse_relationships`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (see the
    /// module docs). The answer is the list of `LOCATABLE_REF`s to the
    /// `PARTY_RELATIONSHIP`s other parties hold with this party as target (RM
    /// demographic `master02-demographic_package.adoc` §Party Relationships),
    /// the list a stable-profile read of the same party and version carries
    /// in `reverse_relationships`.
    #[utoipa::path(
        get, path = "/demographic/party/{uid_based_id}/reverse_relationships",
        tag = "demographic-relationship",
        params(
            ("uid_based_id" = String, Path,
             description = "The party: its `HIER_OBJECT_ID` container id, for the \
                            relationships as they stand now, or one version's \
                            `OBJECT_VERSION_ID`, for the relationships as they \
                            stood when that version was committed. Any of the \
                            five party kinds.",
             example = "8849182c-82ad-4088-a07f-48ead4180515"),
            ("version_at_time" = Option<String>, Query,
             description = "An extended ISO 8601 instant: the version of the party \
                            current at that instant is found as the released party \
                            read finds it, and the relationships are answered as \
                            they stood when that version was committed.",
             example = "2015-01-20T19:30:22.765+01:00"),
            ("Accept" = Option<String>, Header,
             description = "`application/json` (default). The list is served in \
                            canonical JSON only.",
             example = "application/json")
        ),
        responses(
            (status = 200, description = "The evaluated function: a JSON array of \
                                          canonical `LOCATABLE_REF`s, each naming \
                                          the source party's container and the \
                                          relationship's path in its data; `[]` \
                                          when no relationship targets the party.",
             body = serde_json::Value),
            (status = 204, description = "The version the request selects is a \
                                          deletion marker, as the released party \
                                          read answers it."),
            (status = 400, description = "The `uid_based_id` is neither an \
                                          OBJECT_VERSION_ID nor a HIER_OBJECT_ID, or \
                                          `version_at_time` is not an extended ISO \
                                          8601 instant.",
             body = serde_json::Value),
            (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                          principal).",
             body = serde_json::Value),
            (status = 404, description = "No party has this id, or it has no version \
                                          at the requested version or instant.",
             body = serde_json::Value),
            (status = 406, description = "The `Accept` header admits no JSON.",
             body = serde_json::Value)
        )
    )]
    party_reverse_relationships_get => get
}

/// The `party_reverse_relationships_get` operation.
pub(crate) async fn get(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let Some(super::party::Addressed {
        vo_id,
        pinned,
        resp,
    }) = super::party::read_addressed(&state, None, &parts).await?
    else {
        return Ok(negotiate::body::empty(StatusCode::NO_CONTENT));
    };
    let at = super::party::pinned_instant(pinned, &resp);
    let reverse = state
        .backend()
        .party_reverse_relationships(vo_id, at)
        .await?;
    let mut out = negotiate::body::respond(h, StatusCode::OK, &reverse.references);
    super::party::audit_served(&mut out, reverse.sources);
    Ok(out)
}
