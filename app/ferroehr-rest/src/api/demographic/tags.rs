// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ITEM_TAG` sub-resources — `operations/person_tags_get.yaml`,
//! `person_tags_update.yaml`, `person_tags_delete.yaml` (and the field-identical
//! `agent_*`/`group_*`/`organisation_*`/`role_*`), plus the kind-agnostic
//! collection filter `demographic_tags_get.yaml`. Tags use the **canonical**
//! content negotiation (`Accept_canonical`/`ContentType_canonical`).
//!
//! **Each kind decodes through its OWN generated params type.** The five
//! families' parameter sets are field-identical in Release-1.1.0, so one type
//! would decode all five today — but that is a property of the current release,
//! not a contract. Routing every kind through `Agent*Params` would mean a
//! future release that adds a parameter to (say) `person_tags_get` alone would
//! be silently mis-decoded on four other families, with no compile error. The
//! per-kind match below makes any such divergence a build failure instead.

use axum::response::Response;

use openehr_its::rest::generated::demographic::{
    AgentTagsDeleteParams, AgentTagsGetParams, AgentTagsUpdateParams, DemographicTagsGetParams,
    GroupTagsDeleteParams, GroupTagsGetParams, GroupTagsUpdateParams, OrganisationTagsDeleteParams,
    OrganisationTagsGetParams, OrganisationTagsUpdateParams, PersonTagsDeleteParams,
    PersonTagsGetParams, PersonTagsUpdateParams, RoleTagsDeleteParams, RoleTagsGetParams,
    RoleTagsUpdateParams,
};

use ferroehr::versioning::object_version_id::parse_uid_based_id;

use crate::api::RequestParts;
use crate::api::item_tags;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;
use ferroehr::service::demographic::types::PartyKind;
use http::StatusCode;

/// The `uid_based_id` of a request, decoded through the routed kind's own
/// generated params struct.
macro_rules! uid_based_id {
    ($parts:ident, $params:ident) => {
        $parts
            .decode($params::from_request, $params::PARAMS)?
            .uid_based_id
    };
}

/// The per-kind `ITEM_TAG` read (`{kind}_tags_get`).
pub(crate) async fn get(
    state: AppState,
    kind: PartyKind,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    // Each kind's own generated params type (see the module doc).
    let uid_based_id = match kind {
        PartyKind::Agent => uid_based_id!(parts, AgentTagsGetParams),
        PartyKind::Group => uid_based_id!(parts, GroupTagsGetParams),
        PartyKind::Organisation => uid_based_id!(parts, OrganisationTagsGetParams),
        PartyKind::Person => uid_based_id!(parts, PersonTagsGetParams),
        PartyKind::Role => uid_based_id!(parts, RoleTagsGetParams),
    };
    let tags = state
        .backend()
        .party_tags_get(kind, parse_uid_based_id(&uid_based_id)?)
        .await?;
    Ok(negotiate::body::respond(
        h,
        StatusCode::OK,
        &openehr_its::json::to_canonical_value(&tags),
    ))
}

/// The per-kind `ITEM_TAG` update (`{kind}_tags_update`).
pub(crate) async fn update(
    state: AppState,
    kind: PartyKind,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let uid_based_id = match kind {
        PartyKind::Agent => uid_based_id!(parts, AgentTagsUpdateParams),
        PartyKind::Group => uid_based_id!(parts, GroupTagsUpdateParams),
        PartyKind::Organisation => uid_based_id!(parts, OrganisationTagsUpdateParams),
        PartyKind::Person => uid_based_id!(parts, PersonTagsUpdateParams),
        PartyKind::Role => uid_based_id!(parts, RoleTagsUpdateParams),
    };
    item_tags::replace(
        &state,
        item_tags::TagTarget::Party(kind),
        uid_based_id,
        &parts,
    )
    .await
}

/// The per-kind `ITEM_TAG` delete (`{kind}_tags_delete`).
pub(crate) async fn delete(
    state: AppState,
    kind: PartyKind,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let (uid_based_id, key) = match kind {
        PartyKind::Agent => {
            let p = parts.decode(
                AgentTagsDeleteParams::from_request,
                AgentTagsDeleteParams::PARAMS,
            )?;
            (p.uid_based_id, p.key)
        }
        PartyKind::Group => {
            let p = parts.decode(
                GroupTagsDeleteParams::from_request,
                GroupTagsDeleteParams::PARAMS,
            )?;
            (p.uid_based_id, p.key)
        }
        PartyKind::Organisation => {
            let p = parts.decode(
                OrganisationTagsDeleteParams::from_request,
                OrganisationTagsDeleteParams::PARAMS,
            )?;
            (p.uid_based_id, p.key)
        }
        PartyKind::Person => {
            let p = parts.decode(
                PersonTagsDeleteParams::from_request,
                PersonTagsDeleteParams::PARAMS,
            )?;
            (p.uid_based_id, p.key)
        }
        PartyKind::Role => {
            let p = parts.decode(
                RoleTagsDeleteParams::from_request,
                RoleTagsDeleteParams::PARAMS,
            )?;
            (p.uid_based_id, p.key)
        }
    };
    state
        .backend()
        .party_tags_delete(kind, parse_uid_based_id(&uid_based_id)?, key)
        .await?;
    Ok(negotiate::body::empty(StatusCode::NO_CONTENT))
}

/// `GET /demographic/tags` — the kind-agnostic `ITEM_TAG` collection filter
/// (`demographic_tags_get.yaml`).
pub(crate) async fn collection(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        DemographicTagsGetParams::from_request,
        DemographicTagsGetParams::PARAMS,
    )?;
    let tags = state
        .backend()
        .demographic_tags_get(
            p.tag_key.as_deref(),
            p.tag_value.as_deref(),
            p.tag_target_path.as_deref(),
        )
        .await?;
    Ok(negotiate::body::respond(
        h,
        StatusCode::OK,
        &openehr_its::json::to_canonical_value(&tags),
    ))
}
